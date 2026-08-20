use crate::state::WorkKind;
use std::sync::{Condvar, LazyLock, Mutex};

struct Waiting {
    ticket: u64,
    work_id: Option<String>,
}

#[derive(Default)]
struct State {
    next: u64,
    queue: std::collections::VecDeque<Waiting>,
    running: bool,
}

static GATE: LazyLock<(Mutex<State>, Condvar)> =
    LazyLock::new(|| (Mutex::new(State::default()), Condvar::new()));

pub struct Permit;

#[cfg(test)]
pub fn acquire(cancelled: impl Fn() -> bool) -> Option<Permit> {
    acquire_registered(None, cancelled)
}

pub fn acquire_work(
    kind: WorkKind,
    source_id: &str,
    cancelled: impl Fn() -> bool,
) -> Option<Permit> {
    acquire_registered(Some(format!("{}:{source_id}", kind.as_str())), cancelled)
}

fn acquire_registered(work_id: Option<String>, cancelled: impl Fn() -> bool) -> Option<Permit> {
    let (lock, wake) = &*GATE;
    let mut state = lock.lock().unwrap();
    let ticket = state.next;
    state.next = state.next.wrapping_add(1);
    state.queue.push_back(Waiting { ticket, work_id });
    loop {
        if cancelled() {
            state.queue.retain(|queued| queued.ticket != ticket);
            wake.notify_all();
            return None;
        }
        if !state.running && next_ticket(&state.queue) == Some(ticket) {
            state.queue.retain(|queued| queued.ticket != ticket);
            state.running = true;
            return Some(Permit);
        }
        state = wake
            .wait_timeout(state, std::time::Duration::from_millis(100))
            .unwrap()
            .0;
    }
}

fn next_ticket(queue: &std::collections::VecDeque<Waiting>) -> Option<u64> {
    let positions = crate::work_queue::QueueCoordinator::ordered()
        .unwrap_or_default()
        .into_iter()
        .enumerate()
        .map(|(position, item)| (item.work_id, position))
        .collect::<std::collections::HashMap<_, _>>();
    next_ticket_with_positions(queue, &positions)
}

fn next_ticket_with_positions(
    queue: &std::collections::VecDeque<Waiting>,
    positions: &std::collections::HashMap<String, usize>,
) -> Option<u64> {
    queue
        .iter()
        .min_by_key(|waiting| {
            (
                waiting
                    .work_id
                    .as_ref()
                    .and_then(|work_id| positions.get(work_id))
                    .copied()
                    .unwrap_or(usize::MAX),
                waiting.ticket,
            )
        })
        .map(|waiting| waiting.ticket)
}

pub(crate) fn wake() {
    GATE.1.notify_all();
}

impl Drop for Permit {
    fn drop(&mut self) {
        let (lock, wake) = &*GATE;
        lock.lock().unwrap().running = false;
        wake.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex, mpsc};

    #[test]
    fn operations_run_one_at_a_time_in_fifo_order() {
        let order = Arc::new(Mutex::new(Vec::new()));
        let (started_sender, started_receiver) = mpsc::channel();
        let (release_sender, release_receiver) = mpsc::channel();
        let first_order = order.clone();
        let first = std::thread::spawn(move || {
            let _permit = acquire(|| false).unwrap();
            first_order.lock().unwrap().push(0);
            started_sender.send(()).unwrap();
            release_receiver.recv().unwrap();
        });
        started_receiver.recv().unwrap();
        let mut workers = Vec::new();
        for value in 1..3 {
            let order = order.clone();
            workers.push(std::thread::spawn(move || {
                let _permit = acquire(|| false).unwrap();
                order.lock().unwrap().push(value);
            }));
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        release_sender.send(()).unwrap();
        first.join().unwrap();
        for worker in workers {
            worker.join().unwrap();
        }
        assert_eq!(*order.lock().unwrap(), [0, 1, 2]);
    }

    #[test]
    fn registered_work_uses_the_durable_global_order() {
        let queue = std::collections::VecDeque::from([
            Waiting {
                ticket: 0,
                work_id: Some("download:first".into()),
            },
            Waiting {
                ticket: 1,
                work_id: Some("depot:second".into()),
            },
        ]);
        let positions = std::collections::HashMap::from([
            ("download:first".into(), 2),
            ("depot:second".into(), 1),
        ]);
        assert_eq!(next_ticket_with_positions(&queue, &positions), Some(1));
    }
}
