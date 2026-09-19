//! In-process token-bucket rate limit keyed by API key or client IP.
//!
//! Enabled when `GRAPHNIGHT_RATE_LIMIT_RPS` is a positive number. `0` or unset disables.

use axum::{
    extract::{ConnectInfo, Request, State},
    http::{HeaderMap, HeaderValue, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Mutex;
use std::time::Instant;
use tracing::{info, warn};

use crate::auth_config::extract_api_key;

/// Shared limiter state (cheap to clone; buckets live behind the mutex).
#[derive(Clone)]
pub struct RateLimitState {
    /// Tokens added per second (and bucket capacity).
    rps: f64,
    buckets: std::sync::Arc<Mutex<HashMap<String, Bucket>>>,
}

struct Bucket {
    tokens: f64,
    last_refill: Instant,
}

impl RateLimitState {
    /// Load from `GRAPHNIGHT_RATE_LIMIT_RPS`. Returns `None` when disabled.
    pub fn from_env() -> Option<Self> {
        let raw = std::env::var("GRAPHNIGHT_RATE_LIMIT_RPS").ok()?;
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed == "0" {
            return None;
        }
        match trimmed.parse::<f64>() {
            Ok(rps) if rps > 0.0 && rps.is_finite() => {
                info!(rps, "In-process rate limiting enabled (per API key or IP)");
                Some(Self {
                    rps,
                    buckets: std::sync::Arc::new(Mutex::new(HashMap::new())),
                })
            }
            Ok(_) => {
                warn!(
                    "GRAPHNIGHT_RATE_LIMIT_RPS={raw} is not a positive rate; rate limiting disabled"
                );
                None
            }
            Err(_) => {
                warn!("GRAPHNIGHT_RATE_LIMIT_RPS={raw} is not a number; rate limiting disabled");
                None
            }
        }
    }

    /// Try to take one token for `key`. Returns `Ok(())` or seconds until retry.
    fn try_acquire(&self, key: &str) -> Result<(), u64> {
        let mut map = self.buckets.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        let bucket = map.entry(key.to_string()).or_insert_with(|| Bucket {
            tokens: self.rps,
            last_refill: now,
        });

        let elapsed = now.duration_since(bucket.last_refill).as_secs_f64();
        bucket.tokens = (bucket.tokens + elapsed * self.rps).min(self.rps);
        bucket.last_refill = now;

        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            Ok(())
        } else {
            let need = 1.0 - bucket.tokens;
            let wait_secs = (need / self.rps).ceil().max(1.0) as u64;
            Err(wait_secs)
        }
    }
}

fn client_key(headers: &HeaderMap, peer: Option<SocketAddr>) -> String {
    if let Some(api_key) = extract_api_key(headers) {
        // Avoid storing raw secrets as map keys; hash is enough for bucketing.
        return format!("key:{}", simple_hash(&api_key));
    }
    if let Some(fwd) = headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.split(',').next())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return format!("ip:{fwd}");
    }
    match peer {
        Some(addr) => format!("ip:{}", addr.ip()),
        None => "ip:unknown".to_string(),
    }
}

fn simple_hash(s: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// Axum middleware (`from_fn_with_state`). No-op when state is `None`.
pub async fn rate_limit_middleware(
    State(state): State<Option<RateLimitState>>,
    request: Request,
    next: Next,
) -> Response {
    let Some(limiter) = state else {
        return next.run(request).await;
    };

    // Keep probes usable under load / scrapes.
    let path = request.uri().path();
    if path == "/health" || path == "/metrics" {
        return next.run(request).await;
    }

    let peer = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|c| c.0);
    let key = client_key(request.headers(), peer);

    match limiter.try_acquire(&key) {
        Ok(()) => next.run(request).await,
        Err(retry_after) => {
            let mut res = (
                StatusCode::TOO_MANY_REQUESTS,
                axum::Json(serde_json::json!({
                    "error": "rate_limit_exceeded",
                    "message": "Too many requests; slow down and retry",
                })),
            )
                .into_response();
            if let Ok(v) = HeaderValue::from_str(&retry_after.to_string()) {
                res.headers_mut().insert("retry-after", v);
            }
            res
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bucket_allows_then_rejects() {
        let state = RateLimitState {
            rps: 2.0,
            buckets: std::sync::Arc::new(Mutex::new(HashMap::new())),
        };
        assert!(state.try_acquire("a").is_ok());
        assert!(state.try_acquire("a").is_ok());
        assert!(state.try_acquire("a").is_err());
        // Different key is independent.
        assert!(state.try_acquire("b").is_ok());
    }

    #[test]
    fn from_env_disabled_on_zero() {
        std::env::set_var("GRAPHNIGHT_RATE_LIMIT_RPS", "0");
        assert!(RateLimitState::from_env().is_none());
        std::env::remove_var("GRAPHNIGHT_RATE_LIMIT_RPS");
        assert!(RateLimitState::from_env().is_none());
    }

    #[test]
    fn from_env_parses_positive() {
        std::env::set_var("GRAPHNIGHT_RATE_LIMIT_RPS", "10");
        let s = RateLimitState::from_env().unwrap();
        assert!((s.rps - 10.0).abs() < f64::EPSILON);
        std::env::remove_var("GRAPHNIGHT_RATE_LIMIT_RPS");
    }
}
