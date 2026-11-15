use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Default)]
pub struct RateLimiter {
    inner: Mutex<HashMap<String, Vec<Instant>>>,
}

impl RateLimiter {
    pub fn allow(&self, key: &str, limit: usize, window: Duration) -> bool {
        let mut map = self.inner.lock().unwrap_or_else(|err| err.into_inner());
        if map.len() > 10_000 {
            map.retain(|_, hits| hits.iter().any(|hit| hit.elapsed() < window));
            if map.len() > 10_000 {
                map.clear();
            }
        }
        let hits = map.entry(key.to_string()).or_default();
        hits.retain(|hit| hit.elapsed() < window);
        if hits.len() >= limit {
            return false;
        }
        hits.push(Instant::now());
        true
    }
}
