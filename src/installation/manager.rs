use super::{AdditionalInstaller, InstallationEvent, UninstallationEvent};
use crate::{
    domain::{DepotOperationKind, InstalledGame},
    state::{DepotOperationRecord, InstallationOperationRecord, StateStore},
};
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
    game: InstalledGame,
    additional_installers: Vec<AdditionalInstaller>,
    install_base: bool,
    interactive_prompts: bool,
}

#[derive(Clone)]
enum OperationControl {
    Installation(super::executor::InstallationControl),
    Uninstallation(super::executor::UninstallationControl),
}

#[derive(Clone)]
enum QueuedOperation {
    Installation(PersistedInstallationPlan),
    Uninstallation(InstalledGame),
}

impl QueuedOperation {
    fn product_id(&self) -> i64 {
        match self {
            Self::Installation(plan) => plan.game.product_id,
            Self::Uninstallation(game) => game.product_id,
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
}

static MANAGER: LazyLock<Mutex<ManagerState>> =
    LazyLock::new(|| Mutex::new(ManagerState::default()));

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
            | "verifying"
            | "verifying_existing"
            | "calculating"
            | "downloading"
            | "materializing"
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

pub fn enqueue_depot_operation(mut request: DepotOperationRequest) -> bool {
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
        || persist_depot_request(&request).is_err()
    {
        return false;
    }
    let cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    manager
        .active
        .insert(request.operation_id.clone(), cancelled.clone());
    manager.reservations.insert(
        request.operation_id.clone(),
        (request.product_id, request.destination.clone()),
    );
    let snapshot = DepotOperationSnapshot {
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
    manager
        .snapshots
        .insert(request.operation_id.clone(), snapshot.clone());
    drop(manager);
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
    let resumed_operation_id = operation_id.clone();
    let result = super::operation_journal::find_depot(&operation_id)
        .map(|(_, record)| record)
        .and_then(|record| {
            let plan: PersistedDepotPlan = serde_json::from_str(&record.plan_json)?;
            Ok(DepotOperationRequest {
                operation_id: resumed_operation_id,
                product_id: plan.product_id,
                build_id: plan.build_id,
                branch: plan.branch,
                kind: plan.kind,
                sources: plan.sources,
                current_sources: plan.current_sources,
                current_manifest_json: plan.current_manifest_json,
                library_id: plan.library_id,
                dependencies: plan.dependencies,
                entitlement_dlc: plan.entitlement_dlc,
                library_root: plan.library_root,
                slug: plan.slug,
                destination: plan.destination,
                staging_path: plan.staging_path,
                target_marker: plan.target_marker,
                access_token,
            })
        });
    match result {
        Ok(request) => enqueue_depot_operation(request),
        Err(error) => {
            publish_depot(DepotOperationSnapshot {
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
    DEPOT_MANAGER
        .lock()
        .unwrap()
        .active
        .remove(&request.operation_id);
    let abandon = DEPOT_MANAGER
        .lock()
        .unwrap()
        .abandon_requested
        .remove(&request.operation_id);
    DEPOT_MANAGER
        .lock()
        .unwrap()
        .reservations
        .remove(&request.operation_id);
    if abandon {
        let _ = abandon_saved_depot_operation(&request.operation_id);
    } else if let Some(snapshot) = failure_snapshot {
        publish_depot(snapshot);
    }
}

fn abandon_saved_depot_operation(operation_id: &str) -> anyhow::Result<()> {
    let (journal_path, record) = super::operation_journal::find_depot(operation_id)?;
    let plan: PersistedDepotPlan = serde_json::from_str(&record.plan_json)?;
    let request = DepotOperationRequest {
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

fn run_depot_operation_inner(
    request: &DepotOperationRequest,
    cancelled: &std::sync::atomic::AtomicBool,
) -> anyhow::Result<()> {
    publish_depot(DepotOperationSnapshot {
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
    let target_totals = target.totals()?;
    let support_totals = support.totals()?;
    let total = target_totals
        .compressed
        .checked_add(support_totals.compressed)
        .ok_or_else(|| anyhow::anyhow!("depot operation size overflows"))?;
    let write_total = target_totals
        .uncompressed
        .checked_add(support_totals.uncompressed)
        .ok_or_else(|| anyhow::anyhow!("depot write size overflows"))?;
    let payload_total = target_totals.compressed;
    if cancelled.load(std::sync::atomic::Ordering::Relaxed) {
        update_depot_record(&request.operation_id, "cancelled", 0, None, true)?;
        publish_depot(DepotOperationSnapshot {
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
    let commit_marker = dependency_commit_marker(&request.target_marker, installed_marker.as_ref());
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
    };
    let forced_remove_paths = forced_dlc_removals(request)?;
    let mut retry_states = HashMap::<usize, SourceRetryState>::new();
    loop {
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
                update_depot_record(&operation_id, "materializing", completed, None, false)?;
                Ok(())
            },
            || cancelled.load(std::sync::atomic::Ordering::Relaxed),
            || {
                update_depot_record(
                    &request.operation_id,
                    "committing",
                    payload_total,
                    None,
                    false,
                )?;
                publish_depot(DepotOperationSnapshot {
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
    let finalization = finalize_depot_metadata(request, support_root.as_deref(), &removed_actions);
    let cleanup = super::depot_actions::remove_support_staging(&request.staging_path);
    finalization?;
    cleanup?;
    crate::download::depot::finish_journal(&request.staging_path)?;
    update_depot_record(&request.operation_id, "complete", total, None, true)?;
    publish_depot(DepotOperationSnapshot {
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
    if request.current_sources.is_empty() {
        return request
            .current_manifest_json
            .as_deref()
            .map(|json| crate::gog::depot_manifest::parse(json.as_bytes()))
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
    current.target_marker = marker;
    merge_depot_sources(&current).map(|(manifest, _)| Some(manifest))
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
) -> anyhow::Result<()> {
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
        use crate::compatibility::CompatibilityBackend;
        let compatibility = marker
            .compatibility
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Windows depot marker has no compatibility profile"))?;
        if request.library_id.is_empty() {
            anyhow::bail!("Windows depot operation has no library identity");
        }
        let backend = crate::compatibility::default_backend();
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
        let prefix = backend.initialize_prefix(crate::compatibility::InitializePrefixRequest {
            library_id: request.library_id.clone(),
            library: request.library_root.clone(),
            slug: request.slug.clone(),
            profile: compatibility.profile.clone(),
            log_path: log_path.clone(),
        })?;
        let prefix = request.library_root.join(prefix.relative_path);
        crate::compatibility::append_step_log(
            &log_path,
            &format!("prefix path: {}", prefix.display()),
        )?;
        let installed_dependencies = super::marker::load(&request.destination)?
            .map(|installed| installed.dependencies)
            .unwrap_or_default();
        let verbs = changed_dependency_verbs(&installed_dependencies, &request.dependencies)?;
        if !verbs.is_empty() {
            let mut process = backend.run_winetricks(
                &prefix,
                &compatibility.profile,
                &verbs,
                &request.destination,
                &log_path,
            )?;
            if !process.wait()?.success() {
                anyhow::bail!("installing required Windows dependencies failed");
            }
        }
        marker.dependencies = request.dependencies.clone();
        let context = super::depot_actions::ActionContext {
            product_id: request.product_id,
            app: request.destination.clone(),
            support: super::depot_actions::support_staging(&request.staging_path)?,
            prefix,
            windows_app: crate::compatibility::windows_destination(&request.slug),
            profile: compatibility.profile.clone(),
            log_path,
        };
        for (product_id, actions) in removed_actions {
            let context = super::depot_actions::ActionContext {
                product_id: *product_id,
                ..context.clone()
            };
            super::depot_actions::execute_actions(&backend, &context, actions, true)?;
        }
        let mut products = vec![request.product_id];
        products.extend(marker.dlc.iter().map(|dlc| dlc.product_id));
        for product_id in products {
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
                ..context.clone()
            };
            super::depot_actions::execute_actions(&backend, &context, &actions, false)?;
        }
    }
    super::marker::write(&marker, &request.destination)
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
            unknown => anyhow::bail!("unsupported required GOG dependency {unknown}"),
        }
    }
    Ok(verbs.into_iter().collect())
}

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
    let previous = super::operation_journal::read(&journal_path)
        .ok()
        .and_then(|journal| match journal {
            super::operation_journal::OperationJournal::Depot { record, .. } => Some(record),
            _ => None,
        });
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
        && provenance.manifest_fingerprint != manifest.identity()
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
    Ok(merge_depot_sources(request)?.0.identity())
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

fn entries_equivalent(
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
    let (path, mut record) = super::operation_journal::scan()?
        .into_iter()
        .find_map(|(path, journal)| match journal {
            super::operation_journal::OperationJournal::Depot { record, .. }
                if record.operation_id == operation_id =>
            {
                Some((path, record))
            }
            _ => None,
        })
        .ok_or_else(|| anyhow::anyhow!("saved depot operation was not found"))?;
    let now = chrono::Utc::now().timestamp();
    record.state = state.into();
    record.bytes_completed = bytes_completed;
    record.error = error.map(str::to_owned);
    record.updated_at = now;
    record.completed_at = terminal.then_some(now);
    if terminal && state == "complete" {
        super::operation_journal::remove(&path)
    } else {
        super::operation_journal::write_depot(&path, &record)
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
    let product_id = plan.product_id;
    let persisted_plan = PersistedInstallationPlan {
        game: plan.clone(),
        additional_installers: additional_installers.clone(),
        install_base,
        interactive_prompts,
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
    schedule_next();
    true
}

pub fn enqueue_uninstallation(game: InstalledGame) -> bool {
    let product_id = game.product_id;
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
            .push_back(QueuedOperation::Uninstallation(game.clone()));
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
        &game,
        Some("Queued for uninstallation"),
        None,
        Some(queue_position),
    );
    if let Some(snapshot) = installation_operation_snapshot(product_id) {
        publish(InstallationManagerEvent::OperationQueued(snapshot));
    }
    schedule_next();
    true
}

fn schedule_next() {
    let operation = {
        let mut manager = MANAGER.lock().unwrap();
        if manager.shutting_down || !manager.active.is_empty() {
            return;
        }
        manager.queue.pop_front()
    };
    match operation {
        Some(QueuedOperation::Installation(plan)) => start_queued_installation(plan),
        Some(QueuedOperation::Uninstallation(game)) => start_queued_uninstallation(game),
        None => {}
    }
}

fn start_queued_installation(persisted_plan: PersistedInstallationPlan) {
    let product_id = persisted_plan.game.product_id;
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
    let handle = super::executor::start_installation(
        persisted_plan.game.clone(),
        persisted_plan.additional_installers.clone(),
        persisted_plan.install_base,
        persisted_plan.interactive_prompts,
    );
    {
        let mut manager = MANAGER.lock().unwrap();
        manager
            .active
            .insert(product_id, OperationControl::Installation(handle.control()));
    }
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
            let terminal = matches!(
                event,
                InstallationEvent::Complete { .. }
                    | InstallationEvent::Cancelled
                    | InstallationEvent::Failed(_)
            );
            let shutting_down = MANAGER.lock().unwrap().shutting_down;
            if shutting_down {
                if terminal {
                    persist_existing_operation(
                        product_id,
                        "queued",
                        Some("Queued after application shutdown"),
                        None,
                        None,
                    );
                }
            } else {
                update_installation_snapshot(product_id, &event);
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

fn start_queued_uninstallation(game: InstalledGame) {
    let product_id = game.product_id;
    let handle = super::executor::start_uninstallation(game);
    {
        let mut manager = MANAGER.lock().unwrap();
        manager.active.insert(
            product_id,
            OperationControl::Uninstallation(handle.control()),
        );
    }
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
        while let Ok(event) = handle.events.recv() {
            let terminal = matches!(
                event,
                UninstallationEvent::Complete
                    | UninstallationEvent::Cancelled
                    | UninstallationEvent::Failed(_)
            );
            let shutting_down = MANAGER.lock().unwrap().shutting_down;
            if shutting_down {
                if terminal {
                    persist_existing_operation(
                        product_id,
                        "queued",
                        Some("Queued after application shutdown"),
                        None,
                        None,
                    );
                }
            } else {
                update_uninstallation_snapshot(product_id, &event);
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

pub fn respond_to_installation(product_id: i64, response: String) -> bool {
    let manager = MANAGER.lock().unwrap();
    let Some(OperationControl::Installation(control)) = manager.active.get(&product_id) else {
        return false;
    };
    control.respond(response);
    true
}

pub fn cancel_operation(product_id: i64) -> bool {
    let queued_snapshot = {
        let mut manager = MANAGER.lock().unwrap();
        if let Some(control) = manager.active.get(&product_id) {
            match control {
                OperationControl::Installation(control) => control.cancel(),
                OperationControl::Uninstallation(control) => control.cancel(),
            }
            return true;
        }
        let Some(index) = manager
            .queue
            .iter()
            .position(|operation| operation.product_id() == product_id)
        else {
            return false;
        };
        manager.queue.remove(index);
        let snapshot = InstallationOperationSnapshot {
            product_id,
            state: crate::domain::InstallationState::Failed,
            message: Some("Operation cancelled".into()),
            percentage: None,
            queued: false,
        };
        manager.snapshots.insert(product_id, snapshot.clone());
        snapshot
    };
    persist_existing_operation(
        product_id,
        "cancelled",
        Some("Operation cancelled"),
        None,
        Some(chrono::Utc::now().timestamp()),
    );
    publish(InstallationManagerEvent::OperationCancelled(
        queued_snapshot,
    ));
    schedule_next();
    true
}

pub fn recover_interrupted_operations() -> anyhow::Result<usize> {
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
            "uninstall" => serde_json::from_str::<InstalledGame>(&operation.plan_json)
                .map(QueuedOperation::Uninstallation),
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

pub fn shutdown() {
    let active_products = {
        let mut manager = MANAGER.lock().unwrap();
        manager.shutting_down = true;
        for control in manager.active.values() {
            match control {
                OperationControl::Installation(control) => control.cancel(),
                OperationControl::Uninstallation(control) => control.cancel(),
            }
        }
        manager.active.keys().copied().collect::<Vec<_>>()
    };
    for product_id in active_products {
        persist_existing_operation(
            product_id,
            "queued",
            Some("Queued after application shutdown"),
            None,
            None,
        );
    }
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

fn update_installation_snapshot(product_id: i64, event: &InstallationEvent) {
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
    MANAGER.lock().unwrap().snapshots.insert(
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
}

fn update_uninstallation_snapshot(product_id: i64, event: &UninstallationEvent) {
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
    MANAGER.lock().unwrap().snapshots.insert(
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
    use crate::{
        domain::{
            GalaxyDepotDlcProvenance, GalaxyDepotIdentity, GalaxyDepotProvenance,
            InstallationSource,
        },
        installation::marker::{InstallationMarker, InstalledComponent},
    };

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
            access_token: "token-password-sentinel".into(),
        }
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
        assert!(!depot_state_is_active("interrupted"));
        assert!(!depot_state_is_active("failed"));
    }
}
