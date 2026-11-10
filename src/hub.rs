use std::collections::HashMap;
use std::sync::Mutex;

use tokio::sync::mpsc;

struct Conn {
    id: u64,
    tx: mpsc::UnboundedSender<String>,
}

#[derive(Default)]
struct Inner {
    next: u64,
    users: HashMap<i64, Vec<Conn>>,
}

#[derive(Default)]
pub struct Hub {
    inner: Mutex<Inner>,
}

pub struct Subscription {
    pub id: u64,
    pub rx: mpsc::UnboundedReceiver<String>,
}

impl Hub {
    pub fn subscribe(&self, user_id: i64, limit: usize) -> Option<Subscription> {
        let mut inner = self.inner.lock().unwrap_or_else(|err| err.into_inner());
        let full = inner
            .users
            .get(&user_id)
            .is_some_and(|list| list.len() >= limit);
        if full {
            return None;
        }
        inner.next += 1;
        let id = inner.next;
        let (tx, rx) = mpsc::unbounded_channel();
        inner
            .users
            .entry(user_id)
            .or_default()
            .push(Conn { id, tx });
        Some(Subscription { id, rx })
    }

    pub fn unsubscribe(&self, user_id: i64, id: u64) {
        let mut inner = self.inner.lock().unwrap_or_else(|err| err.into_inner());
        if let Some(list) = inner.users.get_mut(&user_id) {
            list.retain(|conn| conn.id != id);
            if list.is_empty() {
                inner.users.remove(&user_id);
            }
        }
    }

    pub fn send_users(&self, user_ids: &[i64], message: &str) {
        let inner = self.inner.lock().unwrap_or_else(|err| err.into_inner());
        for user_id in user_ids {
            if let Some(list) = inner.users.get(user_id) {
                for conn in list {
                    let _ = conn.tx.send(message.to_string());
                }
            }
        }
    }

    pub fn is_online(&self, user_id: i64) -> bool {
        let inner = self.inner.lock().unwrap_or_else(|err| err.into_inner());
        inner
            .users
            .get(&user_id)
            .is_some_and(|list| !list.is_empty())
    }

    pub fn connection_count(&self) -> usize {
        let inner = self.inner.lock().unwrap_or_else(|err| err.into_inner());
        inner.users.values().map(|list| list.len()).sum()
    }
}
