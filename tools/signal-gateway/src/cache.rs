//! LRU Cache for recipient UUID lookups
#![allow(dead_code)]

use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Cache for recipient phone -> UUID mappings
#[derive(Clone)]
pub struct RecipientCache {
    inner: Arc<RwLock<RecipientCacheInner>>,
    ttl_secs: u64,
}

struct RecipientCacheInner {
    phone_to_uuid: HashMap<String, (String, Instant)>,
    uuid_to_phone: HashMap<String, String>,
}

impl RecipientCache {
    /// Create new cache with TTL in seconds
    pub fn new(ttl_secs: u64) -> Self {
        Self {
            inner: Arc::new(RwLock::new(RecipientCacheInner {
                phone_to_uuid: HashMap::new(),
                uuid_to_phone: HashMap::new(),
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
        inner
            .phone_to_uuid
            .insert(phone.clone(), (uuid.clone(), Instant::now()));
        inner.uuid_to_phone.insert(uuid, phone);
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
}
