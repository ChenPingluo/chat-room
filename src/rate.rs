use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub const WINDOW_MINUTE: Duration = Duration::from_secs(60);
pub const WINDOW_15MIN: Duration = Duration::from_secs(900);
pub const WINDOW_HOUR: Duration = Duration::from_secs(3600);

pub const LOGIN_USER: usize = 8;
pub const LOGIN_IP: usize = 30;
pub const REGISTER_IP: usize = 30;
pub const SEARCH: usize = 20;
pub const FRIEND: usize = 20;
pub const MSG: usize = 120;
pub const IMG: usize = 20;
pub const INVITE_CREATE: usize = 10;
pub const INVITE_REVOKE: usize = 20;
pub const API_IP: usize = 180;
pub const WS_IP: usize = 20;
pub const PASSWORD: usize = 6;
pub const TOTP_BEGIN: usize = 8;
pub const TOTP_CODE: usize = 10;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_up_to_limit_then_blocks() {
        let limiter = RateLimiter::default();
        assert!(limiter.allow("login:user:alice", LOGIN_USER, WINDOW_15MIN));
        for _ in 1..LOGIN_USER {
            assert!(limiter.allow("login:user:alice", LOGIN_USER, WINDOW_15MIN));
        }
        assert!(!limiter.allow("login:user:alice", LOGIN_USER, WINDOW_15MIN));
        assert!(
            limiter.allow("login:user:bob", LOGIN_USER, WINDOW_15MIN),
            "另一个用户不应被锁"
        );
        for _ in 0..LOGIN_IP {
            assert!(limiter.allow("login:ip:1.1.1.1", LOGIN_IP, WINDOW_15MIN));
        }
        assert!(!limiter.allow("login:ip:1.1.1.1", LOGIN_IP, WINDOW_15MIN));
        assert!(limiter.allow("login:ip:2.2.2.2", LOGIN_IP, WINDOW_15MIN));
    }

    #[test]
    fn api_and_invite_caps_leave_room_for_normal_use() {
        let limiter = RateLimiter::default();
        for _ in 0..8 {
            assert!(limiter.allow("api:ip:local", API_IP, WINDOW_MINUTE));
        }
        assert!(limiter.allow("invite:create:7", INVITE_CREATE, WINDOW_15MIN));
        assert!(limiter.allow("invite:create:7", INVITE_CREATE, WINDOW_15MIN));
        for _ in 2..INVITE_CREATE {
            assert!(limiter.allow("invite:create:7", INVITE_CREATE, WINDOW_15MIN));
        }
        assert!(!limiter.allow("invite:create:7", INVITE_CREATE, WINDOW_15MIN));
        for _ in 0..API_IP {
            assert!(limiter.allow("api:ip:flood", API_IP, WINDOW_MINUTE));
        }
        assert!(!limiter.allow("api:ip:flood", API_IP, WINDOW_MINUTE));
        assert!(limiter.allow("api:ip:neighbor", API_IP, WINDOW_MINUTE));
    }
}
