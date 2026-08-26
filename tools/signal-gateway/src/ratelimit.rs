//! Rate limiting middleware
#![allow(dead_code)]

use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Rate limiter for API requests
#[derive(Clone)]
pub struct RateLimiter {
    inner: Arc<RwLock<RateLimiterInner>>,
    max_requests: usize,
    window_secs: u64,
}

struct RateLimiterInner {
    ips: HashMap<String, Vec<Instant>>,
}

impl RateLimiter {
    /// Create new rate limiter
    pub fn new(max_requests: usize, window_secs: u64) -> Self {
        Self {
            inner: Arc::new(RwLock::new(RateLimiterInner {
                ips: HashMap::new(),
            })),
            max_requests,
            window_secs,
        }
    }

    /// Check if request from IP is allowed
    pub fn is_allowed(&self, ip: &str) -> bool {
        let mut inner = self.inner.write();
        // checked_sub: a window longer than time-since-boot saturates to
        // "nothing expires" instead of panicking on Instant underflow.
        // checked_sub: a window longer than time-since-boot saturates to
        // "nothing expires" instead of panicking on Instant underflow.
        let cutoff = Instant::now()
            .checked_sub(Duration::from_secs(self.window_secs))
            .unwrap_or_else(Instant::now);

        let requests = inner.ips.entry(ip.to_string()).or_default();
        requests.retain(|t| *t > cutoff);

        if requests.len() >= self.max_requests {
            return false;
        }

        requests.push(Instant::now());
        true
    }

    /// Get remaining requests for IP
    pub fn remaining(&self, ip: &str) -> usize {
        let inner = self.inner.read();
        // checked_sub: a window longer than time-since-boot saturates to
        // "nothing expires" instead of panicking on Instant underflow.
        // checked_sub: a window longer than time-since-boot saturates to
        // "nothing expires" instead of panicking on Instant underflow.
        let cutoff = Instant::now()
            .checked_sub(Duration::from_secs(self.window_secs))
            .unwrap_or_else(Instant::now);

        if let Some(requests) = inner.ips.get(ip) {
            let valid: Vec<_> = requests.iter().filter(|t| **t > cutoff).collect();
            self.max_requests.saturating_sub(valid.len())
        } else {
            self.max_requests
        }
    }

    /// Reset rate limit for IP (admin function)
    #[allow(dead_code)]
    pub fn reset(&self, ip: &str) {
        let mut inner = self.inner.write();
        inner.ips.remove(ip);
    }
}

/// Simple in-memory rate limiter for development
pub fn create_rate_limiter() -> RateLimiter {
    RateLimiter::new(100, 60) // 100 requests per minute
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rate_limit_allows() {
        let limiter = RateLimiter::new(5, 60);
        for _ in 0..5 {
            assert!(limiter.is_allowed("127.0.0.1"));
        }
    }

    #[test]
    fn test_rate_limit_blocks() {
        let limiter = RateLimiter::new(2, 60);
        assert!(limiter.is_allowed("127.0.0.1"));
        assert!(limiter.is_allowed("127.0.0.1"));
        assert!(!limiter.is_allowed("127.0.0.1"));
    }

    #[test]
    fn test_rate_limit_per_ip() {
        let limiter = RateLimiter::new(1, 60);
        assert!(limiter.is_allowed("127.0.0.1"));
        assert!(!limiter.is_allowed("127.0.0.1"));
        assert!(limiter.is_allowed("127.0.0.2")); // Different IP allowed
    }

    #[test]
    fn test_remaining() {
        let limiter = RateLimiter::new(5, 60);
        assert_eq!(limiter.remaining("127.0.0.1"), 5);
        limiter.is_allowed("127.0.0.1");
        assert_eq!(limiter.remaining("127.0.0.1"), 4);
    }
}
