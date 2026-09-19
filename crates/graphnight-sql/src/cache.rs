use lru::LruCache;
use parking_lot::Mutex;
use serde_json::Value;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// In-memory LRU cache for generated SQL plans.
pub struct PlanCache {
    inner: Mutex<LruCache<u64, String>>,
    hits: AtomicU64,
    misses: AtomicU64,
}

impl PlanCache {
    pub fn new(capacity: usize) -> Self {
        let capacity = NonZeroUsize::new(capacity.max(1)).unwrap();
        Self {
            inner: Mutex::new(LruCache::new(capacity)),
            hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
        }
    }

    pub fn get(&self, key: u64) -> Option<String> {
        let mut guard = self.inner.lock();
        if let Some(sql) = guard.get(&key) {
            self.hits.fetch_add(1, Ordering::Relaxed);
            Some(sql.clone())
        } else {
            self.misses.fetch_add(1, Ordering::Relaxed);
            None
        }
    }

    pub fn insert(&self, key: u64, sql: String) {
        self.inner.lock().put(key, sql);
    }

    pub fn hits(&self) -> u64 {
        self.hits.load(Ordering::Relaxed)
    }

    pub fn misses(&self) -> u64 {
        self.misses.load(Ordering::Relaxed)
    }
}

#[derive(Clone)]
struct ResultEntry {
    rows: Vec<HashMap<String, Value>>,
    inserted_at: Instant,
}

/// In-memory TTL cache for query result sets.
pub struct ResultCache {
    inner: Mutex<LruCache<u64, ResultEntry>>,
    ttl: Duration,
    hits: AtomicU64,
    misses: AtomicU64,
}

impl ResultCache {
    pub fn new(capacity: usize, ttl: Duration) -> Self {
        let capacity = NonZeroUsize::new(capacity.max(1)).unwrap();
        Self {
            inner: Mutex::new(LruCache::new(capacity)),
            ttl,
            hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
        }
    }

    pub fn get(&self, key: u64) -> Option<Vec<HashMap<String, Value>>> {
        let mut guard = self.inner.lock();
        if let Some(entry) = guard.get(&key) {
            if entry.inserted_at.elapsed() <= self.ttl {
                self.hits.fetch_add(1, Ordering::Relaxed);
                return Some(entry.rows.clone());
            }
            guard.pop(&key);
        }
        self.misses.fetch_add(1, Ordering::Relaxed);
        None
    }

    pub fn insert(&self, key: u64, rows: Vec<HashMap<String, Value>>) {
        self.inner.lock().put(
            key,
            ResultEntry {
                rows,
                inserted_at: Instant::now(),
            },
        );
    }

    pub fn invalidate_all(&self) {
        self.inner.lock().clear();
    }

    pub fn hits(&self) -> u64 {
        self.hits.load(Ordering::Relaxed)
    }

    pub fn misses(&self) -> u64 {
        self.misses.load(Ordering::Relaxed)
    }
}

/// Stable hash for cache keys.
pub fn hash_key(parts: &[&str]) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for p in parts {
        p.hash(&mut hasher);
        0u8.hash(&mut hasher);
    }
    hasher.finish()
}
