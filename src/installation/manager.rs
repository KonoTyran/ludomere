use super::{AdditionalInstaller, InstallationEvent, UninstallationEvent};
use crate::{
    domain::InstalledGame,
    state::{InstallationOperationRecord, StateStore},
};
use std::{
    collections::{HashMap, VecDeque},
    sync::{LazyLock, Mutex, mpsc},
    thread,
};

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
    let store = StateStore::open()?;
    let mut recovered = 0;
    let mut operations = store
        .installation_operations()?
        .into_iter()
        .filter(|operation| matches!(operation.state.as_str(), "running" | "queued"))
        .collect::<Vec<_>>();
    operations.sort_by_key(recovery_sort_key);

    let mut recovered_snapshots = Vec::new();
    let mut manager = MANAGER.lock().unwrap();
    for mut operation in operations {
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
            store.upsert_installation_operation(&operation)?;
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
            store.upsert_installation_operation(&operation)?;
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
    if let Ok(store) = StateStore::open() {
        let _ = store.upsert_installation_operation(&record);
    }
}

fn persist_existing_operation(
    product_id: i64,
    state: &str,
    message: Option<&str>,
    percentage: Option<u8>,
    completed_at: Option<i64>,
) {
    let Ok(store) = StateStore::open() else {
        return;
    };
    let Ok(Some(mut record)) = store.installation_operations().map(|records| {
        records
            .into_iter()
            .find(|record| record.product_id == product_id)
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
    let _ = store.upsert_installation_operation(&record);
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
