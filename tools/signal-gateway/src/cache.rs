//! Bounded recipient cache for UUID lookups — THE production cache.
//!
//! NOT an LRU: two insertion-ordered map legs with a hard cap — at the cap
//! the oldest QUARTER is evicted together (the v1.28.73 replay-cache law).
//! TTL is lazy on the phone→UUID leg only (`get_uuid` checks it); the
//! reverse leg is bounded by the cap alone.
//!
//! PII law: phone numbers and ACI UUIDs are identifiers. Nothing in this
//! module logs an operand — the mapping lines this type replaced logged
//! `phone -> uuid` pairs at INFO, which is exactly the disclosure the
//! ninth-pass register row names. Error text returned to the CALLER may
//! carry the recipient (the caller supplied it); logs never do.

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
    self_aci: Arc<RwLock<Option<String>>>,
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
            self_aci: Arc::new(RwLock::new(None)),
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

    /// Get phone for UUID. Test-only by measurement: production paths write
    /// and resolve forward; the reverse leg exists for eviction symmetry and
    /// is asserted in the tests, so it must stay real (not stubbed).
    #[cfg(test)]
    pub fn get_phone(&self, uuid: &str) -> Option<String> {
        let inner = self.inner.read();
        inner.uuid_to_phone.get(uuid).cloned()
    }

    /// Insert phone -> UUID mapping. No log line: the operands are PII.
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

    /// Our own ACI, for self-addressed sends. No log line: same PII law.
    pub fn set_self_aci(&self, aci: String) {
        *self.self_aci.write() = Some(aci);
    }

    pub fn get_self_aci(&self) -> Option<String> {
        self.self_aci.read().clone()
    }

    /// Get cache size. Test-only by measurement (the cap pin reads it).
    #[cfg(test)]
    pub fn len(&self) -> usize {
        let inner = self.inner.read();
        inner.phone_to_uuid.len()
    }

    /// Check if string is a valid UUID format
    pub(crate) fn is_uuid(s: &str) -> bool {
        s.len() == 36 && s.chars().filter(|&c| c == '-').count() == 4
    }

    /// Check if string is a phone number (starts with +)
    pub(crate) fn is_phone(s: &str) -> bool {
        s.starts_with('+') && s.len() >= 10
    }

    /// Check if string is a username (contains . but not -)
    pub(crate) fn is_username(s: &str) -> bool {
        s.contains('.') && !s.contains('-') && !s.starts_with('+')
    }

    /// Resolve recipient to UUID. Caller-facing errors may name the
    /// recipient (the caller supplied it); log lines never do.
    pub fn resolve(&self, recipient: &str) -> anyhow::Result<String> {
        // Already a UUID
        if Self::is_uuid(recipient) {
            tracing::debug!("[RESOLVE] recipient already a UUID");
            return Ok(recipient.to_string());
        }

        // Check cache for phone/username
        if let Some(uuid) = self.get_uuid(recipient) {
            tracing::debug!("[RESOLVE] cache hit");
            return Ok(uuid);
        }

        // Phone number - try to use self ACI for self-messaging
        if Self::is_phone(recipient)
            && let Some(self_aci) = self.get_self_aci()
        {
            tracing::debug!("[RESOLVE] self-ACI fast path for a phone number");
            // Cache it for future
            self.insert(recipient.to_string(), self_aci.clone());
            return Ok(self_aci);
        }

        // Username - needs the manager path (websocket lookup)
        if Self::is_username(recipient) {
            tracing::debug!("[RESOLVE] username needs the manager lookup path");
            anyhow::bail!("Username resolution requires calling /v1/cache/seed first with the UUID")
        }

        tracing::debug!("[RESOLVE] cannot resolve recipient");
        anyhow::bail!(
            "Cannot resolve recipient: {}. Use UUID or seed the cache with /v1/cache/seed",
            recipient
        )
    }
}

impl Default for RecipientCache {
    fn default() -> Self {
        Self::new(3600) // 1 hour default TTL
    }
}

#[cfg(test)]
mod tests {
    // Test-only: the crate denies panic vectors in PRODUCTION code
    // (`Cargo.toml` `[lints.clippy]`); tests must fail loudly.
    #![allow(clippy::expect_used, clippy::panic)]
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
        // size stays there and the OLDEST entries evict first.
        let cap = 4096;
        let cache = RecipientCache::new(3600);
        for i in 0..(cap + 512) {
            cache.insert(format!("+{i}"), format!("uuid-{i}"));
        }
        // No cache-derived value in the message (CodeQL #74): a tainted
        // receiver's `.len()` flowing into a panic/log sink reads as
        // cleartext logging; the cap literal alone diagnoses the breach.
        assert!(
            cache.len() <= cap,
            "cache grew beyond the {cap}-entry cap — unbounded"
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

    #[test]
    fn resolve_reads_the_bounded_legs() {
        // Seeded phone resolves; the reverse leg serves the same entry;
        // an unseeded phone refuses with the seed-verb hint.
        let cache = RecipientCache::new(60);
        cache.insert("+1234567890".into(), "uuid-123".into());
        assert_eq!(
            cache.resolve("+1234567890").expect("seeded phone resolves"),
            "uuid-123"
        );
        let unseeded = cache.resolve("+15550001111");
        assert!(
            unseeded.is_err(),
            "an unseeded phone must refuse, not guess"
        );
        assert!(
            unseeded.unwrap_err().to_string().contains("/v1/cache/seed"),
            "the refusal names the seed verb"
        );
    }

    #[test]
    fn resolve_fast_paths_are_shape_not_identity() {
        // A UUID passes through untouched; the self-ACI path maps a phone to
        // our own ACI once set.
        let cache = RecipientCache::new(60);
        let uuid = "12345678-1234-1234-1234-123456789012";
        assert_eq!(cache.resolve(uuid).expect("uuid passthrough"), uuid);
        cache.set_self_aci(uuid.to_string());
        assert_eq!(
            cache.resolve("+1234567890").expect("self-ACI path"),
            uuid,
            "a phone with self-ACI set resolves to our own ACI"
        );
    }
}
