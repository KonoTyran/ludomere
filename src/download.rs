use crate::domain::RemoteArtifact;
use std::{
    path::PathBuf,
    sync::{Arc, atomic::AtomicBool, mpsc},
};

pub mod depot;
mod files;
mod layout;
mod manager;
mod protocol;
mod transfer;
mod verify;
mod worker;

pub use files::{delete_completed_files, prune_empty_directories};
pub use layout::destination;
use layout::{key, staging_directory};
pub use verify::{GogChecksum, file_md5_with_progress, gog_checksum};
use worker::{cancel_worker, start_worker, worker_is_active};

#[derive(Debug, Clone)]
pub enum DownloadEvent {
    Progress { downloaded: u64, total: Option<u64> },
    Finalizing,
    Complete { files: Vec<PathBuf> },
    Cancelled,
    Failed(DownloadFailure),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownloadFailureKind {
    Authentication,
    TransientNetwork,
    ManifestChanged,
    DiskFull,
    PermissionDenied,
    Other,
}

#[derive(Debug, Clone)]
pub struct DownloadFailure {
    pub kind: DownloadFailureKind,
    pub message: String,
}

impl std::fmt::Display for DownloadFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

pub struct DownloadRequest {
    pub artifacts: Vec<RemoteArtifact>,
    pub title: String,
    pub access_token: String,
    pub destination: PathBuf,
    pub events: mpsc::Sender<DownloadEvent>,
}

#[derive(Debug, Clone)]
pub enum DownloadManagerEvent {
    QueueSnapshot(Vec<crate::state::DownloadJobRecord>),
    Progress {
        job_id: String,
        downloaded: u64,
        total: Option<u64>,
    },
    AuthenticationRequired,
    ManagedFilesChanged(i64),
}

pub fn job_id(artifacts: &[&RemoteArtifact]) -> String {
    let first = artifacts[0];
    if let (Some(group_id), Some(category)) = (&first.provider_group_id, first.provider_category) {
        let revision = artifacts
            .iter()
            .map(|artifact| {
                format!(
                    "{}:{}:{}",
                    artifact.provider_file_id.as_deref().unwrap_or_default(),
                    artifact.size_bytes.unwrap_or_default(),
                    artifact.download_path
                )
            })
            .collect::<Vec<_>>()
            .join("|");
        return format!(
            "{}-{}-{}-{}",
            first.product_id,
            category.as_str(),
            key(group_id),
            format!("{:x}", md5::compute(revision))
                .chars()
                .take(12)
                .collect::<String>()
        );
    }
    let name = first.name.split(" (Part ").next().unwrap_or(&first.name);
    format!(
        "{}-{}-{}-{}-{}",
        first.product_id,
        first.kind.as_str(),
        key(first.operating_system.as_deref().unwrap_or("any")),
        key(first.language.as_deref().unwrap_or("neutral")),
        key(&format!(
            "{}-{}",
            first.version.as_deref().unwrap_or("current"),
            name
        ))
    )
}

pub fn enqueue(request: DownloadRequest) -> Arc<AtomicBool> {
    manager::enqueue(
        request.artifacts,
        request.title,
        request.access_token,
        request.destination,
        request.events,
    )
}

pub fn manager_events() -> mpsc::Receiver<DownloadManagerEvent> {
    manager::subscribe()
}

pub fn resume(job_id: &str, access_token: String) -> bool {
    manager::resume(job_id, access_token, false)
}

pub fn retry(job_id: &str, access_token: String) -> bool {
    manager::resume(job_id, access_token, true)
}

pub fn remove(job_id: &str) -> bool {
    manager::remove(job_id)
}

pub fn cancel(job_id: &str) -> bool {
    manager::pause(job_id)
}

pub fn is_active(job_id: &str) -> bool {
    manager::is_active(job_id)
}

pub fn set_concurrency(limit: usize) {
    manager::set_concurrency(limit);
}

pub fn recover(access_token: String) {
    manager::recover(access_token);
}

pub fn set_network_available(available: bool) {
    manager::set_network(available);
}

pub fn set_authenticated(authenticated: bool) {
    manager::set_authenticated(authenticated);
}

pub fn shutdown() {
    manager::shutdown();
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::{protocol::download_url, transfer::run_transfer};
    use anyhow::Result;
    use std::{
        fs,
        io::{Read, Write},
        net::TcpListener,
        path::Path,
        sync::{atomic::AtomicBool, mpsc},
        thread,
        time::{Duration, SystemTime, UNIX_EPOCH},
    };

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new(name: &str) -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir()
                .join(format!("ludomere-{name}-{}-{unique}", std::process::id()));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn local_response_server(
        body: Vec<u8>,
        status: u16,
        honor_range: bool,
    ) -> (
        String,
        mpsc::Receiver<Option<String>>,
        thread::JoinHandle<()>,
    ) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (request_sender, request_receiver) = mpsc::channel();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
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
            let range = request.lines().find_map(|line| {
                line.strip_prefix("Range: ")
                    .or_else(|| line.strip_prefix("range: "))
                    .map(str::to_owned)
            });
            request_sender.send(range.clone()).unwrap();
            let offset = range
                .as_deref()
                .and_then(|range| range.strip_prefix("bytes="))
                .and_then(|range| range.strip_suffix('-'))
                .and_then(|offset| offset.parse::<usize>().ok())
                .filter(|_| honor_range && status == 200)
                .unwrap_or(0);
            let response_status = if offset > 0 { 206 } else { status };
            let response_body = body.get(offset..).unwrap_or_default();
            write!(
                stream,
                "HTTP/1.1 {response_status} Test\r\nContent-Length: {}\r\nContent-Disposition: attachment; filename=\"setup_test.bin\"\r\nConnection: close\r\n\r\n",
                response_body.len()
            )
            .unwrap();
            stream.write_all(response_body).unwrap();
        });
        (
            format!("http://{address}/download"),
            request_receiver,
            handle,
        )
    }

    fn test_artifact(url: String, size: usize) -> RemoteArtifact {
        RemoteArtifact {
            product_id: 42,
            kind: crate::domain::ArtifactKind::Installer,
            name: "Test installer".into(),
            language: Some("English".into()),
            operating_system: Some("windows".into()),
            version: Some("1.0".into()),
            release_date: None,
            size_label: None,
            size_bytes: Some(size as u64),
            part_number: Some(1),
            part_count: Some(1),
            download_path: url,
            provider_group_id: Some("installer_windows_en".into()),
            provider_file_id: Some("en1installer0".into()),
            provider_category: Some(crate::domain::DownloadCategory::Installer),
        }
    }

    struct ScriptedReply {
        body: Vec<u8>,
        filename: &'static str,
        honor_range: bool,
        truncate_after: Option<usize>,
    }

    fn scripted_response_server(
        replies: Vec<ScriptedReply>,
    ) -> (
        String,
        mpsc::Receiver<Option<String>>,
        thread::JoinHandle<()>,
    ) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (request_sender, request_receiver) = mpsc::channel();
        let handle = thread::spawn(move || {
            for reply in replies {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
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
                let range = request.lines().find_map(|line| {
                    line.strip_prefix("Range: ")
                        .or_else(|| line.strip_prefix("range: "))
                        .map(str::to_owned)
                });
                request_sender.send(range.clone()).unwrap();
                let offset = range
                    .as_deref()
                    .and_then(|range| range.strip_prefix("bytes="))
                    .and_then(|range| range.strip_suffix('-'))
                    .and_then(|offset| offset.parse::<usize>().ok())
                    .filter(|_| reply.honor_range)
                    .unwrap_or(0);
                let status = if offset > 0 { 206 } else { 200 };
                let response_body = reply.body.get(offset..).unwrap_or_default();
                write!(
                    stream,
                    "HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nContent-Disposition: attachment; filename=\"{}\"\r\nConnection: close\r\n\r\n",
                    response_body.len(),
                    reply.filename
                )
                .unwrap();
                let written = reply
                    .truncate_after
                    .unwrap_or(response_body.len())
                    .min(response_body.len());
                stream.write_all(&response_body[..written]).unwrap();
            }
        });
        (format!("http://{address}"), request_receiver, handle)
    }

    fn run_test_transfer(artifact: &RemoteArtifact, destination: &Path) -> Result<()> {
        let (sender, _receiver) = mpsc::channel();
        run_transfer(
            std::slice::from_ref(artifact),
            "Test game",
            "test-token",
            destination,
            &AtomicBool::new(false),
            &sender,
            false,
            1,
        )
    }

    #[test]
    fn rejects_path_components_from_server_filenames() {
        assert_eq!(
            files::sanitize_filename("../../setup.exe").as_deref(),
            Some("setup.exe")
        );
        assert_eq!(files::sanitize_filename(".."), None);
    }

    #[test]
    fn expands_relative_gog_download_urls() {
        assert_eq!(
            download_url("/downloads/example/en1installer0"),
            "https://www.gog.com/downloads/example/en1installer0"
        );
    }

    #[test]
    fn resolves_gog_json_descriptor_before_writing_installer() {
        let cdn = TcpListener::bind("127.0.0.1:0").unwrap();
        let cdn_address = cdn.local_addr().unwrap();
        let cdn_thread = thread::spawn(move || {
            let (mut stream, _) = cdn.accept().unwrap();
            let mut request = [0_u8; 2048];
            let _ = stream.read(&mut request).unwrap();
            let body = b"real-installer";
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Disposition: attachment; filename=\"moonlighter.sh\"\r\nContent-Type: application/octet-stream\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .unwrap();
            stream.write_all(body).unwrap();
        });

        let descriptor = TcpListener::bind("127.0.0.1:0").unwrap();
        let descriptor_address = descriptor.local_addr().unwrap();
        let descriptor_thread = thread::spawn(move || {
            let (mut stream, _) = descriptor.accept().unwrap();
            let mut request = [0_u8; 2048];
            let _ = stream.read(&mut request).unwrap();
            let body = format!(r#"{{"downlink":"http://{cdn_address}/moonlighter.sh"}}"#);
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        });

        let directory = TestDirectory::new("gog-descriptor");
        let destination = directory.0.join("game/installer/windows/english");
        let artifact = test_artifact(
            format!("http://{descriptor_address}/downlink/en1installer0"),
            b"real-installer".len(),
        );
        run_test_transfer(&artifact, &destination).unwrap();
        assert_eq!(
            fs::read(destination.join("moonlighter.sh")).unwrap(),
            b"real-installer"
        );
        descriptor_thread.join().unwrap();
        cdn_thread.join().unwrap();
    }

    #[test]
    fn uses_a_safe_product_slug_for_the_download_directory() {
        let artifact = RemoteArtifact {
            product_id: 123,
            kind: crate::domain::ArtifactKind::Installer,
            name: "Example".into(),
            language: Some("English".into()),
            operating_system: Some("windows".into()),
            version: None,
            release_date: None,
            size_label: None,
            size_bytes: None,
            part_number: None,
            part_count: None,
            download_path: "/downloads/example".into(),
            provider_group_id: None,
            provider_file_id: None,
            provider_category: None,
        };
        let path = destination(Path::new("/library"), "example_game", None, &[&artifact]);
        assert_eq!(
            path,
            Path::new("/library/example_game/installer/windows/english")
        );
    }

    #[test]
    fn omits_missing_os_and_language_directories() {
        let artifact = RemoteArtifact {
            product_id: 123,
            kind: crate::domain::ArtifactKind::Extra,
            name: "Manual".into(),
            language: None,
            operating_system: None,
            version: None,
            release_date: None,
            size_label: None,
            size_bytes: None,
            part_number: None,
            part_count: None,
            download_path: "/downloads/example-manual".into(),
            provider_group_id: None,
            provider_file_id: None,
            provider_category: None,
        };
        let path = destination(Path::new("/library"), "example_game", None, &[&artifact]);
        assert_eq!(path, Path::new("/library/example_game/extra"));
    }

    #[test]
    fn nests_dlc_beneath_the_parent_game() {
        let artifact = RemoteArtifact {
            product_id: 456,
            kind: crate::domain::ArtifactKind::Installer,
            name: "Expansion".into(),
            language: Some("English".into()),
            operating_system: Some("windows".into()),
            version: None,
            release_date: None,
            size_label: None,
            size_bytes: None,
            part_number: None,
            part_count: None,
            download_path: "/downloads/expansion".into(),
            provider_group_id: None,
            provider_file_id: None,
            provider_category: None,
        };
        assert_eq!(
            destination(
                Path::new("/library"),
                "main_game",
                Some("expansion"),
                &[&artifact]
            ),
            Path::new("/library/main_game/dlc/expansion/installer/windows/english")
        );
    }

    #[test]
    fn refuses_to_delete_files_outside_the_job_directory() {
        let result = delete_completed_files(
            Path::new("/library/example"),
            &[PathBuf::from("/library/another-game/setup.exe")],
        );
        assert!(result.is_err());
    }

    #[test]
    fn decodes_extended_content_disposition_filenames() {
        assert_eq!(
            protocol::content_disposition_filename(
                "attachment; filename*=UTF-8''setup_game_1.0%20%2864bit%29.exe"
            )
            .as_deref(),
            Some("setup_game_1.0 (64bit).exe")
        );
    }

    #[test]
    fn classifies_filesystem_failures_without_parsing_messages() {
        let permission = anyhow::Error::new(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "localized or provider-specific text",
        ));
        assert_eq!(
            worker::classify_download_error(&permission).kind,
            DownloadFailureKind::PermissionDenied
        );

        let disk_full = anyhow::Error::new(std::io::Error::from_raw_os_error(28));
        assert_eq!(
            worker::classify_download_error(&disk_full).kind,
            DownloadFailureKind::DiskFull
        );
    }

    #[test]
    fn error_words_do_not_change_the_failure_class() {
        let error = anyhow::anyhow!("a filename contains 401 connection timeout");
        assert_eq!(
            worker::classify_download_error(&error).kind,
            DownloadFailureKind::Other
        );
    }

    #[test]
    fn downloads_a_file_from_a_local_http_server() {
        let body = b"a complete offline installer".to_vec();
        let (url, request, server) = local_response_server(body.clone(), 200, true);
        let directory = TestDirectory::new("fresh-transfer");
        let destination = directory.0.join("game/installer/windows/english");
        let artifact = test_artifact(url, body.len());

        run_test_transfer(&artifact, &destination).unwrap();
        server.join().unwrap();

        assert_eq!(request.recv().unwrap(), None);
        assert_eq!(fs::read(destination.join("setup_test.bin")).unwrap(), body);
    }

    #[test]
    fn resumes_a_partial_file_with_an_http_range_request() {
        let body = b"0123456789abcdefghijklmnopqrstuvwxyz".to_vec();
        let (url, request, server) = local_response_server(body.clone(), 200, true);
        let directory = TestDirectory::new("range-resume");
        let destination = directory.0.join("game/installer/windows/english");
        let artifact = test_artifact(url, body.len());
        let id = job_id(&[&artifact]);
        let staging = staging_directory(&destination, std::slice::from_ref(&artifact), &id);
        fs::create_dir_all(&staging).unwrap();
        fs::write(staging.join("1.download"), &body[..10]).unwrap();

        run_test_transfer(&artifact, &destination).unwrap();
        server.join().unwrap();

        assert_eq!(request.recv().unwrap().as_deref(), Some("bytes=10-"));
        assert_eq!(fs::read(destination.join("setup_test.bin")).unwrap(), body);
    }

    #[test]
    fn safely_restarts_when_a_server_ignores_range() {
        let body = b"replacement bytes from the beginning".to_vec();
        let (url, request, server) = local_response_server(body.clone(), 200, false);
        let directory = TestDirectory::new("ignored-range");
        let destination = directory.0.join("game/installer/windows/english");
        let artifact = test_artifact(url, body.len());
        let id = job_id(&[&artifact]);
        let staging = staging_directory(&destination, std::slice::from_ref(&artifact), &id);
        fs::create_dir_all(&staging).unwrap();
        fs::write(staging.join("1.download"), b"stale partial").unwrap();

        run_test_transfer(&artifact, &destination).unwrap();
        server.join().unwrap();

        assert_eq!(request.recv().unwrap().as_deref(), Some("bytes=13-"));
        assert_eq!(fs::read(destination.join("setup_test.bin")).unwrap(), body);
    }

    #[test]
    fn classifies_a_local_not_found_response_as_a_stale_manifest() {
        let (url, _request, server) = local_response_server(Vec::new(), 404, false);
        let directory = TestDirectory::new("stale-manifest");
        let destination = directory.0.join("game/installer/windows/english");
        let artifact = test_artifact(url, 0);

        let error = run_test_transfer(&artifact, &destination).unwrap_err();
        server.join().unwrap();

        assert_eq!(
            worker::classify_download_error(&error).kind,
            DownloadFailureKind::ManifestChanged
        );
    }

    #[test]
    fn classifies_local_http_authentication_and_retryable_failures() {
        for (status, expected) in [
            (401, DownloadFailureKind::Authentication),
            (429, DownloadFailureKind::TransientNetwork),
            (503, DownloadFailureKind::TransientNetwork),
        ] {
            let (url, _request, server) = local_response_server(Vec::new(), status, false);
            let directory = TestDirectory::new(&format!("http-{status}"));
            let destination = directory.0.join("game/installer/windows/english");
            let artifact = test_artifact(url, 0);

            let error = run_test_transfer(&artifact, &destination).unwrap_err();
            server.join().unwrap();
            assert_eq!(worker::classify_download_error(&error).kind, expected);
        }
    }

    #[test]
    fn downloads_multipart_installers_in_manifest_order() {
        let first = b"installer part one".to_vec();
        let second = b"installer part two".to_vec();
        let (base_url, requests, server) = scripted_response_server(vec![
            ScriptedReply {
                body: first.clone(),
                filename: "setup_test-1.bin",
                honor_range: true,
                truncate_after: None,
            },
            ScriptedReply {
                body: second.clone(),
                filename: "setup_test-2.bin",
                honor_range: true,
                truncate_after: None,
            },
        ]);
        let directory = TestDirectory::new("multipart");
        let destination = directory.0.join("game/installer/windows/english");
        let mut first_artifact = test_artifact(format!("{base_url}/part-1"), first.len());
        first_artifact.part_count = Some(2);
        let mut second_artifact = test_artifact(format!("{base_url}/part-2"), second.len());
        second_artifact.provider_file_id = Some("en1installer1".into());
        second_artifact.part_number = Some(2);
        second_artifact.part_count = Some(2);
        let (sender, _receiver) = mpsc::channel();

        run_transfer(
            &[first_artifact, second_artifact],
            "Multipart game",
            "test-token",
            &destination,
            &AtomicBool::new(false),
            &sender,
            false,
            2,
        )
        .unwrap();
        server.join().unwrap();

        assert_eq!(requests.recv().unwrap(), None);
        assert_eq!(requests.recv().unwrap(), None);
        assert_eq!(
            fs::read(destination.join("setup_test-1.bin")).unwrap(),
            first
        );
        assert_eq!(
            fs::read(destination.join("setup_test-2.bin")).unwrap(),
            second
        );
    }

    #[test]
    fn resumes_after_an_interrupted_http_body() {
        let body = b"an installer interrupted halfway through transfer".to_vec();
        let (base_url, requests, server) = scripted_response_server(vec![
            ScriptedReply {
                body: body.clone(),
                filename: "setup_interrupted.bin",
                honor_range: true,
                truncate_after: Some(17),
            },
            ScriptedReply {
                body: body.clone(),
                filename: "setup_interrupted.bin",
                honor_range: true,
                truncate_after: None,
            },
        ]);
        let directory = TestDirectory::new("interrupted-body");
        let destination = directory.0.join("game/installer/windows/english");
        let artifact = test_artifact(format!("{base_url}/installer"), body.len());

        assert!(run_test_transfer(&artifact, &destination).is_err());
        run_test_transfer(&artifact, &destination).unwrap();
        server.join().unwrap();

        assert_eq!(requests.recv().unwrap(), None);
        assert_eq!(requests.recv().unwrap().as_deref(), Some("bytes=17-"));
        assert_eq!(
            fs::read(destination.join("setup_interrupted.bin")).unwrap(),
            body
        );
    }
}
