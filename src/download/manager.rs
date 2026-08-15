use super::{
    DownloadEvent, DownloadFailure, DownloadFailureKind, DownloadManagerEvent, cancel_worker,
    delete_completed_files, job_id, staging_directory, start_worker, worker_is_active,
};
use crate::{
    domain::RemoteArtifact,
    state::{DownloadJobUpdate, DownloadState, StateStore},
};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    fs,
    path::PathBuf,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

#[derive(Clone)]
struct Request {
    id: String,
    artifacts: Vec<RemoteArtifact>,
    title: String,
    access_token: String,
    destination: PathBuf,
    listener: mpsc::Sender<DownloadEvent>,
    handle: Arc<AtomicBool>,
    retry_started: Option<Instant>,
    retry_attempt: u32,
    ready_at: Instant,
    queue_position: Option<i64>,
    manifest_refresh_attempted: bool,
}

enum Command {
    Enqueue(Request),
    Pause(String),
    Resume {
        id: String,
        access_token: String,
        reset_retry: bool,
    },
    Remove(String),
    Terminal(Request, DownloadEvent),
    ManifestRefreshed(
        Request,
        DownloadFailure,
        Result<Vec<RemoteArtifact>, String>,
    ),
    SetConcurrency(usize),
    Recover(String),
    SetNetwork(bool),
    SetAuthentication(bool),
    Shutdown(mpsc::Sender<()>),
}

struct ManagerHandle {
    commands: mpsc::Sender<Command>,
    active: Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>,
    subscribers: Arc<Mutex<Vec<mpsc::Sender<DownloadManagerEvent>>>>,
}

static MANAGER: OnceLock<ManagerHandle> = OnceLock::new();

fn manager() -> &'static ManagerHandle {
    MANAGER.get_or_init(|| {
        let (commands, receiver) = mpsc::channel();
        let active = Arc::new(Mutex::new(HashMap::new()));
        let manager_active = active.clone();
        let subscribers = Arc::new(Mutex::new(Vec::new()));
        let manager_subscribers = subscribers.clone();
        let manager_commands = commands.clone();
        std::thread::spawn(move || {
            run(
                receiver,
                manager_commands,
                manager_active,
                manager_subscribers,
            )
        });
        ManagerHandle {
            commands,
            active,
            subscribers,
        }
    })
}

pub(super) fn subscribe() -> mpsc::Receiver<DownloadManagerEvent> {
    let (sender, receiver) = mpsc::channel();
    if let Ok(mut subscribers) = manager().subscribers.lock() {
        subscribers.push(sender);
    }
    publish_queue_snapshot(&manager().subscribers);
    receiver
}

pub(super) fn enqueue(
    artifacts: Vec<RemoteArtifact>,
    title: String,
    access_token: String,
    destination: PathBuf,
    listener: mpsc::Sender<DownloadEvent>,
) -> Arc<AtomicBool> {
    let refs = artifacts.iter().collect::<Vec<_>>();
    let id = job_id(&refs);
    let handle = Arc::new(AtomicBool::new(false));
    let request = Request {
        id,
        artifacts,
        title,
        access_token,
        destination,
        listener,
        handle: handle.clone(),
        retry_started: None,
        retry_attempt: 0,
        ready_at: Instant::now(),
        queue_position: None,
        manifest_refresh_attempted: false,
    };
    let _ = manager().commands.send(Command::Enqueue(request));
    handle
}

pub(super) fn pause(id: &str) -> bool {
    manager()
        .commands
        .send(Command::Pause(id.to_owned()))
        .is_ok()
}

pub(super) fn resume(id: &str, access_token: String, reset_retry: bool) -> bool {
    manager()
        .commands
        .send(Command::Resume {
            id: id.to_owned(),
            access_token,
            reset_retry,
        })
        .is_ok()
}

pub(super) fn remove(id: &str) -> bool {
    manager()
        .commands
        .send(Command::Remove(id.to_owned()))
        .is_ok()
}

pub(super) fn is_active(id: &str) -> bool {
    manager()
        .active
        .lock()
        .is_ok_and(|active| active.contains_key(id))
        || worker_is_active(id)
}

pub(super) fn set_concurrency(limit: usize) {
    let _ = manager()
        .commands
        .send(Command::SetConcurrency(limit.clamp(1, 4)));
}

pub(super) fn recover(access_token: String) {
    let _ = manager().commands.send(Command::Recover(access_token));
}

pub(super) fn set_network(available: bool) {
    let _ = manager().commands.send(Command::SetNetwork(available));
}

pub(super) fn set_authenticated(authenticated: bool) {
    let _ = manager()
        .commands
        .send(Command::SetAuthentication(authenticated));
}

pub(super) fn shutdown() {
    let Some(manager) = MANAGER.get() else {
        return;
    };
    let (acknowledgement, receiver) = mpsc::channel();
    if manager
        .commands
        .send(Command::Shutdown(acknowledgement))
        .is_ok()
    {
        let _ = receiver.recv_timeout(Duration::from_secs(3));
    }
}

fn run(
    receiver: mpsc::Receiver<Command>,
    commands: mpsc::Sender<Command>,
    active: Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>,
    subscribers: Arc<Mutex<Vec<mpsc::Sender<DownloadManagerEvent>>>>,
) {
    let mut queued = VecDeque::<Request>::new();
    let mut concurrency = 2_usize;
    let mut network_available = true;
    let mut authentication_available = false;
    let mut removing = HashSet::<String>::new();
    let mut shutdown_acknowledgement = None::<mpsc::Sender<()>>;
    loop {
        let command = match receiver.recv_timeout(Duration::from_millis(250)) {
            Ok(command) => Some(command),
            Err(mpsc::RecvTimeoutError::Timeout) => None,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
        if let Some(command) = command {
            match command {
                Command::Enqueue(request) => {
                    authentication_available = true;
                    if !queued.iter().any(|queued| queued.id == request.id)
                        && !active
                            .lock()
                            .is_ok_and(|active| active.contains_key(&request.id))
                    {
                        persist_queued(&request);
                        queued.push_back(request);
                    }
                }
                Command::Pause(id) => {
                    if let Some(position) = queued.iter().position(|request| request.id == id)
                        && let Some(request) = queued.remove(position)
                    {
                        persist_state(&request, DownloadState::Paused);
                        let _ = request.listener.send(DownloadEvent::Cancelled);
                    } else {
                        cancel_worker(&id);
                    }
                }
                Command::Resume {
                    id,
                    access_token,
                    reset_retry,
                } => {
                    authentication_available = true;
                    if let Some(request) = queued.iter_mut().find(|request| request.id == id) {
                        if reset_retry {
                            request.access_token = access_token;
                            request.retry_started = None;
                            request.retry_attempt = 0;
                            request.ready_at = Instant::now();
                            set_waiting_status(&request.id, None);
                            persist_queued(request);
                        }
                    } else if !active.lock().is_ok_and(|active| active.contains_key(&id))
                        && let Some(mut request) = request_from_job(&id, access_token)
                    {
                        if reset_retry {
                            request.retry_started = None;
                            request.retry_attempt = 0;
                            request.ready_at = Instant::now();
                        }
                        persist_queued(&request);
                        insert_in_queue_order(&mut queued, request);
                    }
                }
                Command::Remove(id) => {
                    if active.lock().is_ok_and(|active| active.contains_key(&id)) {
                        removing.insert(id.clone());
                        cancel_worker(&id);
                    } else {
                        queued.retain(|request| request.id != id);
                        cleanup_job(&id);
                    }
                }
                Command::Terminal(mut request, event) => {
                    if let Ok(mut active) = active.lock() {
                        active.remove(&request.id);
                    }
                    if removing.remove(&request.id) {
                        cleanup_job(&request.id);
                        let _ = request.listener.send(DownloadEvent::Cancelled);
                    } else if shutdown_acknowledgement.is_some()
                        && !matches!(event, DownloadEvent::Complete { .. })
                    {
                        persist_queued(&request);
                        let _ = request.listener.send(DownloadEvent::Cancelled);
                    } else if let DownloadEvent::Failed(failure) = &event
                        && should_refresh_manifest(failure.kind, request.manifest_refresh_attempted)
                    {
                        request.manifest_refresh_attempted = true;
                        persist_queued(&request);
                        set_waiting_status(&request.id, Some("Refreshing GOG file manifest"));
                        let failure = failure.clone();
                        let commands = commands.clone();
                        std::thread::spawn(move || {
                            let result = refresh_request_manifest(&request);
                            let _ =
                                commands.send(Command::ManifestRefreshed(request, failure, result));
                        });
                    } else if !authentication_available
                        || matches!(&event, DownloadEvent::Failed(failure) if failure.kind == DownloadFailureKind::Authentication)
                    {
                        authentication_available = false;
                        persist_queued(&request);
                        set_waiting_status(&request.id, Some("Authentication required"));
                        publish(&subscribers, DownloadManagerEvent::AuthenticationRequired);
                        queued.push_back(request);
                    } else if let DownloadEvent::Failed(failure) = &event
                        && failure.kind == DownloadFailureKind::TransientNetwork
                        && request
                            .retry_started
                            .get_or_insert_with(Instant::now)
                            .elapsed()
                            < Duration::from_secs(300)
                    {
                        request.retry_attempt += 1;
                        let delay = 2_u64.saturating_pow(request.retry_attempt.min(5)).min(60);
                        request.ready_at = Instant::now() + Duration::from_secs(delay);
                        persist_queued(&request);
                        queued.push_back(request);
                    } else {
                        if matches!(event, DownloadEvent::Complete { .. }) {
                            publish(
                                &subscribers,
                                DownloadManagerEvent::ManagedFilesChanged(
                                    request.artifacts[0].product_id,
                                ),
                            );
                        }
                        let _ = request.listener.send(event);
                    }
                }
                Command::ManifestRefreshed(mut request, mut failure, result) => match result {
                    Ok(artifacts) => {
                        let old_id = request.id.clone();
                        request.artifacts = artifacts;
                        let refs = request.artifacts.iter().collect::<Vec<_>>();
                        request.id = job_id(&refs);
                        if request.id != old_id {
                            // A refreshed mutable GOG slot can represent different bytes. Never
                            // append those bytes to partial files belonging to the old revision.
                            cleanup_job(&old_id);
                        }
                        if queued.iter().any(|queued| queued.id == request.id)
                            || active
                                .lock()
                                .is_ok_and(|active| active.contains_key(&request.id))
                        {
                            failure.message =
                                "The refreshed GOG revision is already in the download queue"
                                    .to_owned();
                            let _ = request.listener.send(DownloadEvent::Failed(failure));
                            continue;
                        }
                        request.handle = Arc::new(AtomicBool::new(false));
                        request.ready_at = Instant::now();
                        persist_queued(&request);
                        set_waiting_status(
                            &request.id,
                            Some("Manifest refreshed; waiting to retry"),
                        );
                        insert_in_queue_order(&mut queued, request);
                    }
                    Err(error) => {
                        failure.message = format!(
                            "{}; refreshing this product's GOG manifest did not recover the download: {error}",
                            failure.message
                        );
                        if let Ok(store) = StateStore::open() {
                            let _ = store.set_download_job_failure(&request.id, &failure.message);
                        }
                        let _ = request.listener.send(DownloadEvent::Failed(failure));
                    }
                },
                Command::SetConcurrency(limit) => concurrency = limit,
                Command::Recover(token) => {
                    authentication_available = true;
                    for request in &mut queued {
                        request.access_token.clone_from(&token);
                        set_waiting_status(&request.id, None);
                    }
                    recover_jobs(&mut queued, &active, token);
                }
                Command::SetNetwork(available) => {
                    if available && !network_available {
                        for request in &mut queued {
                            request.ready_at = Instant::now();
                            request.retry_started = None;
                            request.retry_attempt = 0;
                        }
                    }
                    network_available = available;
                    if let Ok(store) = StateStore::open() {
                        let _ = store.set_queued_download_status(
                            (!available).then_some("Waiting for network"),
                        );
                    }
                    for request in &queued {
                        set_waiting_status(
                            &request.id,
                            (!available).then_some("Waiting for network"),
                        );
                    }
                }
                Command::SetAuthentication(available) => {
                    authentication_available = available;
                    if !available {
                        if let Ok(store) = StateStore::open() {
                            let _ = store.set_queued_download_status(Some(if network_available {
                                "Authentication required"
                            } else {
                                "Waiting for network"
                            }));
                        }
                        if let Ok(active) = active.lock() {
                            for id in active.keys() {
                                cancel_worker(id);
                            }
                        }
                        for request in &queued {
                            set_waiting_status(
                                &request.id,
                                Some(if network_available {
                                    "Authentication required"
                                } else {
                                    "Waiting for network"
                                }),
                            );
                        }
                    }
                }
                Command::Shutdown(acknowledgement) => {
                    shutdown_acknowledgement = Some(acknowledgement);
                    network_available = false;
                    if let Ok(active) = active.lock() {
                        for id in active.keys() {
                            cancel_worker(id);
                        }
                    }
                }
            }
            publish_queue_snapshot(&subscribers);
        }
        if shutdown_acknowledgement.is_some() {
            if active.lock().is_ok_and(|active| active.is_empty()) {
                if let Some(acknowledgement) = shutdown_acknowledgement.take() {
                    let _ = acknowledgement.send(());
                }
                break;
            }
            continue;
        }
        if network_available && authentication_available {
            schedule(&mut queued, concurrency, &commands, &active, &subscribers);
        }
    }
}

fn schedule(
    queued: &mut VecDeque<Request>,
    concurrency: usize,
    commands: &mpsc::Sender<Command>,
    active: &Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>,
    subscribers: &Arc<Mutex<Vec<mpsc::Sender<DownloadManagerEvent>>>>,
) {
    loop {
        // Only one logical game/artifact group is active. The configured limit is used by that
        // worker for multipart file concurrency.
        if active.lock().map_or(true, |active| !active.is_empty()) {
            return;
        }
        let Some(position) = queued
            .iter()
            .position(|request| request.ready_at <= Instant::now())
        else {
            return;
        };
        let Some(mut request) = queued.remove(position) else {
            return;
        };
        if let Err(error) = ensure_download_directory(&request.destination) {
            set_waiting_status(
                &request.id,
                Some(&format!("Download directory unavailable: {error}")),
            );
            request.ready_at = Instant::now() + Duration::from_secs(2);
            queued.insert(position, request);
            return;
        }
        set_waiting_status(&request.id, None);
        persist_state(&request, DownloadState::Downloading);
        let (worker_sender, worker_receiver) = mpsc::channel();
        let worker = start_worker(
            request.artifacts.clone(),
            request.title.clone(),
            request.access_token.clone(),
            request.destination.clone(),
            concurrency,
            worker_sender,
        );
        if let Ok(mut active) = active.lock() {
            active.insert(request.id.clone(), worker);
        }
        let id = request.id.clone();
        let listener = request.listener.clone();
        let requested_pause = request.handle.clone();
        let terminal_request = request.clone();
        let commands = commands.clone();
        let event_subscribers = subscribers.clone();
        std::thread::spawn(move || {
            loop {
                if requested_pause.load(Ordering::Relaxed) {
                    cancel_worker(&id);
                }
                let event =
                    match worker_receiver.recv_timeout(std::time::Duration::from_millis(100)) {
                        Ok(event) => event,
                        Err(mpsc::RecvTimeoutError::Timeout) => continue,
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    };
                let terminal = matches!(
                    event,
                    DownloadEvent::Complete { .. }
                        | DownloadEvent::Cancelled
                        | DownloadEvent::Failed(_)
                );
                if terminal {
                    let _ = commands.send(Command::Terminal(terminal_request.clone(), event));
                    break;
                }
                if let DownloadEvent::Progress { downloaded, total } = &event {
                    publish(
                        &event_subscribers,
                        DownloadManagerEvent::Progress {
                            job_id: id.clone(),
                            downloaded: *downloaded,
                            total: *total,
                        },
                    );
                }
                let _ = listener.send(event);
            }
        });
        publish_queue_snapshot(subscribers);
    }
}

fn publish(
    subscribers: &Arc<Mutex<Vec<mpsc::Sender<DownloadManagerEvent>>>>,
    event: DownloadManagerEvent,
) {
    if let Ok(mut subscribers) = subscribers.lock() {
        subscribers.retain(|subscriber| subscriber.send(event.clone()).is_ok());
    }
}

fn publish_queue_snapshot(subscribers: &Arc<Mutex<Vec<mpsc::Sender<DownloadManagerEvent>>>>) {
    if let Ok(store) = StateStore::open()
        && let Ok(jobs) = store.download_jobs()
    {
        publish(subscribers, DownloadManagerEvent::QueueSnapshot(jobs));
    }
}

fn ensure_download_directory(destination: &std::path::Path) -> std::io::Result<()> {
    fs::create_dir_all(destination)?;
    let probe = destination.join(crate::identity::WRITE_PROBE);
    fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&probe)?;
    fs::remove_file(probe)
}

fn persist_queued(request: &Request) {
    persist_state(request, DownloadState::Queued);
}

fn persist_state(request: &Request, state: DownloadState) {
    let total = request
        .artifacts
        .iter()
        .map(|artifact| artifact.size_bytes)
        .collect::<Option<Vec<_>>>()
        .map(|sizes| sizes.into_iter().sum());
    if let Ok(store) = StateStore::open() {
        let _ = store.save_download_job(&DownloadJobUpdate {
            job_id: &request.id,
            product_id: request.artifacts[0].product_id,
            title: &request.title,
            artifacts: &request.artifacts,
            destination: &request.destination,
            state,
            bytes_downloaded: 0,
            total_bytes: total,
            completed_files: &[],
            error: None,
        });
    }
}

fn set_waiting_status(id: &str, message: Option<&str>) {
    if let Ok(store) = StateStore::open() {
        let _ = store.set_download_job_status(id, message);
    }
}

fn recover_jobs(
    queued: &mut VecDeque<Request>,
    active: &Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>,
    access_token: String,
) {
    let Ok(store) = StateStore::open() else {
        return;
    };
    let Ok(jobs) = store.download_jobs() else {
        return;
    };
    for job in jobs {
        if !matches!(
            job.state,
            DownloadState::Queued | DownloadState::Downloading
        ) || job.artifacts.is_empty()
            || queued.iter().any(|request| request.id == job.job_id)
            || active
                .lock()
                .is_ok_and(|active| active.contains_key(&job.job_id))
        {
            continue;
        }
        let downloaded = recovered_bytes(&job);
        let _ = store.recover_download_job(&job.job_id, downloaded);
        let (listener, _) = mpsc::channel();
        queued.push_back(Request {
            id: job.job_id,
            artifacts: job.artifacts,
            title: job.title,
            access_token: access_token.clone(),
            destination: job.destination,
            listener,
            handle: Arc::new(AtomicBool::new(false)),
            retry_started: None,
            retry_attempt: 0,
            ready_at: Instant::now(),
            queue_position: job.queue_position,
            manifest_refresh_attempted: false,
        });
    }
}

fn recovered_bytes(job: &crate::state::DownloadJobRecord) -> u64 {
    let completed = job
        .completed_files
        .iter()
        .filter_map(|path| path.metadata().ok())
        .map(|metadata| metadata.len())
        .sum::<u64>();
    if job.artifacts.is_empty() {
        return completed;
    }
    let staging = staging_directory(&job.destination, &job.artifacts, &job.job_id);
    let staged = fs::read_dir(staging)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter_map(|entry| entry.metadata().ok())
        .filter(|metadata| metadata.is_file())
        .map(|metadata| metadata.len())
        .sum::<u64>();
    completed + staged
}

fn request_from_job(id: &str, access_token: String) -> Option<Request> {
    let job = StateStore::open().ok()?.download_job(id).ok()??;
    if job.artifacts.is_empty() || job.state == DownloadState::Complete {
        return None;
    }
    let (listener, _) = mpsc::channel();
    Some(Request {
        id: job.job_id,
        artifacts: job.artifacts,
        title: job.title,
        access_token,
        destination: job.destination,
        listener,
        handle: Arc::new(AtomicBool::new(false)),
        retry_started: None,
        retry_attempt: 0,
        ready_at: Instant::now(),
        queue_position: job.queue_position,
        manifest_refresh_attempted: false,
    })
}

fn refresh_request_manifest(request: &Request) -> Result<Vec<RemoteArtifact>, String> {
    let first = request
        .artifacts
        .first()
        .ok_or_else(|| "the queued download has no artifacts".to_owned())?;
    let group_id = first
        .provider_group_id
        .as_deref()
        .ok_or_else(|| "the queued download has no official GOG group ID".to_owned())?;
    let category = first
        .provider_category
        .ok_or_else(|| "the queued download has no official GOG category".to_owned())?;
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .user_agent(crate::identity::USER_AGENT)
        .build()
        .map_err(|error| error.to_string())?;
    let product = crate::gog::product::fetch(&client, first.product_id)
        .map_err(|error| format!("could not fetch product {}: {error}", first.product_id))?;
    let all_artifacts = crate::gog::product::download_artifacts(first.product_id, &product);
    if let Ok(store) = StateStore::open() {
        store
            .observe_download_manifest(first.product_id, &all_artifacts)
            .map_err(|error| format!("could not store the refreshed manifest: {error}"))?;
        store
            .cache_download_manifest(first.product_id, &all_artifacts)
            .map_err(|error| format!("could not cache the refreshed manifest: {error}"))?;
    }
    let replacement = all_artifacts
        .into_iter()
        .filter(|artifact| {
            artifact.provider_group_id.as_deref() == Some(group_id)
                && artifact.provider_category == Some(category)
        })
        .collect::<Vec<_>>();
    if replacement.is_empty() {
        Err(format!(
            "GOG no longer offers group {group_id} for this product"
        ))
    } else {
        Ok(replacement)
    }
}

fn should_refresh_manifest(kind: DownloadFailureKind, attempted: bool) -> bool {
    kind == DownloadFailureKind::ManifestChanged && !attempted
}

fn insert_in_queue_order(queued: &mut VecDeque<Request>, request: Request) {
    let positions = queued
        .iter()
        .map(|queued| queued.queue_position)
        .collect::<Vec<_>>();
    let position = queue_insertion_index(&positions, request.queue_position);
    queued.insert(position, request);
}

fn queue_insertion_index(queued: &[Option<i64>], position: Option<i64>) -> usize {
    queued
        .iter()
        .position(|queued| match (queued, position) {
            (None, Some(_)) => true,
            (Some(queued), Some(position)) => *queued > position,
            _ => false,
        })
        .unwrap_or(queued.len())
}

fn cleanup_job(id: &str) {
    let Ok(store) = StateStore::open() else {
        return;
    };
    let Ok(Some(job)) = store.download_job(id) else {
        return;
    };
    if job.state != DownloadState::Complete {
        let _ = delete_completed_files(&job.destination, &job.completed_files);
        if !job.artifacts.is_empty() {
            let staging = staging_directory(&job.destination, &job.artifacts, id);
            if staging.is_dir() {
                let _ = fs::remove_dir_all(staging);
            }
        }
    }
    let _ = store.delete_download_job(id);
}

#[cfg(test)]
mod tests {
    use super::{queue_insertion_index, should_refresh_manifest};
    use crate::download::DownloadFailureKind;

    #[test]
    fn resumed_job_returns_to_its_persisted_queue_position() {
        assert_eq!(queue_insertion_index(&[Some(1), Some(3), None], Some(2)), 1);
        assert_eq!(queue_insertion_index(&[Some(1), Some(2)], Some(3)), 2);
        assert_eq!(queue_insertion_index(&[Some(1), Some(2)], None), 2);
    }

    #[test]
    fn stale_manifest_is_refreshed_only_once() {
        assert!(should_refresh_manifest(
            DownloadFailureKind::ManifestChanged,
            false
        ));
        assert!(!should_refresh_manifest(
            DownloadFailureKind::ManifestChanged,
            true
        ));
        assert!(!should_refresh_manifest(
            DownloadFailureKind::TransientNetwork,
            false
        ));
    }
}
