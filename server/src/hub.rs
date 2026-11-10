use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use tokio::sync::mpsc;

const MAX_CONNECTIONS_PER_USER: usize = 5;

struct Conn {
    id: u64,
    tx: mpsc::UnboundedSender<String>,
}

#[derive(Default)]
pub struct Hub {
    inner: Mutex<HashMap<i64, Vec<Conn>>>,
    next_id: AtomicU64,
}

impl Hub {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn connect(&self, user_id: i64) -> Option<(u64, mpsc::UnboundedReceiver<String>)> {
        let mut guard = self.inner.lock().expect("连接表锁");
        let list = guard.entry(user_id).or_default();
        list.retain(|conn| !conn.tx.is_closed());
        if list.len() >= MAX_CONNECTIONS_PER_USER {
            return None;
        }
        let (tx, rx) = mpsc::unbounded_channel();
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        list.push(Conn { id, tx });
        Some((id, rx))
    }

    pub fn disconnect(&self, user_id: i64, id: u64) {
        let mut guard = self.inner.lock().expect("连接表锁");
        if let Some(list) = guard.get_mut(&user_id) {
            list.retain(|conn| conn.id != id);
            if list.is_empty() {
                guard.remove(&user_id);
            }
        }
    }

    pub fn emit(&self, user_ids: &[i64], payload: String) {
        let guard = self.inner.lock().expect("连接表锁");
        for user_id in user_ids {
            if let Some(list) = guard.get(user_id) {
                for conn in list {
                    let _ = conn.tx.send(payload.clone());
                }
            }
        }
    }
}
