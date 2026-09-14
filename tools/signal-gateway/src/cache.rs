//! Bounded recipient cache for UUID lookups.
//!
//! NOT an LRU: two insertion-ordered map legs with a hard cap — at the cap
//! the oldest QUARTER is evicted together (the v1.28.73 replay-cache law).
//! TTL is lazy on the phone→UUID leg only (`get_uuid` checks it); the
//! reverse leg is bounded by the cap alone.
#![allow(dead_code)]

use parking_lot::RwLock;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Hard cap on cached entries. Matches the server's replay-cache convention
/// (`CAP_REPLAY_CAP`): at the cap, evict the OLDEST QUARTER, never clear-all.
const RECIPIENT_CACHE_CAP: usize = 4096;

/// Cache for recipient phone -> UUID mappings
#[derive(Clone)]
pub struct RecipientCache {
    inner: Arc<RwLock<RecipientCacheInner>>,
    ttl_secs: u64,
}

struct RecipientCacheInner {
    phone_to_uuid: HashMap<String, (String, Instant)>,
    uuid_to_phone: HashMap<String, String>,
    /// Insertion order of phone keys — the eviction queue (front = oldest).
    order: VecDeque<String>,
}

impl RecipientCache {
    /// Create new cache with TTL in seconds
    pub fn new(ttl_secs: u64) -> Self {
        Self {
            inner: Arc::new(RwLock::new(RecipientCacheInner {
                phone_to_uuid: HashMap::new(),
                uuid_to_phone: HashMap::new(),
                order: VecDeque::new(),
            })),
            ttl_secs,
        }
    }

    /// Get UUID for phone number
    pub fn get_uuid(&self, phone: &str) -> Option<String> {
        let inner = self.inner.read();
        inner.phone_to_uuid.get(phone).and_then(|(uuid, time)| {
            if time.elapsed() < Duration::from_secs(self.ttl_secs) {
                Some(uuid.clone())
            } else {
                None
            }
        })
    }

    /// Get phone for UUID
    pub fn get_phone(&self, uuid: &str) -> Option<String> {
        let inner = self.inner.read();
        inner.uuid_to_phone.get(uuid).cloned()
    }

    /// Insert phone -> UUID mapping
    pub fn insert(&self, phone: String, uuid: String) {
        let mut inner = self.inner.write();
        if !inner.phone_to_uuid.contains_key(&phone) {
            inner.order.push_back(phone.clone());
        }
        inner
            .phone_to_uuid
            .insert(phone.clone(), (uuid.clone(), Instant::now()));
        inner.uuid_to_phone.insert(uuid, phone);
        if inner.phone_to_uuid.len() > RECIPIENT_CACHE_CAP {
            // Evict the OLDEST QUARTER (insertion order — drain the front);
            // both legs shrink together, 1:1 by construction.
            let evict = inner.phone_to_uuid.len() / 4;
            for _ in 0..evict {
                let Some(oldest) = inner.order.pop_front() else {
                    break;
                };
                if let Some((uuid, _)) = inner.phone_to_uuid.remove(&oldest) {
                    inner.uuid_to_phone.remove(&uuid);
                }
            }
        }
    }

    /// Clear all cached entries
    #[allow(dead_code)]
    pub fn clear(&self) {
        let mut inner = self.inner.write();
        inner.phone_to_uuid.clear();
        inner.uuid_to_phone.clear();
    }

    /// Get cache size
    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        let inner = self.inner.read();
        inner.phone_to_uuid.len()
    }
}

impl Default for RecipientCache {
    fn default() -> Self {
        Self::new(3600) // 1 hour default TTL
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_insert_get() {
        let cache = RecipientCache::new(60);
        cache.insert("+1234567890".into(), "uuid-123".into());
        assert_eq!(cache.get_uuid("+1234567890"), Some("uuid-123".into()));
    }

    #[test]
    fn test_cache_reverse_lookup() {
        let cache = RecipientCache::new(60);
        cache.insert("+1234567890".into(), "uuid-123".into());
        assert_eq!(cache.get_phone("uuid-123"), Some("+1234567890".into()));
    }

    #[test]
    fn test_cache_miss() {
        let cache = RecipientCache::new(60);
        assert_eq!(cache.get_uuid("+0000000000"), None);
    }

    #[test]
    fn signal_gateway_cache_is_bounded() {
        // The contract: at the cap (4,096 — the replay-cache convention),
        // size stays there and the OLDEST entries evict first. Fails today:
        // the cache is two plain HashMaps with no cap and no eviction.
        let cap = 4096;
        let cache = RecipientCache::new(3600);
        for i in 0..(cap + 512) {
            cache.insert(format!("+{i}"), format!("uuid-{i}"));
        }
        assert!(
            cache.len() <= cap,
            "cache grew to {} entries — unbounded",
            cache.len()
        );
        // Oldest evicted first (insertion order, not arbitrary).
        assert_eq!(cache.get_uuid("+0"), None, "oldest entry must evict first");
        assert_eq!(
            cache.get_phone("uuid-0"),
            None,
            "reverse map must evict with the forward map"
        );
        // Recent entries survive the quarter-drain.
        let survivor = cap + 511;
        assert_eq!(
            cache.get_uuid(&format!("+{survivor}")),
            Some(format!("uuid-{survivor}"))
        );
        assert_eq!(
            cache.get_phone(&format!("uuid-{survivor}")),
            Some(format!("+{survivor}"))
        );
    }
}
