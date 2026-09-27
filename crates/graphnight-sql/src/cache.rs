use lru::LruCache;
use parking_lot::Mutex;
use serde_json::Value;
use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use graphnight_core::models::{AggregationType, FilterOperator, Query, TimeGranularity};



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
    /// Final response shape (`serde_json::Map`) so a cache hit is a single
    /// clone with no per-row conversion. The executor returns `HashMap`s and
    /// the REST/GraphQL layers consume `Map`s; caching the converted shape
    /// removes a `rows.len()` allocation + insert loop from the hot path.
    rows: Vec<serde_json::Map<String, Value>>,
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

    /// Fetch cached rows in their final `Map` form.
    pub fn get_maps(&self, key: u64) -> Option<Vec<serde_json::Map<String, Value>>> {
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

    pub fn insert_maps(&self, key: u64, rows: Vec<serde_json::Map<String, Value>>) {
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
    let mut hasher = FxHasher::new();
    for p in parts {
        hasher.feed_str(p);
        hasher.feed_u8(0);
    }
    hasher.digest()
}

/// Hash a pre-serialized byte payload (e.g. `canonical_query_bytes`).
pub fn hash_bytes(bytes: &[u8]) -> u64 {
    let mut hasher = FxHasher::new();
    hasher.feed_bytes(bytes);
    hasher.digest()
}

/// FNV-1a 64-bit hasher.
///
/// SipHash (the std default) is cryptographically strong but slow; for cache
/// keys we only need good distribution over a small, attacker-uncontrolled
/// domain. FNV-1a is ~2-3x faster to calculate and produces the same collision
/// profile for our key sizes. It is not a MAC and must not be used for
/// security-sensitive inputs.
///
/// The `Hasher` trait's `write_str`/`write` methods are unstable, so the
/// public API exposes inherent `feed_*`/`digest` methods and only relies
/// on the stable `write_u8`/`finish` implementations.
#[derive(Debug, Default)]
pub struct FxHasher(u64);

impl FxHasher {
    pub const fn new() -> Self {
        Self(0xcbf29ce484222325)
    }

    /// Push bytes into the running hash.
    pub fn feed_bytes(&mut self, bytes: &[u8]) {
        for b in bytes {
            self.feed_u8(*b);
        }
    }

    /// Push a string into the running hash.
    pub fn feed_str(&mut self, s: &str) {
        for b in s.bytes() {
            self.feed_u8(b);
        }
    }

    /// Push a single byte into the running hash.
    pub fn feed_u8(&mut self, i: u8) {
        self.0 ^= i as u64;
        self.0 = self.0.wrapping_mul(0x100000001b3);
    }

    /// Finalize and return the 64-bit digest.
    pub const fn digest(self) -> u64 {
        self.0
    }
}

impl std::hash::Hasher for FxHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write_u8(&mut self, i: u8) {
        self.feed_u8(i);
    }

    fn write(&mut self, bytes: &[u8]) {
        for b in bytes {
            self.feed_u8(*b);
        }
    }
}

/// Canonical, compact serialization of a `Query` for hashing.
///
/// `serde_json::to_string` allocates and formats with whitespace and field
/// names; on the hot path (every plan-cache lookup) that cost is pure waste.
/// This writes a delimiter-separated, field-ordered byte stream that is
/// stable for identical queries and ~5-10× cheaper to produce.
pub fn canonical_query_bytes(query: &Query) -> Vec<u8> {
    let mut buf = Vec::with_capacity(256);
    write_opt(&mut buf, query.name.as_deref());
    write_opt(&mut buf, query.source_model.as_ref().map(|s| s.model.as_str()));
    write_opt(
        &mut buf,
        query.source_model.as_ref().and_then(|s| s.alias.as_deref()),
    );
    write_opt(
        &mut buf,
        query.source_model.as_ref().and_then(|s| s.datasource.as_deref()),
    );
    for m in &query.measures {
        write_str(&mut buf, &m.formula.expression);
        write_opt(&mut buf, m.formula.label.as_deref());
        write_opt(&mut buf, m.formula.format.as_deref());
        write_str(&mut buf, agg_name(&m.aggregation));
    }
    for d in &query.dimensions {
        write_str(&mut buf, &d.name);
        write_opt(&mut buf, d.label.as_deref());
    }
    for td in &query.time_dimensions {
        write_str(&mut buf, &td.dimension);
        write_str(&mut buf, gran_name(&td.granularity));
        write_opt(&mut buf, td.label.as_deref());
    }
    for f in &query.filters {
        write_str(&mut buf, &f.field);
        write_str(&mut buf, op_name(&f.operator));
        write_str(&mut buf, &f.value.to_string());
        write_u8(&mut buf, f.or_condition as u8);
    }
    for o in &query.order {
        write_str(&mut buf, &o.field);
        write_u8(&mut buf, o.descending as u8);
    }
    write_opt(&mut buf, query.limit.map(|n| n.to_string()).as_deref());
    write_opt(&mut buf, query.offset.map(|n| n.to_string()).as_deref());
    write_opt(
        &mut buf,
        query.whole_periods_only.map(|b| b.to_string()).as_deref(),
    );
    write_opt(
        &mut buf,
        query.distinct_dimension_values.map(|b| b.to_string()).as_deref(),
    );
    write_opt(&mut buf, query.stage_ref.as_deref());
    buf
}

fn write_str(buf: &mut Vec<u8>, s: &str) {
    buf.extend_from_slice(s.as_bytes());
    buf.push(0x1f); // unit separator
}

fn write_opt(buf: &mut Vec<u8>, s: Option<&str>) {
    match s {
        Some(s) => {
            buf.extend_from_slice(s.as_bytes());
            buf.push(0x1f);
        }
        None => buf.push(0x1e), // record separator for absent
    }
}

fn write_u8(buf: &mut Vec<u8>, b: u8) {
    buf.push(b);
    buf.push(0x1f);
}

fn agg_name(a: &AggregationType) -> &'static str {
    match a {
        AggregationType::Sum => "sum",
        AggregationType::Avg => "avg",
        AggregationType::Count => "count",
        AggregationType::Min => "min",
        AggregationType::Max => "max",
        AggregationType::CountDistinct => "count_distinct",
        AggregationType::Custom(_) => "custom",
    }
}

fn gran_name(g: &TimeGranularity) -> &'static str {
    match g {
        TimeGranularity::Second => "second",
        TimeGranularity::Minute => "minute",
        TimeGranularity::Hour => "hour",
        TimeGranularity::Day => "day",
        TimeGranularity::Week => "week",
        TimeGranularity::Month => "month",
        TimeGranularity::Quarter => "quarter",
        TimeGranularity::Year => "year",
    }
}

fn op_name(o: &FilterOperator) -> &'static str {
    match o {
        FilterOperator::Eq => "eq",
        FilterOperator::Neq => "neq",
        FilterOperator::Gt => "gt",
        FilterOperator::Gte => "gte",
        FilterOperator::Lt => "lt",
        FilterOperator::Lte => "lte",
        FilterOperator::Like => "like",
        FilterOperator::ILike => "ilike",
        FilterOperator::In => "in",
        FilterOperator::NotIn => "not_in",
        FilterOperator::IsNull => "is_null",
        FilterOperator::IsNotNull => "is_not_null",
        FilterOperator::Between => "between",
        FilterOperator::NotBetween => "not_between",
    }
}
