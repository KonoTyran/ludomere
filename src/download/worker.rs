use super::{
    DownloadEvent, DownloadFailure, DownloadFailureKind,
    transfer::{DownloadSnapshot, downloaded_on_disk, persist, run},
};
use crate::{domain::RemoteArtifact, state::DownloadState};
use reqwest::StatusCode;
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
};

static ACTIVE_DOWNLOADS: OnceLock<Mutex<HashMap<String, Arc<AtomicBool>>>> = OnceLock::new();

#[allow(clippy::too_many_arguments)]
pub(super) fn start_worker(
    active_job_id: String,
    artifacts: Vec<RemoteArtifact>,
    title: String,
    access_token: String,
    destination: PathBuf,
    part_concurrency: usize,
    session: u64,
    sender: mpsc::Sender<DownloadEvent>,
) -> Arc<AtomicBool> {
    let cancelled = Arc::new(AtomicBool::new(false));
    {
        let mut downloads = ACTIVE_DOWNLOADS
            .get_or_init(|| Mutex::new(HashMap::new()))
            .lock()
            .expect("active download registry");
        if let Some(existing) = downloads.get(&active_job_id) {
            return existing.clone();
        }
        downloads.insert(active_job_id.clone(), cancelled.clone());
    }
    let worker_cancelled = cancelled.clone();
    std::thread::spawn(move || {
        let permit = crate::operation_gate::acquire(|| worker_cancelled.load(Ordering::Relaxed));
        let result = match permit {
            Some(_permit) => Some(
                super::manager::validate_destination(&artifacts, &destination, None).and_then(
                    |()| {
                        run(
                            &artifacts,
                            &title,
                            &access_token,
                            &destination,
                            &worker_cancelled,
                            &sender,
                            part_concurrency,
                            session,
                        )
                    },
                ),
            ),
            None => {
                let _ = sender.send(DownloadEvent::Cancelled);
                None
            }
        };
        if let Some(Err(error)) = result {
            let failure = classify_download_error(&error);
            let message = failure.message.clone();
            let downloaded = downloaded_on_disk(&destination);
            let total = artifacts
                .iter()
                .map(|artifact| artifact.size_bytes)
                .collect::<Option<Vec<_>>>()
                .map(|sizes| sizes.into_iter().sum());
            if error.is::<super::transfer::BookkeepingError>() {
                // The receipt survives even when SQLite cannot record this failure.
                let _ = crate::state::StateStore::open().and_then(|store| {
                    store.try_record_bookkeeping_failure(&active_job_id, session, &message)
                });
            } else {
                persist(
                    &artifacts,
                    &title,
                    DownloadSnapshot {
                        destination: &destination,
                        state: DownloadState::Failed,
                        downloaded,
                        total,
                        files: &[],
                        error: Some(&message),
                    },
                );
            }
            let _ = sender.send(DownloadEvent::Failed(failure));
        }
        if let Some(downloads) = ACTIVE_DOWNLOADS.get() {
            downloads
                .lock()
                .expect("active download registry")
                .remove(&active_job_id);
        }
    });
    cancelled
}

pub(super) fn cancel_worker(job_id: &str) -> bool {
    ACTIVE_DOWNLOADS
        .get()
        .and_then(|downloads| downloads.lock().ok()?.get(job_id).cloned())
        .is_some_and(|cancelled| {
            cancelled.store(true, Ordering::Relaxed);
            true
        })
}

pub(super) fn worker_is_active(job_id: &str) -> bool {
    ACTIVE_DOWNLOADS
        .get()
        .and_then(|downloads| downloads.lock().ok()?.get(job_id).cloned())
        .is_some()
}

pub(super) fn classify_download_error(error: &anyhow::Error) -> DownloadFailure {
    if let Some(bookkeeping) = error.downcast_ref::<super::transfer::BookkeepingError>() {
        let summary = if bookkeeping.files.is_empty() {
            "Downloaded-file registration failed. Existing files were preserved; check the download destination and retry.".into()
        } else {
            bookkeeping.to_string()
        };
        let details = error
            .chain()
            .map(ToString::to_string)
            // anyhow's context wrapper displays the bookkeeping message but
            // cannot itself be downcast to BookkeepingError.
            .filter(|cause| cause != &bookkeeping.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        return DownloadFailure {
            kind: DownloadFailureKind::Bookkeeping,
            message: if details.is_empty() {
                summary
            } else {
                format!(
                    "{summary}\nDetails: {}",
                    crate::installation::runtime_logs::sanitize(&details).trim()
                )
            },
        };
    }
    let mut kind = DownloadFailureKind::Other;
    for cause in error.chain() {
        if let Some(error) = cause.downcast_ref::<reqwest::Error>() {
            if let Some(status) = error.status() {
                kind = if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
                    DownloadFailureKind::Authentication
                } else if status == StatusCode::NOT_FOUND {
                    DownloadFailureKind::ManifestChanged
                } else if status == StatusCode::REQUEST_TIMEOUT
                    || status == StatusCode::TOO_MANY_REQUESTS
                    || status.is_server_error()
                {
                    DownloadFailureKind::TransientNetwork
                } else {
                    kind
                };
            } else if error.is_timeout() || error.is_connect() {
                kind = DownloadFailureKind::TransientNetwork;
            }
        }
        if let Some(error) = cause.downcast_ref::<std::io::Error>() {
            kind = match error.kind() {
                std::io::ErrorKind::PermissionDenied => DownloadFailureKind::PermissionDenied,
                std::io::ErrorKind::StorageFull => DownloadFailureKind::DiskFull,
                _ if error.raw_os_error() == Some(28) => DownloadFailureKind::DiskFull,
                _ => kind,
            };
        }
    }
    DownloadFailure {
        kind,
        message: format!("{error:#}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bookkeeping_errors_retain_safe_causes_on_first_attempt_and_retry() {
        for files in [vec![], vec![PathBuf::from("/inert/installer")]] {
            let error = anyhow::anyhow!(
                "Catalog size mismatch: expected 100 bytes, found 104 bytes\nhttps://example.invalid/file?signature=inert-signature\naccess_token=inert-secret"
            )
            .context(super::super::transfer::BookkeepingError { files: files.clone() })
            .context("Registering completed download")
            .context(super::super::transfer::BookkeepingError { files });
            let failure = classify_download_error(&error);
            assert_eq!(failure.kind, DownloadFailureKind::Bookkeeping);
            assert!(
                failure
                    .message
                    .contains("expected 100 bytes, found 104 bytes")
            );
            assert!(failure.message.contains("[URL redacted]"));
            assert!(!failure.message.contains("inert-signature"));
            assert!(!failure.message.contains("inert-secret"));
            assert!(failure.message.contains("preserved"));
            assert_eq!(
                failure
                    .message
                    .matches("recording completion failed")
                    .count(),
                if failure.message.starts_with("Downloaded-file") {
                    0
                } else {
                    1
                }
            );
            assert!(failure.message.contains("Registering completed download"));
        }
    }
}
