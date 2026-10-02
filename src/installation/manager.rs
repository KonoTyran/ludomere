use super::{AdditionalInstaller, InstallationEvent, UninstallationEvent};
use crate::{
    domain::{DepotOperationKind, InstalledGame},
    state::{DepotOperationRecord, InstallationOperationRecord, StateStore},
};
use anyhow::Context;
use std::{
    collections::{HashMap, VecDeque},
    path::PathBuf,
    sync::{LazyLock, Mutex, mpsc},
    thread,
    time::Duration,
};

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct DepotSource {
    pub product_id: i64,
    pub depot_id: String,
    pub manifest_id: String,
    pub manifest_json: Option<String>,
    pub content_root: Option<String>,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct EntitlementDlc {
    pub product_id: i64,
    pub name: String,
}

#[derive(Clone)]
pub struct DepotOperationRequest {
    pub account_session: u64,
    pub recovery_generation: u64,
    pub operation_id: String,
    pub product_id: i64,
    pub build_id: String,
    pub branch: Option<String>,
    pub kind: DepotOperationKind,
    pub sources: Vec<DepotSource>,
    pub current_sources: Vec<DepotSource>,
    pub current_manifest_json: Option<String>,
    pub library_id: String,
    pub dependencies: Vec<String>,
    pub dependency_plan: Option<crate::gog::dependencies::Plan>,
    pub entitlement_dlc: Vec<EntitlementDlc>,
    pub library_root: PathBuf,
    pub slug: String,
    pub destination: PathBuf,
    pub staging_path: PathBuf,
    pub target_marker: super::marker::InstallationMarker,
    pub access_token: String,
}

impl std::fmt::Debug for DepotOperationRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DepotOperationRequest")
            .field("operation_id", &self.operation_id)
            .field("product_id", &self.product_id)
            .field("build_id", &self.build_id)
            .field("branch", &self.branch)
            .field("kind", &self.kind)
            .field("source_count", &self.sources.len())
            .field("destination", &self.destination)
            .field("library_id", &self.library_id)
            .field("dependency_count", &self.dependencies.len())
            .field("entitlement_dlc_count", &self.entitlement_dlc.len())
            .field("library_root", &self.library_root)
            .field("staging_path", &self.staging_path)
            .field("access_token", &"[redacted]")
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DepotSetupProgress {
    pub component: String,
    pub completed: usize,
    pub total: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DepotOperationSnapshot {
    pub operation_id: String,
    pub product_id: i64,
    pub state: String,
    pub bytes_completed: u64,
    pub bytes_downloaded: u64,
    pub bytes_written: u64,
    pub total_write_bytes: u64,
    pub total_bytes: u64,
    pub download_total_bytes: Option<u64>,
    pub error: Option<String>,
    pub setup: Option<DepotSetupProgress>,
}

#[derive(Debug, Clone)]
pub enum DepotManagerEvent {
    Snapshot(DepotOperationSnapshot),
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct PersistedDepotPlan {
    product_id: i64,
    build_id: String,
    branch: Option<String>,
    kind: DepotOperationKind,
    sources: Vec<DepotSource>,
    #[serde(default)]
    current_sources: Vec<DepotSource>,
    current_manifest_json: Option<String>,
    #[serde(default)]
    library_id: String,
    #[serde(default)]
    dependencies: Vec<String>,
    #[serde(default)]
    dependency_plan: Option<crate::gog::dependencies::Plan>,
    #[serde(default)]
    entitlement_dlc: Vec<EntitlementDlc>,
    library_root: PathBuf,
    slug: String,
    destination: PathBuf,
    staging_path: PathBuf,
    target_marker: super::marker::InstallationMarker,
}

impl From<&DepotOperationRequest> for PersistedDepotPlan {
    fn from(request: &DepotOperationRequest) -> Self {
        Self {
            product_id: request.product_id,
            build_id: request.build_id.clone(),
            branch: request.branch.clone(),
            kind: request.kind,
            sources: request.sources.clone(),
            current_sources: request.current_sources.clone(),
            current_manifest_json: request.current_manifest_json.clone(),
            library_id: request.library_id.clone(),
            dependencies: request.dependencies.clone(),
            dependency_plan: request.dependency_plan.clone(),
            entitlement_dlc: request.entitlement_dlc.clone(),
            library_root: request.library_root.clone(),
            slug: request.slug.clone(),
            destination: request.destination.clone(),
            staging_path: request.staging_path.clone(),
            target_marker: request.target_marker.clone(),
        }
    }
}

#[derive(Default)]
struct DepotManagerState {
    active: HashMap<String, std::sync::Arc<std::sync::atomic::AtomicBool>>,
    reservations: HashMap<String, (i64, PathBuf)>,
    snapshots: HashMap<String, DepotOperationSnapshot>,
    snapshot_sequence: HashMap<String, u64>,
    next_snapshot_sequence: u64,
    last_event_at: HashMap<String, std::time::Instant>,
    abandon_requested: std::collections::HashSet<String>,
    subscribers: Vec<mpsc::Sender<DepotManagerEvent>>,
    shutting_down: bool,
    paused_for_sign_out: bool,
}

static DEPOT_MANAGER: LazyLock<Mutex<DepotManagerState>> =
    LazyLock::new(|| Mutex::new(DepotManagerState::default()));

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RetryAction {
    Stop,
    NextEndpoint,
    Refresh,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct SourceRetryState {
    endpoint: usize,
    refreshes: u8,
    attempts: u8,
}

#[derive(Debug)]
struct SourceTransferFailure {
    source: usize,
    kind: crate::download::depot::TransferErrorKind,
}

#[cfg(test)]
fn run_transfer_workers<T, R, F>(jobs: Vec<T>, worker: F) -> anyhow::Result<Vec<R>>
where
    T: Send,
    R: Send,
    F: Fn(T) -> anyhow::Result<R> + Sync,
{
    run_transfer_workers_with(jobs, worker, |_, _| Ok(()))
}

fn run_transfer_workers_with<T, R, F, C>(
    jobs: Vec<T>,
    worker: F,
    mut completed: C,
) -> anyhow::Result<Vec<R>>
where
    T: Send,
    R: Send,
    F: Fn(T) -> anyhow::Result<R> + Sync,
    C: FnMut(usize, &R) -> anyhow::Result<()>,
{
    let job_count = jobs.len();
    let queue = std::sync::Mutex::new(
        jobs.into_iter()
            .enumerate()
            .collect::<std::collections::VecDeque<_>>(),
    );
    let mut results = std::iter::repeat_with(|| None)
        .take(job_count)
        .collect::<Vec<Option<R>>>();
    let stopped = std::sync::atomic::AtomicBool::new(false);
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::scope(|scope| {
        let mut workers = Vec::new();
        for _ in 0..job_count.min(crate::download::depot::TRANSFER_WORKERS) {
            let sender = sender.clone();
            let stopped = &stopped;
            let queue = &queue;
            let worker = &worker;
            workers.push(scope.spawn(move || {
                loop {
                    if stopped.load(std::sync::atomic::Ordering::Relaxed) {
                        return;
                    }
                    let Some((index, job)) = queue.lock().unwrap().pop_front() else {
                        return;
                    };
                    if sender.send((index, worker(job))).is_err() {
                        return;
                    }
                }
            }));
        }
        drop(sender);
        let mut received = 0;
        while received < job_count {
            let (index, result) = receiver
                .recv()
                .map_err(|_| anyhow::anyhow!("depot transfer worker stopped unexpectedly"))?;
            received += 1;
            match result {
                Ok(result) => {
                    if let Err(error) = completed(index, &result) {
                        stopped.store(true, std::sync::atomic::Ordering::Relaxed);
                        return Err(error);
                    }
                    results[index] = Some(result);
                }
                Err(error) => {
                    stopped.store(true, std::sync::atomic::Ordering::Relaxed);
                    return Err(error);
                }
            }
        }
        for worker in workers {
            worker
                .join()
                .map_err(|_| anyhow::anyhow!("depot transfer worker panicked"))?;
        }
        Ok::<(), anyhow::Error>(())
    })?;
    results
        .into_iter()
        .map(|result| result.ok_or_else(|| anyhow::anyhow!("depot transfer result is missing")))
        .collect()
}

impl std::fmt::Display for SourceTransferFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("depot content transfer failed")
    }
}

impl std::error::Error for SourceTransferFailure {}

fn retry_action(
    kind: crate::download::depot::TransferErrorKind,
    state: SourceRetryState,
    endpoint_count: usize,
) -> (RetryAction, SourceRetryState) {
    use crate::download::depot::TransferErrorKind::*;
    let mut next = state;
    next.attempts = next.attempts.saturating_add(1);
    let action = if next.attempts > 4 {
        RetryAction::Stop
    } else {
        match kind {
            AuthenticationOrExpired if next.refreshes == 0 => {
                next.refreshes = 1;
                next.endpoint = 0;
                RetryAction::Refresh
            }
            Transient if next.endpoint + 1 < endpoint_count => {
                next.endpoint += 1;
                RetryAction::NextEndpoint
            }
            Transient if next.refreshes == 0 => {
                next.refreshes = 1;
                next.endpoint = 0;
                RetryAction::Refresh
            }
            _ => RetryAction::Stop,
        }
    };
    (action, next)
}

type ChunkSources = HashMap<String, (i64, String)>;

#[derive(Debug, Clone)]
pub enum InstallationManagerEvent {
    OperationQueued(InstallationOperationSnapshot),
    OperationRecovered(InstallationOperationSnapshot),
    OperationCancelled(InstallationOperationSnapshot),
    Installation {
        product_id: i64,
        event: InstallationEvent,
    },
    Uninstallation {
        product_id: i64,
        event: UninstallationEvent,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallationOperationSnapshot {
    pub product_id: i64,
    pub state: crate::domain::InstallationState,
    pub message: Option<String>,
    pub percentage: Option<u8>,
    pub queued: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct PersistedInstallationPlan {
    #[serde(skip)]
    tracking: Option<InstallationTracking>,
    #[serde(skip)]
    recovery_generation: u64,
    game: InstalledGame,
    additional_installers: Vec<AdditionalInstaller>,
    install_base: bool,
    interactive_prompts: bool,
    #[serde(default)]
    download_intent_id: Option<String>,
}

#[derive(Debug, Clone)]
struct InstallationTracking {
    sender: mpsc::Sender<InstallationEvent>,
    control: TrackedInstallationControl,
}

pub struct TrackedInstallation {
    pub events: mpsc::Receiver<InstallationEvent>,
    control: TrackedInstallationControl,
}

impl TrackedInstallation {
    pub fn control(&self) -> TrackedInstallationControl {
        self.control.clone()
    }
}

#[derive(Debug, Clone)]
pub struct TrackedInstallationControl {
    product_id: i64,
    generation: u64,
    cancellation: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl TrackedInstallationControl {
    pub fn cancel(&self) -> bool {
        self.cancellation
            .store(true, std::sync::atomic::Ordering::Release);
        cancel_operation_checked(self.product_id, Some(self))
    }
}

#[derive(Clone)]
enum OperationControl {
    Installation(super::executor::InstallationControl),
    Uninstallation(super::executor::UninstallationControl),
}

#[derive(Clone)]
enum QueuedOperation {
    Installation(PersistedInstallationPlan),
    Uninstallation(PersistedUninstallationPlan),
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct PersistedUninstallationPlan {
    #[serde(skip)]
    recovery_generation: u64,
    #[serde(flatten)]
    game: InstalledGame,
    #[serde(default)]
    cleanup: Option<crate::download::ManagedDownloads>,
}

impl QueuedOperation {
    fn product_id(&self) -> i64 {
        match self {
            Self::Installation(plan) => plan.game.product_id,
            Self::Uninstallation(plan) => plan.game.product_id,
        }
    }
}

#[derive(Default)]
struct ManagerState {
    active: HashMap<i64, OperationControl>,
    queue: VecDeque<QueuedOperation>,
    next_queue_position: i64,
    snapshots: HashMap<i64, InstallationOperationSnapshot>,
    subscribers: Vec<mpsc::Sender<InstallationManagerEvent>>,
    shutting_down: bool,
    paused_for_sign_out: bool,
    pause_errors: Vec<String>,
}

static MANAGER: LazyLock<Mutex<ManagerState>> =
    LazyLock::new(|| Mutex::new(ManagerState::default()));
static SIGN_OUT_PAUSE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub(crate) fn request_sign_out_pause() {
    SIGN_OUT_PAUSE.store(true, std::sync::atomic::Ordering::Release);
}

pub(crate) fn finish_sign_out_pause() {
    MANAGER.lock().unwrap().paused_for_sign_out = false;
    SIGN_OUT_PAUSE.store(false, std::sync::atomic::Ordering::Release);
}

pub fn subscribe_installation_events() -> mpsc::Receiver<InstallationManagerEvent> {
    let (sender, receiver) = mpsc::channel();
    MANAGER.lock().unwrap().subscribers.push(sender);
    receiver
}

pub fn subscribe_depot_events() -> mpsc::Receiver<DepotManagerEvent> {
    let (sender, receiver) = mpsc::channel();
    DEPOT_MANAGER.lock().unwrap().subscribers.push(sender);
    receiver
}

pub fn depot_operation_snapshot(operation_id: &str) -> Option<DepotOperationSnapshot> {
    DEPOT_MANAGER
        .lock()
        .unwrap()
        .snapshots
        .get(operation_id)
        .cloned()
}

pub fn depot_operation_snapshot_for_product(product_id: i64) -> Option<DepotOperationSnapshot> {
    let manager = DEPOT_MANAGER.lock().unwrap();
    manager
        .snapshots
        .values()
        .filter(|snapshot| snapshot.product_id == product_id)
        .max_by_key(|snapshot| {
            (
                depot_state_is_active(&snapshot.state),
                manager
                    .snapshot_sequence
                    .get(&snapshot.operation_id)
                    .copied()
                    .unwrap_or_default(),
            )
        })
        .cloned()
}

pub fn depot_operation_snapshots() -> Vec<DepotOperationSnapshot> {
    let manager = DEPOT_MANAGER.lock().unwrap();
    let mut snapshots = manager.snapshots.values().cloned().collect::<Vec<_>>();
    snapshots.sort_by_key(|snapshot| {
        manager
            .snapshot_sequence
            .get(&snapshot.operation_id)
            .copied()
            .unwrap_or_default()
    });
    snapshots
}

fn depot_state_is_active(state: &str) -> bool {
    matches!(
        state,
        "queued"
            | "preparing"
            | "dependencies"
            | "setup"
            | "verifying"
            | "verifying_existing"
            | "calculating"
            | "downloading"
            | "materializing"
            | "extracting"
            | "committing"
            | "finalizing"
    )
}

fn migrate_legacy_operation_records() -> anyhow::Result<()> {
    let store = StateStore::open()?;
    for record in store.installation_operations()? {
        if matches!(record.state.as_str(), "complete" | "cancelled") {
            store.delete_installation_operation(record.product_id)?;
            continue;
        }
        let path = super::operation_journal::offline_path(&record)?;
        if !path.exists() {
            super::operation_journal::write_offline(&path, &record)?;
        }
        store.delete_installation_operation(record.product_id)?;
    }
    for record in store.depot_operations()? {
        let path = super::operation_journal::depot_path(&record.staging_path);
        if !path.exists() {
            super::operation_journal::write_depot(&path, &record)?;
        }
        store.delete_depot_operation(&record.operation_id)?;
    }
    store.clear_depot_operations()?;
    Ok(())
}

pub fn recover_depot_operations() -> anyhow::Result<usize> {
    migrate_legacy_operation_records()?;
    let operations = super::operation_journal::scan()?
        .into_iter()
        .filter_map(|(path, journal)| match journal {
            super::operation_journal::OperationJournal::Depot { record, .. } => {
                Some((path, record))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    let mut recovered = 0;
    for (path, mut operation) in operations {
        if operation.state != "failed" {
            operation.state = "interrupted".into();
            operation.error = None;
            operation.updated_at = chrono::Utc::now().timestamp();
            super::operation_journal::write_depot(&path, &operation)?;
        }
        publish_depot(DepotOperationSnapshot {
            setup: None,
            operation_id: operation.operation_id,
            product_id: operation.product_id,
            state: operation.state,
            bytes_completed: operation.bytes_completed,
            bytes_downloaded: 0,
            bytes_written: 0,
            total_write_bytes: 0,
            total_bytes: operation.total_bytes.unwrap_or_default(),
            download_total_bytes: None,
            error: operation.error,
        });
        recovered += 1;
    }
    Ok(recovered)
}

pub fn enqueue_depot_operation(request: DepotOperationRequest) -> bool {
    enqueue_depot_operation_with(request, persist_depot_request)
}

fn enqueue_depot_operation_with(
    mut request: DepotOperationRequest,
    persist: impl FnOnce(&DepotOperationRequest) -> anyhow::Result<()>,
) -> bool {
    if SIGN_OUT_PAUSE.load(std::sync::atomic::Ordering::Acquire) {
        return false;
    }
    let Ok(_activity) = crate::profile_reset::begin_activity("installation registration") else {
        return false;
    };
    if super::recovery::pending(&request.destination, request.product_id).unwrap_or(true) {
        return false;
    }
    let Ok(_admission) =
        super::recovery::admit_generation(request.product_id, request.recovery_generation)
    else {
        return false;
    };
    let Ok(staging) = super::depot::operation_staging_path(
        &request.library_root,
        &request.destination,
        &request.slug,
        &request.operation_id,
    ) else {
        return false;
    };
    request.staging_path = staging;
    let mut manager = DEPOT_MANAGER.lock().unwrap();
    let offline_conflict = MANAGER
        .lock()
        .unwrap()
        .active
        .contains_key(&request.product_id)
        || MANAGER
            .lock()
            .unwrap()
            .queue
            .iter()
            .any(|queued| queued.product_id() == request.product_id);
    if manager.active.contains_key(&request.operation_id)
        || manager
            .reservations
            .values()
            .any(|(product_id, destination)| {
                *product_id == request.product_id || destination == &request.destination
            })
        || offline_conflict
        || MANAGER.lock().unwrap().shutting_down
        || manager.shutting_down
        || (manager.paused_for_sign_out && !crate::auth::session_is_current(crate::auth::session()))
    {
        return false;
    }
    let cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    manager.paused_for_sign_out = false;
    manager
        .active
        .insert(request.operation_id.clone(), cancelled.clone());
    manager.reservations.insert(
        request.operation_id.clone(),
        (request.product_id, request.destination.clone()),
    );
    // Reserve before doing disk work, so competing operations and sign-out can see/cancel
    // registration without making GTK snapshot polling wait for journal serialization/fsync.
    drop(manager);
    if persist(&request).is_err() {
        let mut manager = DEPOT_MANAGER.lock().unwrap();
        manager.active.remove(&request.operation_id);
        manager.reservations.remove(&request.operation_id);
        manager.abandon_requested.remove(&request.operation_id);
        return false;
    }
    let snapshot = DepotOperationSnapshot {
        setup: None,
        operation_id: request.operation_id.clone(),
        product_id: request.product_id,
        state: "queued".into(),
        bytes_completed: 0,
        bytes_downloaded: 0,
        bytes_written: 0,
        total_write_bytes: 0,
        total_bytes: 0,
        download_total_bytes: None,
        error: None,
    };
    publish_depot(snapshot);
    thread::spawn(move || run_depot_operation(request, cancelled));
    true
}

pub fn cancel_depot_operation(operation_id: &str) -> bool {
    let manager = DEPOT_MANAGER.lock().unwrap();
    let Some(cancelled) = manager.active.get(operation_id) else {
        return false;
    };
    cancelled.store(true, std::sync::atomic::Ordering::Relaxed);
    true
}

pub fn abandon_depot_operation(operation_id: &str) -> bool {
    {
        let mut manager = DEPOT_MANAGER.lock().unwrap();
        if let Some(cancelled) = manager.active.get(operation_id).cloned() {
            manager.abandon_requested.insert(operation_id.to_owned());
            cancelled.store(true, std::sync::atomic::Ordering::Relaxed);
            return true;
        }
    }
    abandon_saved_depot_operation(operation_id).is_ok()
}

pub fn resume_depot_operation(operation_id: String, access_token: String) -> bool {
    if DEPOT_MANAGER
        .lock()
        .unwrap()
        .active
        .contains_key(&operation_id)
    {
        return false;
    }
    let result = prepare_depot_resume(operation_id.clone(), access_token);
    match result {
        Ok(request) => enqueue_depot_operation(request),
        Err(error) => {
            publish_depot(DepotOperationSnapshot {
                setup: None,
                operation_id,
                product_id: 0,
                state: "failed".into(),
                bytes_completed: 0,
                bytes_downloaded: 0,
                bytes_written: 0,
                total_write_bytes: 0,
                total_bytes: 0,
                download_total_bytes: None,
                error: Some(redact_error(&error.to_string(), "")),
            });
            false
        }
    }
}

pub fn prepare_depot_resume(
    operation_id: String,
    access_token: String,
) -> anyhow::Result<DepotOperationRequest> {
    let mut request = super::operation_journal::find_depot(&operation_id)
        .map(|(_, record)| record)
        .and_then(|record| {
            super::dependency_setup::ensure_setup_quiescent(&record)?;
            let plan: PersistedDepotPlan = serde_json::from_str(&record.plan_json)?;
            Ok(DepotOperationRequest {
                account_session: crate::online::account_session(),
                recovery_generation: super::recovery::generation(plan.product_id),
                operation_id: operation_id.clone(),
                product_id: plan.product_id,
                build_id: plan.build_id,
                branch: plan.branch,
                kind: plan.kind,
                sources: plan.sources,
                current_sources: plan.current_sources,
                current_manifest_json: plan.current_manifest_json,
                library_id: plan.library_id,
                dependencies: plan.dependencies,
                dependency_plan: plan.dependency_plan,
                entitlement_dlc: plan.entitlement_dlc,
                library_root: plan.library_root,
                slug: plan.slug,
                destination: plan.destination,
                staging_path: plan.staging_path,
                target_marker: plan.target_marker,
                access_token,
            })
        })?;
    super::validate_game_library(
        &crate::storage::read_config()?,
        &request.library_id,
        &request.destination,
    )?;
    prepare_required_dependencies(&mut request, &std::sync::atomic::AtomicBool::new(false))?;
    Ok(request)
}

fn publish_depot(snapshot: DepotOperationSnapshot) {
    let mut manager = DEPOT_MANAGER.lock().unwrap();
    manager.next_snapshot_sequence = manager.next_snapshot_sequence.wrapping_add(1);
    let sequence = manager.next_snapshot_sequence;
    manager
        .snapshot_sequence
        .insert(snapshot.operation_id.clone(), sequence);
    let previous = manager
        .snapshots
        .insert(snapshot.operation_id.clone(), snapshot.clone());
    let terminal = matches!(
        snapshot.state.as_str(),
        "complete" | "failed" | "cancelled" | "abandoned"
    );
    let state_changed = previous.is_none_or(|previous| previous.state != snapshot.state);
    let now = std::time::Instant::now();
    let due = manager
        .last_event_at
        .get(&snapshot.operation_id)
        .is_none_or(|last| now.duration_since(*last) >= Duration::from_millis(100));
    if !terminal && !state_changed && !due {
        return;
    }
    if terminal {
        manager.last_event_at.remove(&snapshot.operation_id);
    } else {
        manager
            .last_event_at
            .insert(snapshot.operation_id.clone(), now);
    }
    manager.subscribers.retain(|subscriber| {
        subscriber
            .send(DepotManagerEvent::Snapshot(snapshot.clone()))
            .is_ok()
    });
}

fn run_depot_operation(
    request: DepotOperationRequest,
    cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
) {
    let permit =
        crate::operation_gate::acquire(|| cancelled.load(std::sync::atomic::Ordering::Relaxed));
    let result = match permit {
        Some(_permit) => run_depot_operation_inner(&request, &cancelled),
        None => Err(crate::download::depot::DepotCancelled.into()),
    };
    let mut failure_snapshot = None;
    if let Err(error) = &result {
        let was_cancelled = error
            .chain()
            .any(|cause| cause.is::<crate::download::depot::DepotCancelled>());
        let interrupted = was_cancelled && DEPOT_MANAGER.lock().unwrap().shutting_down;
        let state = if interrupted || was_cancelled {
            "interrupted"
        } else {
            "failed"
        };
        let message = redact_error(&format!("{error:#}"), &request.access_token);
        if let Ok(log_path) = super::executor::installation_log_path(request.product_id) {
            let _ = crate::compatibility::append_step_log(
                &log_path,
                &format!("installation failed: {message}"),
            );
        }
        let progress = super::operation_journal::find_depot(&request.operation_id)
            .ok()
            .map(|(_, record)| record)
            .map_or(0, |record| record.bytes_completed);
        let previous = DEPOT_MANAGER
            .lock()
            .unwrap()
            .snapshots
            .get(&request.operation_id)
            .cloned();
        let _ = update_depot_record(
            &request.operation_id,
            state,
            progress,
            (!was_cancelled).then_some(message.as_str()),
            !was_cancelled,
        );
        failure_snapshot = Some(DepotOperationSnapshot {
            setup: None,
            operation_id: request.operation_id.clone(),
            product_id: request.product_id,
            state: state.into(),
            bytes_completed: progress,
            bytes_downloaded: previous
                .as_ref()
                .map_or(0, |snapshot| snapshot.bytes_downloaded),
            bytes_written: previous
                .as_ref()
                .map_or(0, |snapshot| snapshot.bytes_written),
            total_write_bytes: previous
                .as_ref()
                .map_or(0, |snapshot| snapshot.total_write_bytes),
            total_bytes: previous.as_ref().map_or(0, |snapshot| snapshot.total_bytes),
            download_total_bytes: previous
                .as_ref()
                .and_then(|snapshot| snapshot.download_total_bytes),
            error: (!was_cancelled).then_some(message),
        });
    }
    let abandon = DEPOT_MANAGER
        .lock()
        .unwrap()
        .abandon_requested
        .remove(&request.operation_id);
    if abandon {
        let _ = abandon_saved_depot_operation(&request.operation_id);
    } else if let Some(snapshot) = failure_snapshot {
        publish_depot(snapshot);
    }
    let mut manager = DEPOT_MANAGER.lock().unwrap();
    manager.active.remove(&request.operation_id);
    manager.reservations.remove(&request.operation_id);
}

fn abandon_saved_depot_operation(operation_id: &str) -> anyhow::Result<()> {
    let (journal_path, record) = super::operation_journal::find_depot(operation_id)?;
    super::dependency_setup::ensure_setup_quiescent(&record)?;
    let plan: PersistedDepotPlan = serde_json::from_str(&record.plan_json)?;
    let request = DepotOperationRequest {
        account_session: crate::online::account_session(),
        recovery_generation: super::recovery::generation(plan.product_id),
        operation_id: operation_id.to_owned(),
        product_id: plan.product_id,
        build_id: plan.build_id,
        branch: plan.branch,
        kind: plan.kind,
        sources: plan.sources,
        current_sources: plan.current_sources,
        current_manifest_json: plan.current_manifest_json,
        library_id: plan.library_id,
        dependencies: plan.dependencies,
        dependency_plan: plan.dependency_plan,
        entitlement_dlc: plan.entitlement_dlc,
        library_root: plan.library_root,
        slug: plan.slug,
        destination: plan.destination,
        staging_path: plan.staging_path,
        target_marker: plan.target_marker,
        access_token: String::new(),
    };
    let (manifest, _) = merge_depot_sources(&request)?;
    if request.staging_path.exists() {
        crate::download::depot::abandon_materialization(
            &manifest,
            &request.destination,
            &request.staging_path,
        )?;
    }
    super::depot_actions::remove_support_staging(&request.staging_path)?;
    super::operation_journal::remove(&journal_path)?;
    publish_depot(DepotOperationSnapshot {
        setup: None,
        operation_id: operation_id.to_owned(),
        product_id: request.product_id,
        state: "abandoned".into(),
        bytes_completed: 0,
        bytes_downloaded: 0,
        bytes_written: 0,
        total_write_bytes: 0,
        total_bytes: 0,
        download_total_bytes: None,
        error: None,
    });
    Ok(())
}

fn prepare_required_dependencies(
    request: &mut DepotOperationRequest,
    cancelled: &std::sync::atomic::AtomicBool,
) -> anyhow::Result<()> {
    let session = request.account_session;
    let stopped = || {
        cancelled.load(std::sync::atomic::Ordering::Relaxed)
            || crate::online::account_session() != session
            || !super::recovery::current(request.product_id, request.recovery_generation)
    };
    anyhow::ensure!(
        !stopped(),
        "Account or recovery changed before prerequisite preparation"
    );
    if request
        .target_marker
        .base
        .operating_system
        .as_deref()
        .is_some_and(|os| os.eq_ignore_ascii_case("windows"))
    {
        let repository = setup_repository(&StateStore::open()?, request, cancelled, |identity| {
            crate::gog::depot_acquisition::fetch_repository(
                &reqwest::blocking::Client::builder()
                    .timeout(Duration::from_secs(45))
                    .build()?,
                &request.access_token,
                identity,
            )
        })?;
        anyhow::ensure!(
            repository.dependencies == request.dependencies,
            "Saved dependency requirements do not match the selected build; prepare it again"
        );
        let mut ids = repository.dependencies.clone();
        if repository.script_interpreter && !ids.iter().any(|id| id == "ISI") {
            ids.push("ISI".into());
        }
        if let Some(plan) = &request.dependency_plan {
            super::dependency_setup::validate_required(plan, &ids)?;
        } else {
            let plan = crate::gog::dependencies::resolve(&ids, stopped)?;
            super::dependency_setup::validate_required(&plan, &ids)?;
            anyhow::ensure!(!stopped(), "Dependency preparation cancelled");
            request.dependency_plan = Some(plan);
            request
                .target_marker
                .galaxy_depot
                .as_mut()
                .unwrap()
                .manifest_fingerprint
                .clear();
            request
                .target_marker
                .galaxy_depot
                .as_mut()
                .unwrap()
                .manifest_fingerprint = planned_manifest_identity(request)?;
        }
    }
    Ok(())
}

fn run_depot_operation_inner(
    request: &DepotOperationRequest,
    cancelled: &std::sync::atomic::AtomicBool,
) -> anyhow::Result<()> {
    super::validate_game_library(
        &crate::storage::read_config()?,
        &request.library_id,
        &request.destination,
    )?;
    super::prefix_recovery::setup_ticket(
        &request.destination,
        &request.target_marker,
        request.kind == crate::domain::DepotOperationKind::Repair,
    )?;
    let mut request = request.clone();
    let session = request.account_session;
    anyhow::ensure!(
        crate::online::account_session() == session,
        "Account changed before dependency setup"
    );
    let windows = request
        .target_marker
        .base
        .operating_system
        .as_deref()
        .is_some_and(|os| os.eq_ignore_ascii_case("windows"));
    prepare_required_dependencies(&mut request, cancelled)?;
    let (path, mut record) = super::operation_journal::find_depot(&request.operation_id)?;
    super::dependency_setup::ensure_setup_quiescent(&record)?;
    record.plan_json = serde_json::to_string(&PersistedDepotPlan::from(&request))?;
    super::operation_journal::write_depot(&path, &record)?;
    let request = &request;
    let stopped = || {
        cancelled.load(std::sync::atomic::Ordering::Relaxed)
            || crate::online::account_session() != session
            || !super::recovery::current(request.product_id, request.recovery_generation)
    };
    let prepared = if let Some(plan) = &request.dependency_plan {
        let mut acquisition = plan.clone();
        acquisition.entries.retain(|entry| {
            super::dependency_setup::override_verbs(&entry.id, &request.dependencies).is_none()
        });
        crate::gog::dependencies::acquire(&acquisition, stopped, |done, total| {
            publish_depot_progress(request, "dependencies", done, done, 0, 0, total);
        })?
    } else {
        Vec::new()
    };
    anyhow::ensure!(
        !super::is_game_running(request.product_id),
        "Close the running game before resuming this Depot operation"
    );
    if request
        .target_marker
        .base
        .operating_system
        .as_deref()
        .is_some_and(|os| os.eq_ignore_ascii_case("windows"))
    {
        crate::compatibility::preflight_windows(Some(request.product_id))?;
    }
    publish_depot(DepotOperationSnapshot {
        setup: None,
        operation_id: request.operation_id.clone(),
        product_id: request.product_id,
        state: "preparing".into(),
        bytes_completed: 0,
        bytes_downloaded: 0,
        bytes_written: 0,
        total_write_bytes: 0,
        total_bytes: 0,
        download_total_bytes: None,
        error: None,
    });
    let (target, chunk_sources) = merge_depot_sources(request)?;
    let (support, support_sources) = merge_support_sources(request)?;
    let removed_actions = removed_dlc_actions(request)?;
    let current = current_manifest(request)?;
    let combined =
        super::dependency_setup::combined_manifest(&target, request.dependency_plan.as_ref())?;
    let payload_paths = target
        .entries
        .iter()
        .map(entry_path)
        .collect::<std::collections::HashSet<_>>();
    let dependency_paths = combined
        .entries
        .iter()
        .map(entry_path)
        .filter(|path| !payload_paths.contains(path))
        .map(str::to_owned)
        .collect();
    let target_totals = target.totals()?;
    let support_totals = support.totals()?;
    let total = target_totals
        .compressed
        .checked_add(support_totals.compressed)
        .ok_or_else(|| anyhow::anyhow!("depot operation size overflows"))?;
    let write_total = target_totals
        .uncompressed
        .checked_add(support_totals.uncompressed)
        .and_then(|total| {
            target
                .small_files_containers
                .iter()
                .flat_map(|container| &container.chunks)
                .try_fold(total, |sum, chunk| sum.checked_add(chunk.size))
        })
        .ok_or_else(|| anyhow::anyhow!("depot write size overflows"))?;
    let payload_total = target_totals.compressed;
    if cancelled.load(std::sync::atomic::Ordering::Relaxed) {
        update_depot_record(&request.operation_id, "cancelled", 0, None, true)?;
        publish_depot(DepotOperationSnapshot {
            setup: None,
            operation_id: request.operation_id.clone(),
            product_id: request.product_id,
            state: "cancelled".into(),
            bytes_completed: 0,
            bytes_downloaded: 0,
            bytes_written: 0,
            total_write_bytes: 0,
            total_bytes: total,
            download_total_bytes: None,
            error: None,
        });
        return Ok(());
    }
    let mut trusted_files = if request.kind == DepotOperationKind::Repair {
        let verification_total = target
            .entries
            .iter()
            .filter_map(|entry| match entry {
                crate::gog::depot_manifest::DepotEntry::File(file) => Some(file.size),
                _ => None,
            })
            .try_fold(0_u64, |total, size| total.checked_add(size))
            .ok_or_else(|| anyhow::anyhow!("depot verification size overflows"))?;
        publish_depot(DepotOperationSnapshot {
            setup: None,
            operation_id: request.operation_id.clone(),
            product_id: request.product_id,
            state: "verifying".into(),
            bytes_completed: 0,
            bytes_downloaded: 0,
            bytes_written: 0,
            total_write_bytes: 0,
            total_bytes: verification_total,
            download_total_bytes: None,
            error: None,
        });
        crate::download::depot::verify_installed_files(
            &target,
            &request.destination,
            |checked| {
                publish_depot_progress(request, "verifying", checked, 0, 0, 0, verification_total);
            },
            || cancelled.load(std::sync::atomic::Ordering::Relaxed),
        )?
    } else {
        std::collections::HashSet::new()
    };
    let client = reqwest::blocking::Client::new();
    let source_indices = chunk_sources
        .values()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .enumerate()
        .map(|(index, source)| (source, index))
        .collect::<HashMap<_, _>>();
    let mut endpoints = HashMap::new();
    let verification_total = crate::download::depot::journal_verification_total(
        &target,
        &request.destination,
        &request.staging_path,
    );
    if verification_total > 0 && request.kind != DepotOperationKind::Repair {
        publish_depot(DepotOperationSnapshot {
            setup: None,
            operation_id: request.operation_id.clone(),
            product_id: request.product_id,
            state: "verifying".into(),
            bytes_completed: 0,
            bytes_downloaded: 0,
            bytes_written: 0,
            total_write_bytes: 0,
            total_bytes: verification_total,
            download_total_bytes: None,
            error: None,
        });
    }
    let (mut completed, resumed_files) = crate::download::depot::journal_progress(
        &target,
        &request.destination,
        &request.staging_path,
        |checked| {
            if request.kind != DepotOperationKind::Repair {
                publish_depot_progress(request, "verifying", checked, 0, 0, 0, verification_total);
            }
        },
    );
    trusted_files.extend(resumed_files);
    if request.kind != DepotOperationKind::Repair {
        let verification_total = std::cell::Cell::new(0_u64);
        trusted_files.extend(crate::download::depot::verify_existing_files(
            &target,
            &request.destination,
            &trusted_files,
            |total| {
                verification_total.set(total);
                if total > 0 {
                    publish_depot(DepotOperationSnapshot {
                        setup: None,
                        operation_id: request.operation_id.clone(),
                        product_id: request.product_id,
                        state: "verifying_existing".into(),
                        bytes_completed: 0,
                        bytes_downloaded: 0,
                        bytes_written: 0,
                        total_write_bytes: 0,
                        total_bytes: total,
                        download_total_bytes: None,
                        error: None,
                    });
                }
            },
            |checked| {
                publish_depot_progress(
                    request,
                    "verifying_existing",
                    checked,
                    0,
                    0,
                    0,
                    verification_total.get(),
                );
            },
            || cancelled.load(std::sync::atomic::Ordering::Relaxed),
        )?);
    }
    publish_depot(DepotOperationSnapshot {
        setup: None,
        operation_id: request.operation_id.clone(),
        product_id: request.product_id,
        state: "calculating".into(),
        bytes_completed: completed,
        bytes_downloaded: 0,
        bytes_written: 0,
        total_write_bytes: write_total,
        total_bytes: total,
        download_total_bytes: None,
        error: None,
    });
    let local_chunks = current
        .as_ref()
        .map(local_chunk_candidates)
        .unwrap_or_default();
    let pending_chunks = crate::download::depot::pending_chunks(
        &target,
        &request.destination,
        &request.staging_path,
        &trusted_files,
    )?;
    let reusable = reusable_local_chunks(&request.destination, &local_chunks, &pending_chunks)?;
    let pending_support =
        super::depot_actions::pending_support_chunks(&support, &request.staging_path)?;
    let support_download =
        required_network_bytes(&pending_support, &std::collections::HashSet::new(), 0)?;
    let download_total = required_network_bytes(&pending_chunks, &reusable, support_download)?;
    update_depot_record(&request.operation_id, "downloading", completed, None, false)?;
    publish_depot(DepotOperationSnapshot {
        setup: None,
        operation_id: request.operation_id.clone(),
        product_id: request.product_id,
        state: "downloading".into(),
        bytes_completed: completed,
        bytes_downloaded: 0,
        bytes_written: 0,
        total_write_bytes: write_total,
        total_bytes: total,
        download_total_bytes: Some(download_total),
        error: None,
    });
    let downloaded = std::sync::atomic::AtomicU64::new(0);
    let written = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let mut served = std::collections::HashSet::new();
    let operation_id = request.operation_id.clone();
    let installed_marker = super::marker::load(&request.destination)?;
    let mut commit_marker =
        dependency_commit_marker(&request.target_marker, installed_marker.as_ref());
    commit_marker
        .galaxy_depot
        .as_mut()
        .unwrap()
        .manifest_fingerprint = target.identity();
    let plan = super::depot::DepotInstallPlan {
        operation: match request.kind {
            DepotOperationKind::Install => super::depot::DepotOperationKind::Install,
            DepotOperationKind::Update => super::depot::DepotOperationKind::Update,
            DepotOperationKind::BranchSwitch => super::depot::DepotOperationKind::BranchSwitch,
            DepotOperationKind::Repair => super::depot::DepotOperationKind::Repair,
        },
        target: request.destination.clone(),
        target_manifest: &target,
        current_manifest: current.as_ref(),
        target_marker: commit_marker,
        publish_marker: !windows,
        retained_paths: dependency_paths,
    };
    let forced_remove_paths = forced_dlc_removals(request)?;
    let mut retry_states = HashMap::<usize, SourceRetryState>::new();
    let mut recorded_progress = std::time::Instant::now();
    loop {
        let extraction_written = std::cell::Cell::new(0);
        let mut extraction_reported = std::time::Instant::now();
        let result = super::depot::execute_streamed_forward(
            &plan,
            &request.staging_path,
            &forced_remove_paths,
            &trusted_files,
            |chunks, output, completed_chunk| {
                let jobs = chunks
                    .iter()
                    .map(|job| {
                        let chunk = job.chunk;
                        let source = chunk_sources
                            .get(&chunk.compressed_md5)
                            .ok_or_else(|| anyhow::anyhow!("depot chunk has no content source"))?;
                        let source_index = *source_indices.get(source).ok_or_else(|| {
                            anyhow::anyhow!("depot content source is not indexed")
                        })?;
                        if let std::collections::hash_map::Entry::Vacant(entry) =
                            endpoints.entry(source_index)
                        {
                            entry.insert(crate::download::depot::acquire_secure_links(
                                &client,
                                &request.access_token,
                                source.0,
                                &source.1,
                            )?);
                        }
                        let index = retry_states
                            .get(&source_index)
                            .map_or(0, |state| state.endpoint);
                        let endpoint = endpoints
                            .get(&source_index)
                            .and_then(|links| links.urls.get(index))
                            .ok_or_else(|| {
                                anyhow::anyhow!("depot content source has no secure endpoint")
                            })?;
                        Ok((
                            chunk,
                            job.offset,
                            source_index,
                            crate::download::depot::chunk_url(endpoint, &chunk.compressed_md5)?,
                        ))
                    })
                    .collect::<anyhow::Result<Vec<_>>>()?;
                let progress_before_file = completed;
                run_transfer_workers_with(
                    jobs,
                    |(chunk, offset, source_index, url)| {
                        let client = &client;
                        let cancelled = &cancelled;
                        let destination = &request.destination;
                        let local_chunks = &local_chunks;
                        let output = output.try_clone();
                        let mut output = crate::download::depot::FileRegionWriter::new_counted(
                            output?,
                            offset,
                            written.clone(),
                        );
                        if cancelled.load(std::sync::atomic::Ordering::Relaxed) {
                            return Err(crate::download::depot::DepotCancelled.into());
                        }
                        if reuse_local_chunk(destination, local_chunks, chunk, &mut output)? {
                            return Ok(chunk);
                        }
                        crate::download::depot::download_chunk_to_with_progress(
                            client,
                            &url,
                            chunk,
                            &mut output,
                            |bytes| {
                                let downloaded = downloaded
                                    .fetch_add(bytes, std::sync::atomic::Ordering::Relaxed)
                                    + bytes;
                                publish_depot_progress(
                                    request,
                                    "materializing",
                                    progress_before_file,
                                    downloaded,
                                    written.load(std::sync::atomic::Ordering::Relaxed),
                                    write_total,
                                    total,
                                );
                            },
                        )
                        .map_err(|error| SourceTransferFailure {
                            source: source_index,
                            kind: error.kind(),
                        })?;
                        Ok(chunk)
                    },
                    |index, chunk| {
                        completed_chunk(index)?;
                        if served.insert(chunk.compressed_md5.clone()) {
                            completed = completed
                                .checked_add(chunk.compressed_size)
                                .ok_or_else(|| anyhow::anyhow!("depot progress overflow"))?;
                        }
                        publish_depot_progress(
                            request,
                            "materializing",
                            completed,
                            downloaded.load(std::sync::atomic::Ordering::Relaxed),
                            written.load(std::sync::atomic::Ordering::Relaxed),
                            write_total,
                            total,
                        );
                        Ok(())
                    },
                )?;
                if recorded_progress.elapsed().as_secs() >= 1 {
                    update_depot_record_at(
                        &super::operation_journal::depot_path(&request.staging_path),
                        &operation_id,
                        "materializing",
                        completed,
                        None,
                        false,
                    )?;
                    recorded_progress = std::time::Instant::now();
                }
                Ok(())
            },
            (
                || cancelled.load(std::sync::atomic::Ordering::Relaxed),
                |progress: crate::download::depot::ExtractionProgress| {
                    written.fetch_add(
                        progress
                            .written
                            .saturating_sub(extraction_written.replace(progress.written)),
                        std::sync::atomic::Ordering::Relaxed,
                    );
                    if progress.completed == 0
                        || progress.completed == progress.total
                        || extraction_reported.elapsed().as_millis() >= 100
                    {
                        publish_depot_progress(
                            request,
                            "extracting",
                            progress.completed,
                            downloaded.load(std::sync::atomic::Ordering::Relaxed),
                            written.load(std::sync::atomic::Ordering::Relaxed),
                            write_total,
                            progress.total,
                        );
                        extraction_reported = std::time::Instant::now();
                    }
                },
            ),
            || {
                update_depot_record(
                    &request.operation_id,
                    "committing",
                    payload_total,
                    None,
                    false,
                )?;
                publish_depot(DepotOperationSnapshot {
                    setup: None,
                    operation_id: request.operation_id.clone(),
                    product_id: request.product_id,
                    state: "committing".into(),
                    bytes_completed: payload_total,
                    bytes_downloaded: downloaded.load(std::sync::atomic::Ordering::Relaxed),
                    bytes_written: written.load(std::sync::atomic::Ordering::Relaxed),
                    total_write_bytes: write_total,
                    total_bytes: total,
                    download_total_bytes: Some(download_total),
                    error: None,
                });
                Ok(())
            },
        );
        let Err(error) = result else { break };
        let Some(failure) = error
            .chain()
            .find_map(|cause| cause.downcast_ref::<SourceTransferFailure>())
        else {
            return Err(error);
        };
        let state = retry_states
            .get(&failure.source)
            .copied()
            .unwrap_or_default();
        let endpoint_count = endpoints
            .get(&failure.source)
            .map_or(0, |links| links.urls.len());
        let (action, next) = retry_action(failure.kind, state, endpoint_count);
        retry_states.insert(failure.source, next);
        match action {
            RetryAction::Refresh => {
                endpoints.remove(&failure.source);
            }
            RetryAction::NextEndpoint => {}
            RetryAction::Stop => return Err(error),
        }
    }
    let mut downloaded = downloaded.load(std::sync::atomic::Ordering::Relaxed);
    let support_root = if support.entries.is_empty() {
        None
    } else {
        Some(materialize_support_network(
            request,
            &support,
            &support_sources,
            cancelled,
            &mut completed,
            &mut downloaded,
            total,
        )?)
    };
    let finalization = finalize_depot_metadata(
        request,
        support_root.as_deref(),
        &removed_actions,
        cancelled,
        &prepared,
        session,
    );
    let cleanup = super::depot_actions::remove_support_staging(&request.staging_path);
    finalization?;
    cleanup?;
    crate::download::depot::finish_journal(&request.staging_path)?;
    update_depot_record(&request.operation_id, "complete", total, None, true)?;
    publish_depot(DepotOperationSnapshot {
        setup: None,
        operation_id: request.operation_id.clone(),
        product_id: request.product_id,
        state: "complete".into(),
        bytes_completed: total,
        bytes_downloaded: downloaded,
        bytes_written: written.load(std::sync::atomic::Ordering::Relaxed),
        total_write_bytes: write_total,
        total_bytes: total,
        download_total_bytes: Some(download_total),
        error: None,
    });
    Ok(())
}

fn publish_depot_progress(
    request: &DepotOperationRequest,
    state: &str,
    completed: u64,
    downloaded: u64,
    written: u64,
    write_total: u64,
    total: u64,
) {
    let download_total_bytes = DEPOT_MANAGER
        .lock()
        .unwrap()
        .snapshots
        .get(&request.operation_id)
        .and_then(|snapshot| snapshot.download_total_bytes);
    publish_depot(DepotOperationSnapshot {
        setup: None,
        operation_id: request.operation_id.clone(),
        product_id: request.product_id,
        state: state.into(),
        bytes_completed: completed,
        bytes_downloaded: downloaded,
        bytes_written: written,
        total_write_bytes: write_total,
        total_bytes: total,
        download_total_bytes,
        error: None,
    });
}

fn publish_depot_setup(
    request: &DepotOperationRequest,
    component: &str,
    completed: usize,
    total: usize,
) {
    publish_depot(DepotOperationSnapshot {
        operation_id: request.operation_id.clone(),
        product_id: request.product_id,
        state: "setup".into(),
        setup: Some(DepotSetupProgress {
            component: component.into(),
            completed,
            total,
        }),
        bytes_completed: 0,
        bytes_downloaded: 0,
        bytes_written: 0,
        total_write_bytes: 0,
        total_bytes: 0,
        download_total_bytes: None,
        error: None,
    });
}

#[derive(Clone)]
struct LocalChunk {
    path: String,
    offset: u64,
    size: u64,
    md5: String,
}

fn local_chunk_candidates(
    manifest: &crate::gog::depot_manifest::DepotManifest,
) -> HashMap<String, Vec<LocalChunk>> {
    use crate::gog::depot_manifest::DepotEntry;
    let mut candidates = HashMap::<String, Vec<LocalChunk>>::new();
    for entry in &manifest.entries {
        let DepotEntry::File(file) = entry else {
            continue;
        };
        let mut offset = 0_u64;
        for chunk in &file.chunks {
            candidates
                .entry(chunk.md5.clone())
                .or_default()
                .push(LocalChunk {
                    path: file.path.clone(),
                    offset,
                    size: chunk.size,
                    md5: chunk.md5.clone(),
                });
            offset = offset.saturating_add(chunk.size);
        }
    }
    candidates
}

fn reuse_local_chunk(
    root: &std::path::Path,
    candidates: &HashMap<String, Vec<LocalChunk>>,
    chunk: &crate::gog::depot_manifest::DepotChunk,
    output: &mut dyn std::io::Write,
) -> anyhow::Result<bool> {
    use std::io::{Read, Seek};
    let Some(candidates) = candidates.get(&chunk.md5) else {
        return Ok(false);
    };
    for candidate in candidates {
        if candidate.size != chunk.size || candidate.md5 != chunk.md5 {
            continue;
        }
        let path = root.join(&candidate.path);
        if std::fs::symlink_metadata(&path)
            .ok()
            .is_none_or(|metadata| !metadata.is_file() || metadata.file_type().is_symlink())
        {
            continue;
        }
        let mut file = std::fs::File::open(&path)?;
        file.seek(std::io::SeekFrom::Start(candidate.offset))?;
        let mut remaining = candidate.size;
        let mut digest = md5::Context::new();
        let mut buffer = [0_u8; 64 * 1024];
        while remaining > 0 {
            let limit = usize::try_from(remaining.min(buffer.len() as u64)).unwrap();
            let read = file.read(&mut buffer[..limit])?;
            if read == 0 {
                break;
            }
            digest.consume(&buffer[..read]);
            remaining -= read as u64;
        }
        if remaining != 0 || format!("{:x}", digest.compute()) != candidate.md5 {
            continue;
        }
        file.seek(std::io::SeekFrom::Start(candidate.offset))?;
        let mut source = file.take(candidate.size);
        if std::io::copy(&mut source, output)? != candidate.size {
            anyhow::bail!("installed depot chunk changed while being reused");
        }
        return Ok(true);
    }
    Ok(false)
}

fn reusable_local_chunks(
    root: &std::path::Path,
    candidates: &HashMap<String, Vec<LocalChunk>>,
    chunks: &[crate::gog::depot_manifest::DepotChunk],
) -> anyhow::Result<std::collections::HashSet<(String, u64)>> {
    let mut reusable = std::collections::HashSet::new();
    for chunk in chunks {
        let key = (chunk.md5.clone(), chunk.size);
        if reusable.contains(&key) {
            continue;
        }
        let mut sink = std::io::sink();
        if reuse_local_chunk(root, candidates, chunk, &mut sink)? {
            reusable.insert(key);
        }
    }
    Ok(reusable)
}

fn required_network_bytes(
    chunks: &[crate::gog::depot_manifest::DepotChunk],
    reusable: &std::collections::HashSet<(String, u64)>,
    support_bytes: u64,
) -> anyhow::Result<u64> {
    let mut served = std::collections::HashSet::new();
    chunks
        .iter()
        .filter(|chunk| served.insert(chunk.compressed_md5.clone()))
        .filter(|chunk| !reusable.contains(&(chunk.md5.clone(), chunk.size)))
        .try_fold(support_bytes, |total, chunk| {
            total
                .checked_add(chunk.compressed_size)
                .ok_or_else(|| anyhow::anyhow!("depot network size overflows"))
        })
}

fn current_manifest(
    request: &DepotOperationRequest,
) -> anyhow::Result<Option<crate::gog::depot_manifest::DepotManifest>> {
    if let Some(marker) = super::marker::load(&request.destination)?
        && let Some(provenance) = marker.galaxy_depot.as_ref()
        && let Some(record) = StateStore::open()?.depot_manifest(
            &provenance.manifest_fingerprint,
            request.product_id,
            &provenance.build_id,
            "ludomere:installed-with-dependencies",
        )?
    {
        let manifest = crate::gog::depot_manifest::parse_snapshot(record.manifest_json.as_bytes())?;
        anyhow::ensure!(
            manifest.identity() == provenance.manifest_fingerprint,
            "Installed dependency ownership manifest is damaged"
        );
        return Ok(Some(manifest));
    }
    if request.current_sources.is_empty() {
        return request
            .current_manifest_json
            .as_deref()
            .map(|json| crate::gog::depot_manifest::parse_snapshot(json.as_bytes()))
            .transpose();
    }
    let marker = super::marker::load(&request.destination)?
        .ok_or_else(|| anyhow::anyhow!("existing depot marker is missing"))?;
    let provenance = marker
        .galaxy_depot
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("existing depot provenance is missing"))?;
    let mut current = request.clone();
    current.sources = request.current_sources.clone();
    current.current_sources.clear();
    current.build_id = provenance.build_id.clone();
    current.branch = provenance.branch.clone();
    let fingerprint = provenance.manifest_fingerprint.clone();
    current.target_marker = marker;
    // Old markers hash only their installed wire payload, not today's target dependencies.
    // Derive indices afresh from those strict sources; never accept this legacy hash for
    // a typed snapshot or a newly planned target.
    current.dependency_plan = None;
    current
        .target_marker
        .galaxy_depot
        .as_mut()
        .unwrap()
        .manifest_fingerprint
        .clear();
    let manifest = merge_depot_sources(&current)?.0;
    anyhow::ensure!(
        manifest.identity() == fingerprint
            || (manifest.small_files_containers.len() > 1
                && manifest.legacy_payload_identity() == fingerprint),
        "Installed depot source manifests do not match marker provenance; prepare recovery before updating"
    );
    Ok(Some(manifest))
}

fn materialize_support_network(
    request: &DepotOperationRequest,
    manifest: &crate::gog::depot_manifest::DepotManifest,
    chunk_sources: &HashMap<String, (i64, String)>,
    cancelled: &std::sync::atomic::AtomicBool,
    completed: &mut u64,
    downloaded: &mut u64,
    total: u64,
) -> anyhow::Result<PathBuf> {
    let support_staging = super::depot_actions::support_staging(&request.staging_path)?;
    super::depot::disk_preflight(
        manifest,
        &request.library_root,
        &support_staging,
        &request.staging_path.with_extension("json.support"),
    )?;
    let client = reqwest::blocking::Client::new();
    let roots = chunk_sources
        .values()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .enumerate()
        .map(|(index, root)| (root, index))
        .collect::<HashMap<_, _>>();
    let mut endpoints = HashMap::new();
    let mut states = HashMap::<usize, SourceRetryState>::new();
    let mut served = std::collections::HashSet::new();
    let operation_id = request.operation_id.clone();
    super::depot_actions::materialize_support(
        manifest,
        &request.staging_path,
        |chunks, output, completed_chunk| {
            for (index, job) in chunks.iter().enumerate() {
                let chunk = job.chunk;
                let mut output =
                    crate::download::depot::FileRegionWriter::new(output.try_clone()?, job.offset);
                let source = chunk_sources
                    .get(&chunk.compressed_md5)
                    .ok_or_else(|| anyhow::anyhow!("support chunk has no content source"))?;
                let source_index = roots[source];
                let temporary = super::depot_actions::support_staging(&request.staging_path)?
                    .join(format!(".fetch-{}", chunk.compressed_md5));
                loop {
                    if cancelled.load(std::sync::atomic::Ordering::Relaxed) {
                        return Err(crate::download::depot::DepotCancelled.into());
                    }
                    if let std::collections::hash_map::Entry::Vacant(entry) =
                        endpoints.entry(source_index)
                    {
                        entry.insert(crate::download::depot::acquire_secure_links(
                            &client,
                            &request.access_token,
                            source.0,
                            &source.1,
                        )?);
                    }
                    let state = states.get(&source_index).copied().unwrap_or_default();
                    let endpoint = endpoints
                        .get(&source_index)
                        .and_then(|links| links.urls.get(state.endpoint))
                        .ok_or_else(|| anyhow::anyhow!("support source has no secure endpoint"))?;
                    let url = crate::download::depot::chunk_url(endpoint, &chunk.compressed_md5)?;
                    let mut staged = std::fs::OpenOptions::new()
                        .create(true)
                        .truncate(true)
                        .read(true)
                        .write(true)
                        .open(&temporary)?;
                    match crate::download::depot::download_chunk_to(
                        &client,
                        &url,
                        chunk,
                        &mut staged,
                    ) {
                        Ok(()) => {
                            *downloaded = downloaded
                                .checked_add(chunk.compressed_size)
                                .ok_or_else(|| {
                                    anyhow::anyhow!("depot download progress overflow")
                                })?;
                            use std::io::Seek;
                            staged.seek(std::io::SeekFrom::Start(0))?;
                            std::io::copy(&mut staged, &mut output)?;
                            drop(staged);
                            std::fs::remove_file(&temporary)?;
                            break;
                        }
                        Err(error) => {
                            let count = endpoints
                                .get(&source_index)
                                .map_or(0, |links| links.urls.len());
                            let (action, next) = retry_action(error.kind(), state, count);
                            states.insert(source_index, next);
                            match action {
                                RetryAction::Refresh => {
                                    endpoints.remove(&source_index);
                                }
                                RetryAction::NextEndpoint => {}
                                RetryAction::Stop => return Err(error.into()),
                            }
                        }
                    }
                }
                if served.insert(chunk.compressed_md5.clone()) {
                    *completed = completed
                        .checked_add(chunk.compressed_size)
                        .ok_or_else(|| anyhow::anyhow!("depot progress overflow"))?;
                }
                update_depot_record(&operation_id, "finalizing", *completed, None, false).map(
                    |()| {
                        publish_depot_progress(
                            request,
                            "finalizing",
                            *completed,
                            *downloaded,
                            0,
                            0,
                            total,
                        )
                    },
                )?;
                completed_chunk(index)?;
            }
            Ok(())
        },
        || cancelled.load(std::sync::atomic::Ordering::Relaxed),
    )?;
    super::depot_actions::support_staging(&request.staging_path)
}

fn finalize_depot_metadata(
    request: &DepotOperationRequest,
    _support: Option<&std::path::Path>,
    removed_actions: &[(i64, Vec<super::depot_metadata::DepotScriptAction>)],
    cancelled: &std::sync::atomic::AtomicBool,
    prepared: &[crate::gog::dependencies::PreparedDependency],
    session: u64,
) -> anyhow::Result<()> {
    let prefix_recovery = super::prefix_recovery::setup_ticket(
        &request.destination,
        &request.target_marker,
        request.kind == crate::domain::DepotOperationKind::Repair,
    )?;
    let language = request
        .target_marker
        .base
        .language
        .as_deref()
        .unwrap_or("en-US");
    let bitness = request
        .target_marker
        .galaxy_depot
        .as_ref()
        .and_then(|provenance| provenance.architecture.as_deref());
    let mut marker = request.target_marker.clone();
    write_entitlement_markers(request, language)?;
    let info = request
        .destination
        .join(format!("goggame-{}.info", request.product_id));
    if info.is_file() {
        super::depot_metadata::set_marker_launch(
            &mut marker,
            &std::fs::read(info)?,
            language,
            bitness,
        )?;
    }
    if marker
        .base
        .operating_system
        .as_deref()
        .is_some_and(|value| value.eq_ignore_ascii_case("windows"))
    {
        let repository = setup_repository(&StateStore::open()?, request, cancelled, |identity| {
            crate::gog::depot_acquisition::fetch_repository(
                &reqwest::blocking::Client::builder()
                    .connect_timeout(Duration::from_secs(15))
                    .timeout(Duration::from_secs(45))
                    .build()?,
                &request.access_token,
                identity,
            )
        })?;
        let compatibility = marker
            .compatibility
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Windows depot marker has no compatibility profile"))?;
        if request.library_id.is_empty() {
            anyhow::bail!("Windows depot operation has no library identity");
        }
        let backend = crate::compatibility::backend_for_game(request.product_id)?;
        let log_path = super::executor::installation_log_path(request.product_id)?;
        std::fs::File::create(&log_path)?;
        for (name, path) in [
            ("library", request.library_root.as_path()),
            ("game", request.destination.as_path()),
            ("operation journal", request.staging_path.as_path()),
        ] {
            crate::compatibility::append_step_log(
                &log_path,
                &format!("{name} path: {}", path.display()),
            )?;
        }
        let stopped = || {
            cancelled.load(std::sync::atomic::Ordering::Relaxed)
                || crate::online::account_session() != session
                || !super::recovery::current(request.product_id, request.recovery_generation)
        };
        publish_depot_setup(request, "Preparing Windows prefix", 0, 0);
        let prefix = backend.initialize_prefix_controlled(
            crate::compatibility::InitializePrefixRequest {
                library_id: request.library_id.clone(),
                library: request.library_root.clone(),
                slug: request.slug.clone(),
                profile: compatibility.profile.clone(),
                log_path: log_path.clone(),
            },
            |command, log| {
                super::dependency_setup::run_tracked(
                    Some(&request.operation_id),
                    &stopped,
                    "Prefix initialization",
                    log,
                    || {
                        Ok(crate::compatibility::CompatibilityProcess::spawn(
                            command, log,
                        )?)
                    },
                )
            },
        )?;
        let prefix = request.library_root.join(prefix.relative_path);
        crate::compatibility::append_step_log(
            &log_path,
            &format!("prefix path: {}", prefix.display()),
        )?;
        let context = super::depot_actions::ActionContext {
            operation_id: Some(request.operation_id.clone()),
            product_id: request.product_id,
            app: request.destination.clone(),
            support: super::depot_actions::support_staging(&request.staging_path)?,
            prefix,
            windows_app: crate::compatibility::windows_destination(&request.slug),
            profile: compatibility.profile.clone(),
            log_path,
            galaxy_setup: None,
        };
        let plan = request.dependency_plan.as_ref().ok_or_else(|| {
            anyhow::anyhow!("Missing resolved dependency plan; retry preparation")
        })?;
        for dependency in prepared.iter().filter(|item| {
            matches!(
                item.dependency.method,
                crate::gog::dependencies::Method::GameFiles
            )
        }) {
            crate::gog::dependencies::publish_game_files(
                dependency,
                &request.destination,
                stopped,
            )?;
        }
        super::dependency_setup::apply(
            &backend,
            plan,
            prepared,
            &context,
            stopped,
            |name, completed, total| {
                let _ = crate::compatibility::append_step_log(
                    &context.log_path,
                    &format!("Applying required component: {name}"),
                );
                publish_depot_setup(request, name, completed, total);
            },
        )?;
        publish_depot_setup(request, "Finishing game setup", 0, 0);
        marker.dependencies = request.dependencies.clone();
        for (product_id, actions) in removed_actions {
            let context = super::depot_actions::ActionContext {
                product_id: *product_id,
                ..context.clone()
            };
            super::depot_actions::execute_actions_controlled(
                &backend, &context, actions, true, &stopped,
            )?;
        }
        let mut products = vec![request.product_id];
        products.extend(marker.dlc.iter().map(|dlc| dlc.product_id));
        for product_id in products {
            if repository.script_interpreter {
                let context = super::depot_actions::ActionContext {
                    product_id,
                    ..context.clone()
                };
                super::dependency_setup::interpret(
                    &backend,
                    plan,
                    prepared,
                    &context,
                    &super::depot_actions::GalaxySetup {
                        executable: String::new(),
                        arguments: String::new(),
                        language: language.into(),
                        build_id: request.build_id.clone(),
                        version: marker.base.version.clone().unwrap_or_default(),
                    },
                    &request.operation_id,
                    stopped,
                )?;
                continue;
            }
            let script = request
                .destination
                .join(format!("goggame-{product_id}.script"));
            if !script.is_file() {
                continue;
            }
            let actions = super::depot_metadata::script_actions(
                &std::fs::read(script)?,
                product_id,
                language,
            )?;
            let context = super::depot_actions::ActionContext {
                product_id,
                galaxy_setup: if repository.script_interpreter {
                    None
                } else {
                    repository
                        .products
                        .iter()
                        .find(|product| product.product_id == product_id.to_string())
                        .and_then(|product| {
                            product
                                .temp_executable
                                .as_ref()
                                .filter(|name| !name.is_empty())
                                .map(|executable| super::depot_actions::GalaxySetup {
                                    executable: executable.clone(),
                                    arguments: product.temp_arguments.clone().unwrap_or_default(),
                                    language: language.to_owned(),
                                    build_id: request.build_id.clone(),
                                    version: marker.base.version.clone().unwrap_or_default(),
                                })
                        })
                },
                ..context.clone()
            };
            super::depot_actions::execute_actions_controlled(
                &backend, &context, &actions, false, &stopped,
            )?;
        }
    }
    anyhow::ensure!(
        !cancelled.load(std::sync::atomic::Ordering::Relaxed)
            && crate::online::account_session() == session
            && super::recovery::current(request.product_id, request.recovery_generation),
        "Account or operation changed before publishing installation success"
    );
    let (payload, _) = merge_depot_sources(request)?;
    let combined =
        super::dependency_setup::combined_manifest(&payload, request.dependency_plan.as_ref())?;
    let now = chrono::Utc::now().timestamp();
    StateStore::open()?.save_depot_manifest(&crate::state::DepotManifestRecord {
        manifest_identity: combined.identity(),
        product_id: request.product_id,
        build_id: request.build_id.clone(),
        depot_id: "ludomere:installed-with-dependencies".into(),
        manifest_json: combined.snapshot_json()?,
        first_seen_at: now,
        last_seen_at: now,
    })?;
    crate::online::with_account_session(session, || {
        let _admission =
            super::recovery::admit_generation(request.product_id, request.recovery_generation)?;
        anyhow::ensure!(
            !cancelled.load(std::sync::atomic::Ordering::Relaxed),
            "Setup cancelled before publishing installation success"
        );
        super::marker::write(&marker, &request.destination)
    })?;
    super::prefix_recovery::setup_completed(prefix_recovery, &marker, false)
}

fn setup_repository(
    store: &StateStore,
    request: &DepotOperationRequest,
    cancelled: &std::sync::atomic::AtomicBool,
    fetch: impl FnOnce(&str) -> anyhow::Result<crate::gog::types::GenerationTwoRepository>,
) -> anyhow::Result<crate::gog::types::GenerationTwoRepository> {
    use anyhow::{Context, ensure};
    if cancelled.load(std::sync::atomic::Ordering::Relaxed) {
        return Err(crate::download::depot::DepotCancelled.into());
    }
    let identity = &request
        .target_marker
        .galaxy_depot
        .as_ref()
        .context("Depot setup has no selected repository identity")?
        .repository_id;
    let cached = store.depot_repository(request.product_id, "windows", &request.build_id)?;
    if let Some(cached) = &cached {
        ensure!(
            cached.manifest_identity == *identity && cached.branch == request.branch,
            "Cached setup repository does not match the selected build"
        );
    }
    let parsed = cached
        .as_ref()
        .map(|record| {
            serde_json::from_str::<crate::gog::types::GenerationTwoRepository>(
                &record.repository_json,
            )
        })
        .transpose()?;
    let refresh = parsed
        .as_ref()
        .is_none_or(|repository| repository.setup_metadata_version == 0);
    let repository = if refresh {
        fetch(identity)
            .context("Refreshing selected build setup metadata; retry Resume when online")?
    } else {
        parsed.unwrap()
    };
    ensure!(
        repository.setup_metadata_version == 1
            && repository.generation == 2
            && repository.root_product_id == request.product_id.to_string()
            && repository
                .build_id
                .as_deref()
                .is_none_or(|id| id == request.build_id)
            && repository
                .platform
                .as_deref()
                .is_none_or(|platform| platform.eq_ignore_ascii_case("windows")),
        "Setup repository identity does not match the selected Windows build"
    );
    if cancelled.load(std::sync::atomic::Ordering::Relaxed) {
        return Err(crate::download::depot::DepotCancelled.into());
    }
    if refresh {
        let now = chrono::Utc::now().timestamp();
        store.save_depot_repository(&crate::state::DepotRepositoryRecord {
            product_id: request.product_id,
            operating_system: "windows".into(),
            build_id: request.build_id.clone(),
            branch: request.branch.clone(),
            manifest_identity: identity.clone(),
            repository_json: serde_json::to_string(&repository)?,
            first_seen_at: cached.as_ref().map_or(now, |record| record.first_seen_at),
            last_seen_at: now,
        })?;
    }
    Ok(repository)
}

fn write_entitlement_markers(
    request: &DepotOperationRequest,
    language: &str,
) -> anyhow::Result<()> {
    let expected = request
        .target_marker
        .galaxy_depot
        .as_ref()
        .into_iter()
        .flat_map(|provenance| &provenance.dlc)
        .filter(|dlc| dlc.entitlement_only_marker)
        .map(|dlc| dlc.product_id)
        .collect::<std::collections::BTreeSet<_>>();
    let supplied = request
        .entitlement_dlc
        .iter()
        .map(|dlc| dlc.product_id)
        .collect::<std::collections::BTreeSet<_>>();
    if expected != supplied || supplied.len() != request.entitlement_dlc.len() {
        anyhow::bail!("entitlement-only DLC metadata does not match target provenance");
    }
    for dlc in &request.entitlement_dlc {
        if dlc.product_id <= 0 || dlc.name.is_empty() || dlc.name.contains(['\0', '\r', '\n']) {
            anyhow::bail!("invalid entitlement-only DLC metadata");
        }
        let path = request
            .destination
            .join(format!("goggame-{}.info", dlc.product_id));
        if std::fs::symlink_metadata(&path).is_ok_and(|metadata| metadata.file_type().is_symlink())
        {
            anyhow::bail!("entitlement-only DLC marker is a symlink");
        }
        let mut bytes = serde_json::to_vec_pretty(&serde_json::json!({
            "languages": [language],
            "version": 1,
            "name": dlc.name,
            "language": language,
            "gameId": dlc.product_id.to_string(),
            "rootGameId": request.product_id.to_string(),
            "playTasks": [],
            "buildId": request.build_id,
        }))?;
        bytes.push(b'\n');
        let staging = super::depot_actions::support_staging(&request.staging_path)?;
        std::fs::create_dir_all(&staging)?;
        let temporary = staging.join(format!("entitlement-{}.info", dlc.product_id));
        if temporary.exists() {
            std::fs::remove_file(&temporary)?;
        }
        let mut options = std::fs::OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        use std::io::Write;
        let mut file = options.open(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        std::fs::rename(temporary, path)?;
    }
    Ok(())
}

#[cfg(test)]
fn dependency_verbs(dependencies: &[String]) -> anyhow::Result<Vec<String>> {
    let mut verbs = std::collections::BTreeSet::new();
    for dependency in dependencies {
        match dependency.as_str() {
            "DirectX" => {
                verbs.extend(
                    ["d3dcompiler_43", "d3dx9", "xact", "xinput"]
                        .into_iter()
                        .map(str::to_owned),
                );
            }
            "MSVC2010" | "MSVC2010_x64" => {
                verbs.insert("vcrun2010".into());
            }
            "MSVC2012" | "MSVC2012_x64" => {
                verbs.insert("vcrun2012".into());
            }
            "MSVC2013" | "MSVC2013_x64" => {
                verbs.insert("vcrun2013".into());
            }
            "MSVC2015" | "MSVC2015_x64" => {
                verbs.insert("vcrun2015".into());
            }
            "MSVC2019" | "MSVC2019_x64" => {
                verbs.insert("vcrun2019".into());
            }
            unknown => anyhow::bail!("unsupported required GOG dependency {unknown}"),
        }
    }
    // Winetricks' 2019 runtime covers 2015 too; installing both verbs conflicts.
    if verbs.contains("vcrun2019") {
        verbs.remove("vcrun2015");
    }
    Ok(verbs.into_iter().collect())
}

#[cfg(test)]
fn changed_dependency_verbs(
    installed: &[String],
    requested: &[String],
) -> anyhow::Result<Vec<String>> {
    if installed == requested {
        Ok(Vec::new())
    } else {
        dependency_verbs(requested)
    }
}

pub(crate) fn pending_dependency_verbs(
    prefix: &std::path::Path,
    requested: Vec<String>,
) -> anyhow::Result<Vec<String>> {
    use anyhow::Context;
    use std::io::Read;
    use std::os::{
        fd::{AsRawFd, FromRawFd},
        unix::fs::{MetadataExt, OpenOptionsExt},
    };
    if requested.is_empty() {
        return Ok(requested);
    }
    let directory = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
        .open(prefix)?;
    // The selected prefix anchors this read; a log link cannot redirect it elsewhere.
    let descriptor = unsafe {
        libc::openat(
            directory.as_raw_fd(),
            c"winetricks.log".as_ptr(),
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
        )
    };
    if descriptor < 0 {
        let error = std::io::Error::last_os_error();
        if error.kind() == std::io::ErrorKind::NotFound {
            return Ok(requested);
        }
        return Err(anyhow::Error::from(error))
            .context("could not inspect installed Windows dependencies");
    }
    let file = unsafe { std::fs::File::from_raw_fd(descriptor) };
    let metadata = file.metadata()?;
    anyhow::ensure!(
        metadata.is_file() && metadata.nlink() == 1,
        "Winetricks history must be a regular file with a single link"
    );
    let mut text = String::new();
    file.take(1024 * 1024 + 1)
        .read_to_string(&mut text)
        .context("could not read installed Windows dependencies")?;
    anyhow::ensure!(
        text.len() <= 1024 * 1024,
        "Winetricks history exceeds its size limit"
    );
    let installed = text
        .lines()
        .map(str::trim)
        .collect::<std::collections::HashSet<_>>();
    anyhow::ensure!(
        installed
            .iter()
            .all(|line| line.len() <= 1024 && !line.chars().any(char::is_control)),
        "Winetricks history contains invalid entries"
    );
    Ok(requested
        .into_iter()
        .filter(|verb| !installed.contains(verb.as_str()))
        .collect())
}

fn dependency_commit_marker(
    target: &super::marker::InstallationMarker,
    installed: Option<&super::marker::InstallationMarker>,
) -> super::marker::InstallationMarker {
    let mut marker = target.clone();
    marker.dependencies = installed
        .map(|marker| marker.dependencies.clone())
        .unwrap_or_default();
    marker
}

fn removed_dlc_actions(
    request: &DepotOperationRequest,
) -> anyhow::Result<Vec<(i64, Vec<super::depot_metadata::DepotScriptAction>)>> {
    if request.kind == DepotOperationKind::Install || !request.destination.is_dir() {
        return Ok(Vec::new());
    }
    let Some(current) = super::marker::load(&request.destination)? else {
        return Ok(Vec::new());
    };
    let retained = request
        .target_marker
        .dlc
        .iter()
        .map(|dlc| dlc.product_id)
        .collect::<std::collections::BTreeSet<_>>();
    let language = current.base.language.as_deref().unwrap_or("en-US");
    current
        .dlc
        .into_iter()
        .filter(|dlc| !retained.contains(&dlc.product_id))
        .filter_map(|dlc| {
            let path = request
                .destination
                .join(format!("goggame-{}.script", dlc.product_id));
            path.is_file().then_some((dlc.product_id, path))
        })
        .map(|(product_id, path)| {
            let parsed =
                super::depot_metadata::script_actions(&std::fs::read(path)?, product_id, language)?;
            let mut uninstall = parsed
                .iter()
                .filter(|action| action.uninstall)
                .cloned()
                .collect::<Vec<_>>();
            let names = uninstall
                .iter()
                .map(|action| action.name.clone())
                .collect::<std::collections::BTreeSet<_>>();
            uninstall.extend(
                parsed
                    .iter()
                    .filter(|action| {
                        !action.uninstall
                            && action.kind
                                == super::depot_metadata::DepotScriptActionKind::SetRegistry
                            && !names.contains(&action.name)
                    })
                    .cloned()
                    .map(|mut action| {
                        action.uninstall = true;
                        action
                    }),
            );
            Ok((product_id, uninstall))
        })
        .collect()
}

fn forced_dlc_removals(
    request: &DepotOperationRequest,
) -> anyhow::Result<std::collections::BTreeSet<String>> {
    if request.kind == DepotOperationKind::Install {
        return Ok(Default::default());
    }
    let current_marker = super::marker::load(&request.destination)?
        .ok_or_else(|| anyhow::anyhow!("existing depot marker is missing"))?;
    forced_dlc_removals_for(request, &current_marker)
}

fn forced_dlc_removals_for(
    request: &DepotOperationRequest,
    current_marker: &super::marker::InstallationMarker,
) -> anyhow::Result<std::collections::BTreeSet<String>> {
    use crate::gog::depot_manifest::DepotEntry;
    let current = current_marker
        .galaxy_depot
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("existing depot provenance is missing"))?;
    let target = request
        .target_marker
        .galaxy_depot
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("target depot provenance is missing"))?;
    let expected = current
        .depots
        .iter()
        .map(|depot| {
            (
                request.product_id,
                depot.depot_id.as_str(),
                depot.manifest_id.as_str(),
            )
        })
        .chain(current.dlc.iter().flat_map(|dlc| {
            dlc.depots.iter().map(move |depot| {
                (
                    dlc.product_id,
                    depot.depot_id.as_str(),
                    depot.manifest_id.as_str(),
                )
            })
        }))
        .collect::<std::collections::BTreeSet<_>>();
    let supplied = request
        .current_sources
        .iter()
        .map(|source| {
            (
                source.product_id,
                source.depot_id.as_str(),
                source.manifest_id.as_str(),
            )
        })
        .collect::<std::collections::BTreeSet<_>>();
    if expected != supplied {
        anyhow::bail!("current depot sources do not match installed provenance");
    }
    let target_dlc = target
        .dlc
        .iter()
        .map(|dlc| dlc.product_id)
        .collect::<std::collections::BTreeSet<_>>();
    let removed = current
        .dlc
        .iter()
        .map(|dlc| dlc.product_id)
        .filter(|product_id| !target_dlc.contains(product_id))
        .collect::<Vec<_>>();
    let mut paths = super::depot::removed_dlc_marker_paths(&removed)?;
    let mut target_entries = HashMap::new();
    for source in request
        .sources
        .iter()
        .filter(|source| source.product_id == request.product_id)
        .chain(
            request
                .sources
                .iter()
                .filter(|source| source.product_id != request.product_id),
        )
    {
        if let Some(raw) = source.manifest_json.as_deref() {
            for entry in crate::gog::depot_manifest::parse(raw.as_bytes())?.entries {
                let key = match &entry {
                    DepotEntry::Directory { path } | DepotEntry::Link { path, .. } => path,
                    DepotEntry::File(file) => &file.path,
                }
                .to_lowercase();
                target_entries.insert(key, entry);
            }
        }
    }
    for source in request
        .current_sources
        .iter()
        .filter(|source| removed.contains(&source.product_id))
    {
        let Some(raw) = source.manifest_json.as_deref() else {
            continue;
        };
        for entry in crate::gog::depot_manifest::parse(raw.as_bytes())?.entries {
            match entry {
                DepotEntry::Directory { .. } => {}
                DepotEntry::Link { ref path, .. } => {
                    if !target_entries
                        .get(&path.to_lowercase())
                        .is_some_and(|target| entries_equivalent(target, &entry))
                    {
                        paths.insert(path.clone());
                    }
                }
                DepotEntry::File(ref file) => {
                    if !target_entries
                        .get(&file.path.to_lowercase())
                        .is_some_and(|target| entries_equivalent(target, &entry))
                    {
                        paths.insert(file.path.clone());
                    }
                }
            }
        }
    }
    Ok(paths)
}

fn persist_depot_request(request: &DepotOperationRequest) -> anyhow::Result<()> {
    let (manifest, _) = merge_depot_sources(request)?;
    let (support, _) = merge_support_sources(request)?;
    let total = manifest
        .totals()?
        .compressed
        .checked_add(support.totals()?.compressed)
        .ok_or_else(|| anyhow::anyhow!("depot operation size overflows"))?;
    let journal_path = super::operation_journal::depot_path(&request.staging_path);
    let previous = match super::operation_journal::read(&journal_path) {
        Ok(super::operation_journal::OperationJournal::Depot { record, .. }) => Some(record),
        Ok(_) => anyhow::bail!("A different operation already owns this game journal"),
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound) =>
        {
            None
        }
        Err(error) => return Err(error),
    };
    if let Some(record) = &previous {
        super::dependency_setup::ensure_setup_quiescent(record)?;
    }
    let now = chrono::Utc::now().timestamp();
    let record = DepotOperationRecord {
        operation_id: request.operation_id.clone(),
        product_id: request.product_id,
        build_id: request.build_id.clone(),
        branch: request.branch.clone(),
        kind: format!("{:?}", request.kind).to_lowercase(),
        state: "queued".into(),
        destination: request.destination.clone(),
        staging_path: request.staging_path.clone(),
        plan_json: serde_json::to_string(&PersistedDepotPlan::from(request))?,
        bytes_completed: previous.as_ref().map_or(0, |record| record.bytes_completed),
        total_bytes: Some(total),
        error: None,
        created_at: previous.as_ref().map_or(now, |record| record.created_at),
        updated_at: now,
        completed_at: None,
    };
    super::operation_journal::write_depot(&journal_path, &record)?;
    Ok(())
}

fn merge_depot_sources(
    request: &DepotOperationRequest,
) -> anyhow::Result<(crate::gog::depot_manifest::DepotManifest, ChunkSources)> {
    use crate::gog::depot_manifest::{DepotEntry, DepotManifest};
    let provenance = request
        .target_marker
        .galaxy_depot
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("target marker has no depot provenance"))?;
    if provenance.build_id != request.build_id || provenance.branch != request.branch {
        anyhow::bail!("depot request does not match target build provenance");
    }
    let expected = provenance
        .depots
        .iter()
        .map(|depot| {
            (
                request.product_id,
                depot.depot_id.as_str(),
                depot.manifest_id.as_str(),
            )
        })
        .chain(provenance.dlc.iter().flat_map(|dlc| {
            dlc.depots.iter().map(move |depot| {
                (
                    dlc.product_id,
                    depot.depot_id.as_str(),
                    depot.manifest_id.as_str(),
                )
            })
        }))
        .collect::<std::collections::BTreeSet<_>>();
    let supplied = request
        .sources
        .iter()
        .map(|source| {
            (
                source.product_id,
                source.depot_id.as_str(),
                source.manifest_id.as_str(),
            )
        })
        .collect::<std::collections::BTreeSet<_>>();
    if expected != supplied {
        anyhow::bail!("selected depots do not match target marker provenance");
    }
    struct PathContributions {
        winner: usize,
        base: Option<DepotEntry>,
        dlc: Option<DepotEntry>,
    }
    let mut contributions = Vec::<(DepotEntry, (i64, String))>::new();
    let mut paths = HashMap::<String, PathContributions>::new();
    let mut chunk_sources = HashMap::<String, (i64, String)>::new();
    let mut chunks = HashMap::<String, (u64, String, u64)>::new();
    let mut small_files_containers = Vec::new();
    for source in request
        .sources
        .iter()
        .filter(|source| source.product_id == request.product_id)
        .chain(
            request
                .sources
                .iter()
                .filter(|source| source.product_id != request.product_id),
        )
    {
        match (&source.manifest_json, &source.content_root) {
            (None, None) => continue,
            (Some(raw), Some(root)) => {
                let (mut manifest, _) =
                    crate::gog::depot_manifest::parse(raw.as_bytes())?.split_support()?;
                let container_base = small_files_containers.len();
                for container in &manifest.small_files_containers {
                    for chunk in &container.chunks {
                        chunk_sources
                            .entry(chunk.compressed_md5.clone())
                            .or_insert_with(|| (source.product_id, root.clone()));
                    }
                }
                small_files_containers.append(&mut manifest.small_files_containers);
                for mut entry in manifest.entries {
                    if let DepotEntry::File(file) = &mut entry
                        && let Some(reference) = &mut file.small_file
                    {
                        reference.container_index = reference
                            .container_index
                            .checked_add(container_base)
                            .ok_or_else(|| {
                                anyhow::anyhow!("small-files container index overflows")
                            })?;
                    }
                    let path = match &entry {
                        DepotEntry::Directory { path } | DepotEntry::Link { path, .. } => path,
                        DepotEntry::File(file) => &file.path,
                    };
                    let key = path.to_lowercase();
                    let is_base = source.product_id == request.product_id;
                    if let Some(stack) = paths.get_mut(&key) {
                        let peer = if is_base {
                            &mut stack.base
                        } else {
                            &mut stack.dlc
                        };
                        if peer
                            .as_ref()
                            .is_some_and(|existing| !entries_equivalent(existing, &entry))
                        {
                            anyhow::bail!("selected peer depots contain a path collision");
                        }
                        if peer.is_some() {
                            continue;
                        }
                        *peer = Some(entry.clone());
                        if !is_base {
                            contributions[stack.winner] =
                                (entry, (source.product_id, root.clone()));
                        }
                        continue;
                    }
                    paths.insert(
                        key,
                        PathContributions {
                            winner: contributions.len(),
                            base: is_base.then(|| entry.clone()),
                            dlc: (!is_base).then(|| entry.clone()),
                        },
                    );
                    contributions.push((entry, (source.product_id, root.clone())));
                }
            }
            _ => anyhow::bail!("payload depot requires both a manifest and content root"),
        }
    }
    for (entry, source) in &contributions {
        if let DepotEntry::File(file) = entry {
            for chunk in &file.chunks {
                let metadata = (chunk.compressed_size, chunk.md5.clone(), chunk.size);
                if chunks
                    .get(&chunk.compressed_md5)
                    .is_some_and(|old| old != &metadata)
                {
                    anyhow::bail!("selected depots contain inconsistent chunk metadata");
                }
                chunks.insert(chunk.compressed_md5.clone(), metadata);
                chunk_sources
                    .entry(chunk.compressed_md5.clone())
                    .or_insert_with(|| source.clone());
            }
        }
    }
    let mut entries = contributions
        .into_iter()
        .map(|(entry, _)| entry)
        .collect::<Vec<_>>();
    let used = entries
        .iter()
        .filter_map(|entry| match entry {
            DepotEntry::File(file) => file.small_file.map(|reference| reference.container_index),
            _ => None,
        })
        .collect::<std::collections::BTreeSet<_>>();
    let remap = used
        .iter()
        .enumerate()
        .map(|(new, old)| (*old, new))
        .collect::<HashMap<_, _>>();
    let small_files_containers = small_files_containers
        .into_iter()
        .enumerate()
        .filter_map(|(index, container)| used.contains(&index).then_some(container))
        .collect();
    for entry in &mut entries {
        if let DepotEntry::File(file) = entry
            && let Some(reference) = &mut file.small_file
        {
            reference.container_index = remap[&reference.container_index];
        }
    }
    let manifest = DepotManifest {
        small_files_containers,
        generation: 2,
        entries,
    };
    if !provenance.manifest_fingerprint.is_empty()
        && provenance.manifest_fingerprint
            != super::dependency_setup::combined_manifest(
                &manifest,
                request.dependency_plan.as_ref(),
            )?
            .identity()
    {
        anyhow::bail!("combined depot manifest does not match target marker provenance");
    }
    Ok((manifest, chunk_sources))
}

pub(crate) fn planned_manifest_identity(request: &DepotOperationRequest) -> anyhow::Result<String> {
    if request
        .target_marker
        .galaxy_depot
        .as_ref()
        .is_none_or(|provenance| !provenance.manifest_fingerprint.is_empty())
    {
        anyhow::bail!("planned marker fingerprint must be empty");
    }
    Ok(super::dependency_setup::combined_manifest(
        &merge_depot_sources(request)?.0,
        request.dependency_plan.as_ref(),
    )?
    .identity())
}

fn merge_support_sources(
    request: &DepotOperationRequest,
) -> anyhow::Result<(crate::gog::depot_manifest::DepotManifest, ChunkSources)> {
    use crate::gog::depot_manifest::{DepotEntry, DepotManifest};
    let mut entries = Vec::new();
    let mut paths = HashMap::<String, DepotEntry>::new();
    let mut containers = Vec::new();
    let mut chunk_sources = HashMap::new();
    for source in &request.sources {
        let (Some(raw), Some(root)) = (&source.manifest_json, &source.content_root) else {
            continue;
        };
        let (_, mut support) =
            crate::gog::depot_manifest::parse(raw.as_bytes())?.split_support()?;
        let base = containers.len();
        for container in &support.small_files_containers {
            for chunk in &container.chunks {
                chunk_sources
                    .entry(chunk.compressed_md5.clone())
                    .or_insert_with(|| (source.product_id, root.clone()));
            }
        }
        containers.append(&mut support.small_files_containers);
        for mut entry in support.entries {
            let DepotEntry::File(file) = &mut entry else {
                anyhow::bail!("support depot contains a non-file entry");
            };
            if let Some(reference) = &mut file.small_file {
                reference.container_index = reference
                    .container_index
                    .checked_add(base)
                    .ok_or_else(|| anyhow::anyhow!("support container index overflows"))?;
            }
            let key = file.path.to_lowercase();
            if let Some(existing) = paths.get(&key) {
                if !entries_equivalent(existing, &entry) {
                    anyhow::bail!("selected support depots contain a path collision");
                }
                continue;
            }
            for chunk in &file.chunks {
                chunk_sources
                    .entry(chunk.compressed_md5.clone())
                    .or_insert_with(|| (source.product_id, root.clone()));
            }
            paths.insert(key, entry.clone());
            entries.push(entry);
        }
    }
    Ok((
        DepotManifest {
            generation: 2,
            entries,
            small_files_containers: containers,
        },
        chunk_sources,
    ))
}

fn entry_path(entry: &crate::gog::depot_manifest::DepotEntry) -> &str {
    use crate::gog::depot_manifest::DepotEntry;
    match entry {
        DepotEntry::File(file) => &file.path,
        DepotEntry::Directory { path } | DepotEntry::Link { path, .. } => path,
    }
}

pub(crate) fn entries_equivalent(
    left: &crate::gog::depot_manifest::DepotEntry,
    right: &crate::gog::depot_manifest::DepotEntry,
) -> bool {
    use crate::gog::depot_manifest::DepotEntry;
    match (left, right) {
        (DepotEntry::Directory { .. }, DepotEntry::Directory { .. }) => true,
        (DepotEntry::Link { target: left, .. }, DepotEntry::Link { target: right, .. }) => {
            left == right
        }
        (DepotEntry::File(left), DepotEntry::File(right)) => {
            left.size == right.size
                && left.executable == right.executable
                && left.support == right.support
                && left.md5 == right.md5
                && left.sha256 == right.sha256
                && left.chunks == right.chunks
        }
        _ => false,
    }
}

fn update_depot_record(
    operation_id: &str,
    state: &str,
    bytes_completed: u64,
    error: Option<&str>,
    terminal: bool,
) -> anyhow::Result<()> {
    let path = super::operation_journal::scan()?
        .into_iter()
        .find_map(|(path, journal)| match journal {
            super::operation_journal::OperationJournal::Depot { record, .. }
                if record.operation_id == operation_id =>
            {
                Some(path)
            }
            _ => None,
        })
        .ok_or_else(|| anyhow::anyhow!("saved depot operation was not found"))?;
    update_depot_record_at(&path, operation_id, state, bytes_completed, error, terminal)
}

fn update_depot_record_at(
    path: &std::path::Path,
    operation_id: &str,
    state: &str,
    bytes_completed: u64,
    error: Option<&str>,
    terminal: bool,
) -> anyhow::Result<()> {
    let super::operation_journal::OperationJournal::Depot { mut record, .. } =
        super::operation_journal::read(path)?
    else {
        anyhow::bail!("saved operation is not a depot operation");
    };
    if record.operation_id != operation_id {
        anyhow::bail!("saved depot operation identity changed");
    }
    let now = chrono::Utc::now().timestamp();
    record.state = state.into();
    record.bytes_completed = bytes_completed;
    record.error = error.map(str::to_owned);
    record.updated_at = now;
    record.completed_at = terminal.then_some(now);
    if terminal && state == "complete" {
        super::operation_journal::remove(path)
    } else {
        super::operation_journal::write_depot(path, &record)
    }
}

fn redact_error(message: &str, access_token: &str) -> String {
    message
        .split_whitespace()
        .map(|word| {
            if (!access_token.is_empty() && word.contains(access_token))
                || word.starts_with("http://")
                || word.starts_with("https://")
            {
                "[redacted]"
            } else {
                word
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn installation_operation_snapshot(product_id: i64) -> Option<InstallationOperationSnapshot> {
    MANAGER.lock().unwrap().snapshots.get(&product_id).cloned()
}

pub fn enqueue_installation(
    plan: InstalledGame,
    additional_installers: Vec<AdditionalInstaller>,
    install_base: bool,
    interactive_prompts: bool,
) -> bool {
    enqueue_installation_inner(
        plan,
        additional_installers,
        install_base,
        interactive_prompts,
        None,
    )
}

/// Receive only this accepted operation's events, with attempt-scoped cancellation.
pub fn enqueue_installation_tracked(
    plan: InstalledGame,
    additional_installers: Vec<AdditionalInstaller>,
    install_base: bool,
    interactive_prompts: bool,
) -> Option<TrackedInstallation> {
    let (sender, events) = mpsc::channel();
    let control = TrackedInstallationControl {
        product_id: plan.product_id,
        generation: super::recovery::generation(plan.product_id),
        cancellation: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
    };
    enqueue_installation_inner(
        plan,
        additional_installers,
        install_base,
        interactive_prompts,
        Some(InstallationTracking {
            sender,
            control: control.clone(),
        }),
    )
    .then_some(TrackedInstallation { events, control })
}

fn enqueue_installation_inner(
    plan: InstalledGame,
    additional_installers: Vec<AdditionalInstaller>,
    install_base: bool,
    interactive_prompts: bool,
    tracking: Option<InstallationTracking>,
) -> bool {
    if SIGN_OUT_PAUSE.load(std::sync::atomic::Ordering::Acquire) {
        return false;
    }
    let Ok(_activity) = crate::profile_reset::begin_activity("installation registration") else {
        return false;
    };
    let product_id = plan.product_id;
    if super::recovery::pending(&plan.installation_directory, product_id).unwrap_or(true) {
        return false;
    }
    let generation = tracking
        .as_ref()
        .map(|tracking| tracking.control.generation)
        .unwrap_or_else(|| super::recovery::generation(product_id));
    let Ok(admission) = super::recovery::admit_generation(product_id, generation) else {
        return false;
    };
    let persisted_plan = PersistedInstallationPlan {
        tracking,
        recovery_generation: generation,
        game: plan.clone(),
        additional_installers: additional_installers.clone(),
        install_base,
        interactive_prompts,
        download_intent_id: None,
    };
    let queue_position = {
        let mut manager = MANAGER.lock().unwrap();
        if manager.active.contains_key(&product_id)
            || manager
                .queue
                .iter()
                .any(|queued| queued.product_id() == product_id)
        {
            return false;
        }
        if let Some(tracking) = &persisted_plan.tracking {
            let _ = tracking.sender.send(InstallationEvent::Starting {
                message: "Queued for installation".into(),
            });
        }
        manager.next_queue_position += 1;
        let position = manager.next_queue_position;
        manager
            .queue
            .push_back(QueuedOperation::Installation(persisted_plan.clone()));
        manager.snapshots.insert(
            product_id,
            InstallationOperationSnapshot {
                product_id,
                state: crate::domain::InstallationState::Pending,
                message: Some("Queued for installation".into()),
                percentage: None,
                queued: true,
            },
        );
        position
    };
    persist_operation(
        product_id,
        "install",
        "queued",
        &persisted_plan,
        Some("Queued for installation"),
        None,
        Some(queue_position),
    );
    if let Some(snapshot) = installation_operation_snapshot(product_id) {
        publish(InstallationManagerEvent::OperationQueued(snapshot));
    }
    drop(admission);
    schedule_next();
    true
}

/// Persist the runnable journal and consume the explicit download intent before scheduling.
pub fn enqueue_downloaded_installation(
    store: &StateStore,
    intent_id: &str,
    game: InstalledGame,
    additional_installers: Vec<AdditionalInstaller>,
) -> anyhow::Result<()> {
    let _activity = crate::profile_reset::begin_activity("installation registration")?;
    anyhow::ensure!(
        !SIGN_OUT_PAUSE.load(std::sync::atomic::Ordering::Acquire),
        "Sign-out cleanup is pending; retry installation afterward"
    );
    let product_id = game.product_id;
    anyhow::ensure!(
        !super::recovery::pending(&game.installation_directory, product_id)?,
        "Finish this game's interrupted recovery before installing"
    );
    let generation = super::recovery::generation(product_id);
    let admission = super::recovery::admit_generation(product_id, generation)?;
    let plan = PersistedInstallationPlan {
        tracking: None,
        recovery_generation: generation,
        game,
        additional_installers,
        install_base: true,
        interactive_prompts: false,
        download_intent_id: Some(intent_id.to_owned()),
    };
    let mut manager = MANAGER.lock().unwrap();
    anyhow::ensure!(
        !manager.shutting_down,
        "Ludomere is closing; retry installation after restarting"
    );
    anyhow::ensure!(
        !manager.active.contains_key(&product_id)
            && !manager
                .queue
                .iter()
                .any(|queued| queued.product_id() == product_id),
        "An installation operation already exists for this game"
    );
    manager.next_queue_position += 1;
    persist_download_handoff(store, &plan, manager.next_queue_position)?;
    manager.queue.push_back(QueuedOperation::Installation(plan));
    let snapshot = InstallationOperationSnapshot {
        product_id,
        state: crate::domain::InstallationState::Pending,
        message: Some("Queued for automatic installation".into()),
        percentage: None,
        queued: true,
    };
    manager.snapshots.insert(product_id, snapshot.clone());
    drop(manager);
    publish(InstallationManagerEvent::OperationQueued(snapshot));
    drop(admission);
    schedule_next();
    Ok(())
}

fn persist_download_handoff(
    store: &StateStore,
    plan: &PersistedInstallationPlan,
    position: i64,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        download_intent_matches(store, plan)?,
        "The automatic installation request was removed or replaced"
    );
    let now = chrono::Utc::now().timestamp();
    let record = InstallationOperationRecord {
        product_id: plan.game.product_id,
        operation: "install".into(),
        state: "queued".into(),
        plan_json: serde_json::to_string(plan)?,
        message: Some("Queued for automatic installation".into()),
        percentage: None,
        queue_position: Some(position),
        created_at: now,
        updated_at: now,
        completed_at: None,
    };
    let path = super::operation_journal::offline_path(&record)?;
    if path.exists() {
        let super::operation_journal::OperationJournal::Offline {
            record: existing, ..
        } = super::operation_journal::read(&path)?
        else {
            anyhow::bail!("Another installation operation needs attention");
        };
        let existing: PersistedInstallationPlan = serde_json::from_str(&existing.plan_json)?;
        anyhow::ensure!(
            existing.download_intent_id == plan.download_intent_id,
            "Another installation operation needs attention"
        );
    } else {
        super::operation_journal::write_offline(&path, &record)?;
    }
    store.set_download_install_state(
        plan.download_intent_id.as_deref().unwrap(),
        "handed_off",
        None,
    )?;
    Ok(())
}

fn download_intent_matches(
    store: &StateStore,
    plan: &PersistedInstallationPlan,
) -> anyhow::Result<bool> {
    let Some(id) = &plan.download_intent_id else {
        return Ok(true);
    };
    Ok(store.download_install_intents()?.iter().any(|intent| {
        intent.intent_id == *id
            && intent.product_id == plan.game.product_id
            && matches!(intent.state.as_str(), "waiting" | "blocked" | "handed_off")
    }))
}

fn authorize_recovered_download(
    store: &StateStore,
    path: &std::path::Path,
    plan: &PersistedInstallationPlan,
) -> anyhow::Result<bool> {
    if !download_intent_matches(store, plan)? {
        std::fs::remove_file(path)?;
        return Ok(false);
    }
    store.set_download_install_state(
        plan.download_intent_id.as_deref().unwrap(),
        "handed_off",
        None,
    )?;
    Ok(true)
}

pub fn enqueue_uninstallation(game: InstalledGame) -> bool {
    enqueue_uninstallation_with_cleanup(game, None)
}

pub fn enqueue_uninstallation_with_cleanup(
    game: InstalledGame,
    cleanup: Option<crate::download::ManagedDownloads>,
) -> bool {
    if SIGN_OUT_PAUSE.load(std::sync::atomic::Ordering::Acquire) {
        return false;
    }
    let Ok(_activity) = crate::profile_reset::begin_activity("installation registration") else {
        return false;
    };
    let product_id = game.product_id;
    if super::recovery::pending(&game.installation_directory, product_id).unwrap_or(true) {
        return false;
    }
    let generation = super::recovery::generation(product_id);
    let Ok(admission) = super::recovery::admit_generation(product_id, generation) else {
        return false;
    };
    let plan = PersistedUninstallationPlan {
        game,
        cleanup,
        recovery_generation: generation,
    };
    let queue_position = {
        let mut manager = MANAGER.lock().unwrap();
        if manager.active.contains_key(&product_id)
            || manager
                .queue
                .iter()
                .any(|queued| queued.product_id() == product_id)
        {
            return false;
        }
        manager.next_queue_position += 1;
        let position = manager.next_queue_position;
        manager
            .queue
            .push_back(QueuedOperation::Uninstallation(plan.clone()));
        manager.snapshots.insert(
            product_id,
            InstallationOperationSnapshot {
                product_id,
                state: crate::domain::InstallationState::Pending,
                message: Some("Queued for uninstallation".into()),
                percentage: None,
                queued: true,
            },
        );
        position
    };
    persist_operation(
        product_id,
        "uninstall",
        "queued",
        &plan,
        Some("Queued for uninstallation"),
        None,
        Some(queue_position),
    );
    if let Some(snapshot) = installation_operation_snapshot(product_id) {
        publish(InstallationManagerEvent::OperationQueued(snapshot));
    }
    drop(admission);
    schedule_next();
    true
}

fn schedule_next() {
    let (operation, session) = {
        let mut manager = MANAGER.lock().unwrap();
        if SIGN_OUT_PAUSE.load(std::sync::atomic::Ordering::Acquire)
            || manager.shutting_down
            || (manager.paused_for_sign_out
                && !crate::auth::session_is_current(crate::auth::session()))
            || !manager.active.is_empty()
        {
            return;
        }
        manager.paused_for_sign_out = false;
        (manager.queue.pop_front(), crate::auth::session())
    };
    match operation {
        Some(QueuedOperation::Installation(plan)) => start_queued_installation(plan, session),
        Some(QueuedOperation::Uninstallation(game)) => start_queued_uninstallation(game, session),
        None => {}
    }
}

fn start_queued_installation(persisted_plan: PersistedInstallationPlan, session: u64) {
    let Ok(admission) = super::recovery::admit_generation(
        persisted_plan.game.product_id,
        persisted_plan.recovery_generation,
    ) else {
        if let Some(tracking) = &persisted_plan.tracking {
            let _ = tracking.sender.send(InstallationEvent::Failed(
                "This game's recovery state changed; reopen setup and retry".into(),
            ));
        }
        schedule_next();
        return;
    };
    let authorization = if persisted_plan.download_intent_id.is_some() {
        StateStore::open().and_then(|store| download_intent_matches(&store, &persisted_plan))
    } else {
        Ok(true)
    };
    if !matches!(authorization, Ok(true)) {
        if let Some(tracking) = &persisted_plan.tracking {
            let _ = tracking.sender.send(InstallationEvent::Failed(
                "The installation request could not be authorized; reopen setup and retry".into(),
            ));
        }
        persist_existing_operation(
            persisted_plan.game.product_id,
            if authorization.is_err() {
                "interrupted"
            } else {
                "cancelled"
            },
            Some(if authorization.is_err() {
                "Could not verify the automatic installation request; retry after restarting"
            } else {
                "Automatic installation request was removed"
            }),
            None,
            None,
        );
        MANAGER
            .lock()
            .unwrap()
            .snapshots
            .remove(&persisted_plan.game.product_id);
        drop(admission);
        schedule_next();
        return;
    }
    let product_id = persisted_plan.game.product_id;
    let paused_game = persisted_plan.game.clone();
    let download_intent = persisted_plan.download_intent_id.clone();
    let running_message = if persisted_plan
        .game
        .installer_operating_system
        .as_deref()
        .is_some_and(|os| os.eq_ignore_ascii_case("windows"))
    {
        "Preparing Windows installer"
    } else {
        "Running native installer"
    };
    let mut manager = MANAGER.lock().unwrap();
    if SIGN_OUT_PAUSE.load(std::sync::atomic::Ordering::Acquire)
        || crate::auth::session() != session
        || manager.paused_for_sign_out
        || manager.shutting_down
    {
        drop(manager);
        if let Some(tracking) = &persisted_plan.tracking {
            let _ = tracking.sender.send(InstallationEvent::Cancelled);
        }
        persist_existing_operation(
            product_id,
            "paused",
            Some("Interrupted by sign-out; start this operation again to resume"),
            None,
            None,
        );
        return;
    }
    let handle = super::executor::start_installation_with_cancellation(
        persisted_plan.game.clone(),
        persisted_plan.additional_installers.clone(),
        persisted_plan.install_base,
        persisted_plan.interactive_prompts,
        persisted_plan
            .tracking
            .as_ref()
            .map(|tracking| tracking.control.cancellation.clone())
            .unwrap_or_else(|| std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false))),
    );
    manager
        .active
        .insert(product_id, OperationControl::Installation(handle.control()));
    drop(manager);
    drop(admission);
    persist_existing_operation(product_id, "running", Some(running_message), None, None);
    {
        let mut manager = MANAGER.lock().unwrap();
        manager.snapshots.insert(
            product_id,
            InstallationOperationSnapshot {
                product_id,
                state: crate::domain::InstallationState::Installing,
                message: Some(running_message.into()),
                percentage: None,
                queued: false,
            },
        );
    }
    thread::spawn(move || {
        while let Ok(event) = handle.events.recv() {
            if let Some(tracking) = &persisted_plan.tracking {
                let _ = tracking.sender.send(event.clone());
            }
            let terminal = matches!(
                event,
                InstallationEvent::Complete { .. }
                    | InstallationEvent::Cancelled
                    | InstallationEvent::Failed(_)
            );
            let (interrupted, publish_pause) =
                if matches!(event, InstallationEvent::Complete { .. }) {
                    (false, false)
                } else {
                    let mut manager = MANAGER.lock().unwrap();
                    if manager.shutting_down {
                        (true, false)
                    } else if manager.paused_for_sign_out {
                        if terminal && let Err(error) = pause_game_operation(&paused_game) {
                            manager.pause_errors.push(error.to_string());
                        }
                        (true, terminal)
                    } else {
                        (false, false)
                    }
                };
            if interrupted {
                if publish_pause {
                    publish_sign_out_pause(product_id);
                }
            } else if update_installation_snapshot(product_id, &event) {
                if matches!(event, InstallationEvent::Complete { .. })
                    && let Some(intent_id) = &download_intent
                    && let Ok(store) = StateStore::open()
                {
                    let _ = store.complete_download_install_intent(product_id, intent_id);
                }
                publish(InstallationManagerEvent::Installation { product_id, event });
            }
            if terminal {
                break;
            }
        }
        MANAGER.lock().unwrap().active.remove(&product_id);
        schedule_next();
    });
}

fn start_queued_uninstallation(plan: PersistedUninstallationPlan, session: u64) {
    let product_id = plan.game.product_id;
    let paused_game = plan.game.clone();
    let Ok(admission) = super::recovery::admit_generation(product_id, plan.recovery_generation)
    else {
        schedule_next();
        return;
    };
    let mut manager = MANAGER.lock().unwrap();
    if SIGN_OUT_PAUSE.load(std::sync::atomic::Ordering::Acquire)
        || crate::auth::session() != session
        || manager.paused_for_sign_out
        || manager.shutting_down
    {
        drop(manager);
        persist_existing_operation(
            product_id,
            "paused",
            Some("Interrupted by sign-out; start this operation again to resume"),
            None,
            None,
        );
        return;
    }
    let handle = super::executor::start_uninstallation(plan.game);
    manager.active.insert(
        product_id,
        OperationControl::Uninstallation(handle.control()),
    );
    drop(manager);
    drop(admission);
    persist_existing_operation(
        product_id,
        "running",
        Some("Running native uninstaller"),
        None,
        None,
    );
    {
        let mut manager = MANAGER.lock().unwrap();
        manager.snapshots.insert(
            product_id,
            InstallationOperationSnapshot {
                product_id,
                state: crate::domain::InstallationState::Uninstalling,
                message: Some("Running native uninstaller".into()),
                percentage: None,
                queued: false,
            },
        );
    }
    thread::spawn(move || {
        while let Ok(mut event) = handle.events.recv() {
            event = cleanup_successful_uninstall(event, plan.cleanup.as_ref(), |cleanup| {
                // The payload is already uninstalled. Cleanup failures are retried from Manage,
                // never by replaying the native uninstaller on the next launch.
                persist_existing_operation(
                    product_id,
                    "complete",
                    Some("Game uninstalled; finishing downloaded-file cleanup"),
                    None,
                    Some(chrono::Utc::now().timestamp()),
                );
                crate::download::cleanup_after_uninstall(cleanup.clone())
            });
            let terminal = matches!(
                event,
                UninstallationEvent::Complete
                    | UninstallationEvent::Cancelled
                    | UninstallationEvent::Failed(_)
            );
            let (interrupted, publish_pause) = if matches!(event, UninstallationEvent::Complete) {
                (false, false)
            } else {
                let mut manager = MANAGER.lock().unwrap();
                if manager.shutting_down {
                    (true, false)
                } else if manager.paused_for_sign_out {
                    if terminal && let Err(error) = pause_game_operation(&paused_game) {
                        manager.pause_errors.push(error.to_string());
                    }
                    (true, terminal)
                } else {
                    (false, false)
                }
            };
            if interrupted {
                if publish_pause {
                    publish_sign_out_pause(product_id);
                }
            } else if update_uninstallation_snapshot(product_id, &event) {
                publish(InstallationManagerEvent::Uninstallation { product_id, event });
            }
            if terminal {
                break;
            }
        }
        MANAGER.lock().unwrap().active.remove(&product_id);
        schedule_next();
    });
}

fn cleanup_successful_uninstall(
    event: UninstallationEvent,
    cleanup: Option<&crate::download::ManagedDownloads>,
    delete: impl FnOnce(
        &crate::download::ManagedDownloads,
    ) -> anyhow::Result<crate::download::CleanupResult>,
) -> UninstallationEvent {
    let Some(cleanup) = cleanup.filter(|_| matches!(event, UninstallationEvent::Complete)) else {
        return event;
    };
    match delete(cleanup) {
        Ok(result) if result.failures.is_empty() => UninstallationEvent::Complete,
        Ok(result) => UninstallationEvent::Failed(format!(
            "Game uninstalled, but some downloaded files could not be removed: {}. Retry from Manage.",
            result.failures.join("; ")
        )),
        Err(error) => UninstallationEvent::Failed(format!(
            "Game uninstalled, but downloaded-file cleanup failed: {error}. Retry from Manage."
        )),
    }
}

pub fn respond_to_installation(product_id: i64, response: String) -> bool {
    let manager = MANAGER.lock().unwrap();
    let Some(OperationControl::Installation(control)) = manager.active.get(&product_id) else {
        return false;
    };
    control.respond(response);
    true
}

pub fn cancel_operation(product_id: i64) -> bool {
    cancel_operation_checked(product_id, None)
}

fn cancel_operation_checked(
    product_id: i64,
    expected: Option<&TrackedInstallationControl>,
) -> bool {
    // Enqueue holds this same admission through journal publication. Do not let
    // a new attempt replace the journal while retiring this exact queued plan.
    let admission = if let Some(expected) = expected {
        let Ok(admission) = super::recovery::admit_generation(product_id, expected.generation)
        else {
            return false;
        };
        Some(admission)
    } else {
        None
    };
    let (mut queued_snapshot, removed) = {
        let mut manager = MANAGER.lock().unwrap();
        if let Some(control) = manager.active.get(&product_id) {
            if expected.is_some_and(|expected| !matches!(control, OperationControl::Installation(control) if control.uses_cancellation(&expected.cancellation))) {
                return false;
            }
            match control {
                OperationControl::Installation(control) => control.cancel(),
                OperationControl::Uninstallation(control) => control.cancel(),
            }
            return true;
        }
        let Some(index) = manager
            .queue
            .iter()
            .position(|operation| operation.product_id() == product_id && expected.is_none_or(|expected| matches!(operation, QueuedOperation::Installation(plan) if plan.tracking.as_ref().is_some_and(|tracking| std::sync::Arc::ptr_eq(&tracking.control.cancellation, &expected.cancellation)))))
        else {
            return false;
        };
        let removed = manager.queue.remove(index);
        let snapshot = InstallationOperationSnapshot {
            product_id,
            state: crate::domain::InstallationState::Failed,
            message: Some("Operation cancelled".into()),
            percentage: None,
            queued: false,
        };
        manager.snapshots.insert(product_id, snapshot.clone());
        (snapshot, removed)
    };
    let terminal = if expected.is_some() {
        let Some(QueuedOperation::Installation(plan)) = &removed else {
            unreachable!()
        };
        let result = (|| -> anyhow::Result<()> {
            let directory = &plan.game.installation_directory;
            let path = super::operation_journal::path(
                directory.parent().context("Missing installation library")?,
                directory
                    .file_name()
                    .and_then(|name| name.to_str())
                    .context("Invalid installation slug")?,
            )?;
            if let Some(value) = super::recovery::read_json(&path)? {
                let super::operation_journal::OperationJournal::Offline { record, .. } =
                    serde_json::from_value(value)?
                else {
                    anyhow::bail!("The saved installation operation changed; it was retained");
                };
                anyhow::ensure!(
                    record.product_id == product_id
                        && serde_json::from_str::<serde_json::Value>(&record.plan_json)?
                            == serde_json::to_value(plan)?,
                    "The saved installation operation changed; it was retained"
                );
                super::recovery::remove_control_file(&path)?;
            }
            Ok(())
        })();
        match result {
            Ok(()) => InstallationEvent::Cancelled,
            Err(error) => {
                let message = format!(
                    "Setup stopped, but its saved operation could not be cleared: {error:#}. Review this game's saved operation before retrying."
                );
                queued_snapshot.message = Some(message.clone());
                MANAGER
                    .lock()
                    .unwrap()
                    .snapshots
                    .insert(product_id, queued_snapshot.clone());
                InstallationEvent::Failed(message)
            }
        }
    } else {
        persist_existing_operation(
            product_id,
            "cancelled",
            Some("Operation cancelled"),
            None,
            Some(chrono::Utc::now().timestamp()),
        );
        InstallationEvent::Cancelled
    };
    if let Some(QueuedOperation::Installation(plan)) = removed
        && let Some(tracking) = plan.tracking
    {
        let _ = tracking.sender.send(terminal);
    }
    publish(InstallationManagerEvent::OperationCancelled(
        queued_snapshot,
    ));
    drop(admission);
    schedule_next();
    true
}

pub(super) fn recovery_busy(ids: &[i64]) -> bool {
    let manager = MANAGER.lock().unwrap();
    let busy = manager.active.keys().any(|id| ids.contains(id))
        || manager
            .queue
            .iter()
            .any(|operation| ids.contains(&operation.product_id()))
        || manager.snapshots.iter().any(|(id, snapshot)| {
            ids.contains(id)
                && matches!(
                    snapshot.state,
                    crate::domain::InstallationState::Failed
                        | crate::domain::InstallationState::UninstallFailed
                )
        });
    drop(manager);
    busy || DEPOT_MANAGER
        .lock()
        .unwrap()
        .snapshots
        .values()
        .any(|snapshot| {
            ids.contains(&snapshot.product_id)
                && !matches!(snapshot.state.as_str(), "complete" | "abandoned")
        })
}

pub(super) fn quiesce_recovery(
    ids: &[i64],
    cancelled: &std::sync::atomic::AtomicBool,
    config: &crate::config::Config,
    slug: &str,
) -> anyhow::Result<()> {
    {
        let mut manager = MANAGER.lock().unwrap();
        manager
            .queue
            .retain(|operation| !ids.contains(&operation.product_id()));
        for (id, control) in &manager.active {
            if ids.contains(id) {
                match control {
                    OperationControl::Installation(control) => control.cancel(),
                    OperationControl::Uninstallation(control) => control.cancel(),
                }
            }
        }
    }
    {
        let manager = DEPOT_MANAGER.lock().unwrap();
        for (operation, (id, _)) in &manager.reservations {
            if ids.contains(id)
                && let Some(cancel) = manager.active.get(operation)
            {
                cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            }
        }
    }
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        anyhow::ensure!(
            !cancelled.load(std::sync::atomic::Ordering::Relaxed),
            "Recovery cancelled; stopped operations remain available for review"
        );
        let offline = MANAGER
            .lock()
            .unwrap()
            .active
            .keys()
            .any(|id| ids.contains(id));
        let depot = {
            let manager = DEPOT_MANAGER.lock().unwrap();
            manager.reservations.iter().any(|(operation, (id, _))| {
                ids.contains(id) && manager.active.contains_key(operation)
            })
        };
        if !offline && !depot {
            break;
        }
        anyhow::ensure!(
            std::time::Instant::now() < deadline,
            "The installer is still stopping. No files were removed; retry recovery after it stops"
        );
        thread::sleep(Duration::from_millis(25));
    }
    for library in &config.game_libraries {
        let path = super::operation_journal::path(&library.path, slug)?;
        match super::operation_journal::read(&path) {
            Ok(super::operation_journal::OperationJournal::Depot { record, .. }) => {
                anyhow::ensure!(ids.contains(&record.product_id), "Another game's operation owns this recovery journal");
                super::dependency_setup::ensure_setup_quiescent(&record)?;
            }
            Ok(_) => {}
            Err(error) if error.downcast_ref::<std::io::Error>().is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound) => {}
            Err(error) => return Err(error.context("Cannot verify this game's setup process journal. No files were removed; restore a valid operation record before retrying recovery")),
        }
    }
    Ok(())
}

pub(super) fn finish_recovery(ids: &[i64]) {
    {
        let mut manager = MANAGER.lock().unwrap();
        for id in ids {
            manager.snapshots.remove(id);
        }
    }
    {
        let mut manager = DEPOT_MANAGER.lock().unwrap();
        manager
            .snapshots
            .retain(|_, snapshot| !ids.contains(&snapshot.product_id));
    }
    for id in ids {
        publish(InstallationManagerEvent::Uninstallation {
            product_id: *id,
            event: UninstallationEvent::Complete,
        });
    }
}

pub fn recover_interrupted_operations() -> anyhow::Result<usize> {
    let signed_out = crate::auth::restoration_blocked()?;
    migrate_legacy_operation_records()?;
    let mut recovered = 0;
    let mut operations = super::operation_journal::scan()?
        .into_iter()
        .filter_map(|(path, journal)| match journal {
            super::operation_journal::OperationJournal::Offline { record, .. } => {
                Some((path, record))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    operations.sort_by_key(|(_, operation)| recovery_sort_key(operation));

    let mut recovered_snapshots = Vec::new();
    let mut manager = MANAGER.lock().unwrap();
    for (path, mut operation) in operations {
        let queued = match operation.operation.as_str() {
            "install" => serde_json::from_str::<PersistedInstallationPlan>(&operation.plan_json)
                .map(QueuedOperation::Installation),
            "uninstall" => {
                serde_json::from_str::<PersistedUninstallationPlan>(&operation.plan_json)
                    .map(QueuedOperation::Uninstallation)
            }
            _ => continue,
        };
        let Ok(queued) = queued else {
            operation.state = "interrupted".into();
            operation.message = Some(
                "The saved operation plan could not be restored. Start the operation again.".into(),
            );
            operation.percentage = None;
            operation.queue_position = None;
            operation.updated_at = chrono::Utc::now().timestamp();
            super::operation_journal::write_offline(&path, &operation)?;
            continue;
        };
        let product_id = queued.product_id();
        if operation.state == "paused" || signed_out {
            let snapshot = InstallationOperationSnapshot {
                product_id,
                state: crate::domain::InstallationState::Failed,
                message: Some(
                    "Interrupted by sign-out; start this operation again to resume".into(),
                ),
                percentage: None,
                queued: false,
            };
            manager.snapshots.insert(product_id, snapshot.clone());
            recovered_snapshots.push(snapshot);
            continue;
        }
        if let QueuedOperation::Installation(plan) = &queued
            && plan.download_intent_id.is_some()
        {
            let store = StateStore::open()?;
            if !authorize_recovered_download(&store, &path, plan)? {
                continue;
            }
        }
        let message = if operation.operation == "uninstall" {
            "Queued for resumed uninstallation"
        } else {
            "Queued for resumed installation"
        };
        manager.next_queue_position = manager
            .next_queue_position
            .max(operation.queue_position.unwrap_or_default());
        if !manager
            .queue
            .iter()
            .any(|item| item.product_id() == product_id)
        {
            manager.queue.push_back(queued);
            let snapshot = InstallationOperationSnapshot {
                product_id,
                state: crate::domain::InstallationState::Pending,
                message: Some(message.into()),
                percentage: None,
                queued: true,
            };
            manager.snapshots.insert(product_id, snapshot.clone());
            recovered_snapshots.push(snapshot);
            operation.state = "queued".into();
            operation.message = Some(message.into());
            operation.percentage = None;
            operation.updated_at = chrono::Utc::now().timestamp();
            super::operation_journal::write_offline(&path, &operation)?;
            recovered += 1;
        }
    }
    drop(manager);
    for snapshot in recovered_snapshots {
        publish(InstallationManagerEvent::OperationRecovered(snapshot));
    }
    Ok(recovered)
}

fn recovery_sort_key(operation: &InstallationOperationRecord) -> (bool, i64, i64, i64) {
    (
        operation.state != "running",
        operation.queue_position.unwrap_or(i64::MAX),
        operation.created_at,
        operation.product_id,
    )
}

pub fn start_recovered_operations() {
    schedule_next();
}

pub fn pause_for_sign_out() -> anyhow::Result<()> {
    let queued = {
        let mut manager = MANAGER.lock().unwrap();
        manager.paused_for_sign_out = true;
        manager.pause_errors.clear();
        let queued = manager.queue.drain(..).collect::<Vec<_>>();
        for control in manager.active.values() {
            match control {
                OperationControl::Installation(control) => control.cancel(),
                OperationControl::Uninstallation(control) => control.cancel(),
            }
        }
        queued
    };
    for operation in queued {
        if let QueuedOperation::Installation(plan) = &operation
            && let Some(tracking) = &plan.tracking
        {
            let _ = tracking.sender.send(InstallationEvent::Cancelled);
        }
        let game = match &operation {
            QueuedOperation::Installation(plan) => &plan.game,
            QueuedOperation::Uninstallation(plan) => &plan.game,
        };
        if let Err(error) = pause_game_operation(game) {
            MANAGER.lock().unwrap().pause_errors.push(error.to_string());
        }
        publish_sign_out_pause(game.product_id);
    }
    let mut manager = DEPOT_MANAGER.lock().unwrap();
    manager.paused_for_sign_out = true;
    for cancelled in manager.active.values() {
        cancelled.store(true, std::sync::atomic::Ordering::Relaxed);
    }
    drop(manager);
    let manager = MANAGER.lock().unwrap();
    anyhow::ensure!(
        manager.pause_errors.is_empty(),
        "Signed out, but saving interrupted installation state failed: {}. Retry cleanup before signing in.",
        manager.pause_errors.join("; ")
    );
    Ok(())
}

fn pause_game_operation(game: &InstalledGame) -> anyhow::Result<()> {
    let library = game
        .installation_directory
        .parent()
        .context("Installation has no library")?;
    let slug = game
        .installation_directory
        .file_name()
        .and_then(|name| name.to_str())
        .context("Invalid installation directory")?;
    let path = super::operation_journal::path(library, slug)?;
    if !path.try_exists()? {
        return Ok(());
    }
    let super::operation_journal::OperationJournal::Offline { mut record, .. } =
        super::operation_journal::read(&path)?
    else {
        anyhow::bail!("Saved installation changed; review cleanup before signing in");
    };
    anyhow::ensure!(
        record.product_id == game.product_id,
        "Saved installation identity changed"
    );
    record.state = "paused".into();
    record.message = Some("Interrupted by sign-out; start this operation again to resume".into());
    record.updated_at = chrono::Utc::now().timestamp();
    super::operation_journal::write_offline(&path, &record)
}

/// Resolve a failed pause before a new explicit login removes the durable sign-out barrier.
pub(crate) fn normalize_signed_out_operations() -> anyhow::Result<()> {
    wait_for_paused()?;
    migrate_legacy_operation_records()?;
    for (path, journal) in super::operation_journal::scan()? {
        if let super::operation_journal::OperationJournal::Offline { mut record, .. } = journal {
            record.state = "paused".into();
            record.message =
                Some("Interrupted by sign-out; start this operation again to resume".into());
            super::operation_journal::write_offline(&path, &record)?;
        }
    }
    Ok(())
}

fn publish_sign_out_pause(product_id: i64) {
    let mut manager = MANAGER.lock().unwrap();
    if !manager.paused_for_sign_out {
        return;
    }
    let snapshot = InstallationOperationSnapshot {
        product_id,
        state: crate::domain::InstallationState::Failed,
        message: Some("Interrupted by sign-out; start this operation again to resume".into()),
        percentage: None,
        queued: false,
    };
    manager.snapshots.insert(product_id, snapshot.clone());
    drop(manager);
    publish(InstallationManagerEvent::OperationRecovered(snapshot));
}

pub fn wait_for_paused() -> anyhow::Result<()> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(45);
    loop {
        if MANAGER.lock().unwrap().active.is_empty()
            && DEPOT_MANAGER.lock().unwrap().active.is_empty()
        {
            let manager = MANAGER.lock().unwrap();
            anyhow::ensure!(
                manager.pause_errors.is_empty(),
                "Signed out, but interrupted installation state needs cleanup before signing in: {}",
                manager.pause_errors.join("; ")
            );
            return Ok(());
        }
        anyhow::ensure!(
            std::time::Instant::now() < deadline,
            "Signed out. Installation helpers are still stopping; retry profile reset shortly. Files and recovery records were kept."
        );
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

pub fn shutdown() {
    let mut manager = MANAGER.lock().unwrap();
    manager.shutting_down = true;
    let (state, message) = if manager.paused_for_sign_out {
        (
            "paused",
            "Interrupted by sign-out; start this operation again to resume",
        )
    } else {
        ("queued", "Queued after application shutdown")
    };
    for product_id in manager.active.keys().copied().collect::<Vec<_>>() {
        persist_existing_operation(product_id, state, Some(message), None, None);
    }
    for control in manager.active.values() {
        match control {
            OperationControl::Installation(control) => control.cancel(),
            OperationControl::Uninstallation(control) => control.cancel(),
        }
    }
    drop(manager);
    let mut depot_manager = DEPOT_MANAGER.lock().unwrap();
    depot_manager.shutting_down = true;
    for cancelled in depot_manager.active.values() {
        cancelled.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

fn publish(event: InstallationManagerEvent) {
    MANAGER
        .lock()
        .unwrap()
        .subscribers
        .retain(|subscriber| subscriber.send(event.clone()).is_ok());
}

fn update_installation_snapshot(product_id: i64, event: &InstallationEvent) -> bool {
    let base_remains_installed = matches!(
        event,
        InstallationEvent::Cancelled | InstallationEvent::Failed(_)
    ) && StateStore::open().is_ok_and(|store| {
        crate::config::Config::load_or_create().is_ok_and(|config| {
            crate::installation::reconcile_installed_games(&store, &config.game_libraries)
                .is_ok_and(|games| {
                    games.into_iter().any(|game| {
                        game.product_id == product_id
                            && game.state == crate::domain::InstallationState::Installed
                            && game
                                .primary_executable
                                .as_ref()
                                .is_some_and(|path| path.is_file())
                    })
                })
        })
    });
    let (state, message, percentage) = match event {
        InstallationEvent::Starting { message } => (
            crate::domain::InstallationState::Installing,
            Some(message.clone()),
            None,
        ),
        InstallationEvent::Running {
            percentage,
            message,
            ..
        } => (
            crate::domain::InstallationState::Installing,
            Some(message.clone()),
            *percentage,
        ),
        InstallationEvent::Prompt { text, .. } => (
            crate::domain::InstallationState::Installing,
            Some(text.clone()),
            None,
        ),
        InstallationEvent::Complete { .. } => (
            crate::domain::InstallationState::Installed,
            Some("Installation complete".into()),
            Some(100),
        ),
        InstallationEvent::Cancelled => (
            if base_remains_installed {
                crate::domain::InstallationState::Installed
            } else {
                crate::domain::InstallationState::Failed
            },
            Some("Installation cancelled".into()),
            None,
        ),
        InstallationEvent::Failed(error) => (
            if base_remains_installed {
                crate::domain::InstallationState::Installed
            } else {
                crate::domain::InstallationState::Failed
            },
            Some(error.clone()),
            None,
        ),
    };
    let mut manager = MANAGER.lock().unwrap();
    if !matches!(event, InstallationEvent::Complete { .. })
        && (manager.shutting_down || manager.paused_for_sign_out)
    {
        return false;
    }
    manager.snapshots.insert(
        product_id,
        InstallationOperationSnapshot {
            product_id,
            state,
            message: message.clone(),
            percentage,
            queued: false,
        },
    );
    let operation_failed = matches!(
        event,
        InstallationEvent::Cancelled | InstallationEvent::Failed(_)
    );
    persist_existing_operation(
        product_id,
        if operation_failed {
            "failed"
        } else {
            match state {
                crate::domain::InstallationState::Installing => "running",
                crate::domain::InstallationState::Installed => "complete",
                _ => "failed",
            }
        },
        message.as_deref(),
        percentage,
        (state == crate::domain::InstallationState::Installed && !operation_failed)
            .then(|| chrono::Utc::now().timestamp()),
    );
    true
}

fn update_uninstallation_snapshot(product_id: i64, event: &UninstallationEvent) -> bool {
    let (state, message) = match event {
        UninstallationEvent::Started => (
            crate::domain::InstallationState::Uninstalling,
            Some("Running native uninstaller".into()),
        ),
        UninstallationEvent::Complete => (
            crate::domain::InstallationState::Pending,
            Some("Uninstallation complete".into()),
        ),
        UninstallationEvent::Cancelled => (
            crate::domain::InstallationState::Installed,
            Some("Uninstallation cancelled".into()),
        ),
        UninstallationEvent::Failed(error) => (
            crate::domain::InstallationState::UninstallFailed,
            Some(error.clone()),
        ),
    };
    let mut manager = MANAGER.lock().unwrap();
    if !matches!(event, UninstallationEvent::Complete)
        && (manager.shutting_down || manager.paused_for_sign_out)
    {
        return false;
    }
    manager.snapshots.insert(
        product_id,
        InstallationOperationSnapshot {
            product_id,
            state,
            message: message.clone(),
            percentage: None,
            queued: false,
        },
    );
    persist_existing_operation(
        product_id,
        match state {
            crate::domain::InstallationState::Uninstalling => "running",
            crate::domain::InstallationState::Pending => "complete",
            _ => "failed",
        },
        message.as_deref(),
        None,
        (state == crate::domain::InstallationState::Pending)
            .then(|| chrono::Utc::now().timestamp()),
    );
    true
}

fn persist_operation<T: serde::Serialize>(
    product_id: i64,
    operation: &str,
    state: &str,
    plan: &T,
    message: Option<&str>,
    percentage: Option<u8>,
    queue_position: Option<i64>,
) {
    let now = chrono::Utc::now().timestamp();
    let Ok(plan_json) = serde_json::to_string(plan) else {
        return;
    };
    let record = InstallationOperationRecord {
        product_id,
        operation: operation.into(),
        state: state.into(),
        plan_json,
        message: message.map(str::to_owned),
        percentage,
        queue_position,
        created_at: now,
        updated_at: now,
        completed_at: None,
    };
    if let Ok(path) = super::operation_journal::offline_path(&record) {
        let _ = super::operation_journal::write_offline(&path, &record);
    }
}

fn persist_existing_operation(
    product_id: i64,
    state: &str,
    message: Option<&str>,
    percentage: Option<u8>,
    completed_at: Option<i64>,
) {
    let Ok(Some((path, mut record))) = super::operation_journal::scan().map(|journals| {
        journals
            .into_iter()
            .find_map(|(path, journal)| match journal {
                super::operation_journal::OperationJournal::Offline { record, .. }
                    if record.product_id == product_id =>
                {
                    Some((path, record))
                }
                _ => None,
            })
    }) else {
        return;
    };
    record.state = state.into();
    record.message = message.map(str::to_owned);
    record.percentage = percentage;
    if !matches!(state, "queued" | "running") {
        record.queue_position = None;
    }
    record.updated_at = chrono::Utc::now().timestamp();
    record.completed_at = completed_at;
    if state == "complete" || state == "cancelled" {
        let _ = super::operation_journal::remove(&path);
    } else {
        let _ = super::operation_journal::write_offline(&path, &record);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracked_setup_excludes_prior_events_and_cancellation_without_serializing_tracking() {
        const CHILD: &str = "LUDOMERE_TRACKED_SETUP_FIXTURE";
        if std::env::var_os(CHILD).is_none() {
            let root = tempfile::tempdir().unwrap();
            let mut command = std::process::Command::new(std::env::current_exe().unwrap());
            command.args(["--exact", "installation::manager::tests::tracked_setup_excludes_prior_events_and_cancellation_without_serializing_tracking", "--nocapture"])
                .env(CHILD, "1");
            for key in [
                "HOME",
                "XDG_CONFIG_HOME",
                "XDG_DATA_HOME",
                "XDG_CACHE_HOME",
                "XDG_STATE_HOME",
                "XDG_RUNTIME_DIR",
                "TMPDIR",
            ] {
                let path = root.path().join(key);
                std::fs::create_dir(&path).unwrap();
                command.env(key, path);
            }
            assert!(command.status().unwrap().success());
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let game = super::super::marker::game_from_marker(
            &marker(false),
            "fixture".into(),
            root.path().join("game"),
            None,
        );
        // Keep the inert queue from dispatching; no installer or helper is executed.
        MANAGER.lock().unwrap().shutting_down = true;
        let old = enqueue_installation_tracked(game.clone(), vec![], true, false).unwrap();
        assert!(matches!(
            old.events.try_recv(),
            Ok(InstallationEvent::Starting { .. })
        ));
        assert!(enqueue_installation_tracked(game.clone(), vec![], true, false).is_none());
        // Reproduce the dequeue-to-active interval without spawning the operation.
        let QueuedOperation::Installation(popped) =
            MANAGER.lock().unwrap().queue.pop_front().unwrap()
        else {
            panic!("installation expected")
        };
        let encoded = serde_json::to_value(&popped).unwrap();
        assert!(encoded.get("tracking").is_none());
        assert!(
            serde_json::from_value::<PersistedInstallationPlan>(encoded)
                .unwrap()
                .tracking
                .is_none()
        );
        let current = enqueue_installation_tracked(game.clone(), vec![], true, false).unwrap();
        assert!(matches!(
            current.events.try_recv(),
            Ok(InstallationEvent::Starting { .. })
        ));
        let journal = root.path().join(".ludomere/staging/game.operation.json");
        let current_journal = std::fs::read(&journal).unwrap();
        assert!(!old.control().cancel());
        assert_eq!(std::fs::read(&journal).unwrap(), current_journal);
        assert!(
            !current
                .control
                .cancellation
                .load(std::sync::atomic::Ordering::Acquire)
        );
        popped
            .tracking
            .unwrap()
            .sender
            .send(InstallationEvent::Failed("older attempt".into()))
            .unwrap();
        assert!(matches!(
            old.events.try_recv(),
            Ok(InstallationEvent::Failed(_))
        ));
        assert!(matches!(
            current.events.try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));
        assert!(current.control().cancel());
        assert!(matches!(
            current.events.try_recv(),
            Ok(InstallationEvent::Cancelled)
        ));
        assert!(MANAGER.lock().unwrap().queue.is_empty());
        assert!(!journal.exists());
        let failed_cleanup = enqueue_installation_tracked(game, vec![], true, false).unwrap();
        assert!(matches!(
            failed_cleanup.events.try_recv(),
            Ok(InstallationEvent::Starting { .. })
        ));
        std::fs::write(&journal, b"inert malformed saved operation").unwrap();
        assert!(failed_cleanup.control().cancel());
        assert!(
            matches!(failed_cleanup.events.try_recv(), Ok(InstallationEvent::Failed(message)) if message.contains("saved operation could not be cleared"))
        );
        assert_eq!(
            std::fs::read(&journal).unwrap(),
            b"inert malformed saved operation"
        );
    }

    #[test]
    fn setup_snapshot_resets_component_progress_before_other_phases() {
        let mut request = request(false);
        request.operation_id = "setup-progress-fixture".into();
        publish_depot_setup(&request, "OpenAL", 1, 3);
        let snapshot = depot_operation_snapshot(&request.operation_id).unwrap();
        assert_eq!(
            snapshot.setup.unwrap(),
            DepotSetupProgress {
                component: "OpenAL".into(),
                completed: 1,
                total: 3
            }
        );
        publish_depot_setup(&request, "Finishing game setup", 0, 0);
        assert_eq!(
            depot_operation_snapshot(&request.operation_id)
                .unwrap()
                .setup
                .unwrap()
                .total,
            0
        );
        for phase in ["verifying", "failed", "cancelled", "complete"] {
            publish_depot_setup(&request, "Cached prerequisite", 3, 3);
            publish_depot_progress(&request, phase, 0, 0, 0, 0, 0);
            let snapshot = depot_operation_snapshot(&request.operation_id).unwrap();
            assert_eq!(snapshot.state, phase);
            assert!(snapshot.setup.is_none());
        }
        let mut manager = DEPOT_MANAGER.lock().unwrap();
        manager.snapshots.remove(&request.operation_id);
        manager.snapshot_sequence.remove(&request.operation_id);
        manager.last_event_at.remove(&request.operation_id);
    }

    #[test]
    fn signed_out_native_journal_is_retained_without_automatic_replay() {
        let root = tempfile::tempdir().unwrap();
        let previous = std::fs::read(crate::config::Config::path()).ok();
        let config = crate::config::Config {
            game_libraries: vec![crate::config::GameLibrary {
                id: "signout-fixture".into(),
                name: "Fixture".into(),
                path: root.path().to_owned(),
                default: true,
            }],
            ..crate::config::Config::default()
        };
        config.save().unwrap();
        let game = super::super::marker::game_from_marker(
            &marker(false),
            "signout-fixture".into(),
            root.path().join("fixture"),
            None,
        );
        let plan = PersistedInstallationPlan {
            tracking: None,
            recovery_generation: super::super::recovery::generation(game.product_id),
            game,
            additional_installers: vec![],
            install_base: true,
            interactive_prompts: false,
            download_intent_id: None,
        };
        persist_operation(
            plan.game.product_id,
            "install",
            "queued",
            &plan,
            None,
            None,
            Some(1),
        );
        let path = super::super::operation_journal::path(root.path(), "fixture").unwrap();
        let unrelated = super::super::operation_journal::path(root.path(), "unrelated").unwrap();
        std::fs::write(&unrelated, b"malformed unrelated record").unwrap();
        MANAGER
            .lock()
            .unwrap()
            .queue
            .push_back(QueuedOperation::Installation(plan.clone()));
        request_sign_out_pause();
        schedule_next();
        assert_eq!(MANAGER.lock().unwrap().queue.len(), 1);
        assert!(MANAGER.lock().unwrap().active.is_empty());
        pause_for_sign_out().unwrap();
        wait_for_paused().unwrap();
        assert!(MANAGER.lock().unwrap().queue.is_empty());
        std::fs::remove_file(unrelated).unwrap();
        if unsafe { libc::geteuid() } != 0 {
            use std::os::unix::fs::PermissionsExt;
            persist_operation(
                plan.game.product_id,
                "install",
                "queued",
                &plan,
                None,
                None,
                Some(1),
            );
            std::fs::set_permissions(
                path.parent().unwrap(),
                std::fs::Permissions::from_mode(0o500),
            )
            .unwrap();
            MANAGER
                .lock()
                .unwrap()
                .queue
                .push_back(QueuedOperation::Installation(plan.clone()));
            assert!(pause_for_sign_out().is_err());
            crate::auth::persist_sign_out().unwrap();
            assert_eq!(recover_interrupted_operations().unwrap(), 0);
            assert!(MANAGER.lock().unwrap().queue.is_empty());
            MANAGER.lock().unwrap().pause_errors.clear();
            assert!(normalize_signed_out_operations().is_err());
            std::fs::set_permissions(
                path.parent().unwrap(),
                std::fs::Permissions::from_mode(0o700),
            )
            .unwrap();
            normalize_signed_out_operations().unwrap();
            std::fs::remove_file(crate::identity::config_root().join(".gog-signed-out")).unwrap();
        }
        MANAGER.lock().unwrap().paused_for_sign_out = false;
        assert_eq!(recover_interrupted_operations().unwrap(), 0);
        assert!(MANAGER.lock().unwrap().queue.is_empty());
        start_queued_installation(plan, crate::auth::session().wrapping_sub(1));
        assert!(MANAGER.lock().unwrap().active.is_empty());
        let super::super::operation_journal::OperationJournal::Offline { record, .. } =
            super::super::operation_journal::read(&path).unwrap()
        else {
            panic!()
        };
        assert_eq!(record.state, "paused");
        *MANAGER.lock().unwrap() = ManagerState::default();
        finish_sign_out_pause();
        DEPOT_MANAGER.lock().unwrap().paused_for_sign_out = false;
        if let Some(previous) = previous {
            std::fs::write(crate::config::Config::path(), previous).unwrap();
        } else {
            std::fs::remove_file(crate::config::Config::path()).unwrap();
        }
    }
    use crate::{
        domain::{
            GalaxyDepotDlcProvenance, GalaxyDepotIdentity, GalaxyDepotProvenance,
            InstallationSource,
        },
        installation::marker::{InstallationMarker, InstalledComponent},
    };

    #[test]
    fn uninstall_cleanup_is_opt_in_durable_and_runs_only_after_success() {
        let cleanup: crate::download::ManagedDownloads =
            serde_json::from_value(serde_json::json!({"product_id":7,"files":[]})).unwrap();
        let game = super::super::marker::game_from_marker(
            &marker(false),
            "test".into(),
            PathBuf::from("/fixture/game"),
            None,
        );
        let legacy: PersistedUninstallationPlan =
            serde_json::from_value(serde_json::to_value(&game).unwrap()).unwrap();
        assert!(legacy.cleanup.is_none());
        let encoded = serde_json::to_vec(&PersistedUninstallationPlan {
            recovery_generation: 0,
            game,
            cleanup: Some(cleanup.clone()),
        })
        .unwrap();
        let restored: PersistedUninstallationPlan = serde_json::from_slice(&encoded).unwrap();
        assert!(restored.cleanup.is_some());
        for event in [
            UninstallationEvent::Started,
            UninstallationEvent::Cancelled,
            UninstallationEvent::Failed("fixture".into()),
        ] {
            cleanup_successful_uninstall(event, Some(&cleanup), |_| {
                panic!("non-success triggered deletion")
            });
        }
        cleanup_successful_uninstall(UninstallationEvent::Complete, None, |_| {
            panic!("unchecked cleanup triggered deletion")
        });
        let called = std::cell::Cell::new(false);
        let event = cleanup_successful_uninstall(
            UninstallationEvent::Complete,
            restored.cleanup.as_ref(),
            |_| {
                called.set(true);
                Ok(crate::download::CleanupResult {
                    deleted: 1,
                    failures: vec!["fixture file busy".into()],
                })
            },
        );
        assert!(called.get());
        assert!(
            matches!(event,UninstallationEvent::Failed(message) if message.starts_with("Game uninstalled, but"))
        );
    }

    #[test]
    fn download_handoff_is_durable_and_revoked_or_replaced_intents_cannot_recover() {
        let root = tempfile::tempdir().unwrap();
        let database = root.path().join("state.db");
        let store = StateStore::open_at(&database).unwrap();
        let plan = PersistedInstallationPlan {
            tracking: None,
            recovery_generation: 0,
            game: super::super::marker::game_from_marker(
                &marker(false),
                "library".into(),
                root.path().join("library/game"),
                None,
            ),
            additional_installers: vec![],
            install_base: true,
            interactive_prompts: false,
            download_intent_id: Some("explicit-choice".into()),
        };
        let mut intent = crate::state::DownloadInstallIntent {
            product_id: 7,
            intent_id: "explicit-choice".into(),
            job_ids: vec!["job".into()],
            plan_json: "{}".into(),
            state: "waiting".into(),
            error: None,
        };
        store.save_download_install_intent(&intent).unwrap();
        persist_download_handoff(&store, &plan, 1).unwrap();
        let journal = root
            .path()
            .join("library/.ludomere/staging/game.operation.json");
        assert!(journal.is_file());
        assert_eq!(
            store.download_install_intents().unwrap()[0].state,
            "handed_off"
        );
        drop(store);
        let store = StateStore::open_at(&database).unwrap();
        assert!(download_intent_matches(&store, &plan).unwrap());
        // Crash window: runnable journal exists, but the SQLite handoff did not commit.
        store
            .set_download_install_state("explicit-choice", "waiting", None)
            .unwrap();
        std::fs::remove_file(&journal).unwrap();
        let fault = rusqlite::Connection::open(&database).unwrap();
        fault.execute_batch("CREATE TRIGGER reject_handoff BEFORE UPDATE ON download_install_intents BEGIN SELECT RAISE(FAIL, 'fixture handoff failure'); END;").unwrap();
        assert!(persist_download_handoff(&store, &plan, 1).is_err());
        assert!(journal.is_file());
        assert_eq!(
            store.download_install_intents().unwrap()[0].state,
            "waiting"
        );
        fault.execute_batch("DROP TRIGGER reject_handoff").unwrap();
        assert!(authorize_recovered_download(&store, &journal, &plan).unwrap());
        assert_eq!(store.download_install_intents().unwrap().len(), 1);
        // Remove/unchecked consent must prevent both recovery and a queued start.
        store.clear_download_install_intent_for_job("job").unwrap();
        assert!(!download_intent_matches(&store, &plan).unwrap());
        assert!(
            store
                .set_download_install_state("explicit-choice", "handed_off", None)
                .is_err()
        );
        assert!(persist_download_handoff(&store, &plan, 1).is_err());
        assert!(!authorize_recovered_download(&store, &journal, &plan).unwrap());
        assert!(!journal.exists());
        store.save_download_install_intent(&intent).unwrap();
        persist_download_handoff(&store, &plan, 1).unwrap();
        intent.intent_id = "replacement-choice".into();
        store.save_download_install_intent(&intent).unwrap();
        assert!(!download_intent_matches(&store, &plan).unwrap());
        assert!(!authorize_recovered_download(&store, &journal, &plan).unwrap());
        assert!(!journal.exists());
        assert_eq!(
            store.download_install_intents().unwrap()[0].state,
            "waiting"
        );
    }

    #[test]
    fn popped_native_plan_cannot_start_after_recovery_and_play_never_waits_for_admission() {
        let root = tempfile::tempdir().unwrap();
        let mut game = super::super::marker::game_from_marker(
            &marker(false),
            "fixture".into(),
            root.path().join("game"),
            None,
        );
        game.product_id = 910010;
        let plan = PersistedInstallationPlan {
            tracking: None,
            recovery_generation: super::super::recovery::generation(game.product_id),
            game: game.clone(),
            additional_installers: vec![],
            install_base: true,
            interactive_prompts: false,
            download_intent_id: None,
        };
        // Model the real schedule_next gap: plan was removed from the queue before recovery.
        let reservation = super::super::recovery::Reservation::reserve(&[game.product_id]).unwrap();
        drop(reservation);
        start_queued_installation(plan, crate::auth::session());
        assert!(
            !MANAGER
                .lock()
                .unwrap()
                .active
                .contains_key(&game.product_id)
        );
        assert!(!game.installation_directory.exists());
        let _registration = super::super::recovery::admit_generation(
            game.product_id,
            super::super::recovery::generation(game.product_id),
        )
        .unwrap();
        let start = std::time::Instant::now();
        let events = super::super::launch_game(game);
        assert!(start.elapsed() < std::time::Duration::from_millis(100));
        assert!(
            matches!(events.recv_timeout(std::time::Duration::from_millis(100)).unwrap(), super::super::LaunchEvent::Failed(message) if message.contains("busy"))
        );
    }

    #[test]
    fn failed_download_handoff_keeps_intent_unconsumed() {
        let root = tempfile::tempdir().unwrap();
        let store = StateStore::open_at(&root.path().join("state.db")).unwrap();
        let library = root.path().join("library");
        std::fs::create_dir_all(&library).unwrap();
        std::fs::write(library.join(".ludomere"), b"preserve").unwrap();
        let plan = PersistedInstallationPlan {
            tracking: None,
            recovery_generation: 0,
            game: super::super::marker::game_from_marker(
                &marker(false),
                "library".into(),
                library.join("game"),
                None,
            ),
            additional_installers: vec![],
            install_base: true,
            interactive_prompts: false,
            download_intent_id: Some("choice".into()),
        };
        store
            .save_download_install_intent(&crate::state::DownloadInstallIntent {
                product_id: 7,
                intent_id: "choice".into(),
                job_ids: vec!["job".into()],
                plan_json: "{}".into(),
                state: "waiting".into(),
                error: None,
            })
            .unwrap();
        assert!(persist_download_handoff(&store, &plan, 1).is_err());
        assert_eq!(
            store.download_install_intents().unwrap()[0].state,
            "waiting"
        );
        assert_eq!(
            std::fs::read(library.join(".ludomere")).unwrap(),
            b"preserve"
        );
    }

    fn marker(dlc: bool) -> InstallationMarker {
        InstallationMarker {
            schema_version: 1,
            product_id: 7,
            slug: "game".into(),
            base: InstalledComponent {
                operating_system: Some("linux".into()),
                language: Some("en".into()),
                version: None,
                revision_id: None,
                installed_at: 1,
            },
            dlc: Vec::new(),
            compatibility: None,
            source: InstallationSource::GalaxyDepot,
            galaxy_depot: Some(GalaxyDepotProvenance {
                build_id: "build".into(),
                repository_id: "repo".into(),
                manifest_fingerprint: "fingerprint".into(),
                branch: None,
                language: None,
                architecture: None,
                depots: vec![GalaxyDepotIdentity {
                    depot_id: "base".into(),
                    manifest_id: "base-m".into(),
                }],
                dlc: dlc
                    .then(|| GalaxyDepotDlcProvenance {
                        product_id: 9,
                        depots: Vec::new(),
                        has_payload: false,
                        entitlement_only_marker: true,
                    })
                    .into_iter()
                    .collect(),
            }),
            launch: None,
            dependencies: Vec::new(),
        }
    }

    fn request(target_dlc: bool) -> DepotOperationRequest {
        DepotOperationRequest {
            recovery_generation: 0,
            operation_id: "op".into(),
            product_id: 7,
            build_id: "build".into(),
            branch: None,
            kind: DepotOperationKind::Update,
            sources: vec![DepotSource {
                product_id: 7,
                depot_id: "base".into(),
                manifest_id: "base-m".into(),
                manifest_json: None,
                content_root: None,
            }],
            current_sources: vec![DepotSource {
                product_id: 7,
                depot_id: "base".into(),
                manifest_id: "base-m".into(),
                manifest_json: None,
                content_root: None,
            }],
            current_manifest_json: None,
            library_id: "library".into(),
            dependencies: Vec::new(),
            entitlement_dlc: target_dlc
                .then(|| EntitlementDlc {
                    product_id: 9,
                    name: "DLC".into(),
                })
                .into_iter()
                .collect(),
            library_root: PathBuf::from("/library"),
            slug: "game".into(),
            destination: PathBuf::from("/library/game"),
            staging_path: PathBuf::from("/library/.ludomere/staging/game.json"),
            target_marker: marker(target_dlc),
            account_session: crate::online::account_session(),
            dependency_plan: None,
            access_token: "token-password-sentinel".into(),
        }
    }

    #[test]
    fn setup_repository_refreshes_old_resume_cache_once_and_validates_context() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let directory = tempfile::tempdir().unwrap();
        let store = StateStore::open_at(&directory.path().join("state.db")).unwrap();
        let request = request(false);
        let fixture = br#"{"version":2,"baseProductId":"7","buildId":"build","platform":"windows","installDirectory":"Game","products":[{"productId":"7","temp_executable":"setup.exe","temp_arguments":"/custom"}],"depots":[{"productId":"7","manifest":"abcd","size":1}]}"#;
        let repository = crate::gog::repository::parse(fixture).unwrap();
        let mut old = serde_json::to_value(&repository).unwrap();
        old.as_object_mut().unwrap().remove("setupMetadataVersion");
        old["products"][0]["tempExecutable"] = serde_json::Value::Null;
        old["products"][0]["tempArguments"] = serde_json::Value::Null;
        store
            .save_depot_repository(&crate::state::DepotRepositoryRecord {
                product_id: 7,
                operating_system: "windows".into(),
                build_id: "build".into(),
                branch: None,
                manifest_identity: "repo".into(),
                repository_json: old.to_string(),
                first_seen_at: 1,
                last_seen_at: 1,
            })
            .unwrap();
        let cancelled = AtomicBool::new(false);
        assert!(
            setup_repository(&store, &request, &cancelled, |_| anyhow::bail!("offline")).is_err()
        );
        let restored = setup_repository(&store, &request, &cancelled, |identity| {
            assert_eq!(identity, "repo");
            crate::gog::repository::parse(fixture)
        })
        .unwrap();
        assert_eq!(
            restored.products[0].temp_executable.as_deref(),
            Some("setup.exe")
        );
        assert_eq!(
            setup_repository(&store, &request, &cancelled, |_| panic!(
                "current cache must not refetch"
            ))
            .unwrap(),
            restored
        );
        let mut cached = store
            .depot_repository(7, "windows", "build")
            .unwrap()
            .unwrap();
        for field in ["baseProductId", "buildId", "platform"] {
            let mut wrong = serde_json::to_value(&restored).unwrap();
            wrong[field] = serde_json::json!("wrong");
            cached.repository_json = wrong.to_string();
            store.save_depot_repository(&cached).unwrap();
            assert!(
                setup_repository(&store, &request, &cancelled, |_| panic!(
                    "mismatch must fail"
                ))
                .is_err()
            );
        }
        let mut no_build = serde_json::to_value(&restored).unwrap();
        no_build.as_object_mut().unwrap().remove("buildId");
        cached.repository_json = no_build.to_string();
        store.save_depot_repository(&cached).unwrap();
        assert!(
            setup_repository(&store, &request, &cancelled, |_| panic!(
                "identity-bound cache accepts omitted optional build ID"
            ))
            .is_ok()
        );
        cached.repository_json = old.to_string();
        store.save_depot_repository(&cached).unwrap();
        let error = setup_repository(&store, &request, &cancelled, |_| {
            cancelled.store(true, Ordering::Relaxed);
            crate::gog::repository::parse(fixture)
        })
        .unwrap_err();
        assert!(
            error
                .downcast_ref::<crate::download::depot::DepotCancelled>()
                .is_some()
        );
        assert_eq!(
            store
                .depot_repository(7, "windows", "build")
                .unwrap()
                .unwrap()
                .repository_json,
            old.to_string()
        );
    }

    #[test]
    fn legacy_resume_resolves_before_payload_and_incomplete_saved_plan_is_rejected() {
        let store = StateStore::open().unwrap();
        let mut request = request(false);
        request.recovery_generation = super::super::recovery::generation(request.product_id);
        request.target_marker.base.operating_system = Some("windows".into());
        request.sources[0].manifest_json = Some(r#"{"version":2,"depot":{"items":[]}}"#.into());
        request.sources[0].content_root = Some("7".into());
        let mut repository=crate::gog::repository::parse(br#"{"version":2,"baseProductId":"7","buildId":"build","platform":"windows","installDirectory":"Game","products":[{"productId":"7"}],"depots":[{"productId":"7","manifest":"abcd","size":1}]}"#).unwrap();
        let save = |repository: &crate::gog::types::GenerationTwoRepository| {
            store
                .save_depot_repository(&crate::state::DepotRepositoryRecord {
                    product_id: 7,
                    operating_system: "windows".into(),
                    build_id: "build".into(),
                    branch: None,
                    manifest_identity: "repo".into(),
                    repository_json: serde_json::to_string(repository).unwrap(),
                    first_seen_at: 1,
                    last_seen_at: 1,
                })
                .unwrap();
        };
        save(&repository);
        let mut old = serde_json::to_value(PersistedDepotPlan::from(&request)).unwrap();
        old.as_object_mut().unwrap().remove("dependency_plan");
        let old: PersistedDepotPlan = serde_json::from_value(old).unwrap();
        assert!(old.dependency_plan.is_none());
        request.dependency_plan = old.dependency_plan;
        prepare_required_dependencies(&mut request, &std::sync::atomic::AtomicBool::new(false))
            .unwrap();
        assert!(request.dependency_plan.as_ref().unwrap().entries.is_empty());
        let frozen: PersistedDepotPlan = serde_json::from_str(
            &serde_json::to_string(&PersistedDepotPlan::from(&request)).unwrap(),
        )
        .unwrap();
        assert!(frozen.dependency_plan.is_some());
        repository.dependencies = vec!["openAL".into()];
        request.dependencies = repository.dependencies.clone();
        save(&repository);
        let error =
            prepare_required_dependencies(&mut request, &std::sync::atomic::AtomicBool::new(false))
                .unwrap_err();
        assert!(error.to_string().contains("complete selected build"));
    }

    #[test]
    fn merged_small_file_manifest_prepares_persists_and_resumes_with_dependencies() {
        use crate::gog::depot_manifest::{DepotEntry, parse_snapshot};
        use std::io::Write;
        let root = tempfile::tempdir().unwrap();
        let wire = |path: &str, number: u64| {
            let digest = format!("{number:032x}");
            let chunk = serde_json::json!({"compressedMd5":digest,"compressedSize":1,"md5":digest,"size":1});
            serde_json::json!({"version":2,"depot":{"items":[{"type":"DepotFile","path":path,"chunks":[chunk.clone()],"sfcRef":{"offset":0,"size":1}}],"smallFilesContainer":{"chunks":[chunk]}}}).to_string()
        };
        let mut request = request(false);
        request.destination = root.path().join("game");
        request.library_root = root.path().to_owned();
        request.staging_path = root.path().join(".ludomere/staging/game.json");
        request.current_sources.clear();
        request.sources[0].manifest_json = Some(wire("first.dat", 1));
        request.sources[0].content_root = Some("/".into());
        let mut second = request.sources[0].clone();
        second.depot_id = "second".into();
        second.manifest_id = "second-m".into();
        second.manifest_json = Some(wire("second.dat", 2));
        request.sources.push(second);
        let provenance = request.target_marker.galaxy_depot.as_mut().unwrap();
        provenance.manifest_fingerprint.clear();
        provenance.depots.push(GalaxyDepotIdentity {
            depot_id: "second".into(),
            manifest_id: "second-m".into(),
        });
        let mut compressed =
            flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
        compressed
            .write_all(wire("dependency.dat", 3).as_bytes())
            .unwrap();
        let bytes = compressed.finish().unwrap();
        request.dependency_plan = Some(crate::gog::dependencies::Plan {
            version: 1,
            catalog_build: "1".into(),
            entries: vec![crate::gog::dependencies::Dependency {
                id: "GameLocal".into(),
                name: "Game local fixture".into(),
                manifest_id: format!("{:x}", md5::compute(&bytes)),
                manifest_bytes: bytes,
                method: crate::gog::dependencies::Method::GameFiles,
            }],
        });
        let identity = planned_manifest_identity(&request).unwrap();
        request
            .target_marker
            .galaxy_depot
            .as_mut()
            .unwrap()
            .manifest_fingerprint = identity.clone();
        let payload = merge_depot_sources(&request).unwrap().0;
        assert_eq!(payload.small_files_containers.len(), 2);
        // Both plan=None merged payload and payload+dependency triggered the old wire error.
        assert!(
            payload
                .canonical_json()
                .unwrap_err()
                .to_string()
                .contains("invalid small-files container index")
        );
        assert_eq!(
            super::super::dependency_setup::combined_manifest(&payload, None).unwrap(),
            payload
        );
        let combined = super::super::dependency_setup::combined_manifest(
            &payload,
            request.dependency_plan.as_ref(),
        )
        .unwrap();
        assert_eq!(combined.small_files_containers.len(), 3);
        assert_eq!(
            combined
                .entries
                .iter()
                .filter_map(|entry| match entry {
                    DepotEntry::File(file) =>
                        file.small_file.map(|reference| reference.container_index),
                    _ => None,
                })
                .collect::<Vec<_>>(),
            [0, 1, 2]
        );
        assert_eq!(combined.identity(), identity);
        request.current_manifest_json = Some(combined.snapshot_json().unwrap());
        persist_depot_request(&request).unwrap();
        let journal = super::super::operation_journal::read(
            &super::super::operation_journal::depot_path(&request.staging_path),
        )
        .unwrap();
        let super::super::operation_journal::OperationJournal::Depot { record, .. } = journal
        else {
            panic!()
        };
        let saved: PersistedDepotPlan = serde_json::from_str(&record.plan_json).unwrap();
        request.current_manifest_json = saved.current_manifest_json;
        request.dependency_plan = saved.dependency_plan;
        assert_eq!(current_manifest(&request).unwrap().unwrap(), combined);

        super::super::marker::write(&request.target_marker, &request.destination).unwrap();
        let store = StateStore::open().unwrap();
        let mut record = crate::state::DepotManifestRecord {
            manifest_identity: identity,
            product_id: 7,
            build_id: request.build_id.clone(),
            depot_id: "ludomere:installed-with-dependencies".into(),
            manifest_json: combined.snapshot_json().unwrap(),
            first_seen_at: 1,
            last_seen_at: 1,
        };
        store.save_depot_manifest(&record).unwrap();
        request.current_manifest_json = None;
        assert_eq!(current_manifest(&request).unwrap().unwrap(), combined);
        assert_eq!(
            parse_snapshot(record.manifest_json.as_bytes()).unwrap(),
            combined
        );

        // Pre-combined-snapshot markers bind only strict original wire payloads.
        let mut old_marker = request.target_marker.clone();
        old_marker
            .galaxy_depot
            .as_mut()
            .unwrap()
            .manifest_fingerprint = payload.legacy_payload_identity();
        super::super::marker::write(&old_marker, &request.destination).unwrap();
        request.current_sources = request.sources.clone();
        assert_eq!(
            current_manifest(&request).unwrap().unwrap(),
            payload,
            "today's target GameFiles plan must not alter historical installed identity"
        );
        request.current_sources[0].manifest_json = Some(wire("tampered.dat", 1));
        assert!(current_manifest(&request).is_err());

        // Legacy identity is never a bypass for tampered typed snapshots.
        super::super::marker::write(&request.target_marker, &request.destination).unwrap();
        let mut swapped = combined.clone();
        if let DepotEntry::File(file) = &mut swapped.entries[0] {
            file.small_file.as_mut().unwrap().container_index = 1;
        }
        record.manifest_json = swapped.snapshot_json().unwrap();
        store.save_depot_manifest(&record).unwrap();
        assert!(
            current_manifest(&request)
                .unwrap_err()
                .to_string()
                .contains("ownership manifest is damaged")
        );
        let mut target = request.clone();
        target
            .target_marker
            .galaxy_depot
            .as_mut()
            .unwrap()
            .manifest_fingerprint = payload.legacy_payload_identity();
        assert!(
            merge_depot_sources(&target).is_err(),
            "new targets cannot use legacy identity acceptance"
        );
    }

    #[test]
    fn installed_dependency_snapshot_is_loaded_by_exact_marker_identity() {
        let root = tempfile::tempdir().unwrap();
        let bytes = include_bytes!("../../tests/fixtures/gog-dependencies/DOSBox074.zlib");
        let manifest = crate::gog::depot_manifest::parse(bytes).unwrap();
        let mut installed = marker(false);
        let provenance = installed.galaxy_depot.as_mut().unwrap();
        provenance.build_id = "dependency-snapshot-fixture".into();
        provenance.manifest_fingerprint = manifest.identity();
        super::super::marker::write(&installed, root.path()).unwrap();
        let store = StateStore::open().unwrap();
        let mut record = crate::state::DepotManifestRecord {
            manifest_identity: manifest.identity(),
            product_id: 7,
            build_id: "dependency-snapshot-fixture".into(),
            depot_id: "ludomere:installed-with-dependencies".into(),
            manifest_json: manifest.canonical_json().unwrap(),
            first_seen_at: 1,
            last_seen_at: 1,
        };
        store.save_depot_manifest(&record).unwrap();
        let mut request = request(false);
        request.destination = root.path().to_owned();
        assert_eq!(
            current_manifest(&request).unwrap().unwrap().identity(),
            manifest.identity()
        );
        record.manifest_json = r#"{"version":2,"depot":{"items":[]}}"#.into();
        store.save_depot_manifest(&record).unwrap();
        assert!(
            current_manifest(&request)
                .unwrap_err()
                .to_string()
                .contains("ownership manifest is damaged")
        );
    }

    #[test]
    fn retry_policy_is_bounded_and_kind_driven() {
        use crate::download::depot::TransferErrorKind::*;
        let root_b = SourceRetryState::default();
        let (action, root_a) = retry_action(Transient, SourceRetryState::default(), 2);
        assert_eq!(action, RetryAction::NextEndpoint);
        assert_eq!(root_a.endpoint, 1);
        assert_eq!(root_b, SourceRetryState::default());
        let (action, root_a) = retry_action(AuthenticationOrExpired, root_a, 2);
        assert_eq!(action, RetryAction::Refresh);
        assert_eq!(root_a.refreshes, 1);
        assert_eq!(
            retry_action(AuthenticationOrExpired, root_a, 2).0,
            RetryAction::Stop
        );

        let (action, state) = retry_action(Transient, SourceRetryState::default(), 1);
        assert_eq!(action, RetryAction::Refresh);
        assert_eq!(retry_action(Transient, state, 1).0, RetryAction::Stop);
        let maxed = SourceRetryState {
            attempts: 4,
            ..Default::default()
        };
        assert_eq!(retry_action(Transient, maxed, 3).0, RetryAction::Stop);
        for kind in [PermanentHttp, Integrity, DecodeOrManifest] {
            assert_eq!(
                retry_action(kind, SourceRetryState::default(), 2).0,
                RetryAction::Stop
            );
        }
        let failure = SourceTransferFailure {
            source: 3,
            kind: Transient,
        };
        assert_eq!(failure.to_string(), "depot content transfer failed");
        assert!(!format!("{failure:?}").contains("token"));
        assert!(!format!("{failure:?}").contains("http"));
    }

    #[test]
    fn maps_known_gog_dependencies_and_rejects_unknown_required_ones() {
        assert_eq!(
            dependency_verbs(&[
                "DirectX".into(),
                "MSVC2010".into(),
                "MSVC2010_x64".into(),
                "MSVC2015".into(),
            ])
            .unwrap(),
            [
                "d3dcompiler_43",
                "d3dx9",
                "vcrun2010",
                "vcrun2015",
                "xact",
                "xinput"
            ]
        );
        assert!(dependency_verbs(&["FutureRuntime".into()]).is_err());
    }

    #[test]
    fn only_exact_completed_dependency_verbs_are_skipped_on_retry() {
        let prefix = tempfile::tempdir().unwrap();
        let requested =
            dependency_verbs(&["DirectX".into(), "MSVC2010".into(), "MSVC2012".into()]).unwrap();
        assert_eq!(
            pending_dependency_verbs(prefix.path(), requested.clone()).unwrap(),
            requested
        );
        std::fs::write(prefix.path().join("winetricks.log"), "").unwrap();
        assert_eq!(
            pending_dependency_verbs(prefix.path(), requested.clone()).unwrap(),
            requested
        );
        std::fs::write(
            prefix.path().join("winetricks.log"),
            "  d3dcompiler_43  \nvcrun2010_extra\n",
        )
        .unwrap();
        let pending = pending_dependency_verbs(prefix.path(), requested.clone()).unwrap();
        assert!(!pending.iter().any(|verb| verb == "d3dcompiler_43"));
        assert!(pending.iter().any(|verb| verb == "vcrun2010"));
        // A failed remaining command leaves the same requirements pending on retry.
        assert_eq!(
            pending_dependency_verbs(prefix.path(), requested.clone()).unwrap(),
            pending
        );
        std::fs::write(prefix.path().join("winetricks.log"), requested.join("\n")).unwrap();
        assert!(
            pending_dependency_verbs(prefix.path(), requested)
                .unwrap()
                .is_empty()
        );
        std::fs::write(prefix.path().join("winetricks.log"), "vcrun2015\n").unwrap();
        assert_eq!(
            pending_dependency_verbs(
                prefix.path(),
                dependency_verbs(&["MSVC2019".into()]).unwrap()
            )
            .unwrap(),
            ["vcrun2019"]
        );
        assert!(dependency_verbs(&["UnknownRuntime".into()]).is_err());
    }

    #[test]
    fn invalid_dependency_history_never_claims_setup_complete() {
        let prefix = tempfile::tempdir().unwrap();
        let path = prefix.path().join("winetricks.log");
        for invalid in [
            vec![0xff],
            b"d3dcompiler_43\0\n".to_vec(),
            vec![b'x'; 1024 * 1024 + 1],
        ] {
            std::fs::write(&path, invalid).unwrap();
            assert!(
                pending_dependency_verbs(prefix.path(), vec!["d3dcompiler_43".into()]).is_err()
            );
        }
        std::fs::remove_file(&path).unwrap();
        std::os::unix::fs::symlink(prefix.path().join("absent"), &path).unwrap();
        assert!(pending_dependency_verbs(prefix.path(), vec!["d3dcompiler_43".into()]).is_err());
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert!(pending_dependency_verbs(prefix.path(), vec!["d3dcompiler_43".into()]).is_err());
        std::fs::remove_dir(&path).unwrap();
        let name = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        assert!(pending_dependency_verbs(prefix.path(), vec!["d3dcompiler_43".into()]).is_err());
    }

    #[test]
    fn msvc2019_satisfies_requested_2015_without_conflicting_verbs() {
        assert_eq!(
            dependency_verbs(&["MSVC2019".into()]).unwrap(),
            ["vcrun2019"]
        );
        for dependencies in [
            vec!["MSVC2019_x64"],
            vec!["MSVC2019", "MSVC2019_x64"],
            vec!["MSVC2015", "MSVC2019", "MSVC2015_x64", "MSVC2019"],
            vec!["MSVC2019", "MSVC2015_x64", "MSVC2015"],
            vec!["MSVC2015", "MSVC2015_x64", "MSVC2019_x64"],
            vec!["MSVC2019_x64", "MSVC2015_x64", "MSVC2015"],
        ] {
            assert_eq!(
                dependency_verbs(
                    &dependencies
                        .into_iter()
                        .map(str::to_owned)
                        .collect::<Vec<_>>()
                )
                .unwrap(),
                ["vcrun2019"]
            );
        }
        assert_eq!(
            dependency_verbs(&[
                "MSVC2010".into(),
                "MSVC2012".into(),
                "MSVC2013".into(),
                "MSVC2015".into(),
                "MSVC2019".into(),
                "DirectX".into()
            ])
            .unwrap(),
            [
                "d3dcompiler_43",
                "d3dx9",
                "vcrun2010",
                "vcrun2012",
                "vcrun2013",
                "vcrun2019",
                "xact",
                "xinput"
            ]
        );
        for unknown in ["FutureRuntime", "MSVC2019_unknown"] {
            assert_eq!(
                dependency_verbs(&["MSVC2019".into(), unknown.into()])
                    .unwrap_err()
                    .to_string(),
                format!("unsupported required GOG dependency {unknown}")
            );
        }
    }

    #[test]
    fn uncommitted_msvc2019_is_retried_after_first_install_failure() {
        let mut target = marker(false);
        target.dependencies = vec!["MSVC2019".into()];
        let committed = dependency_commit_marker(&target, None);
        assert!(committed.dependencies.is_empty());
        assert_eq!(
            changed_dependency_verbs(&committed.dependencies, &target.dependencies).unwrap(),
            ["vcrun2019"]
        );
        // Resume preserves the old committed set until dependency setup succeeds.
        let resumed = dependency_commit_marker(&target, Some(&committed));
        assert_eq!(
            changed_dependency_verbs(&resumed.dependencies, &target.dependencies).unwrap(),
            ["vcrun2019"]
        );
        assert!(
            changed_dependency_verbs(&target.dependencies, &target.dependencies)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn changed_gog_dependencies_schedule_winetricks() {
        let mut target = marker(false);
        target.dependencies = vec!["MSVC2015".into()];
        let committed = dependency_commit_marker(&target, None);
        assert_eq!(
            changed_dependency_verbs(&committed.dependencies, &target.dependencies).unwrap(),
            ["vcrun2015"]
        );
        assert!(
            changed_dependency_verbs(&["MSVC2015".into()], &["MSVC2015".into()])
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn writes_exact_build_entitlement_marker_in_controlled_staging() {
        let root = std::env::temp_dir().join(format!(
            "ludomere-entitlement-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap()
        ));
        let mut request = request(true);
        request.library_root = root.clone();
        request.destination = root.join("game");
        request.staging_path = root.join(".ludomere/staging/game.json");
        std::fs::create_dir_all(&request.destination).unwrap();
        write_entitlement_markers(&request, "en-US").unwrap();
        let value: serde_json::Value = serde_json::from_slice(
            &std::fs::read(request.destination.join("goggame-9.info")).unwrap(),
        )
        .unwrap();
        assert_eq!(value["gameId"], "9");
        assert_eq!(value["rootGameId"], "7");
        assert_eq!(value["buildId"], "build");
        assert_eq!(value["playTasks"], serde_json::json!([]));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn reuses_only_verified_installed_chunks_for_update_and_repair() {
        use crate::gog::depot_manifest::{DepotChunk, DepotEntry, DepotFile, DepotManifest};
        let root = std::env::temp_dir().join(format!(
            "ludomere-local-chunk-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap()
        ));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("game.dat"), b"goodbad!").unwrap();
        let chunk = |bytes: &[u8]| DepotChunk {
            compressed_md5: format!("{:x}", md5::compute(bytes)),
            compressed_size: bytes.len() as u64,
            md5: format!("{:x}", md5::compute(bytes)),
            size: bytes.len() as u64,
        };
        let good = chunk(b"good");
        let mut recompressed = good.clone();
        recompressed.compressed_md5 = "f".repeat(32);
        recompressed.compressed_size = 3;
        let expected = chunk(b"best");
        let manifest = DepotManifest {
            generation: 2,
            entries: vec![DepotEntry::File(DepotFile {
                path: "game.dat".into(),
                size: 8,
                executable: false,
                support: false,
                md5: None,
                sha256: None,
                chunks: vec![good.clone(), expected.clone()],
                small_file: None,
            })],
            small_files_containers: Vec::new(),
        };
        let candidates = local_chunk_candidates(&manifest);
        let mut output = Vec::new();
        assert!(reuse_local_chunk(&root, &candidates, &recompressed, &mut output).unwrap());
        assert_eq!(output, b"good");
        output.clear();
        assert!(!reuse_local_chunk(&root, &candidates, &expected, &mut output).unwrap());
        assert!(output.is_empty());
        let reusable = reusable_local_chunks(
            &root,
            &candidates,
            &[recompressed.clone(), expected.clone()],
        )
        .unwrap();
        assert_eq!(
            required_network_bytes(&[recompressed, expected.clone()], &reusable, 7).unwrap(),
            7 + expected.compressed_size
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn support_depot_entries_never_enter_the_publishable_manifest() {
        let chunk = |digest: char| {
            format!(
                r#"{{"compressedMd5":"{0}","compressedSize":1,"md5":"{0}","size":1}}"#,
                digest.to_string().repeat(32)
            )
        };
        let raw = format!(
            r#"{{"version":2,"depot":{{"items":[
              {{"type":"DepotFile","path":"game.exe","flags":[],"chunks":[{}]}},
              {{"type":"DepotFile","path":"app\\config.ini","flags":["support"],"chunks":[{}]}}
            ]}}}}"#,
            chunk('1'),
            chunk('2')
        );
        let mut request = request(false);
        request.sources[0].manifest_json = Some(raw);
        request.sources[0].content_root = Some("/".into());
        request
            .target_marker
            .galaxy_depot
            .as_mut()
            .unwrap()
            .manifest_fingerprint = String::new();
        let identity = planned_manifest_identity(&request).unwrap();
        request
            .target_marker
            .galaxy_depot
            .as_mut()
            .unwrap()
            .manifest_fingerprint = identity;
        let (payload, _) = merge_depot_sources(&request).unwrap();
        let (support, _) = merge_support_sources(&request).unwrap();
        assert!(
            matches!(&payload.entries[..], [crate::gog::depot_manifest::DepotEntry::File(file)] if file.path == "game.exe")
        );
        assert!(
            matches!(&support.entries[..], [crate::gog::depot_manifest::DepotEntry::File(file)] if file.path == "app/config.ini" && file.support)
        );
    }

    #[test]
    fn depot_registration_releases_snapshot_lock_and_cleans_failed_reservation() {
        const CHILD: &str = "LUDOMERE_TEST_DEPOT_REGISTRATION";
        if std::env::var_os(CHILD).is_none() {
            let root = tempfile::tempdir().unwrap();
            let mut command = std::process::Command::new(std::env::current_exe().unwrap());
            command.args(["--exact", "installation::manager::tests::depot_registration_releases_snapshot_lock_and_cleans_failed_reservation", "--nocapture"]);
            command.env(CHILD, "1");
            for key in [
                "HOME",
                "XDG_CONFIG_HOME",
                "XDG_DATA_HOME",
                "XDG_CACHE_HOME",
                "XDG_RUNTIME_DIR",
                "TMPDIR",
            ] {
                let path = root.path().join(key);
                std::fs::create_dir(&path).unwrap();
                command.env(key, path);
            }
            assert!(command.status().unwrap().success());
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let mut operation = request(false);
        operation.library_root = root.path().to_owned();
        operation.destination = root.path().join("game");
        let mut called = false;
        assert!(!enqueue_depot_operation_with(operation, |request| {
            called = true;
            let manager = DEPOT_MANAGER
                .try_lock()
                .expect("disk persistence must not hold UI snapshot lock");
            assert!(manager.reservations.contains_key(&request.operation_id));
            let cancelled = manager.active.get(&request.operation_id).unwrap().clone();
            drop(manager);
            assert!(cancel_depot_operation(&request.operation_id));
            assert!(cancelled.load(std::sync::atomic::Ordering::Relaxed));
            pause_for_sign_out().unwrap();
            assert!(DEPOT_MANAGER.lock().unwrap().paused_for_sign_out);
            anyhow::bail!("synthetic persistence failure")
        }));
        assert!(called);
        let manager = DEPOT_MANAGER.lock().unwrap();
        assert!(manager.active.is_empty());
        assert!(manager.reservations.is_empty());
        assert!(manager.paused_for_sign_out);
    }

    #[test]
    fn persisted_plan_and_debug_exclude_access_secret() {
        let request = request(false);
        let debug = format!("{request:?}");
        let json = serde_json::to_string(&PersistedDepotPlan::from(&request)).unwrap();
        assert!(!debug.contains("token-password-sentinel"));
        assert!(!json.contains("token-password-sentinel"));
        assert!(json.contains("current_sources"));
        assert!(json.contains("library_root"));
        assert!(json.contains("staging_path"));
    }

    #[test]
    fn entitlement_only_dlc_removal_is_explicit_and_validates_base_sources() {
        let removed_request = request(false);
        let paths = forced_dlc_removals_for(&removed_request, &marker(true)).unwrap();
        assert!(paths.contains("goggame-9.info"));
        assert!(
            forced_dlc_removals_for(&request(true), &marker(true))
                .unwrap()
                .is_empty()
        );
        let mut invalid = removed_request;
        invalid.current_sources.clear();
        assert!(forced_dlc_removals_for(&invalid, &marker(true)).is_err());
    }

    #[test]
    fn payload_dlc_forces_only_owned_leaves_and_marker() {
        let raw = r#"{"version":2,"depot":{"items":[
            {"type":"DepotDirectory","path":"dlc"},
            {"type":"DepotFile","path":"dlc/exclusive.dat","chunks":[]},
            {"type":"DepotLink","path":"dlc/current","target":"exclusive.dat"},
            {"type":"DepotFile","path":"shared.dat","chunks":[]}
        ]}}"#;
        let mut current = marker(true);
        current.galaxy_depot.as_mut().unwrap().dlc[0].depots = vec![GalaxyDepotIdentity {
            depot_id: "dlc".into(),
            manifest_id: "dlc-m".into(),
        }];
        current.galaxy_depot.as_mut().unwrap().dlc[0].has_payload = true;
        current.galaxy_depot.as_mut().unwrap().dlc[0].entitlement_only_marker = false;
        let mut removed = request(false);
        removed.current_sources.push(DepotSource {
            product_id: 9,
            depot_id: "dlc".into(),
            manifest_id: "dlc-m".into(),
            manifest_json: Some(raw.into()),
            content_root: Some("dlc-root".into()),
        });
        let paths = forced_dlc_removals_for(&removed, &current).unwrap();
        assert!(paths.contains("dlc/exclusive.dat"));
        assert!(paths.contains("dlc/current"));
        assert!(paths.contains("shared.dat"));
        assert!(paths.contains("goggame-9.info"));
        assert!(!paths.contains("dlc"));
        assert!(!paths.contains("dlc/user-mod.cfg"));
        assert!(!paths.contains("dlc/base.dat"));

        let mut retained = removed;
        retained.target_marker = current.clone();
        assert!(
            forced_dlc_removals_for(&retained, &current)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn dlc_overrides_base_but_peer_conflicts_remain_rejected() {
        let base = r#"{"version":2,"depot":{"items":[{"type":"DepotFile","path":"Shared.dat","md5":"00000000000000000000000000000000","chunks":[]},{"type":"DepotFile","path":"Identical.DAT","md5":"22222222222222222222222222222222","chunks":[]}]}}"#;
        let dlc = r#"{"version":2,"depot":{"items":[{"type":"DepotFile","path":"shared.dat","md5":"11111111111111111111111111111111","chunks":[]},{"type":"DepotFile","path":"identical.dat","md5":"22222222222222222222222222222222","chunks":[]}]}}"#;
        let expected = crate::gog::depot_manifest::parse(dlc.as_bytes()).unwrap();
        let mut scenario = request(true);
        scenario.sources = vec![
            DepotSource {
                product_id: 7,
                depot_id: "base".into(),
                manifest_id: "base-m".into(),
                manifest_json: Some(base.into()),
                content_root: Some("base-root".into()),
            },
            DepotSource {
                product_id: 9,
                depot_id: "dlc".into(),
                manifest_id: "dlc-m".into(),
                manifest_json: Some(dlc.into()),
                content_root: Some("dlc-root".into()),
            },
        ];
        let provenance = scenario.target_marker.galaxy_depot.as_mut().unwrap();
        provenance.manifest_fingerprint = expected.identity();
        provenance.dlc[0].depots = vec![GalaxyDepotIdentity {
            depot_id: "dlc".into(),
            manifest_id: "dlc-m".into(),
        }];
        provenance.dlc[0].has_payload = true;
        let (merged, _) = merge_depot_sources(&scenario).unwrap();
        assert_eq!(merged, expected);

        scenario.sources.push(DepotSource {
            product_id: 10,
            depot_id: "peer".into(),
            manifest_id: "peer-m".into(),
            manifest_json: Some(base.into()),
            content_root: Some("peer-root".into()),
        });
        provenance_with_peer(&mut scenario);
        assert!(merge_depot_sources(&scenario).is_err());

        let mut identical_then_different = request(true);
        identical_then_different.sources = vec![
            DepotSource {
                product_id: 7,
                depot_id: "base".into(),
                manifest_id: "base-m".into(),
                manifest_json: Some(base.into()),
                content_root: Some("base-root".into()),
            },
            DepotSource {
                product_id: 9,
                depot_id: "dlc".into(),
                manifest_id: "dlc-m".into(),
                manifest_json: Some(base.into()),
                content_root: Some("dlc-root".into()),
            },
            DepotSource {
                product_id: 10,
                depot_id: "peer".into(),
                manifest_id: "peer-m".into(),
                manifest_json: Some(dlc.into()),
                content_root: Some("peer-root".into()),
            },
        ];
        let provenance = identical_then_different
            .target_marker
            .galaxy_depot
            .as_mut()
            .unwrap();
        provenance.manifest_fingerprint = crate::gog::depot_manifest::parse(base.as_bytes())
            .unwrap()
            .identity();
        provenance.dlc[0].depots = vec![GalaxyDepotIdentity {
            depot_id: "dlc".into(),
            manifest_id: "dlc-m".into(),
        }];
        provenance.dlc[0].has_payload = true;
        provenance_with_peer(&mut identical_then_different);
        assert!(merge_depot_sources(&identical_then_different).is_err());
    }

    fn provenance_with_peer(request: &mut DepotOperationRequest) {
        request
            .target_marker
            .galaxy_depot
            .as_mut()
            .unwrap()
            .dlc
            .push(GalaxyDepotDlcProvenance {
                product_id: 10,
                depots: vec![GalaxyDepotIdentity {
                    depot_id: "peer".into(),
                    manifest_id: "peer-m".into(),
                }],
                has_payload: true,
                entitlement_only_marker: false,
            });
    }

    #[test]
    fn dlc_deselection_retains_identical_base_bytes_but_restores_differing_winner() {
        let removed_raw = r#"{"version":2,"depot":{"items":[{"type":"DepotFile","path":"Same.DAT","md5":"00000000000000000000000000000000","chunks":[]},{"type":"DepotFile","path":"Different.dat","md5":"11111111111111111111111111111111","chunks":[]}]}}"#;
        let base_raw = r#"{"version":2,"depot":{"items":[{"type":"DepotFile","path":"same.dat","md5":"00000000000000000000000000000000","chunks":[]},{"type":"DepotFile","path":"different.dat","md5":"22222222222222222222222222222222","chunks":[]}]}}"#;
        let mut current = marker(true);
        current.galaxy_depot.as_mut().unwrap().dlc[0].depots = vec![GalaxyDepotIdentity {
            depot_id: "dlc".into(),
            manifest_id: "dlc-m".into(),
        }];
        let mut request = request(false);
        request.sources[0].manifest_json = Some(base_raw.into());
        request.sources[0].content_root = Some("base-root".into());
        request.current_sources.push(DepotSource {
            product_id: 9,
            depot_id: "dlc".into(),
            manifest_id: "dlc-m".into(),
            manifest_json: Some(removed_raw.into()),
            content_root: Some("dlc-root".into()),
        });
        let paths = forced_dlc_removals_for(&request, &current).unwrap();
        assert!(!paths.contains("Same.DAT"));
        assert!(paths.contains("Different.dat"));
        assert!(paths.contains("goggame-9.info"));
    }

    #[test]
    fn supported_operation_serialization_has_no_rollback() {
        for operation in [
            DepotOperationKind::Install,
            DepotOperationKind::Update,
            DepotOperationKind::Repair,
            DepotOperationKind::BranchSwitch,
        ] {
            assert!(
                !serde_json::to_string(&operation)
                    .unwrap()
                    .contains("rollback")
            );
        }
    }

    fn operation(
        product_id: i64,
        state: &str,
        position: Option<i64>,
    ) -> InstallationOperationRecord {
        InstallationOperationRecord {
            product_id,
            operation: "install".into(),
            state: state.into(),
            plan_json: "{}".into(),
            message: None,
            percentage: None,
            queue_position: position,
            created_at: product_id,
            updated_at: product_id,
            completed_at: None,
        }
    }

    #[test]
    fn interrupted_active_operation_keeps_priority_over_queued_work() {
        let mut operations = [
            operation(30, "queued", Some(1)),
            operation(20, "running", Some(2)),
            operation(40, "queued", Some(3)),
        ];
        operations.sort_by_key(recovery_sort_key);
        assert_eq!(
            operations
                .iter()
                .map(|operation| operation.product_id)
                .collect::<Vec<_>>(),
            vec![20, 30, 40]
        );
    }

    #[test]
    fn queued_operations_recover_in_persisted_order() {
        let mut operations = [
            operation(30, "queued", Some(3)),
            operation(10, "queued", Some(1)),
            operation(20, "queued", Some(2)),
        ];
        operations.sort_by_key(recovery_sort_key);
        assert_eq!(
            operations
                .iter()
                .map(|operation| operation.product_id)
                .collect::<Vec<_>>(),
            vec![10, 20, 30]
        );
    }

    #[test]
    fn transfer_queue_runs_eight_workers_concurrently() {
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(9));
        let (started, observed) = std::sync::mpsc::channel();
        let workers = barrier.clone();
        let transfer = std::thread::spawn(move || {
            run_transfer_workers((0..16).collect(), |job| {
                started.send(job).unwrap();
                if job < 8 {
                    workers.wait();
                }
                Ok(job)
            })
            .unwrap()
        });
        for _ in 0..8 {
            observed
                .recv_timeout(std::time::Duration::from_secs(2))
                .expect("all eight transfer workers should start together");
        }
        barrier.wait();
        assert_eq!(transfer.join().unwrap(), (0..16).collect::<Vec<_>>());
    }

    #[test]
    fn interrupted_operation_is_not_prioritized_over_a_newer_failure() {
        assert!(depot_state_is_active("materializing"));
        assert!(depot_state_is_active("extracting"));
        assert!(!depot_state_is_active("interrupted"));
        assert!(!depot_state_is_active("failed"));
    }
}
