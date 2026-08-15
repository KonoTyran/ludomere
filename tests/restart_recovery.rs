use ludomere::{
    domain::{ArtifactKind, DownloadCategory, RemoteArtifact},
    download::{self, DownloadRequest},
    state::{DownloadState, StateStore},
};
use std::{
    fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::Command,
    sync::mpsc,
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[test]
fn recovers_a_partial_download_in_a_fresh_process() {
    let root = temporary_root();
    fs::create_dir_all(&root).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let body = (0..600_000)
        .map(|index| (index % 241) as u8)
        .collect::<Vec<_>>();
    let server_body = body.clone();
    let release_marker = root.join("release-first-response");
    let server_marker = release_marker.clone();
    let (ranges_sender, ranges_receiver) = mpsc::channel();
    let server = thread::spawn(move || {
        let (mut first, _) = listener.accept().unwrap();
        ranges_sender.send(read_range(&mut first)).unwrap();
        write!(
            first,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Disposition: attachment; filename=\"setup_restart.bin\"\r\nConnection: close\r\n\r\n",
            server_body.len()
        )
        .unwrap();
        first.write_all(&server_body[..300_000]).unwrap();
        first.flush().unwrap();
        wait_for_path(&server_marker);
        first.write_all(&server_body[300_000..300_001]).unwrap();
        first.flush().unwrap();
        drop(first);

        let (mut resumed, _) = listener.accept().unwrap();
        let range = read_range(&mut resumed);
        ranges_sender.send(range.clone()).unwrap();
        let offset = range
            .as_deref()
            .and_then(|value| value.strip_prefix("bytes="))
            .and_then(|value| value.strip_suffix('-'))
            .and_then(|value| value.parse::<usize>().ok())
            .expect("recovered download did not send a byte range");
        let remaining = &server_body[offset..];
        write!(
            resumed,
            "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Disposition: attachment; filename=\"setup_restart.bin\"\r\nConnection: close\r\n\r\n",
            remaining.len()
        )
        .unwrap();
        resumed.write_all(remaining).unwrap();
    });

    run_helper("interrupt", &root, &format!("http://{address}/installer"));
    run_helper("recover", &root, &format!("http://{address}/installer"));
    server.join().unwrap();

    assert_eq!(ranges_receiver.recv().unwrap(), None);
    let resumed_range = ranges_receiver.recv().unwrap();
    assert!(
        resumed_range
            .as_deref()
            .is_some_and(|value| value.starts_with("bytes=") && value != "bytes=0-")
    );
    assert_eq!(
        fs::read(root.join("downloads/game/installer/windows/english/setup_restart.bin")).unwrap(),
        body
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
#[ignore = "launched as a subprocess by the restart-recovery integration test"]
fn restart_helper_process() {
    let phase = std::env::var("LUDOMERE_TEST_PHASE").unwrap();
    let root = PathBuf::from(std::env::var_os("LUDOMERE_TEST_ROOT").unwrap());
    let url = std::env::var("LUDOMERE_TEST_URL").unwrap();
    let artifact = artifact(url);
    let id = download::job_id(&[&artifact]);
    let destination = root.join("downloads/game/installer/windows/english");

    match phase.as_str() {
        "interrupt" => {
            download::set_concurrency(1);
            download::set_network_available(true);
            download::set_authenticated(true);
            let (events, _receiver) = mpsc::channel();
            download::enqueue(DownloadRequest {
                artifacts: vec![artifact],
                title: "Restart recovery test".into(),
                access_token: "integration-test-token".into(),
                destination,
                events,
            });
            let staging = root.join("downloads/.ludomere-staging").join(&id);
            wait_for_staging_bytes(&staging);
            fs::write(root.join("release-first-response"), b"release").unwrap();
            download::shutdown();
            assert_eq!(job_state(&id), Some(DownloadState::Queued));
        }
        "recover" => {
            download::set_concurrency(1);
            download::set_network_available(true);
            download::set_authenticated(true);
            download::recover("integration-test-token".into());
            wait_for_job_state(&id, DownloadState::Complete);
            download::shutdown();
        }
        other => panic!("unknown restart helper phase {other}"),
    }
}

fn run_helper(phase: &str, root: &Path, url: &str) {
    let status = Command::new(std::env::current_exe().unwrap())
        .args([
            "--ignored",
            "--exact",
            "restart_helper_process",
            "--nocapture",
        ])
        .env("XDG_DATA_HOME", root.join("state"))
        .env("LUDOMERE_TEST_PHASE", phase)
        .env("LUDOMERE_TEST_ROOT", root)
        .env("LUDOMERE_TEST_URL", url)
        .status()
        .unwrap();
    assert!(status.success(), "restart helper phase {phase} failed");
}

fn read_range(stream: &mut TcpStream) -> Option<String> {
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

fn artifact(url: String) -> RemoteArtifact {
    RemoteArtifact {
        product_id: 77_777,
        kind: ArtifactKind::Installer,
        name: "Restart installer".into(),
        language: Some("English".into()),
        operating_system: Some("windows".into()),
        version: Some("1.0".into()),
        release_date: None,
        size_label: None,
        size_bytes: Some(600_000),
        part_number: Some(1),
        part_count: Some(1),
        download_path: url,
        provider_group_id: Some("installer_windows_en".into()),
        provider_file_id: Some("en1installer0".into()),
        provider_category: Some(DownloadCategory::Installer),
    }
}

fn job_state(job_id: &str) -> Option<DownloadState> {
    StateStore::open()
        .unwrap()
        .download_job(job_id)
        .unwrap()
        .map(|job| job.state)
}

fn wait_for_job_state(job_id: &str, expected: DownloadState) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if job_state(job_id) == Some(expected) {
            return;
        }
        assert!(Instant::now() < deadline, "job did not reach {expected:?}");
        thread::sleep(Duration::from_millis(20));
    }
}

fn wait_for_staging_bytes(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let bytes = fs::read_dir(path)
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .filter_map(|entry| entry.metadata().ok())
            .map(|metadata| metadata.len())
            .sum::<u64>();
        if bytes > 0 {
            return;
        }
        assert!(Instant::now() < deadline, "no staged bytes were written");
        thread::sleep(Duration::from_millis(10));
    }
}

fn wait_for_path(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !path.exists() {
        assert!(Instant::now() < deadline, "release marker was not created");
        thread::sleep(Duration::from_millis(10));
    }
}

fn temporary_root() -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "ludomere-restart-test-{}-{unique}",
        std::process::id()
    ))
}
