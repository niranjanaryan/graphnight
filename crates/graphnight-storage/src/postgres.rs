//! Postgres metadata storage — HA-friendly shared backend for models, datasources, and memories.
//!
//! Search uses `ILIKE` over name / JSON text (not Tantivy). Suitable for multi-replica
//! deployments; there is no process-local cache so all instances see the same rows.

use super::backend::{Memory, MemoryFilter, SearchResult, StorageBackend};
use graphnight_core::errors::StorageError;
use graphnight_core::models::{DataSource, Model};
use graphnight_core::resolve_connection_string;
use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, Row};
use tracing::info;

/// Env var used when `storage.path` is unset for `storage.type = "postgres"`.
pub const METADATA_DATABASE_URL_ENV: &str = "GRAPHNIGHT_METADATA_DATABASE_URL";

/// Shared Postgres metadata store (JSONB rows, no local cache).
pub struct PostgresMetadataStorage {
    pool: PgPool,
}

impl PostgresMetadataStorage {
    /// Connect and ensure schema (`CREATE TABLE IF NOT EXISTS`).
    pub async fn new(database_url: &str) -> Result<Self, StorageError> {
        let pool = PgPoolOptions::new()
            .max_connections(10)
            .connect(database_url)
            .await?;

        Self::init_schema(&pool).await?;
        info!("Initialized Postgres metadata storage");
        Ok(Self { pool })
    }

    /// Resolve connection string from config `path` / CLI override, else
    /// [`METADATA_DATABASE_URL_ENV`]. Supports `env:VARNAME` via
    /// [`resolve_connection_string`].
    pub fn resolve_url(path: Option<&str>) -> Result<String, StorageError> {
        let raw = match path.map(str::trim).filter(|p| !p.is_empty()) {
            Some(p) => p.to_string(),
            None => std::env::var(METADATA_DATABASE_URL_ENV).map_err(|_| {
                StorageError::BackendError(format!(
                    "postgres storage requires storage.path or {METADATA_DATABASE_URL_ENV}"
                ))
            })?,
        };
        resolve_connection_string(&raw).map_err(StorageError::BackendError)
    }

    async fn init_schema(pool: &PgPool) -> Result<(), StorageError> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS models (
                key TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                datasource TEXT NOT NULL,
                data JSONB NOT NULL
            )",
        )
        .execute(pool)
        .await?;

        sqlx::query("CREATE INDEX IF NOT EXISTS idx_models_datasource ON models (datasource)")
            .execute(pool)
            .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS datasources (
                name TEXT PRIMARY KEY,
                data JSONB NOT NULL
            )",
        )
        .execute(pool)
        .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS memories (
                id TEXT PRIMARY KEY,
                data JSONB NOT NULL,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
            )",
        )
        .execute(pool)
        .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS config (
                key TEXT PRIMARY KEY,
                value JSONB NOT NULL
            )",
        )
        .execute(pool)
        .await?;

        Ok(())
    }

    fn model_key(name: &str, datasource: Option<&str>) -> String {
        match datasource {
            Some(ds) => format!("{ds}.{name}"),
            None => name.to_string(),
        }
    }

    fn decode_model(data: serde_json::Value) -> Result<Model, StorageError> {
        Ok(serde_json::from_value(data)?)
    }

    fn decode_datasource(data: serde_json::Value) -> Result<DataSource, StorageError> {
        Ok(serde_json::from_value(data)?)
    }

    fn decode_memory(data: serde_json::Value) -> Result<Memory, StorageError> {
        Ok(serde_json::from_value(data)?)
    }
}

#[async_trait::async_trait]
impl StorageBackend for PostgresMetadataStorage {
    async fn list_models(&self, datasource: Option<&str>) -> Result<Vec<Model>, StorageError> {
        let rows = if let Some(ds) = datasource {
            sqlx::query("SELECT data FROM models WHERE datasource = $1 ORDER BY name")
                .bind(ds)
                .fetch_all(&self.pool)
                .await?
        } else {
            sqlx::query("SELECT data FROM models ORDER BY name")
                .fetch_all(&self.pool)
                .await?
        };

        let mut result = Vec::with_capacity(rows.len());
        for row in rows {
            result.push(Self::decode_model(row.get("data"))?);
        }
        Ok(result)
    }

    async fn get_model(
        &self,
        name: &str,
        datasource: Option<&str>,
    ) -> Result<Option<Model>, StorageError> {
        let row = if let Some(ds) = datasource {
            let key = Self::model_key(name, Some(ds));
            sqlx::query("SELECT data FROM models WHERE key = $1")
                .bind(&key)
                .fetch_optional(&self.pool)
                .await?
        } else {
            // Prefer exact name match; first row if duplicates across datasources.
            sqlx::query("SELECT data FROM models WHERE name = $1 ORDER BY datasource LIMIT 1")
                .bind(name)
                .fetch_optional(&self.pool)
                .await?
        };

        match row {
            Some(row) => Ok(Some(Self::decode_model(row.get("data"))?)),
            None => Ok(None),
        }
    }

    async fn create_model(&self, model: Model) -> Result<Model, StorageError> {
        let key = Self::model_key(&model.name, Some(&model.datasource));
        let data = serde_json::to_value(&model)?;

        let result = sqlx::query(
            "INSERT INTO models (key, name, datasource, data) VALUES ($1, $2, $3, $4)
             ON CONFLICT (key) DO NOTHING",
        )
        .bind(&key)
        .bind(&model.name)
        .bind(&model.datasource)
        .bind(&data)
        .execute(&self.pool)
        .await?;

        if result.rows_affected() == 0 {
            return Err(StorageError::ModelExists(key));
        }
        Ok(model)
    }

    async fn update_model(&self, name: &str, model: Model) -> Result<Model, StorageError> {
        let key = Self::model_key(name, Some(&model.datasource));
        let data = serde_json::to_value(&model)?;

        let result =
            sqlx::query("UPDATE models SET data = $1, name = $2, datasource = $3 WHERE key = $4")
                .bind(&data)
                .bind(&model.name)
                .bind(&model.datasource)
                .bind(&key)
                .execute(&self.pool)
                .await?;

        if result.rows_affected() == 0 {
            return Err(StorageError::ModelNotFound(key));
        }
        Ok(model)
    }

    async fn delete_model(
        &self,
        name: &str,
        datasource: Option<&str>,
    ) -> Result<bool, StorageError> {
        let result = if let Some(ds) = datasource {
            let key = Self::model_key(name, Some(ds));
            sqlx::query("DELETE FROM models WHERE key = $1")
                .bind(&key)
                .execute(&self.pool)
                .await?
        } else {
            sqlx::query("DELETE FROM models WHERE name = $1")
                .bind(name)
                .execute(&self.pool)
                .await?
        };
        Ok(result.rows_affected() > 0)
    }

    async fn list_datasources(&self) -> Result<Vec<DataSource>, StorageError> {
        let rows = sqlx::query("SELECT data FROM datasources ORDER BY name")
            .fetch_all(&self.pool)
            .await?;
        let mut result = Vec::with_capacity(rows.len());
        for row in rows {
            result.push(Self::decode_datasource(row.get("data"))?);
        }
        Ok(result)
    }

    async fn get_datasource(&self, name: &str) -> Result<Option<DataSource>, StorageError> {
        let row = sqlx::query("SELECT data FROM datasources WHERE name = $1")
            .bind(name)
            .fetch_optional(&self.pool)
            .await?;
        match row {
            Some(row) => Ok(Some(Self::decode_datasource(row.get("data"))?)),
            None => Ok(None),
        }
    }

    async fn create_datasource(&self, ds: DataSource) -> Result<DataSource, StorageError> {
        let data = serde_json::to_value(&ds)?;
        let result = sqlx::query(
            "INSERT INTO datasources (name, data) VALUES ($1, $2) ON CONFLICT (name) DO NOTHING",
        )
        .bind(&ds.name)
        .bind(&data)
        .execute(&self.pool)
        .await?;

        if result.rows_affected() == 0 {
            return Err(StorageError::DatasourceExists(ds.name));
        }
        Ok(ds)
    }

    async fn update_datasource(
        &self,
        name: &str,
        ds: DataSource,
    ) -> Result<DataSource, StorageError> {
        let data = serde_json::to_value(&ds)?;
        let result = sqlx::query("UPDATE datasources SET data = $1 WHERE name = $2")
            .bind(&data)
            .bind(name)
            .execute(&self.pool)
            .await?;

        if result.rows_affected() == 0 {
            return Err(StorageError::DatasourceNotFound(name.to_string()));
        }
        Ok(ds)
    }

    async fn delete_datasource(&self, name: &str) -> Result<bool, StorageError> {
        let result = sqlx::query("DELETE FROM datasources WHERE name = $1")
            .bind(name)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    async fn get_datasource_priority(&self) -> Result<Vec<String>, StorageError> {
        let row = sqlx::query("SELECT value FROM config WHERE key = 'datasource_priority'")
            .fetch_optional(&self.pool)
            .await?;

        if let Some(row) = row {
            let value: serde_json::Value = row.get("value");
            Ok(serde_json::from_value(value)?)
        } else {
            Ok(Vec::new())
        }
    }

    async fn set_datasource_priority(&self, priority: Vec<String>) -> Result<(), StorageError> {
        let value = serde_json::to_value(&priority)?;
        sqlx::query(
            "INSERT INTO config (key, value) VALUES ('datasource_priority', $1)
             ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value",
        )
        .bind(&value)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn save_memory(&self, memory: Memory) -> Result<Memory, StorageError> {
        let data = serde_json::to_value(&memory)?;
        sqlx::query(
            "INSERT INTO memories (id, data, created_at) VALUES ($1, $2, $3)
             ON CONFLICT (id) DO UPDATE SET data = EXCLUDED.data",
        )
        .bind(&memory.id)
        .bind(&data)
        .bind(memory.created_at)
        .execute(&self.pool)
        .await?;
        Ok(memory)
    }

    async fn get_memory(&self, id: &str) -> Result<Option<Memory>, StorageError> {
        let row = sqlx::query("SELECT data FROM memories WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        match row {
            Some(row) => Ok(Some(Self::decode_memory(row.get("data"))?)),
            None => Ok(None),
        }
    }

    async fn list_memories(&self, filter: MemoryFilter) -> Result<Vec<Memory>, StorageError> {
        // Basic filters: entity substring via JSON text ILIKE; query filtered in Rust.
        let limit = filter.limit.unwrap_or(1000) as i64;
        let offset = filter.offset.unwrap_or(0) as i64;

        let rows = if let Some(ref entity) = filter.entity {
            let pattern = format!("%{entity}%");
            sqlx::query(
                "SELECT data FROM memories
                 WHERE data::text ILIKE $1
                 ORDER BY created_at DESC
                 LIMIT $2 OFFSET $3",
            )
            .bind(&pattern)
            .bind(limit)
            .bind(offset)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query(
                "SELECT data FROM memories
                 ORDER BY created_at DESC
                 LIMIT $1 OFFSET $2",
            )
            .bind(limit)
            .bind(offset)
            .fetch_all(&self.pool)
            .await?
        };

        let mut result = Vec::new();
        for row in rows {
            let mem = Self::decode_memory(row.get("data"))?;
            if let Some(query_str) = &filter.query {
                if !mem
                    .learning
                    .to_lowercase()
                    .contains(&query_str.to_lowercase())
                {
                    continue;
                }
            }
            result.push(mem);
        }
        Ok(result)
    }

    async fn delete_memory(&self, id: &str) -> Result<bool, StorageError> {
        let result = sqlx::query("DELETE FROM memories WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    async fn index_model(&self, _model: &Model) -> Result<(), StorageError> {
        // No separate search index; `search` uses ILIKE over JSONB.
        Ok(())
    }

    async fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchResult>, StorageError> {
        // Limited: case-insensitive substring match on name + JSON payload (not Tantivy).
        let pattern = format!("%{}%", query);
        let rows = sqlx::query(
            "SELECT name, datasource, data FROM models
             WHERE name ILIKE $1 OR data::text ILIKE $1
             ORDER BY name
             LIMIT $2",
        )
        .bind(&pattern)
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;

        let mut results = Vec::new();
        for row in rows {
            let name: String = row.get("name");
            let datasource: String = row.get("datasource");
            let model = Self::decode_model(row.get("data"))?;
            results.push(SearchResult {
                model_name: name,
                datasource,
                score: 1.0,
                matched_fields: vec![],
                snippet: model.description.unwrap_or_default(),
            });
        }
        Ok(results)
    }
}
