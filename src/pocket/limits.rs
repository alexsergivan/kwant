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
/// Queue tickets (pa11b): 3 takes an hour per guest key, and 30 an hour per address, since a
/// shop's Wi-Fi puts every customer behind one (a cleared cookie is a new key, so the address
/// bounds that). Only a take that gave a ticket counts.
pub const TAKES_PER_KEY_HOUR: f64 = 3.0;
pub const TAKES_PER_IP_HOUR: f64 = 30.0;
/// Past this many tracked keys, full buckets are dropped.
const PRUNE_AT: usize = 10_000;

#[derive(Default)]
pub struct Limits {
    actions: Buckets<(String, String)>,
    signins: Buckets<IpAddr>,
    photos: Buckets<(String, String)>,
    takes: Buckets<(String, String)>,
    take_ips: Buckets<(String, IpAddr)>,
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

    /// `true` when `player` of queue `code`, from `ip`, may take a ticket now. Spends nothing:
    /// [`Self::took`] counts a take that gave a ticket.
    pub fn may_take(&self, code: &str, player: &str, ip: IpAddr) -> bool {
        let now = Instant::now();
        self.takes.has((code.to_string(), player.to_string()), TAKES_PER_KEY_HOUR, TAKES_PER_KEY_HOUR / 3600.0, now)
            && self.take_ips.has((code.to_string(), ip), TAKES_PER_IP_HOUR, TAKES_PER_IP_HOUR / 3600.0, now)
    }

    /// Counts one ticket taken by `player` of `code` from `ip`.
    pub fn took(&self, code: &str, player: &str, ip: IpAddr) {
        let now = Instant::now();
        self.takes.take((code.to_string(), player.to_string()), TAKES_PER_KEY_HOUR, TAKES_PER_KEY_HOUR / 3600.0, now);
        self.take_ips.take((code.to_string(), ip), TAKES_PER_IP_HOUR, TAKES_PER_IP_HOUR / 3600.0, now);
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
    /// `true` when `key` has a token now, without spending it.
    fn has(&self, key: K, capacity: f64, per_sec: f64, now: Instant) -> bool {
        let map = self.map.lock().unwrap_or_else(|e| e.into_inner());
        map.get(&key).is_none_or(|b| (b.tokens + now.duration_since(b.at).as_secs_f64() * per_sec).min(capacity) >= 1.0)
    }

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

    #[test]
    fn has_spends_nothing() {
        let b: Buckets<u8> = Buckets::default();
        let t = Instant::now();
        assert!(b.has(1, 3.0, 3.0 / 3600.0, t), "a new key has the full burst");
        assert!((0..3).all(|_| b.has(1, 3.0, 3.0 / 3600.0, t) && b.take(1, 3.0, 3.0 / 3600.0, t)));
        assert!(!b.has(1, 3.0, 3.0 / 3600.0, t), "a 4th take in the hour waits");
        assert!(b.has(1, 3.0, 3.0 / 3600.0, t + Duration::from_secs(1200)), "one back after 20 minutes");
    }

    #[test]
    fn queue_takes_per_key_and_per_address() {
        let l = Limits::default();
        let ip = IpAddr::from([10, 0, 0, 1]);
        for _ in 0..3 {
            assert!(l.may_take("Q", "a", ip));
            l.took("Q", "a", ip);
        }
        assert!(!l.may_take("Q", "a", ip), "3 takes an hour per key");
        assert!(l.may_take("Q", "b", ip), "another key on the same Wi-Fi");
        assert!(l.may_take("R", "a", ip), "per pocket");
        for i in 0..27 {
            l.took("Q", &format!("k{i}"), ip);
        }
        assert!(!l.may_take("Q", "fresh", ip), "30 an hour per address");
        assert!(l.may_take("Q", "fresh", IpAddr::from([10, 0, 0, 2])));
    }
}
