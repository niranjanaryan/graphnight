pub mod cache;
pub mod dialects;
pub mod executor;
pub mod generator;
pub mod metrics;

use anyhow::Result;
use cache::{hash_key, PlanCache, ResultCache};
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

    pub fn with_metrics(mut self, metrics: Arc<QueryMetrics>) -> Self {
        self.metrics = metrics;
        self
    }

    pub fn with_policy_fingerprint(mut self, fingerprint: impl Into<String>) -> Self {
        self.policy_fingerprint = fingerprint.into();
        self
    }

    pub fn metrics(&self) -> Arc<QueryMetrics> {
        self.metrics.clone()
    }

    pub fn invalidate_result_cache(&self) {
        self.result_cache.invalidate_all();
    }

    /// Generate SQL for a query (plan-cached).
    pub fn generate_sql(&self, query: &Query) -> Result<String> {
        let query_json = serde_json::to_string(query)?;
        let key = hash_key(&[
            self.generator.dialect_name(),
            &self.policy_fingerprint,
            &query_json,
        ]);
        if let Some(sql) = self.plan_cache.get(key) {
            self.metrics
                .plan_cache_hits
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            return Ok(sql);
        }
        self.metrics
            .plan_cache_misses
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let sql = self.generator.generate(query)?;
        self.plan_cache.insert(key, sql.clone());
        Ok(sql)
    }

    /// Execute SQL with optional result caching. Uses row-streaming fetch internally.
    pub async fn execute_sqlx(
        &self,
        ds: &DataSource,
        sql: &str,
    ) -> Result<Vec<serde_json::Map<String, Value>>> {
        self.metrics.inc_queries();
        let key = hash_key(&[&ds.name, &self.policy_fingerprint, sql]);
        if let Some(rows) = self.result_cache.get(key) {
            self.metrics
                .result_cache_hits
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            self.metrics.add_rows(rows.len() as u64);
            return Ok(rows.into_iter().map(|m| m.into_iter().collect()).collect());
        }
        self.metrics
            .result_cache_misses
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        match self.executor.execute_streaming_collect(ds, sql).await {
            Ok(rows) => {
                self.metrics.add_rows(rows.len() as u64);
                self.result_cache.insert(key, rows.clone());
                Ok(rows.into_iter().map(|m| m.into_iter().collect()).collect())
            }
            Err(e) => {
                self.metrics.inc_errors();
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
