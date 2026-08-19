use std::sync::{Condvar, LazyLock, Mutex};

#[derive(Default)]
struct State {
    next: u64,
    queue: std::collections::VecDeque<u64>,
    running: bool,
}

static GATE: LazyLock<(Mutex<State>, Condvar)> =
    LazyLock::new(|| (Mutex::new(State::default()), Condvar::new()));

pub struct Permit;

pub fn acquire(cancelled: impl Fn() -> bool) -> Option<Permit> {
    let (lock, wake) = &*GATE;
    let mut state = lock.lock().unwrap();
    let ticket = state.next;
    state.next = state.next.wrapping_add(1);
    state.queue.push_back(ticket);
    loop {
        if cancelled() {
            state.queue.retain(|queued| *queued != ticket);
            wake.notify_all();
            return None;
        }
        if !state.running && state.queue.front() == Some(&ticket) {
            state.queue.pop_front();
            state.running = true;
            return Some(Permit);
        }
        state = wake
            .wait_timeout(state, std::time::Duration::from_millis(100))
            .unwrap()
            .0;
    }
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
}
