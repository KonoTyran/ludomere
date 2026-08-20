use super::{
    DownloadEvent,
    files::fallback_filename,
    job_id,
    layout::staging_directory,
    protocol::{download_url, resolve_download_response, response_filename},
};
use crate::{
    domain::RemoteArtifact,
    state::{DownloadJobUpdate, DownloadState, StateStore},
};
use anyhow::{Context, Result, bail};
use reqwest::{StatusCode, header};
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

pub(super) struct DownloadSnapshot<'a> {
    pub destination: &'a Path,
    pub state: DownloadState,
    pub downloaded: u64,
    pub total: Option<u64>,
    pub files: &'a [PathBuf],
    pub error: Option<&'a str>,
}

pub(super) fn run(
    artifacts: &[RemoteArtifact],
    title: &str,
    access_token: &str,
    destination: &Path,
    cancelled: &AtomicBool,
    sender: &mpsc::Sender<DownloadEvent>,
    part_concurrency: usize,
) -> Result<()> {
    run_transfer(
        artifacts,
        title,
        access_token,
        destination,
        cancelled,
        sender,
        true,
        part_concurrency,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn run_transfer(
    artifacts: &[RemoteArtifact],
    title: &str,
    access_token: &str,
    destination: &Path,
    cancelled: &AtomicBool,
    sender: &mpsc::Sender<DownloadEvent>,
    persist_state: bool,
    part_concurrency: usize,
) -> Result<()> {
    fs::create_dir_all(destination)?;
    let refs = artifacts.iter().collect::<Vec<_>>();
    let staging = staging_directory(destination, artifacts, &job_id(&refs));
    fs::create_dir_all(&staging)?;
    let expected_total = artifacts
        .iter()
        .map(|artifact| artifact.size_bytes)
        .collect::<Option<Vec<_>>>()
        .map(|sizes| sizes.into_iter().sum());
    persist_if(
        persist_state,
        artifacts,
        title,
        DownloadSnapshot {
            destination,
            state: DownloadState::Downloading,
            downloaded: 0,
            total: expected_total,
            files: &[],
            error: None,
        },
    );

    if artifacts.len() > 1 && part_concurrency > 1 {
        return run_parallel_parts(
            artifacts,
            title,
            access_token,
            destination,
            cancelled,
            sender,
            persist_state,
            part_concurrency,
            expected_total,
        );
    }

    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::limited(10))
        .user_agent(crate::identity::USER_AGENT)
        .build()?;
    let mut completed = Vec::new();
    let mut downloaded_before_part = 0;
    for (index, artifact) in artifacts.iter().enumerate() {
        if cancelled.load(Ordering::Relaxed) {
            persist_if(
                persist_state,
                artifacts,
                title,
                DownloadSnapshot {
                    destination,
                    state: DownloadState::Paused,
                    downloaded: downloaded_before_part,
                    total: expected_total,
                    files: &completed,
                    error: None,
                },
            );
            let _ = sender.send(DownloadEvent::Cancelled);
            return Ok(());
        }
        let temporary = staging.join(format!("{}.download", index + 1));
        let mut existing = temporary.metadata().map_or(0, |metadata| metadata.len());
        let response = match request_download_response(
            &client,
            artifact,
            access_token,
            (existing > 0).then_some(existing),
        ) {
            Err(error) if existing > 0 && is_range_not_satisfiable(&error) => {
                fs::remove_file(&temporary).with_context(|| {
                    format!("removing invalid partial download {}", temporary.display())
                })?;
                existing = 0;
                request_download_response(&client, artifact, access_token, None)
                    .with_context(|| format!("retrying {} from the beginning", artifact.name))?
            }
            result => result?,
        };
        let resumed = existing > 0 && response.status() == StatusCode::PARTIAL_CONTENT;
        let filename = response_filename(&response)
            .unwrap_or_else(|| fallback_filename(artifact, index, artifacts.len()));
        let final_path = destination.join(filename);
        if final_path.is_file() {
            let local_size = final_path.metadata()?.len();
            if artifact
                .size_bytes
                .is_none_or(|expected| expected == local_size)
            {
                downloaded_before_part += local_size;
                completed.push(final_path);
                continue;
            }
        }
        let response_length = response.content_length();
        let total = expected_total
            .or_else(|| response_length.map(|length| downloaded_before_part + existing + length));
        let mut output = OpenOptions::new()
            .create(true)
            .write(true)
            .append(resumed)
            .truncate(!resumed)
            .open(&temporary)?;
        let mut response = response;
        let mut buffer = vec![0_u8; 256 * 1024];
        let mut current = if resumed { existing } else { 0 };
        let mut last_update = Instant::now();
        let mut last_persist = Instant::now();
        loop {
            if cancelled.load(Ordering::Relaxed) {
                persist_if(
                    persist_state,
                    artifacts,
                    title,
                    DownloadSnapshot {
                        destination,
                        state: DownloadState::Paused,
                        downloaded: downloaded_before_part + current,
                        total,
                        files: &completed,
                        error: None,
                    },
                );
                let _ = sender.send(DownloadEvent::Cancelled);
                return Ok(());
            }
            let count = response.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            if !super::bandwidth::acquire(count as u64, || cancelled.load(Ordering::Relaxed)) {
                continue;
            }
            output.write_all(&buffer[..count])?;
            current += count as u64;
            if last_update.elapsed() >= Duration::from_millis(100) {
                let downloaded = downloaded_before_part + current;
                let _ = sender.send(DownloadEvent::Progress { downloaded, total });
                if last_persist.elapsed() >= Duration::from_secs(1) {
                    persist_if(
                        persist_state,
                        artifacts,
                        title,
                        DownloadSnapshot {
                            destination,
                            state: DownloadState::Downloading,
                            downloaded,
                            total,
                            files: &completed,
                            error: None,
                        },
                    );
                    last_persist = Instant::now();
                }
                last_update = Instant::now();
            }
        }
        output.flush()?;
        fs::rename(&temporary, &final_path)?;
        downloaded_before_part += current;
        completed.push(final_path);
    }
    if completed.len() != artifacts.len() {
        bail!("download finished without all expected parts");
    }
    let _ = sender.send(DownloadEvent::Finalizing);
    if persist_state && let Ok(store) = StateStore::open() {
        let _ = store.set_download_job_status(&job_id(&refs), Some("Finalizing…"));
    }
    persist_if(
        persist_state,
        artifacts,
        title,
        DownloadSnapshot {
            destination,
            state: DownloadState::Complete,
            downloaded: downloaded_before_part,
            total: expected_total.or(Some(downloaded_before_part)),
            files: &completed,
            error: None,
        },
    );
    let _ = sender.send(DownloadEvent::Complete { files: completed });
    let _ = fs::remove_dir(&staging);
    Ok(())
}

fn request_download_response(
    client: &reqwest::blocking::Client,
    artifact: &RemoteArtifact,
    access_token: &str,
    range_start: Option<u64>,
) -> Result<reqwest::blocking::Response> {
    let mut request = client
        .get(download_url(&artifact.download_path))
        .bearer_auth(access_token);
    if let Some(existing) = range_start {
        request = request.header(header::RANGE, format!("bytes={existing}-"));
    }
    let response = request
        .send()
        .with_context(|| format!("requesting {}", artifact.name))?
        .error_for_status()
        .with_context(|| format!("GOG rejected the download for {}", artifact.name))?;
    Ok(resolve_download_response(client, response, range_start)?.response)
}

fn is_range_not_satisfiable(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause
            .downcast_ref::<reqwest::Error>()
            .and_then(reqwest::Error::status)
            == Some(StatusCode::RANGE_NOT_SATISFIABLE)
    })
}

#[allow(clippy::too_many_arguments)]
fn run_parallel_parts(
    artifacts: &[RemoteArtifact],
    title: &str,
    access_token: &str,
    destination: &Path,
    cancelled: &AtomicBool,
    sender: &mpsc::Sender<DownloadEvent>,
    persist_state: bool,
    part_concurrency: usize,
    expected_total: Option<u64>,
) -> Result<()> {
    let mut completed = Vec::new();
    let mut completed_bytes = 0_u64;
    for batch in artifacts.chunks(part_concurrency.clamp(1, 4)) {
        let (event_sender, event_receiver) = mpsc::channel();
        let (result_sender, result_receiver) = mpsc::channel();
        let mut part_downloaded = vec![0_u64; batch.len()];
        let mut batch_files = vec![Vec::new(); batch.len()];
        let mut results = Vec::with_capacity(batch.len());
        std::thread::scope(|scope| {
            for (batch_index, artifact) in batch.iter().enumerate() {
                let (worker_events, worker_receiver) = mpsc::channel();
                let event_sender = event_sender.clone();
                scope.spawn(move || {
                    while let Ok(event) = worker_receiver.recv() {
                        if event_sender.send((batch_index, event)).is_err() {
                            break;
                        }
                    }
                });
                let result_sender = result_sender.clone();
                scope.spawn(move || {
                    let result = run_transfer(
                        std::slice::from_ref(artifact),
                        title,
                        access_token,
                        destination,
                        cancelled,
                        &worker_events,
                        false,
                        1,
                    );
                    drop(worker_events);
                    let _ = result_sender.send((batch_index, result));
                });
            }
            drop(event_sender);
            drop(result_sender);

            let mut finished = 0;
            while finished < batch.len() {
                while let Ok((index, event)) = event_receiver.try_recv() {
                    handle_part_event(
                        index,
                        event,
                        completed_bytes,
                        &mut part_downloaded,
                        &mut batch_files,
                        expected_total,
                        sender,
                    );
                }
                match result_receiver.recv_timeout(Duration::from_millis(20)) {
                    Ok(result) => {
                        results.push(result);
                        finished += 1;
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                }
            }
        });

        // Forwarders may have delivered their terminal event immediately before
        // the scoped threads joined, so drain the channel once more.
        while let Ok((index, event)) = event_receiver.try_recv() {
            handle_part_event(
                index,
                event,
                completed_bytes,
                &mut part_downloaded,
                &mut batch_files,
                expected_total,
                sender,
            );
        }
        results.extend(result_receiver.try_iter());
        results.sort_by_key(|(index, _)| *index);
        for (_, result) in results {
            result?;
        }
        for files in batch_files {
            completed_bytes += files
                .iter()
                .filter_map(|path| path.metadata().ok().map(|metadata| metadata.len()))
                .sum::<u64>();
            completed.extend(files);
        }
        if cancelled.load(Ordering::Relaxed) {
            persist_if(
                persist_state,
                artifacts,
                title,
                DownloadSnapshot {
                    destination,
                    state: DownloadState::Paused,
                    downloaded: completed_bytes,
                    total: expected_total,
                    files: &completed,
                    error: None,
                },
            );
            let _ = sender.send(DownloadEvent::Cancelled);
            return Ok(());
        }
    }
    if completed.len() != artifacts.len() {
        bail!("download finished without all expected parts");
    }
    let refs = artifacts.iter().collect::<Vec<_>>();
    let _ = sender.send(DownloadEvent::Finalizing);
    if persist_state && let Ok(store) = StateStore::open() {
        let _ = store.set_download_job_status(&job_id(&refs), Some("Finalizing…"));
    }
    persist_if(
        persist_state,
        artifacts,
        title,
        DownloadSnapshot {
            destination,
            state: DownloadState::Complete,
            downloaded: completed_bytes,
            total: expected_total.or(Some(completed_bytes)),
            files: &completed,
            error: None,
        },
    );
    let _ = sender.send(DownloadEvent::Complete { files: completed });
    Ok(())
}

fn handle_part_event(
    index: usize,
    event: DownloadEvent,
    completed_bytes: u64,
    part_downloaded: &mut [u64],
    batch_files: &mut [Vec<PathBuf>],
    expected_total: Option<u64>,
    sender: &mpsc::Sender<DownloadEvent>,
) {
    match event {
        DownloadEvent::Progress { downloaded, .. } => {
            // Range resumes and retries may repeat an older observation. Never
            // allow aggregate UI progress to move backwards.
            part_downloaded[index] = part_downloaded[index].max(downloaded);
            let downloaded = completed_bytes.saturating_add(part_downloaded.iter().sum::<u64>());
            let _ = sender.send(DownloadEvent::Progress {
                downloaded,
                total: expected_total,
            });
        }
        DownloadEvent::Complete { files } => batch_files[index] = files,
        DownloadEvent::Finalizing | DownloadEvent::Cancelled | DownloadEvent::Failed(_) => {}
    }
}

fn persist_if(
    enabled: bool,
    artifacts: &[RemoteArtifact],
    title: &str,
    snapshot: DownloadSnapshot<'_>,
) {
    if enabled {
        persist(artifacts, title, snapshot);
    }
}

pub(super) fn persist(artifacts: &[RemoteArtifact], title: &str, snapshot: DownloadSnapshot<'_>) {
    let refs = artifacts.iter().collect::<Vec<_>>();
    if let Ok(store) = StateStore::open() {
        let id = job_id(&refs);
        let _ = store.save_download_job(&DownloadJobUpdate {
            job_id: &id,
            product_id: artifacts[0].product_id,
            title,
            artifacts,
            destination: snapshot.destination,
            state: snapshot.state,
            bytes_downloaded: snapshot.downloaded,
            total_bytes: snapshot.total,
            completed_files: snapshot.files,
            error: snapshot.error,
        });
        if snapshot.state == DownloadState::Complete {
            let slug = product_slug_from_destination(snapshot.destination, &artifacts[0]);
            let _ = store.record_completed_artifacts(&id, &slug, artifacts, snapshot.files);
        }
    }
}

fn product_slug_from_destination(destination: &Path, artifact: &RemoteArtifact) -> String {
    let levels =
        1 + usize::from(
            artifact
                .operating_system
                .as_deref()
                .is_some_and(|value| !value.is_empty()),
        ) + usize::from(
            artifact
                .language
                .as_deref()
                .is_some_and(|value| !value.is_empty()),
        );
    let mut product = destination;
    for _ in 0..levels {
        product = product.parent().unwrap_or(product);
    }
    product
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("unknown")
        .to_owned()
}

pub(super) fn downloaded_on_disk(destination: &Path) -> u64 {
    fs::read_dir(destination)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| entry.metadata().ok())
        .filter(|metadata| metadata.is_file())
        .map(|metadata| metadata.len())
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parallel_part_progress_is_aggregated_and_never_moves_backwards() {
        let (sender, receiver) = mpsc::channel();
        let mut downloaded = vec![0, 0];
        let mut files = vec![Vec::new(), Vec::new()];
        for (part, bytes) in [(0, 20), (1, 15), (0, 10), (1, 35)] {
            handle_part_event(
                part,
                DownloadEvent::Progress {
                    downloaded: bytes,
                    total: Some(50),
                },
                100,
                &mut downloaded,
                &mut files,
                Some(200),
                &sender,
            );
        }
        let observations = receiver
            .try_iter()
            .filter_map(|event| match event {
                DownloadEvent::Progress { downloaded, total } => Some((downloaded, total)),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            observations,
            vec![
                (120, Some(200)),
                (135, Some(200)),
                (135, Some(200)),
                (155, Some(200))
            ]
        );
    }
}
