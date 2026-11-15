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
        if limit == 0 {
            return None;
        }
        let mut inner = self.inner.lock().unwrap_or_else(|err| err.into_inner());
        inner.next += 1;
        let id = inner.next;
        let (tx, rx) = mpsc::unbounded_channel();
        let list = inner.users.entry(user_id).or_default();
        while list.len() >= limit {
            list.remove(0);
        }
        list.push(Conn { id, tx });
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
        let mut inner = self.inner.lock().unwrap_or_else(|err| err.into_inner());
        for user_id in user_ids {
            if let Some(list) = inner.users.get_mut(user_id) {
                list.retain(|conn| conn.tx.send(message.to_string()).is_ok());
            }
        }
        inner.users.retain(|_, list| !list.is_empty());
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

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc::error::TryRecvError;

    #[test]
    fn evicts_oldest_connection() {
        let hub = Hub::default();
        let mut oldest = hub.subscribe(7, 2).expect("连接");
        let mut kept = hub.subscribe(7, 2).expect("连接");
        let mut newest = hub.subscribe(7, 2).expect("连接");
        assert_eq!(hub.connection_count(), 2);
        assert_eq!(
            oldest.rx.try_recv().unwrap_err(),
            TryRecvError::Disconnected
        );
        assert_eq!(kept.rx.try_recv().unwrap_err(), TryRecvError::Empty);
        assert_eq!(newest.rx.try_recv().unwrap_err(), TryRecvError::Empty);
        hub.send_users(&[7], "hi");
        assert_eq!(kept.rx.try_recv().expect("保留的连接"), "hi");
        assert_eq!(newest.rx.try_recv().expect("新连接"), "hi");
    }
}
