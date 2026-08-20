use crate::state::{StateStore, WorkKind, WorkQueueItem};
use anyhow::Result;

pub struct QueueCoordinator;

impl QueueCoordinator {
    pub fn register(
        kind: WorkKind,
        source_id: &str,
        product_id: Option<i64>,
    ) -> Result<WorkQueueItem> {
        StateStore::open()?.register_work(kind, source_id, product_id)
    }

    pub fn ordered() -> Result<Vec<WorkQueueItem>> {
        StateStore::open()?.work_queue()
    }

    pub fn move_earlier(work_id: &str) -> Result<bool> {
        let moved = StateStore::open()?.move_work(work_id, -1)?;
        crate::operation_gate::wake();
        Ok(moved)
    }

    pub fn move_later(work_id: &str) -> Result<bool> {
        let moved = StateStore::open()?.move_work(work_id, 1)?;
        crate::operation_gate::wake();
        Ok(moved)
    }

    pub fn move_relative(work_id: &str, target_id: &str, after: bool) -> Result<bool> {
        let moved = StateStore::open()?.move_work_relative(work_id, target_id, after)?;
        crate::operation_gate::wake();
        Ok(moved)
    }

    pub fn complete(kind: WorkKind, source_id: &str) -> Result<()> {
        StateStore::open()?.complete_work(kind, source_id)
    }
}
