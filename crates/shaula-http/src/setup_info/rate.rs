use sha2::{Digest, Sha256};
use std::{collections::HashMap, time::Instant};
use tokio::sync::Mutex;

struct Bucket {
    at: Instant,
    tokens: f64,
}
pub(super) struct RateLimit {
    entries: Mutex<HashMap<[u8; 32], Bucket>>,
    rate: u32,
    burst: u32,
}

impl RateLimit {
    pub(super) fn new(rate: u32, burst: u32) -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
            rate,
            burst,
        }
    }

    pub(super) async fn allow(&self, capability: &str) -> bool {
        let now = Instant::now();
        let key: [u8; 32] = Sha256::digest(capability.as_bytes()).into();
        let mut entries = self.entries.lock().await;
        entries.retain(|_, bucket| now.duration_since(bucket.at).as_secs() < 60);
        if entries.len() >= 16384 && !entries.contains_key(&key) {
            return false;
        }
        let bucket = entries.entry(key).or_insert(Bucket {
            at: now,
            tokens: f64::from(self.burst),
        });
        bucket.tokens = (bucket.tokens
            + now.duration_since(bucket.at).as_secs_f64() * f64::from(self.rate))
        .min(f64::from(self.burst));
        bucket.at = now;
        if bucket.tokens < 1.0 {
            return false;
        }
        bucket.tokens -= 1.0;
        true
    }
}
