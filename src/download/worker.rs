use super::{
    DownloadEvent, DownloadFailure, DownloadFailureKind, job_id,
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

pub(super) fn start_worker(
    artifacts: Vec<RemoteArtifact>,
    title: String,
    access_token: String,
    destination: PathBuf,
    part_concurrency: usize,
    sender: mpsc::Sender<DownloadEvent>,
) -> Arc<AtomicBool> {
    let cancelled = Arc::new(AtomicBool::new(false));
    let refs = artifacts.iter().collect::<Vec<_>>();
    let active_job_id = job_id(&refs);
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
        let permit = crate::operation_gate::acquire_work(
            crate::state::WorkKind::Download,
            &active_job_id,
            || worker_cancelled.load(Ordering::Relaxed),
        );
        let result = match permit {
            Some(_permit) => Some(run(
                &artifacts,
                &title,
                &access_token,
                &destination,
                &worker_cancelled,
                &sender,
                part_concurrency,
            )),
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
