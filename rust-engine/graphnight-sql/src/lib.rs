pub mod dialects;
pub mod executor;
pub mod generator;

use anyhow::Result;
use graphnight_core::models::{DataSource, Model, Query};
use std::sync::Arc;

/// High-level SQL interface combining generator and executor
pub struct SqlEngine {
    generator: generator::SqlGenerator,
    executor: Arc<executor::QueryExecutor>,
}

impl SqlEngine {
    pub fn new(
        dialect: Box<dyn dialects::Dialect>,
        executor: Arc<executor::QueryExecutor>,
    ) -> Result<Self> {
        let generator = generator::SqlGenerator::new(dialect);

        Ok(Self {
            generator,
            executor,
        })
    }

    pub fn with_models(mut self, models: Vec<Model>) -> Self {
        self.generator = self.generator.with_models(models);
        self
    }

    /// Generate SQL for a query
    pub fn generate_sql(&self, query: &Query) -> Result<String> {
        self.generator.generate(query)
    }

    /// Execute query using sqlx
    pub async fn execute_sqlx(
        &self,
        ds: &DataSource,
        sql: &str,
    ) -> Result<Vec<serde_json::Map<String, serde_json::Value>>> {
        let results = self.executor.execute(ds, sql).await?;
        Ok(results
            .into_iter()
            .map(|m| m.into_iter().collect())
            .collect())
    }
}
