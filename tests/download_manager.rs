use ludomere::{
    domain::{ArtifactKind, DownloadCategory, RemoteArtifact},
    download::{self, DownloadEvent, DownloadRequest},
    state::{DownloadState, StateStore},
};
use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    path::PathBuf,
    sync::{Arc, Condvar, Mutex, mpsc},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[test]
fn manager_downloads_one_game_at_a_time_and_parallelizes_its_parts() {
    let root = temporary_root();
    let data_home = root.join("state");
    // This integration-test executable contains one test. Set the isolated XDG path before the
    // manager or any other application thread is created, and never mutate it afterward.
    unsafe { std::env::set_var("XDG_DATA_HOME", &data_home) };

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (accepted_sender, accepted_receiver) = mpsc::channel();
    let release = Arc::new((Mutex::new([false; 3]), Condvar::new()));
    let server_release = release.clone();
    let server = thread::spawn(move || {
        let mut handlers = Vec::new();
        for _ in 0..3 {
            let (mut stream, _) = listener.accept().unwrap();
            let accepted_sender = accepted_sender.clone();
            let release = server_release.clone();
            handlers.push(thread::spawn(move || {
                let mut request = Vec::new();
                let mut buffer = [0_u8; 1024];
                while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                    let count = stream.read(&mut buffer).unwrap();
                    if count == 0 {
                        break;
                    }
                    request.extend_from_slice(&buffer[..count]);
                }
                let request = String::from_utf8_lossy(&request);
                let path = request
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .unwrap();
                let id = path.trim_start_matches('/').parse::<usize>().unwrap();
                accepted_sender.send(id).unwrap();

                let (released, changed) = &*release;
                let mut released = released.lock().unwrap();
                while !released[id] {
                    released = changed.wait(released).unwrap();
                }
                drop(released);

                let body = format!("download-{id}").into_bytes();
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Disposition: attachment; filename=\"setup_{id}.bin\"\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .unwrap();
                stream.write_all(&body).unwrap();
            }));
        }
        for handler in handlers {
            handler.join().unwrap();
        }
    });

    download::set_concurrency(2);
    download::set_network_available(true);
    download::set_authenticated(true);
    let mut first_part = artifact(0, format!("http://{address}/0"));
    first_part.part_count = Some(2);
    let mut second_part = artifact(1, format!("http://{address}/1"));
    second_part.provider_file_id = Some("part-2".into());
    second_part.part_number = Some(2);
    second_part.part_count = Some(2);
    let (first_events, first_receiver) = mpsc::channel();
    download::enqueue(DownloadRequest {
        artifacts: vec![first_part, second_part],
        title: "Multipart game".into(),
        access_token: "integration-test-token".into(),
        destination: root.join("downloads/game-0/installer/windows/english"),
        events: first_events,
    });
    let (second_events, second_receiver) = mpsc::channel();
    download::enqueue(DownloadRequest {
        artifacts: vec![artifact(2, format!("http://{address}/2"))],
        title: "Next game".into(),
        access_token: "integration-test-token".into(),
        destination: root.join("downloads/game-2/installer/windows/english"),
        events: second_events,
    });

    let first = accepted_receiver
        .recv_timeout(Duration::from_secs(3))
        .unwrap();
    let second = accepted_receiver
        .recv_timeout(Duration::from_secs(3))
        .unwrap();
    assert_ne!(first, second);
    assert!(
        accepted_receiver
            .recv_timeout(Duration::from_millis(300))
            .is_err(),
        "the next game started before the multipart game completed"
    );

    release_job(&release, first);
    assert!(
        accepted_receiver
            .recv_timeout(Duration::from_millis(300))
            .is_err(),
        "the next game started while one part of the current game was still active"
    );
    release_job(&release, second);
    let third = accepted_receiver
        .recv_timeout(Duration::from_secs(3))
        .unwrap();
    assert!(![first, second].contains(&third));
    release_job(&release, third);

    wait_for_completion(first_receiver);
    wait_for_completion(second_receiver);
    exercise_queued_pause_and_resume(&root);
    exercise_active_pause_and_resume(&root);
    exercise_active_removal(&root);
    exercise_transient_retry(&root);
    exercise_connectivity_change_resets_backoff(&root);
    exercise_manual_retry_resets_backoff(&root);
    download::shutdown();
    server.join().unwrap();

    for id in 0..3 {
        let game = if id < 2 { 0 } else { 2 };
        assert_eq!(
            fs::read(root.join(format!(
                "downloads/game-{game}/installer/windows/english/setup_{id}.bin"
            )))
            .unwrap(),
            format!("download-{id}").as_bytes()
        );
    }
    fs::remove_dir_all(root).unwrap();
}

fn exercise_connectivity_change_resets_backoff(root: &std::path::Path) {
    let (address, failed, retried, server) = retry_reset_server(8);
    let artifact = artifact(8, format!("http://{address}/8"));
    let id = download::job_id(&[&artifact]);
    let (events, receiver) = mpsc::channel();
    download::enqueue(DownloadRequest {
        artifacts: vec![artifact],
        title: "Connectivity retry reset".into(),
        access_token: "integration-test-token".into(),
        destination: root.join("downloads/game-8/installer/windows/english"),
        events,
    });
    failed.recv_timeout(Duration::from_secs(3)).unwrap();
    wait_for_inactive(&id);

    let reset_at = std::time::Instant::now();
    download::set_network_available(false);
    download::set_network_available(true);
    let retried_at = retried.recv_timeout(Duration::from_secs(3)).unwrap();
    assert!(
        retried_at.duration_since(reset_at) < Duration::from_secs(1),
        "connectivity change did not clear retry backoff"
    );
    wait_for_completion(receiver);
    server.join().unwrap();
}

fn exercise_manual_retry_resets_backoff(root: &std::path::Path) {
    let (address, failed, retried, server) = retry_reset_server(9);
    let artifact = artifact(9, format!("http://{address}/9"));
    let id = download::job_id(&[&artifact]);
    let (events, receiver) = mpsc::channel();
    download::enqueue(DownloadRequest {
        artifacts: vec![artifact],
        title: "Manual retry reset".into(),
        access_token: "integration-test-token".into(),
        destination: root.join("downloads/game-9/installer/windows/english"),
        events,
    });
    failed.recv_timeout(Duration::from_secs(3)).unwrap();
    wait_for_inactive(&id);

    let reset_at = std::time::Instant::now();
    assert!(download::retry(
        &id,
        "replacement-integration-test-token".into()
    ));
    let retried_at = retried.recv_timeout(Duration::from_secs(3)).unwrap();
    assert!(
        retried_at.duration_since(reset_at) < Duration::from_secs(1),
        "manual Retry did not clear retry backoff"
    );
    wait_for_completion(receiver);
    server.join().unwrap();
}

fn retry_reset_server(
    id: usize,
) -> (
    std::net::SocketAddr,
    mpsc::Receiver<()>,
    mpsc::Receiver<std::time::Instant>,
    thread::JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (failed_sender, failed_receiver) = mpsc::channel();
    let (retried_sender, retried_receiver) = mpsc::channel();
    let server = thread::spawn(move || {
        let (mut failed, _) = listener.accept().unwrap();
        assert_eq!(read_request_id(&mut failed), id);
        failed
            .write_all(
                b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            )
            .unwrap();
        failed.flush().unwrap();
        failed_sender.send(()).unwrap();
        drop(failed);

        let (mut retry, _) = listener.accept().unwrap();
        retried_sender.send(std::time::Instant::now()).unwrap();
        assert_eq!(read_request_id(&mut retry), id);
        send_download(&mut retry, id);
    });
    (address, failed_receiver, retried_receiver, server)
}

fn wait_for_inactive(job_id: &str) {
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    while download::is_active(job_id) {
        assert!(
            std::time::Instant::now() < deadline,
            "job {job_id} remained active after its transient failure"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

fn exercise_transient_retry(root: &std::path::Path) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (delay_sender, delay_receiver) = mpsc::channel();
    let server = thread::spawn(move || {
        let (mut failed, _) = listener.accept().unwrap();
        let _ = read_range_header(&mut failed);
        failed
            .write_all(
                b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            )
            .unwrap();
        let failed_at = std::time::Instant::now();

        let (mut retry, _) = listener.accept().unwrap();
        delay_sender.send(failed_at.elapsed()).unwrap();
        assert_eq!(read_request_id(&mut retry), 7);
        send_download(&mut retry, 7);
    });

    let artifact = artifact(7, format!("http://{address}/7"));
    let (events, receiver) = mpsc::channel();
    download::enqueue(DownloadRequest {
        artifacts: vec![artifact],
        title: "Transient retry test".into(),
        access_token: "integration-test-token".into(),
        destination: root.join("downloads/game-7/installer/windows/english"),
        events,
    });

    let retry_delay = delay_receiver.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(
        retry_delay >= Duration::from_millis(1_800),
        "transient failure retried without the configured backoff: {retry_delay:?}"
    );
    assert!(
        retry_delay < Duration::from_secs(4),
        "transient failure was not retried promptly: {retry_delay:?}"
    );
    wait_for_completion(receiver);
    server.join().unwrap();
}

fn exercise_active_pause_and_resume(root: &std::path::Path) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let body = (0..600_000)
        .map(|index| (index % 251) as u8)
        .collect::<Vec<_>>();
    let server_body = body.clone();
    let (first_chunk_sender, first_chunk_receiver) = mpsc::channel();
    let (release_sender, release_receiver) = mpsc::channel();
    let (range_sender, range_receiver) = mpsc::channel();
    let server = thread::spawn(move || {
        let (mut first, _) = listener.accept().unwrap();
        assert_eq!(read_range_header(&mut first), None);
        write!(
            first,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Disposition: attachment; filename=\"setup_5.bin\"\r\nConnection: close\r\n\r\n",
            server_body.len()
        )
        .unwrap();
        first.write_all(&server_body[..300_000]).unwrap();
        first.flush().unwrap();
        first_chunk_sender.send(()).unwrap();
        release_receiver.recv().unwrap();
        first.write_all(&server_body[300_000..300_001]).unwrap();
        first.flush().unwrap();
        drop(first);

        let (mut resumed, _) = listener.accept().unwrap();
        let range = read_range_header(&mut resumed);
        range_sender.send(range.clone()).unwrap();
        let offset = range
            .as_deref()
            .and_then(|value| value.strip_prefix("bytes="))
            .and_then(|value| value.strip_suffix('-'))
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap();
        let remaining = &server_body[offset..];
        write!(
            resumed,
            "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Disposition: attachment; filename=\"setup_5.bin\"\r\nConnection: close\r\n\r\n",
            remaining.len()
        )
        .unwrap();
        resumed.write_all(remaining).unwrap();
    });

    download::set_concurrency(1);
    let artifact = artifact(5, format!("http://{address}/5"));
    let id = download::job_id(&[&artifact]);
    let (events, receiver) = mpsc::channel();
    download::enqueue(DownloadRequest {
        artifacts: vec![artifact],
        title: "Active pause test".into(),
        access_token: "integration-test-token".into(),
        destination: root.join("downloads/game-5/installer/windows/english"),
        events,
    });
    first_chunk_receiver
        .recv_timeout(Duration::from_secs(3))
        .unwrap();
    wait_for_staging_bytes(&root.join("downloads/.ludomere-staging").join(&id));
    assert!(download::cancel(&id));
    release_sender.send(()).unwrap();
    wait_for_cancelled(&receiver);
    wait_for_job_state(&id, DownloadState::Paused);

    assert!(download::resume(&id, "integration-test-token".into()));
    let range = range_receiver.recv_timeout(Duration::from_secs(3)).unwrap();
    assert!(
        range
            .as_deref()
            .is_some_and(|value| value.starts_with("bytes=") && value != "bytes=0-"),
        "resumed active download did not request its staged byte range"
    );
    server.join().unwrap();
    wait_for_job_state(&id, DownloadState::Complete);
    assert_eq!(
        fs::read(root.join("downloads/game-5/installer/windows/english/setup_5.bin")).unwrap(),
        body
    );
}

fn exercise_active_removal(root: &std::path::Path) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let body = (0..600_000)
        .map(|index| (index % 239) as u8)
        .collect::<Vec<_>>();
    let (first_chunk_sender, first_chunk_receiver) = mpsc::channel();
    let (release_sender, release_receiver) = mpsc::channel();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let _ = read_range_header(&mut stream);
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Disposition: attachment; filename=\"setup_6.bin\"\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .unwrap();
        stream.write_all(&body[..300_000]).unwrap();
        stream.flush().unwrap();
        first_chunk_sender.send(()).unwrap();
        release_receiver.recv().unwrap();
        let _ = stream.write_all(&body[300_000..300_001]);
    });

    let artifact = artifact(6, format!("http://{address}/6"));
    let id = download::job_id(&[&artifact]);
    let destination = root.join("downloads/game-6/installer/windows/english");
    let staging = root.join("downloads/.ludomere-staging").join(&id);
    let (events, receiver) = mpsc::channel();
    download::enqueue(DownloadRequest {
        artifacts: vec![artifact],
        title: "Active removal test".into(),
        access_token: "integration-test-token".into(),
        destination,
        events,
    });
    first_chunk_receiver
        .recv_timeout(Duration::from_secs(3))
        .unwrap();
    wait_for_staging_bytes(&staging);
    assert!(download::remove(&id));
    release_sender.send(()).unwrap();
    wait_for_cancelled(&receiver);
    server.join().unwrap();
    wait_for_job_removal(&id);
    assert!(!staging.exists(), "removed job retained its staging data");
}

fn read_range_header(stream: &mut std::net::TcpStream) -> Option<String> {
    let mut request = Vec::new();
    let mut buffer = [0_u8; 1024];
    while !request.windows(4).any(|window| window == b"\r\n\r\n") {
        let count = stream.read(&mut buffer).unwrap();
        if count == 0 {
            break;
        }
        request.extend_from_slice(&buffer[..count]);
    }
    String::from_utf8_lossy(&request).lines().find_map(|line| {
        line.strip_prefix("Range: ")
            .or_else(|| line.strip_prefix("range: "))
            .map(str::to_owned)
    })
}

fn wait_for_job_removal(job_id: &str) {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        if StateStore::open()
            .unwrap()
            .download_job(job_id)
            .unwrap()
            .is_none()
        {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "job {job_id} was not removed"
        );
        thread::sleep(Duration::from_millis(20));
    }
}

fn wait_for_cancelled(receiver: &mpsc::Receiver<DownloadEvent>) {
    loop {
        match receiver.recv_timeout(Duration::from_secs(3)).unwrap() {
            DownloadEvent::Cancelled => return,
            DownloadEvent::Failed(error) => panic!("download failed instead of pausing: {error}"),
            _ => {}
        }
    }
}

fn wait_for_staging_bytes(staging: &std::path::Path) {
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    loop {
        let bytes = fs::read_dir(staging)
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .filter_map(|entry| entry.metadata().ok())
            .map(|metadata| metadata.len())
            .sum::<u64>();
        if bytes > 0 {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "active transfer did not write staging data"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

fn exercise_queued_pause_and_resume(root: &std::path::Path) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (accepted_sender, accepted_receiver) = mpsc::channel();
    let (release_sender, release_receiver) = mpsc::channel();
    let server = thread::spawn(move || {
        for sequence in 0..2 {
            let (mut stream, _) = listener.accept().unwrap();
            let id = read_request_id(&mut stream);
            accepted_sender.send(id).unwrap();
            if sequence == 0 {
                release_receiver.recv().unwrap();
            }
            send_download(&mut stream, id);
        }
    });

    download::set_concurrency(1);
    let first_artifact = artifact(3, format!("http://{address}/3"));
    let second_artifact = artifact(4, format!("http://{address}/4"));
    let second_id = download::job_id(&[&second_artifact]);
    let (first_events, first_receiver) = mpsc::channel();
    let (second_events, second_receiver) = mpsc::channel();
    download::enqueue(DownloadRequest {
        artifacts: vec![first_artifact],
        title: "First queued test".into(),
        access_token: "integration-test-token".into(),
        destination: root.join("downloads/game-3/installer/windows/english"),
        events: first_events,
    });
    download::enqueue(DownloadRequest {
        artifacts: vec![second_artifact],
        title: "Paused queued test".into(),
        access_token: "integration-test-token".into(),
        destination: root.join("downloads/game-4/installer/windows/english"),
        events: second_events,
    });

    assert_eq!(
        accepted_receiver
            .recv_timeout(Duration::from_secs(3))
            .unwrap(),
        3
    );
    assert!(download::cancel(&second_id));
    assert!(matches!(
        second_receiver
            .recv_timeout(Duration::from_secs(3))
            .unwrap(),
        DownloadEvent::Cancelled
    ));
    release_sender.send(()).unwrap();
    wait_for_completion(first_receiver);
    assert!(
        accepted_receiver
            .recv_timeout(Duration::from_millis(300))
            .is_err(),
        "a paused queued download unexpectedly started"
    );

    assert!(download::resume(
        &second_id,
        "integration-test-token".into()
    ));
    assert_eq!(
        accepted_receiver
            .recv_timeout(Duration::from_secs(3))
            .unwrap(),
        4
    );
    server.join().unwrap();
    wait_for_job_state(&second_id, DownloadState::Complete);
}

fn wait_for_job_state(job_id: &str, expected: DownloadState) {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        if StateStore::open()
            .unwrap()
            .download_job(job_id)
            .unwrap()
            .is_some_and(|job| job.state == expected)
        {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "job {job_id} did not reach {expected:?}"
        );
        thread::sleep(Duration::from_millis(20));
    }
}

fn read_request_id(stream: &mut std::net::TcpStream) -> usize {
    let mut request = Vec::new();
    let mut buffer = [0_u8; 1024];
    while !request.windows(4).any(|window| window == b"\r\n\r\n") {
        let count = stream.read(&mut buffer).unwrap();
        if count == 0 {
            break;
        }
        request.extend_from_slice(&buffer[..count]);
    }
    let request = String::from_utf8_lossy(&request);
    request
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap()
        .trim_start_matches('/')
        .parse::<usize>()
        .unwrap()
}

fn send_download(stream: &mut std::net::TcpStream, id: usize) {
    let body = format!("download-{id}").into_bytes();
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Disposition: attachment; filename=\"setup_{id}.bin\"\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .unwrap();
    stream.write_all(&body).unwrap();
}

fn release_job(release: &Arc<(Mutex<[bool; 3]>, Condvar)>, id: usize) {
    let (released, changed) = &**release;
    released.lock().unwrap()[id] = true;
    changed.notify_all();
}

fn wait_for_completion(receiver: mpsc::Receiver<DownloadEvent>) {
    loop {
        match receiver.recv_timeout(Duration::from_secs(5)).unwrap() {
            DownloadEvent::Complete { .. } => return,
            DownloadEvent::Failed(error) => panic!("download failed: {error}"),
            _ => {}
        }
    }
}

fn artifact(id: usize, url: String) -> RemoteArtifact {
    RemoteArtifact {
        product_id: 10_000 + id as i64,
        kind: ArtifactKind::Installer,
        name: format!("Test installer {id}"),
        language: Some("English".into()),
        operating_system: Some("windows".into()),
        version: Some("1.0".into()),
        release_date: None,
        size_label: None,
        size_bytes: Some(format!("download-{id}").len() as u64),
        part_number: Some(1),
        part_count: Some(1),
        download_path: url,
        provider_group_id: Some(format!("installer_windows_en_{id}")),
        provider_file_id: Some(format!("en1installer{id}")),
        provider_category: Some(DownloadCategory::Installer),
    }
}

fn temporary_root() -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "ludomere-manager-test-{}-{unique}",
        std::process::id()
    ))
}
