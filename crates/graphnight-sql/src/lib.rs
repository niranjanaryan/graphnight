pub mod cache;
pub mod dialects;
pub mod executor;
pub mod generator;
pub mod introspection;
pub mod metrics;

use anyhow::Result;
use cache::{FxHasher, PlanCache, ResultCache};
use graphnight_core::models::{DataSource, Model, Query};
use metrics::QueryMetrics;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

/// High-level SQL interface combining generator, caches, and executor.
pub struct SqlEngine {
    generator: generator::SqlGenerator,
    executor: Arc<executor::QueryExecutor>,
    plan_cache: PlanCache,
    result_cache: ResultCache,
    metrics: Arc<QueryMetrics>,
    /// Optional policy fingerprint included in cache keys (tenant/user scope).
    policy_fingerprint: String,
}

impl SqlEngine {
    pub fn new(
        dialect: Box<dyn dialects::Dialect>,
        executor: Arc<executor::QueryExecutor>,
    ) -> Result<Self> {
        Ok(Self {
            generator: generator::SqlGenerator::new(dialect),
            executor,
            plan_cache: PlanCache::new(512),
            result_cache: ResultCache::new(256, Duration::from_secs(60)),
            metrics: QueryMetrics::new(),
            policy_fingerprint: String::new(),
        })
    }

    pub fn with_models(mut self, models: Vec<Model>) -> Self {
        self.generator = self.generator.with_models(models);
        self
    }

    /// Register datasources so each model generates SQL for its own driver.
    pub fn with_datasources(mut self, datasources: Vec<DataSource>) -> Self {
        self.generator = self.generator.with_datasources(datasources);
        self
    }

    /// Dialect that will be used for `model_name` (driver of its datasource).
    pub fn dialect_name_for_model(&self, model_name: &str) -> String {
        self.generator
            .dialect_for_model(model_name)
            .name()
            .to_string()
    }

    /// Register or replace a model in the SQL generator registry.
    /// Register or replace a model at runtime.
    ///
    /// Takes `&self`: the service holds the engine behind an `Arc`, and a model
    /// created through the admin API after startup still has to become
    /// queryable without rebuilding the engine.
    pub fn register_model(&self, model: Model) {
        self.generator.register_model(model);
    }

    /// Drop a model from the in-memory registry.
    pub fn unregister_model(&self, name: &str) {
        self.generator.unregister_model(name);
    }

    pub fn with_metrics(mut self, metrics: Arc<QueryMetrics>) -> Self {
        self.metrics = metrics;
        self
    }

    pub fn with_policy_fingerprint(mut self, fingerprint: impl Into<String>) -> Self {
        self.policy_fingerprint = fingerprint.into();
        self
    }

    /// Get a reference to the query executor for introspection
    pub fn executor(&self) -> Arc<executor::QueryExecutor> {
        self.executor.clone()
    }

    pub fn metrics(&self) -> Arc<QueryMetrics> {
        self.metrics.clone()
    }

    pub fn invalidate_result_cache(&self) {
        self.result_cache.invalidate_all();
    }

    /// Generate SQL for a query (plan-cached).
    ///
    /// Traced as `sql.plan`, the first leg of plan → SQL → execute.
    pub fn generate_sql(&self, query: &Query) -> Result<String> {
        // `info`, not `debug`: the launch criterion is a trace for
        // plan -> SQL -> execute, and a plan span that is invisible at the
        // default level would only show up in a debugging session.
        let span = tracing::info_span!(
            "sql.plan",
            otel.name = %format!("plan {}", query_model_name(query)),
            // The query text can carry literal filter values, so it is never
            // recorded. A digest is enough to correlate a trace with a plan
            // cache entry without putting user data in the span.
            query.hash = %query_fingerprint(query),
            db.system = tracing::field::Empty,
            db.statement = tracing::field::Empty,
            cache.hit = tracing::field::Empty,
        );
        let _guard = span.enter();

        let query_json = serde_json::to_string(query)?;
        let dialect_name = self.generator.resolved_dialect_name(query);
        // Fold dialect + policy scope into the key payload so a single hash
        // covers all three dimensions (same semantics as the old `hash_key(&[
        // dialect_name, policy_fingerprint, query_json])` triple).
        let key = {
            let mut hasher = cache::FxHasher::new();
            cache::FxHasher::feed_str(&mut hasher, &dialect_name);
            cache::FxHasher::feed_str(&mut hasher, &self.policy_fingerprint);
            hasher.feed_str(&query_json);
            hasher.digest()
        };
        if let Some(sql) = self.plan_cache.get(key) {
            self.metrics
                .plan_cache_hits
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            tracing::Span::current().record("db.system", dialect_name.as_str());
            tracing::Span::current().record("cache.hit", true);
            return Ok(sql);
        }
        self.metrics
            .plan_cache_misses
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let sql = self.generator.generate(query)?;
        self.plan_cache.insert(key, sql.clone());
        span.record("db.statement", sql.as_str());
        span.record("db.system", dialect_name.as_str());
        span.record("cache.hit", false);
        Ok(sql)
    }

    /// Execute SQL with optional result caching. Uses row-streaming fetch internally.
    ///
    /// Traced as `sql.execute`, the last leg of plan → SQL → execute.
    pub async fn execute_sqlx(
        &self,
        ds: &DataSource,
        sql: &str,
    ) -> Result<Vec<serde_json::Map<String, Value>>> {
        let span = tracing::info_span!(
            // No `otel.name` override: the span name stays `sql.execute` so it
            // is greppable and stable across deployments. Which datasource ran
            // is the `db.name` attribute's job.
            "sql.execute",
            db.system = %ds.driver,
            db.name = %ds.name,
            // Statements can contain literal values from filters. Record only a
            // digest so a trace never becomes a data leak.
            db.statement.hash = %short_hash(sql),
            db.rows = tracing::field::Empty,
            cache.hit = tracing::field::Empty,
            error = tracing::field::Empty,
        );
        let _guard = span.enter();

        self.metrics.inc_queries();
        let key = {
            let mut hasher = cache::FxHasher::new();
            cache::FxHasher::feed_str(&mut hasher, &ds.name);
            cache::FxHasher::feed_str(&mut hasher, &self.policy_fingerprint);
            hasher.feed_str(sql);
            hasher.digest()
        };
        // Cache the final `serde_json::Map` shape so a hit is a single clone —
        // the executor returns `HashMap`s and every consumer wants `Map`s, so
        // the per-row `into_iter().collect()` conversion is done once on miss
        // and skipped entirely on hit.
        if let Some(rows) = self.result_cache.get_maps(key) {
            self.metrics
                .result_cache_hits
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            self.metrics.add_rows(rows.len() as u64);
            span.record("cache.hit", true);
            span.record("db.rows", rows.len());
            return Ok(rows);
        }
        self.metrics
            .result_cache_misses
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        match self.executor.execute_streaming_collect(ds, sql).await {
            Ok(rows) => {
                let n = rows.len();
                self.metrics.add_rows(n as u64);
                let maps: Vec<serde_json::Map<String, Value>> = rows
                    .into_iter()
                    .map(|m| m.into_iter().collect())
                    .collect();
                self.result_cache.insert_maps(key, maps.clone());
                span.record("cache.hit", false);
                span.record("db.rows", n);
                Ok(maps)
            }
            Err(e) => {
                self.metrics.inc_errors();
                span.record("error", e.to_string().as_str());
                Err(e)
            }
        }
    }

    /// Stream rows without buffering the full result in the executor layer.
    pub fn execute_stream<'a>(
        &'a self,
        ds: &'a DataSource,
        sql: String,
    ) -> impl futures::Stream<Item = Result<HashMap<String, Value>>> + 'a {
        self.executor.execute_stream(ds, sql)
    }
}

/// The model a query targets, for span naming.
fn query_model_name(query: &Query) -> &str {
    query
        .name
        .as_deref()
        .or_else(|| query.source_model.as_ref().map(|s| s.model.as_str()))
        .unwrap_or("<unnamed>")
}

/// Stable digest of a query, used to correlate traces without recording it.
fn query_fingerprint(query: &Query) -> String {
    let bytes = cache::canonical_query_bytes(query);
    let mut hasher = FxHasher::new();
    hasher.feed_bytes(&bytes);
    format!("{:016x}", hasher.digest())
}

/// Short, stable digest of a string.
///
/// `FxHasher` is not cryptographically strong, which is acceptable: this
/// identifies a statement within one deployment's traces, it is not a
/// persisted key or a security boundary.
fn short_hash(input: &str) -> String {
    let mut hasher = FxHasher::new();
    hasher.feed_str(input);
    format!("{:016x}", hasher.digest())
}

pub use introspection::{
    infer_model_from_table, ColumnInfo, ForeignKeyInfo, IntrospectionConfig, SchemaIntrospector,
    TableInfo,
};
