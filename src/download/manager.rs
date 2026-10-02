use super::{
    DownloadEvent, DownloadFailure, DownloadFailureKind, DownloadManagerEvent, cancel_worker,
    delete_completed_files, staging_directory, start_worker, worker_is_active,
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
    library_id: String,
    session: u64,
    recovery_generation: u64,
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
    PauseForSignOut(mpsc::Sender<()>),
    Quiesce(Vec<i64>, mpsc::Sender<anyhow::Result<()>>),
    RetainInstallers {
        product_id: i64,
        generation: u64,
        retention: super::cleanup::Retention,
        job_id: String,
        session: u64,
    },
    RetentionFailed {
        product_id: i64,
        generation: u64,
        job_id: String,
        session: u64,
        message: String,
    },
    DeleteManaged(
        super::ManagedDownloads,
        bool,
        mpsc::Sender<anyhow::Result<super::CleanupResult>>,
    ),
    Enqueue(Request),
    EnqueueBackup(
        super::DownloadRequest,
        u64,
        u64,
        mpsc::Sender<anyhow::Result<()>>,
    ),
    EnqueueWithInstall(
        Vec<super::DownloadRequest>,
        Option<super::AutoInstallRequest>,
        u64,
        Vec<(i64, u64)>,
        mpsc::Sender<anyhow::Result<usize>>,
    ),
    RetryInstall(i64, mpsc::Sender<anyhow::Result<()>>),
    Pause(String),
    Resume {
        id: String,
        access_token: String,
        reset_retry: bool,
        session: u64,
    },
    Remove(String),
    Terminal(Request, DownloadEvent),
    ManifestRefreshed(
        Request,
        DownloadFailure,
        Result<Vec<RemoteArtifact>, String>,
    ),
    SetConcurrency(usize),
    Recover(String, u64),
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
                HashMap::new(),
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

pub(super) fn enqueue_backup(request: super::DownloadRequest, session: u64) -> anyhow::Result<()> {
    let (reply, result) = mpsc::channel();
    let generation = crate::installation::recovery::generation(request.artifacts[0].product_id);
    manager()
        .commands
        .send(Command::EnqueueBackup(request, session, generation, reply))?;
    result.recv()?
}

pub(super) fn check_retention(product_id: i64, token: &str, session: u64) -> anyhow::Result<()> {
    let generation = crate::installation::recovery::generation(product_id);
    for job in StateStore::open()?
        .download_jobs()?
        .into_iter()
        .filter(|job| {
            job.product_id == product_id
                && job.state == DownloadState::Complete
                && !job.artifacts.is_empty()
                && job
                    .artifacts
                    .iter()
                    .all(|artifact| artifact.kind == crate::domain::ArtifactKind::Installer)
        })
    {
        if let Some(retention) = super::cleanup::prepare_retention(
            product_id,
            &job.job_id,
            &job.artifacts,
            &job.completed_files,
            token,
            session,
        )? {
            manager().commands.send(Command::RetainInstallers {
                product_id,
                generation,
                retention,
                job_id: job.job_id,
                session,
            })?;
        }
    }
    Ok(())
}

pub(super) fn enqueue(request: super::DownloadRequest) -> Arc<AtomicBool> {
    let request = request_from_download(request, crate::online::account_session());
    let handle = request.handle.clone();
    let _ = manager().commands.send(Command::Enqueue(request));
    handle
}

fn request_from_download(request: super::DownloadRequest, session: u64) -> Request {
    let super::DownloadRequest {
        library_id,
        artifacts,
        title,
        access_token,
        destination,
        events: listener,
    } = request;
    let refs = artifacts.iter().collect::<Vec<_>>();
    let id = super::job_id_at(&refs, &destination);
    let handle = Arc::new(AtomicBool::new(false));
    Request {
        id,
        library_id,
        session,
        recovery_generation: crate::installation::recovery::generation(artifacts[0].product_id),
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
    }
}

fn validate_request(request: &Request) -> anyhow::Result<()> {
    validate_destination(
        &request.artifacts,
        &request.destination,
        (!request.library_id.is_empty()).then_some(request.library_id.as_str()),
    )
}

pub(super) fn validate_destination(
    artifacts: &[RemoteArtifact],
    destination: &std::path::Path,
    id: Option<&str>,
) -> anyhow::Result<()> {
    let first = artifacts
        .first()
        .ok_or_else(|| anyhow::anyhow!("A download group contains no files"))?;
    let kind = crate::storage::artifact_library_kind(first);
    anyhow::ensure!(
        artifacts
            .iter()
            .all(|artifact| crate::storage::artifact_library_kind(artifact) == kind),
        "Download group mixes library types"
    );
    let config = crate::storage::read_config()?;
    let library = crate::storage::validate_path(&config, kind, destination)?;
    anyhow::ensure!(
        id.is_none_or(|id| id == library.id),
        "The selected download library changed; choose its destination again"
    );
    Ok(())
}

pub(super) fn enqueue_with_install(
    requests: Vec<super::DownloadRequest>,
    install: Option<super::AutoInstallRequest>,
    session: u64,
) -> anyhow::Result<usize> {
    anyhow::ensure!(
        requests.iter().all(|request| !request.artifacts.is_empty()),
        "A download group contains no files"
    );
    let (sender, receiver) = mpsc::channel();
    let mut stamps = requests
        .iter()
        .map(|request| {
            let id = request.artifacts[0].product_id;
            (id, crate::installation::recovery::generation(id))
        })
        .collect::<Vec<_>>();
    if let Some(choice) = &install {
        stamps.push((
            choice.product_id,
            crate::installation::recovery::generation(choice.product_id),
        ));
    }
    manager()
        .commands
        .send(Command::EnqueueWithInstall(
            requests, install, session, stamps, sender,
        ))
        .map_err(|_| anyhow::anyhow!("Download manager is unavailable"))?;
    receiver
        .recv()
        .map_err(|_| anyhow::anyhow!("Download registration stopped"))?
}

pub(crate) fn quiesce_recovery(ids: &[i64], cancel: &AtomicBool) -> anyhow::Result<()> {
    let (reply, receiver) = mpsc::channel();
    manager()
        .commands
        .send(Command::Quiesce(ids.to_vec(), reply))?;
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        match receiver.recv_timeout(Duration::from_millis(50)) {
            Ok(result) => return result,
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                anyhow::bail!("Download manager stopped while pausing this game")
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        anyhow::ensure!(
            !cancel.load(Ordering::Relaxed),
            "Recovery cancelled while stopping downloads"
        );
        anyhow::ensure!(
            Instant::now() < deadline,
            "Downloads are still stopping; no game files were removed. Retry when they stop"
        );
    }
}

pub(super) fn retry_install_after_download(product_id: i64) -> anyhow::Result<()> {
    let (sender, receiver) = mpsc::channel();
    manager()
        .commands
        .send(Command::RetryInstall(product_id, sender))
        .map_err(|_| anyhow::anyhow!("Download manager is unavailable"))?;
    receiver
        .recv()
        .map_err(|_| anyhow::anyhow!("Installation retry stopped"))?
}

pub(super) fn delete_managed_downloads(
    files: super::ManagedDownloads,
    after_uninstall: bool,
) -> anyhow::Result<super::CleanupResult> {
    let (reply, result) = mpsc::channel();
    manager()
        .commands
        .send(Command::DeleteManaged(files, after_uninstall, reply))
        .map_err(|_| anyhow::anyhow!("Download manager unavailable"))?;
    result
        .recv()
        .map_err(|_| anyhow::anyhow!("Downloaded file cleanup stopped"))?
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
            session: crate::online::account_session(),
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
    let _ = manager().commands.send(Command::Recover(
        access_token,
        crate::online::account_session(),
    ));
}

pub(super) fn set_network(available: bool) {
    let _ = manager().commands.send(Command::SetNetwork(available));
}

pub(super) fn set_authenticated(authenticated: bool) {
    let _ = manager()
        .commands
        .send(Command::SetAuthentication(authenticated));
}

pub(super) fn pause_for_sign_out() -> anyhow::Result<()> {
    let (reply, result) = mpsc::channel();
    manager().commands.send(Command::PauseForSignOut(reply))?;
    result.recv_timeout(Duration::from_secs(45))
        .map_err(|_| anyhow::anyhow!("Signed out. Downloads are still stopping; retry profile reset shortly. Partial files were kept."))
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
    mut active_products: HashMap<String, i64>,
) {
    let mut queued = VecDeque::<Request>::new();
    let mut concurrency = 2_usize;
    let mut network_available = true;
    let mut authentication_available = false;
    let mut removing = HashSet::<String>::new();
    let mut shutdown_acknowledgement = None::<mpsc::Sender<()>>;
    let mut pause_acknowledgements = Vec::new();
    let mut quiescence = Vec::<(HashSet<String>, mpsc::Sender<anyhow::Result<()>>)>::new();
    loop {
        let command = match receiver.recv_timeout(Duration::from_millis(250)) {
            Ok(command) => Some(command),
            Err(mpsc::RecvTimeoutError::Timeout) => None,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
        if let Some(command) = command {
            match command {
                Command::PauseForSignOut(reply) => {
                    authentication_available = false;
                    for handle in active.lock().unwrap().values() {
                        handle.store(true, Ordering::Relaxed);
                    }
                    pause_acknowledgements.push(reply);
                }
                Command::Quiesce(ids, reply) => {
                    let result = StateStore::open().and_then(|store| store.download_jobs());
                    match result {
                        Err(error) => {
                            let _ = reply.send(Err(error));
                        }
                        Ok(jobs) => {
                            let mut jobs = jobs
                                .iter()
                                .filter(|job| ids.contains(&job.product_id))
                                .map(|job| job.job_id.clone())
                                .collect::<HashSet<_>>();
                            jobs.extend(
                                active_products
                                    .iter()
                                    .filter(|(_, id)| ids.contains(id))
                                    .map(|(job, _)| job.clone()),
                            );
                            queued
                                .retain(|request| !ids.contains(&request.artifacts[0].product_id));
                            for id in &jobs {
                                if let Some(cancel) = active.lock().unwrap().get(id) {
                                    cancel.store(true, Ordering::Relaxed);
                                }
                                cancel_worker(id);
                            }
                            quiescence.push((jobs, reply));
                        }
                    }
                }
                Command::RetentionFailed {
                    product_id,
                    generation,
                    job_id,
                    session,
                    message,
                } => {
                    if !crate::installation::recovery::current(product_id, generation) {
                        continue;
                    }
                    let _ = crate::online::with_account_session(session, || {
                        let _admission = crate::installation::recovery::admit_generation(
                            product_id, generation,
                        )?;
                        StateStore::open()?.set_download_job_status(
                            &job_id,
                            Some(&format!("Installer cleanup deferred: {message}")),
                        )
                    });
                    publish_queue_snapshot(&subscribers);
                }
                Command::RetainInstallers {
                    product_id,
                    generation,
                    retention,
                    job_id,
                    session,
                } => {
                    if !crate::installation::recovery::current(product_id, generation) {
                        continue;
                    }
                    let result = crate::online::with_account_session(session, || {
                        let _admission = crate::installation::recovery::admit_generation(
                            product_id, generation,
                        )?;
                        anyhow::ensure!(shutdown_acknowledgement.is_none(), "Ludomere is closing");
                        let _activity =
                            crate::profile_reset::begin_activity("installer retention commit")?;
                        super::cleanup::commit_retention(&StateStore::open()?, retention, &job_id)
                    });
                    let message = match result {
                        Ok(result) => format!(
                            "Installer cleanup: {} moved to Trash{}",
                            result.deleted,
                            if result.failures.is_empty() {
                                String::new()
                            } else {
                                format!("; {}", result.failures.join("; "))
                            }
                        ),
                        Err(error) => format!("Installer cleanup deferred: {error}"),
                    };
                    let _ = crate::online::with_account_session(session, || {
                        StateStore::open()?.set_download_job_status(&job_id, Some(&message))
                    });
                    publish_queue_snapshot(&subscribers);
                }
                Command::EnqueueBackup(request, session, generation, reply) => {
                    let result = crate::online::with_account_session(session, || {
                        let _activity = crate::profile_reset::begin_activity("backup scheduling")?;
                        anyhow::ensure!(shutdown_acknowledgement.is_none(), "Ludomere is closing");
                        let store = StateStore::open()?;
                        let mut request = request_from_download(request, session);
                        validate_request(&request)?;
                        let config = crate::storage::read_config()?;
                        let allowed =
                            match crate::storage::artifact_library_kind(&request.artifacts[0]) {
                                crate::config::LibraryKind::OfflineInstallers => {
                                    crate::updates::UpdatePolicy::resolve(
                                        &config,
                                        store
                                            .game_preferences(request.artifacts[0].product_id)?
                                            .as_ref(),
                                    )
                                    .auto_download_offline_installer
                                }
                                crate::config::LibraryKind::Extras => config.auto_download_extras,
                                crate::config::LibraryKind::GameFiles => false,
                            };
                        anyhow::ensure!(
                            allowed,
                            "Automatic archive updates were disabled before registration"
                        );
                        request.id = super::resolve_job_id(
                            &request.artifacts.iter().collect::<Vec<_>>(),
                            &request.destination,
                        )?;
                        let _admission = crate::installation::recovery::admit_generation(
                            request.artifacts[0].product_id,
                            generation,
                        )?;
                        let id = request.artifacts[0].product_id;
                        anyhow::ensure!(
                            !crate::installation::is_game_running(id),
                            "The game started while checking updates; retry after it closes"
                        );
                        anyhow::ensure!(!store.download_install_intents()?.iter().any(|intent| intent.product_id == id && intent.state != "complete"), "A pending installation uses these downloads");
                        if !queued.iter().any(|other| other.id == request.id)
                            && !active
                                .lock()
                                .is_ok_and(|active| active.contains_key(&request.id))
                        {
                            save_request(&store, &request, DownloadState::Queued)?;
                            queued.push_back(request);
                        }
                        authentication_available = true;
                        Ok(())
                    });
                    let _ = reply.send(result);
                }
                Command::DeleteManaged(files, after_uninstall, reply) => {
                    let product_id = files.product_id;
                    let result = StateStore::open()
                        .and_then(|store| super::cleanup::delete(&store, files, after_uninstall));
                    publish(
                        &subscribers,
                        DownloadManagerEvent::ManagedFilesChanged(product_id),
                    );
                    let _ = reply.send(result);
                }
                Command::EnqueueWithInstall(requests, choice, session, stamps, reply) => {
                    let result = crate::online::with_account_session(
                        session,
                        || -> anyhow::Result<usize> {
                            anyhow::ensure!(
                                shutdown_acknowledgement.is_none(),
                                "Ludomere is closing; retry after restarting"
                            );
                            let store = StateStore::open()?;
                            for request in &requests {
                                validate_destination(
                                    &request.artifacts,
                                    &request.destination,
                                    Some(&request.library_id),
                                )?;
                            }
                            let intent = choice
                                .as_ref()
                                .map(|choice| {
                                    super::auto_install::intent(&store, &requests, choice)
                                })
                                .transpose()?
                                .flatten();
                            let prepared = requests
                                .iter()
                                .map(|request| {
                                    request_from_download(
                                        super::DownloadRequest {
                                            library_id: request.library_id.clone(),
                                            artifacts: request.artifacts.clone(),
                                            title: request.title.clone(),
                                            access_token: request.access_token.clone(),
                                            destination: request.destination.clone(),
                                            events: request.events.clone(),
                                        },
                                        session,
                                    )
                                })
                                .collect::<Vec<_>>();
                            let prepared = prepared
                                .into_iter()
                                .map(|mut request| {
                                    request.id = super::resolve_job_id(
                                        &request.artifacts.iter().collect::<Vec<_>>(),
                                        &request.destination,
                                    )?;
                                    Ok(request)
                                })
                                .collect::<anyhow::Result<Vec<_>>>()?;
                            let _admission = crate::installation::recovery::admit_stamps(&stamps)?;
                            super::auto_install::clear_for_requests(&store, &requests)?;
                            if let Some(choice) = choice {
                                store.clear_download_install_intent(choice.product_id)?;
                            }
                            if let Some(intent) = intent {
                                store.save_download_install_intent(&intent)?;
                            }
                            let count = requests.len();
                            for request in prepared {
                                if !queued.iter().any(|queued| queued.id == request.id)
                                    && !active
                                        .lock()
                                        .is_ok_and(|active| active.contains_key(&request.id))
                                    && !store.download_job(&request.id)?.is_some_and(|job| {
                                        job.state == DownloadState::Complete
                                            && !job.completed_files.is_empty()
                                            && job.completed_files.iter().all(|path| path.is_file())
                                    })
                                {
                                    save_request(&store, &request, DownloadState::Queued)?;
                                    queued.push_back(request);
                                }
                            }
                            authentication_available = true;
                            Ok(count)
                        },
                    )
                    .and_then(|count| {
                        super::auto_install::process(&StateStore::open()?)?;
                        Ok(count)
                    });
                    let _ = reply.send(result);
                }
                Command::RetryInstall(product_id, reply) => {
                    let result = (|| -> anyhow::Result<()> {
                        anyhow::ensure!(shutdown_acknowledgement.is_none(), "Ludomere is closing");
                        let store = StateStore::open()?;
                        let intent = store
                            .download_install_intents()?
                            .into_iter()
                            .find(|intent| {
                                intent.product_id == product_id && intent.state == "blocked"
                            })
                            .ok_or_else(|| {
                                anyhow::anyhow!("No blocked automatic installation was found")
                            })?;
                        store.set_download_install_state(&intent.intent_id, "waiting", None)?;
                        super::auto_install::process(&store)?;
                        Ok(())
                    })();
                    let _ = reply.send(result);
                }
                Command::Enqueue(mut request) => {
                    if let Err(error) = validate_request(&request).and_then(|()| {
                        request.id = super::resolve_job_id(
                            &request.artifacts.iter().collect::<Vec<_>>(),
                            &request.destination,
                        )?;
                        Ok(())
                    }) {
                        let _ = request
                            .listener
                            .send(DownloadEvent::Failed(DownloadFailure {
                                kind: DownloadFailureKind::Other,
                                message: error.to_string(),
                            }));
                        continue;
                    }
                    if request.session != crate::online::account_session() {
                        let _ = request.listener.send(DownloadEvent::Cancelled);
                        continue;
                    }
                    let Ok(_admission) = crate::installation::recovery::admit_generation(
                        request.artifacts[0].product_id,
                        request.recovery_generation,
                    ) else {
                        let _ = request.listener.send(DownloadEvent::Cancelled);
                        continue;
                    };
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
                    session,
                } => {
                    if session != crate::online::account_session() {
                        continue;
                    }
                    authentication_available = true;
                    if let Some(request) = queued.iter_mut().find(|request| request.id == id) {
                        let Ok(_admission) = crate::installation::recovery::admit_generation(
                            request.artifacts[0].product_id,
                            request.recovery_generation,
                        ) else {
                            continue;
                        };
                        if reset_retry {
                            request.access_token = access_token;
                            request.session = session;
                            request.handle = Arc::new(AtomicBool::new(false));
                            request.retry_started = None;
                            request.retry_attempt = 0;
                            request.ready_at = Instant::now();
                            set_waiting_status(&request.id, None);
                            persist_queued(request);
                        }
                    } else if !active.lock().is_ok_and(|active| active.contains_key(&id))
                        && let Some(mut request) = request_from_job(&id, access_token)
                    {
                        let Ok(_admission) = crate::installation::recovery::admit_generation(
                            request.artifacts[0].product_id,
                            request.recovery_generation,
                        ) else {
                            continue;
                        };
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
                    if StateStore::open()
                        .and_then(|store| store.clear_download_install_intent_for_job(&id))
                        .is_err()
                    {
                        set_waiting_status(
                            &id,
                            Some(
                                "Could not remove the saved installation request; retry removing this download",
                            ),
                        );
                        continue;
                    }
                    if active.lock().is_ok_and(|active| active.contains_key(&id)) {
                        removing.insert(id.clone());
                        cancel_worker(&id);
                    } else {
                        queued.retain(|request| request.id != id);
                        cleanup_job(&id);
                    }
                }
                Command::Terminal(mut request, event) => {
                    active_products.remove(&request.id);
                    if let Ok(mut active) = active.lock() {
                        active.remove(&request.id);
                    }
                    if !crate::installation::recovery::current(
                        request.artifacts[0].product_id,
                        request.recovery_generation,
                    ) {
                        if !matches!(event, DownloadEvent::Complete { .. }) {
                            persist_state(&request, DownloadState::Paused);
                        }
                        let _ = request.listener.send(DownloadEvent::Cancelled);
                    } else if removing.remove(&request.id) {
                        cleanup_job(&request.id);
                        let _ = request.listener.send(DownloadEvent::Cancelled);
                    } else if let DownloadEvent::Failed(failure) = &event
                        && failure.kind == DownloadFailureKind::Bookkeeping
                    {
                        if request.session == crate::online::account_session() {
                            publish(
                                &subscribers,
                                DownloadManagerEvent::BookkeepingFailed {
                                    job_id: request.id.clone(),
                                    product_id: request.artifacts[0].product_id,
                                    message: failure.message.clone(),
                                    session: request.session,
                                },
                            );
                        }
                        let _ = request.listener.send(event);
                    } else if shutdown_acknowledgement.is_some()
                        && !matches!(event, DownloadEvent::Complete { .. })
                    {
                        persist_queued(&request);
                        let _ = request.listener.send(DownloadEvent::Cancelled);
                    } else if request.session != crate::online::account_session()
                        && !matches!(event, DownloadEvent::Complete { .. })
                    {
                        persist_queued(&request);
                        set_waiting_status(
                            &request.id,
                            Some("Signed out; resume after signing in"),
                        );
                        let _ = request.listener.send(DownloadEvent::Cancelled);
                    } else if let DownloadEvent::Failed(failure) = &event
                        && authentication_available
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
                    } else if (!authentication_available
                        && !matches!(event, DownloadEvent::Complete { .. }))
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
                            let follow_up = authentication_available
                                && shutdown_acknowledgement.is_none()
                                && request.session == crate::online::account_session();
                            if follow_up && let Ok(store) = StateStore::open() {
                                let _ = super::auto_install::process(&store);
                            }
                            if follow_up
                                && let DownloadEvent::Complete { files } = &event
                                && request.artifacts.iter().all(|artifact| {
                                    artifact.kind == crate::domain::ArtifactKind::Installer
                                })
                            {
                                let request = request.clone();
                                let files = files.clone();
                                let commands = commands.clone();
                                std::thread::spawn(
                                    move || match super::cleanup::prepare_retention(
                                        request.artifacts[0].product_id,
                                        &request.id,
                                        &request.artifacts,
                                        &files,
                                        &request.access_token,
                                        request.session,
                                    ) {
                                        Ok(Some(retention)) => {
                                            let _ = commands.send(Command::RetainInstallers {
                                                product_id: request.artifacts[0].product_id,
                                                generation: request.recovery_generation,
                                                retention,
                                                job_id: request.id,
                                                session: request.session,
                                            });
                                        }
                                        Ok(None) => {}
                                        Err(error) => {
                                            let _ = commands.send(Command::RetentionFailed {
                                                product_id: request.artifacts[0].product_id,
                                                generation: request.recovery_generation,
                                                job_id: request.id,
                                                session: request.session,
                                                message: crate::updates::status_error(&error),
                                            });
                                        }
                                    },
                                );
                            }
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
                Command::ManifestRefreshed(mut request, mut failure, result) => {
                    if request.session != crate::online::account_session()
                        || !crate::installation::recovery::current(
                            request.artifacts[0].product_id,
                            request.recovery_generation,
                        )
                    {
                        continue;
                    }
                    match result {
                        Ok(artifacts) => {
                            let old_id = request.id.clone();
                            let unchanged =
                                super::job_id(&request.artifacts.iter().collect::<Vec<_>>())
                                    == super::job_id(&artifacts.iter().collect::<Vec<_>>());
                            request.artifacts = artifacts;
                            let refs = request.artifacts.iter().collect::<Vec<_>>();
                            if !unchanged {
                                request.id = super::job_id_at(&refs, &request.destination);
                            }
                            if request.id != old_id {
                                // A refreshed mutable GOG slot can represent different bytes. Never
                                // append those bytes to partial files belonging to the old revision.
                                if StateStore::open()
                                    .and_then(|store| {
                                        super::auto_install::block_replaced_job(
                                            &store,
                                            &old_id,
                                            &request.id,
                                        )
                                    })
                                    .is_err()
                                {
                                    failure.message = "Could not update the saved installation request. Retry this download after restarting.".into();
                                    let _ = request.listener.send(DownloadEvent::Failed(failure));
                                    continue;
                                }
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
                                let _ =
                                    store.set_download_job_failure(&request.id, &failure.message);
                            }
                            let _ = request.listener.send(DownloadEvent::Failed(failure));
                        }
                    }
                }
                Command::SetConcurrency(limit) => concurrency = limit,
                Command::Recover(token, session) => {
                    if session != crate::online::account_session() {
                        continue;
                    }
                    authentication_available = true;
                    for request in &mut queued {
                        request.access_token.clone_from(&token);
                        request.session = session;
                        request.handle = Arc::new(AtomicBool::new(false));
                        set_waiting_status(&request.id, None);
                    }
                    recover_jobs(&mut queued, &active, token);
                    if let Ok(store) = StateStore::open() {
                        let _ = super::auto_install::process(&store);
                    }
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
        if active.lock().is_ok_and(|active| active.is_empty()) {
            for reply in pause_acknowledgements.drain(..) {
                let _ = reply.send(());
            }
        }
        quiescence.retain(|(jobs, reply)| {
            if active.lock().unwrap().keys().any(|id| jobs.contains(id)) {
                true
            } else {
                let _ = reply.send(Ok(()));
                false
            }
        });
        if network_available && authentication_available {
            schedule(
                &mut queued,
                concurrency,
                &commands,
                &active,
                &subscribers,
                &mut active_products,
            );
        }
    }
}

fn schedule(
    queued: &mut VecDeque<Request>,
    concurrency: usize,
    commands: &mpsc::Sender<Command>,
    active: &Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>,
    subscribers: &Arc<Mutex<Vec<mpsc::Sender<DownloadManagerEvent>>>>,
    active_products: &mut HashMap<String, i64>,
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
        if request.session != crate::online::account_session() {
            persist_queued(&request);
            set_waiting_status(&request.id, Some("Signed out; resume after signing in"));
            continue;
        }
        let Ok(_admission) = crate::installation::recovery::admit_generation(
            request.artifacts[0].product_id,
            request.recovery_generation,
        ) else {
            continue;
        };
        if let Err(error) = validate_request(&request) {
            persist_state(&request, DownloadState::Paused);
            set_waiting_status(
                &request.id,
                Some(&format!(
                    "Library requires attention: {error}. Correct Storage and resume this download."
                )),
            );
            let _ = request
                .listener
                .send(DownloadEvent::Failed(DownloadFailure {
                    kind: DownloadFailureKind::Other,
                    message: error.to_string(),
                }));
            continue;
        }
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
            request.id.clone(),
            request.artifacts.clone(),
            request.title.clone(),
            request.access_token.clone(),
            request.destination.clone(),
            concurrency,
            request.session,
            worker_sender,
        );
        if let Ok(mut active) = active.lock() {
            active.insert(request.id.clone(), worker);
        }
        active_products.insert(request.id.clone(), request.artifacts[0].product_id);
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
    if let Ok(store) = StateStore::open() {
        let _ = save_request(&store, request, state);
    }
}

fn save_request(store: &StateStore, request: &Request, state: DownloadState) -> anyhow::Result<()> {
    let total = request
        .artifacts
        .iter()
        .map(|artifact| artifact.size_bytes)
        .collect::<Option<Vec<_>>>()
        .map(|sizes| sizes.into_iter().sum());
    store.save_download_job(&DownloadJobUpdate {
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
    })
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
        let generation = crate::installation::recovery::generation(job.product_id);
        let Ok(_admission) =
            crate::installation::recovery::admit_generation(job.product_id, generation)
        else {
            continue;
        };
        if let Err(error) = validate_destination(&job.artifacts, &job.destination, None) {
            let _ = store.set_download_job_failure(
                &job.job_id,
                &format!(
                    "Library requires attention: {error}. Correct Storage and resume this download."
                ),
            );
            continue;
        }
        let downloaded = recovered_bytes(&job);
        let _ = store.recover_download_job(&job.job_id, downloaded);
        let (listener, _) = mpsc::channel();
        queued.push_back(Request {
            library_id: String::new(),
            recovery_generation: generation,
            session: crate::online::account_session(),
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
        library_id: String::new(),
        recovery_generation: crate::installation::recovery::generation(job.product_id),
        session: crate::online::account_session(),
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
    let _admission = crate::installation::recovery::admit_generation(
        first.product_id,
        request.recovery_generation,
    )
    .map_err(|error| error.to_string())?;
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
    if let Err(error) = validate_destination(&job.artifacts, &job.destination, None) {
        set_waiting_status(
            id,
            Some(&format!(
                "Library requires attention: {error}; files were retained"
            )),
        );
        return;
    }
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
    fn startup_blocks_legacy_untyped_download_without_touching_its_partial_files() {
        use super::*;
        if std::env::var_os("LUDOMERE_TYPED_RECOVERY_CHILD").is_none() {
            let root = tempfile::tempdir().unwrap();
            let mut command = std::process::Command::new(std::env::current_exe().unwrap());
            command.args(["--exact", "download::manager::tests::startup_blocks_legacy_untyped_download_without_touching_its_partial_files", "--nocapture"])
                .env("LUDOMERE_TYPED_RECOVERY_CHILD", "1");
            for key in [
                "HOME",
                "XDG_CONFIG_HOME",
                "XDG_DATA_HOME",
                "XDG_CACHE_HOME",
                "XDG_STATE_HOME",
                "XDG_RUNTIME_DIR",
            ] {
                let path = root.path().join(key);
                fs::create_dir(&path).unwrap();
                command.env(key, path);
            }
            assert!(command.status().unwrap().success());
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let destination = root.path().join("legacy/game/installer");
        let artifacts: Vec<RemoteArtifact> = serde_json::from_value(serde_json::json!([
            {"product_id":9101952,"kind":"installer","name":"Fixture","size_bytes":4,"download_path":"/never-fetched"}
        ])).unwrap();
        let id = crate::download::job_id(&artifacts.iter().collect::<Vec<_>>());
        let staging = staging_directory(&destination, &artifacts, &id);
        fs::create_dir_all(&staging).unwrap();
        fs::write(staging.join("1.download"), b"pa").unwrap();
        crate::config::Config {
            download_directory: root.path().join("legacy"),
            ..Default::default()
        }
        .save()
        .unwrap();
        let store = StateStore::open().unwrap();
        store
            .save_download_job(&DownloadJobUpdate {
                job_id: &id,
                product_id: 9101952,
                title: "Fixture",
                artifacts: &artifacts,
                destination: &destination,
                state: DownloadState::Queued,
                bytes_downloaded: 2,
                total_bytes: Some(4),
                completed_files: &[],
                error: None,
            })
            .unwrap();
        let mut queue = VecDeque::new();
        recover_jobs(
            &mut queue,
            &Arc::new(Mutex::new(HashMap::new())),
            "inert".into(),
        );
        assert!(queue.is_empty());
        let saved = store.download_job(&id).unwrap().unwrap();
        assert_eq!(saved.state, DownloadState::Failed);
        assert_eq!(saved.destination, destination);
        assert!(saved.error.unwrap().contains("Correct Storage"));
        assert_eq!(fs::read(staging.join("1.download")).unwrap(), b"pa");
        assert!(!destination.exists());
    }

    #[test]
    fn unchanged_manifest_refresh_preserves_legacy_queue_identity_and_partial_files() {
        use super::*;
        let root = tempfile::tempdir().unwrap();
        let artifacts: Vec<RemoteArtifact> = serde_json::from_value(serde_json::json!([
            {"product_id":910195,"kind":"installer","name":"Fixture","size_bytes":4,"download_path":"/never-fetched"}
        ])).unwrap();
        let (events, _) = mpsc::channel();
        let mut request = request_from_download(
            crate::download::DownloadRequest {
                library_id: "offline".into(),
                artifacts: artifacts.clone(),
                title: "Fixture".into(),
                access_token: "inert".into(),
                destination: root.path().join("game/installer"),
                events,
            },
            crate::online::account_session(),
        );
        let scoped = request.id.clone();
        request.id = crate::download::job_id(&artifacts.iter().collect::<Vec<_>>());
        let staging = staging_directory(&request.destination, &artifacts, &request.id);
        fs::create_dir_all(&staging).unwrap();
        fs::write(staging.join("1.download"), b"pa").unwrap();
        let store = StateStore::open().unwrap();
        save_request(&store, &request, DownloadState::Queued).unwrap();
        let (commands, receiver) = mpsc::channel();
        let worker_commands = commands.clone();
        let worker = std::thread::spawn(move || {
            run(
                receiver,
                worker_commands,
                Arc::new(Mutex::new(HashMap::new())),
                Arc::new(Mutex::new(Vec::new())),
                HashMap::new(),
            )
        });
        commands
            .send(Command::ManifestRefreshed(
                request.clone(),
                DownloadFailure {
                    kind: DownloadFailureKind::TransientNetwork,
                    message: "fixture".into(),
                },
                Ok(artifacts),
            ))
            .unwrap();
        let (reply, done) = mpsc::channel();
        commands.send(Command::Shutdown(reply)).unwrap();
        done.recv_timeout(Duration::from_secs(2)).unwrap();
        worker.join().unwrap();
        assert!(store.download_job(&request.id).unwrap().is_some());
        assert!(store.download_job(&scoped).unwrap().is_none());
        assert_eq!(fs::read(staging.join("1.download")).unwrap(), b"pa");
        store.delete_download_job(&request.id).unwrap();
    }

    #[test]
    fn sign_out_drains_active_work_and_ignores_old_resume_and_manifest_commands() {
        use super::*;
        let root = tempfile::tempdir().unwrap();
        let artifacts: Vec<RemoteArtifact> = serde_json::from_value(serde_json::json!([
            {"product_id": 910147, "kind":"installer", "name":"Fixture", "size_bytes":4, "download_path":"/never-fetched"}
        ])).unwrap();
        let (listener, _) = mpsc::channel();
        let request = request_from_download(
            super::super::DownloadRequest {
                library_id: String::new(),
                artifacts: artifacts.clone(),
                title: "Fixture".into(),
                access_token: "inert".into(),
                destination: root.path().join("game/installer"),
                events: listener,
            },
            crate::online::account_session().wrapping_sub(1),
        );
        let cancelled = Arc::new(AtomicBool::new(false));
        let active = Arc::new(Mutex::new(HashMap::from([(
            request.id.clone(),
            cancelled.clone(),
        )])));
        let products = HashMap::from([(request.id.clone(), 910147)]);
        let (commands, receiver) = mpsc::channel();
        let worker_commands = commands.clone();
        let worker = std::thread::spawn(move || {
            run(
                receiver,
                worker_commands,
                active,
                Arc::new(Mutex::new(Vec::new())),
                products,
            )
        });
        let (reply, paused) = mpsc::channel();
        commands.send(Command::PauseForSignOut(reply)).unwrap();
        assert!(paused.recv_timeout(Duration::from_millis(75)).is_err());
        assert!(cancelled.load(Ordering::Relaxed));
        let failure = DownloadFailure {
            kind: DownloadFailureKind::ManifestChanged,
            message: "inert".into(),
        };
        commands
            .send(Command::Terminal(
                request.clone(),
                DownloadEvent::Failed(failure.clone()),
            ))
            .unwrap();
        paused.recv_timeout(Duration::from_secs(2)).unwrap();
        commands
            .send(Command::Recover("old-token".into(), request.session))
            .unwrap();
        commands
            .send(Command::Resume {
                id: request.id.clone(),
                access_token: "old-token".into(),
                reset_retry: true,
                session: request.session,
            })
            .unwrap();
        commands
            .send(Command::ManifestRefreshed(
                request.clone(),
                failure,
                Ok(Vec::new()),
            ))
            .unwrap();
        let (reply, stopped) = mpsc::channel();
        commands.send(Command::Shutdown(reply)).unwrap();
        stopped.recv_timeout(Duration::from_secs(2)).unwrap();
        worker.join().unwrap();
        let store = StateStore::open().unwrap();
        let job = store.download_job(&request.id).unwrap().unwrap();
        assert_eq!(job.state, DownloadState::Queued);
        assert_eq!(job.artifacts.len(), 1);
        assert_eq!(job.artifacts[0].download_path, "/never-fetched");
        assert!(!request.destination.exists());
    }

    #[test]
    fn recovery_drains_unrecorded_active_request_and_rejects_late_manifest() {
        use super::*;
        let root = tempfile::tempdir().unwrap();
        let id = 910009;
        let artifacts: Vec<RemoteArtifact> = serde_json::from_value(serde_json::json!([
            {"product_id":id,"kind":"installer","name":"Fixture","size_bytes":4,"download_path":"/never-fetched"}
        ])).unwrap();
        let (listener, _) = mpsc::channel();
        let request = request_from_download(
            crate::download::DownloadRequest {
                library_id: String::new(),
                artifacts: artifacts.clone(),
                title: "Fixture".into(),
                access_token: "inert".into(),
                destination: root.path().join("game/installer"),
                events: listener,
            },
            crate::online::account_session(),
        );
        let store = StateStore::open().unwrap();
        assert!(store.download_job(&request.id).unwrap().is_none());
        let cancelled = Arc::new(AtomicBool::new(false));
        let active = Arc::new(Mutex::new(HashMap::from([(
            request.id.clone(),
            cancelled.clone(),
        )])));
        let products = HashMap::from([(request.id.clone(), id)]);
        let (commands, receiver) = mpsc::channel();
        let worker_commands = commands.clone();
        let worker = std::thread::spawn(move || {
            run(
                receiver,
                worker_commands,
                active,
                Arc::new(Mutex::new(Vec::new())),
                products,
            )
        });
        let reservation = crate::installation::recovery::Reservation::reserve(&[id]).unwrap();
        let (reply, drained) = mpsc::channel();
        commands.send(Command::Quiesce(vec![id], reply)).unwrap();
        assert!(drained.recv_timeout(Duration::from_millis(75)).is_err());
        assert!(cancelled.load(Ordering::Relaxed));
        commands
            .send(Command::Terminal(request.clone(), DownloadEvent::Cancelled))
            .unwrap();
        drained
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .unwrap();
        store.delete_download_job(&request.id).unwrap();
        drop(reservation);
        commands
            .send(Command::ManifestRefreshed(
                request.clone(),
                crate::download::DownloadFailure {
                    kind: DownloadFailureKind::TransientNetwork,
                    message: "fixture".into(),
                },
                Ok(artifacts),
            ))
            .unwrap();
        let (reply, done) = mpsc::channel();
        commands.send(Command::Shutdown(reply)).unwrap();
        done.recv_timeout(Duration::from_secs(2)).unwrap();
        worker.join().unwrap();
        assert!(store.download_job(&request.id).unwrap().is_none());
        assert!(!request.destination.exists());
    }

    #[test]
    fn committed_completion_survives_auth_disable_or_stale_session_without_follow_up() {
        if std::env::var_os("LUDOMERE_TERMINAL_RACE_CHILD").is_none() {
            let root = tempfile::tempdir().unwrap();
            let mut command = std::process::Command::new(std::env::current_exe().unwrap());
            command.args(["--exact","download::manager::tests::committed_completion_survives_auth_disable_or_stale_session_without_follow_up","--nocapture"])
                .env("LUDOMERE_TERMINAL_RACE_CHILD","1");
            for key in [
                "HOME",
                "XDG_DATA_HOME",
                "XDG_CONFIG_HOME",
                "XDG_CACHE_HOME",
                "XDG_STATE_HOME",
                "XDG_RUNTIME_DIR",
            ] {
                let path = root.path().join(key);
                std::fs::create_dir(&path).unwrap();
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
                command.env(key, path);
            }
            assert!(command.status().unwrap().success());
            return;
        }
        use super::*;
        let root = tempfile::tempdir().unwrap();
        let store = StateStore::open().unwrap();
        for (product, authenticated, session) in [
            (71, false, crate::online::account_session()),
            (72, true, crate::online::account_session().wrapping_sub(1)),
        ] {
            let artifacts:Vec<RemoteArtifact>=serde_json::from_value(serde_json::json!([
                {"product_id":product,"kind":"installer","name":"Terminal fixture","size_bytes":4,"download_path":"/never-requested"}
            ])).unwrap();
            let files = vec![root.path().join(format!("{product}.bin"))];
            fs::write(&files[0], b"data").unwrap();
            let (listener, events) = mpsc::channel();
            let request = request_from_download(
                crate::download::DownloadRequest {
                    library_id: String::new(),
                    artifacts,
                    title: "Fixture".into(),
                    access_token: "inert".into(),
                    destination: root.path().to_owned(),
                    events: listener,
                },
                session,
            );
            store
                .save_download_job(&DownloadJobUpdate {
                    job_id: &request.id,
                    product_id: product,
                    title: "Fixture",
                    artifacts: &request.artifacts,
                    destination: &request.destination,
                    state: DownloadState::Complete,
                    bytes_downloaded: 4,
                    total_bytes: Some(4),
                    completed_files: &files,
                    error: None,
                })
                .unwrap();
            // Invalid inert plan detects any unintended auto-install processing without executing a payload.
            store
                .save_download_install_intent(&crate::state::DownloadInstallIntent {
                    product_id: product,
                    intent_id: format!("consent-{product}"),
                    job_ids: vec![request.id.clone()],
                    plan_json: "{}".into(),
                    state: "waiting".into(),
                    error: None,
                })
                .unwrap();
            let (commands, receiver) = mpsc::channel();
            let active = Arc::new(Mutex::new(HashMap::new()));
            let (subscriber, snapshots) = mpsc::channel();
            let subscribers = Arc::new(Mutex::new(vec![subscriber]));
            let worker_commands = commands.clone();
            let worker = std::thread::spawn(move || {
                run(
                    receiver,
                    worker_commands,
                    active,
                    subscribers,
                    HashMap::new(),
                )
            });
            commands
                .send(Command::SetAuthentication(authenticated))
                .unwrap();
            commands
                .send(Command::Terminal(
                    request.clone(),
                    DownloadEvent::Complete {
                        files: files.clone(),
                    },
                ))
                .unwrap();
            assert!(
                matches!(events.recv_timeout(Duration::from_secs(2)).unwrap(),DownloadEvent::Complete {files:received} if received==files)
            );
            let (acknowledge, closed) = mpsc::channel();
            commands.send(Command::Shutdown(acknowledge)).unwrap();
            closed.recv_timeout(Duration::from_secs(2)).unwrap();
            worker.join().unwrap();
            let job = store.download_job(&request.id).unwrap().unwrap();
            assert_eq!(job.state, DownloadState::Complete);
            assert_eq!(job.completed_files, files);
            assert_eq!(job.bytes_downloaded, 4);
            assert!(
                store
                    .download_install_intents()
                    .unwrap()
                    .iter()
                    .all(|intent| intent.state == "waiting")
            );
            assert!(snapshots.try_iter().any(
                |event| matches!(event,DownloadManagerEvent::ManagedFilesChanged(id) if id==product)
            ));
            assert_eq!(fs::read(&files[0]).unwrap(), b"data");
        }
    }

    #[test]
    fn obsolete_captured_account_cannot_register_intent_or_restore_authentication() {
        let root = tempfile::tempdir().unwrap();
        let store = crate::state::StateStore::open_at(&root.path().join("state.db")).unwrap();
        let queued_session = crate::online::account_session().wrapping_sub(1);
        let mut authentication_available = false;
        let result = crate::online::with_account_session(queued_session, || {
            store.save_download_install_intent(&crate::state::DownloadInstallIntent {
                product_id: 7,
                intent_id: "obsolete-account".into(),
                job_ids: vec!["job".into()],
                plan_json: "{}".into(),
                state: "waiting".into(),
                error: None,
            })?;
            authentication_available = true;
            Ok(())
        });
        assert!(result.is_err());
        assert!(!authentication_available);
        assert!(store.download_install_intents().unwrap().is_empty());
    }

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
