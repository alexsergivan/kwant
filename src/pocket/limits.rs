//! Rate limits for guests, held in memory only: no IP address is ever written anywhere (not the
//! database, not a log line). Token buckets: a burst up to `capacity`, refilled at `per_sec`.

use std::collections::HashMap;
use std::hash::Hash;
use std::net::IpAddr;
use std::sync::Mutex;
use std::time::Instant;

/// Actions one guest may send: 10 a second.
pub const ACTIONS_PER_SEC: f64 = 10.0;
/// Join, rejoin and reset-code tries from one address: a burst of 30, then 2 a second. A pub's
/// Wi-Fi puts everyone behind one address, so this is loose; PIN guessing is stopped per name.
pub const SIGNIN_BURST: f64 = 30.0;
pub const SIGNIN_PER_SEC: f64 = 2.0;
/// Wall photos one guest may send (pa14): a burst of 10, then one every 3 seconds.
pub const PHOTO_BURST: f64 = 10.0;
pub const PHOTOS_PER_SEC: f64 = 1.0 / 3.0;
/// Past this many tracked keys, full buckets are dropped.
const PRUNE_AT: usize = 10_000;

#[derive(Default)]
pub struct Limits {
    actions: Buckets<(String, String)>,
    signins: Buckets<IpAddr>,
    photos: Buckets<(String, String)>,
}

impl Limits {
    /// One action by `player` of `code`; `false` past 10 a second.
    pub fn action(&self, code: &str, player: &str) -> bool {
        self.actions.take((code.to_string(), player.to_string()), ACTIONS_PER_SEC, ACTIONS_PER_SEC, Instant::now())
    }

    /// One Wall photo by `player` of `code`; `false` past the burst until the bucket refills.
    pub fn photo(&self, code: &str, player: &str) -> bool {
        self.photos.take((code.to_string(), player.to_string()), PHOTO_BURST, PHOTOS_PER_SEC, Instant::now())
    }

    /// One join, rejoin or reset-code try from `ip`.
    pub fn signin(&self, ip: IpAddr) -> bool {
        self.signins.take(ip, SIGNIN_BURST, SIGNIN_PER_SEC, Instant::now())
    }
}

struct Bucket {
    tokens: f64,
    at: Instant,
}

struct Buckets<K> {
    map: Mutex<HashMap<K, Bucket>>,
}

impl<K> Default for Buckets<K> {
    fn default() -> Self {
        Self { map: Mutex::new(HashMap::new()) }
    }
}

impl<K: Eq + Hash> Buckets<K> {
    fn take(&self, key: K, capacity: f64, per_sec: f64, now: Instant) -> bool {
        let mut map = self.map.lock().unwrap_or_else(|e| e.into_inner());
        if map.len() > PRUNE_AT {
            map.retain(|_, b| b.tokens + now.duration_since(b.at).as_secs_f64() * per_sec < capacity);
        }
        let b = map.entry(key).or_insert(Bucket { tokens: capacity, at: now });
        b.tokens = (b.tokens + now.duration_since(b.at).as_secs_f64() * per_sec).min(capacity);
        b.at = now;
        if b.tokens >= 1.0 {
            b.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn buckets_refill() {
        let b: Buckets<u8> = Buckets::default();
        let t = Instant::now();
        assert!((0..10).all(|_| b.take(1, 10.0, 10.0, t)));
        assert!(!b.take(1, 10.0, 10.0, t), "the 11th in the same instant waits");
        assert!(b.take(2, 10.0, 10.0, t), "per key");
        assert!(b.take(1, 10.0, 10.0, t + Duration::from_millis(100)), "one back after 100 ms");
        assert!(!b.take(1, 10.0, 10.0, t + Duration::from_millis(100)));
    }
}
