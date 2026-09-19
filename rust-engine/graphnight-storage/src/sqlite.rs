use super::backend::{Memory, MemoryFilter, SearchResult, StorageBackend};
use graphnight_core::errors::StorageError;
use graphnight_core::models::{DataSource, Model};
use sqlx::{Pool, Row, Sqlite, SqlitePool};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::info;

pub struct SqliteStorage {
    pool: Pool<Sqlite>,
    // In-memory cache for hot data
    models_cache: Arc<RwLock<HashMap<String, Model>>>,
    datasources_cache: Arc<RwLock<HashMap<String, DataSource>>>,
}

impl SqliteStorage {
    pub async fn new(db_path: impl AsRef<Path>) -> Result<Self, StorageError> {
        let db_path = db_path.as_ref();
        let db_url = format!("sqlite://{}?mode=rwc", db_path.display());

        let pool = SqlitePool::connect(&db_url).await?;

        // Initialize schema
        Self::init_schema(&pool).await?;

        let storage = Self {
            pool,
            models_cache: Arc::new(RwLock::new(HashMap::new())),
            datasources_cache: Arc::new(RwLock::new(HashMap::new())),
        };

        storage.warm_cache().await?;
        info!("Initialized SQLite storage at {:?}", db_path);
        Ok(storage)
    }

    async fn init_schema(pool: &Pool<Sqlite>) -> Result<(), StorageError> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS models (
                key TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                datasource TEXT NOT NULL,
                data TEXT NOT NULL
            )",
        )
        .execute(pool)
        .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS datasources (
                name TEXT PRIMARY KEY,
                data TEXT NOT NULL
            )",
        )
        .execute(pool)
        .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS memories (
                id TEXT PRIMARY KEY,
                data TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            )",
        )
        .execute(pool)
        .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS config (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            )",
        )
        .execute(pool)
        .await?;

        Ok(())
    }

    async fn warm_cache(&self) -> Result<(), StorageError> {
        // Load models
        let rows = sqlx::query("SELECT data FROM models")
            .fetch_all(&self.pool)
            .await?;
        let mut models = self.models_cache.write().await;
        for row in rows {
            let data: String = row.get("data");
            let model: Model = serde_json::from_str(&data)?;
            models.insert(model.name.clone(), model);
        }

        // Load datasources
        let rows = sqlx::query("SELECT data FROM datasources")
            .fetch_all(&self.pool)
            .await?;
        let mut datasources = self.datasources_cache.write().await;
        for row in rows {
            let data: String = row.get("data");
            let ds: DataSource = serde_json::from_str(&data)?;
            datasources.insert(ds.name.clone(), ds);
        }

        Ok(())
    }

    fn model_key(&self, name: &str, datasource: Option<&str>) -> String {
        match datasource {
            Some(ds) => format!("{}.{}", ds, name),
            None => name.to_string(),
        }
    }
}

#[async_trait::async_trait]
impl StorageBackend for SqliteStorage {
    async fn list_models(&self, datasource: Option<&str>) -> Result<Vec<Model>, StorageError> {
        let models = self.models_cache.read().await;
        let mut result: Vec<Model> = models.values().cloned().collect();
        if let Some(ds) = datasource {
            result.retain(|m| m.datasource == ds);
        }
        Ok(result)
    }

    async fn get_model(
        &self,
        name: &str,
        datasource: Option<&str>,
    ) -> Result<Option<Model>, StorageError> {
        let models = self.models_cache.read().await;
        let key = self.model_key(name, datasource);
        Ok(models.get(&key).cloned())
    }

    async fn create_model(&self, model: Model) -> Result<Model, StorageError> {
        let mut models = self.models_cache.write().await;
        let key = self.model_key(&model.name, Some(&model.datasource));
        if models.contains_key(&key) {
            return Err(StorageError::ModelExists(key));
        }
        let data = serde_json::to_string(&model)?;
        models.insert(key.clone(), model.clone());
        drop(models);

        sqlx::query("INSERT INTO models (key, name, datasource, data) VALUES (?, ?, ?, ?)")
            .bind(&key)
            .bind(&model.name)
            .bind(&model.datasource)
            .bind(&data)
            .execute(&self.pool)
            .await?;

        Ok(model)
    }

    async fn update_model(&self, name: &str, model: Model) -> Result<Model, StorageError> {
        let mut models = self.models_cache.write().await;
        let key = self.model_key(name, Some(&model.datasource));
        if !models.contains_key(&key) {
            return Err(StorageError::ModelNotFound(key));
        }
        let data = serde_json::to_string(&model)?;
        models.insert(key.clone(), model.clone());
        drop(models);

        sqlx::query("UPDATE models SET data = ? WHERE key = ?")
            .bind(&data)
            .bind(&key)
            .execute(&self.pool)
            .await?;

        Ok(model)
    }

    async fn delete_model(
        &self,
        name: &str,
        datasource: Option<&str>,
    ) -> Result<bool, StorageError> {
        let mut models = self.models_cache.write().await;
        let key = self.model_key(name, datasource);
        if models.remove(&key).is_some() {
            drop(models);
            sqlx::query("DELETE FROM models WHERE key = ?")
                .bind(&key)
                .execute(&self.pool)
                .await?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    async fn list_datasources(&self) -> Result<Vec<DataSource>, StorageError> {
        let datasources = self.datasources_cache.read().await;
        Ok(datasources.values().cloned().collect())
    }

    async fn get_datasource(&self, name: &str) -> Result<Option<DataSource>, StorageError> {
        let datasources = self.datasources_cache.read().await;
        Ok(datasources.get(name).cloned())
    }

    async fn create_datasource(&self, ds: DataSource) -> Result<DataSource, StorageError> {
        let mut datasources = self.datasources_cache.write().await;
        if datasources.contains_key(&ds.name) {
            return Err(StorageError::DatasourceExists(ds.name));
        }
        let data = serde_json::to_string(&ds)?;
        datasources.insert(ds.name.clone(), ds.clone());
        drop(datasources);

        sqlx::query("INSERT INTO datasources (name, data) VALUES (?, ?)")
            .bind(&ds.name)
            .bind(&data)
            .execute(&self.pool)
            .await?;

        Ok(ds)
    }

    async fn update_datasource(
        &self,
        name: &str,
        ds: DataSource,
    ) -> Result<DataSource, StorageError> {
        let mut datasources = self.datasources_cache.write().await;
        if !datasources.contains_key(name) {
            return Err(StorageError::DatasourceNotFound(name.to_string()));
        }
        let data = serde_json::to_string(&ds)?;
        datasources.insert(name.to_string(), ds.clone());
        drop(datasources);

        sqlx::query("UPDATE datasources SET data = ? WHERE name = ?")
            .bind(&data)
            .bind(name)
            .execute(&self.pool)
            .await?;

        Ok(ds)
    }

    async fn delete_datasource(&self, name: &str) -> Result<bool, StorageError> {
        let mut datasources = self.datasources_cache.write().await;
        if datasources.remove(name).is_some() {
            drop(datasources);
            sqlx::query("DELETE FROM datasources WHERE name = ?")
                .bind(name)
                .execute(&self.pool)
                .await?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    async fn get_datasource_priority(&self) -> Result<Vec<String>, StorageError> {
        let row = sqlx::query("SELECT value FROM config WHERE key = 'datasource_priority'")
            .fetch_optional(&self.pool)
            .await?;

        if let Some(row) = row {
            let value: String = row.get("value");
            Ok(serde_json::from_str(&value)?)
        } else {
            Ok(Vec::new())
        }
    }

    async fn set_datasource_priority(&self, priority: Vec<String>) -> Result<(), StorageError> {
        let value = serde_json::to_string(&priority)?;
        sqlx::query(
            "INSERT INTO config (key, value) VALUES ('datasource_priority', ?) 
             ON CONFLICT(key) DO UPDATE SET value = ?",
        )
        .bind(&value)
        .bind(&value)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn save_memory(&self, memory: Memory) -> Result<Memory, StorageError> {
        let data = serde_json::to_string(&memory)?;
        sqlx::query(
            "INSERT INTO memories (id, data) VALUES (?, ?) 
             ON CONFLICT(id) DO UPDATE SET data = ?",
        )
        .bind(&memory.id)
        .bind(&data)
        .bind(&data)
        .execute(&self.pool)
        .await?;
        Ok(memory)
    }

    async fn get_memory(&self, id: &str) -> Result<Option<Memory>, StorageError> {
        let row = sqlx::query("SELECT data FROM memories WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;

        if let Some(row) = row {
            let data: String = row.get("data");
            Ok(Some(serde_json::from_str(&data)?))
        } else {
            Ok(None)
        }
    }

    async fn list_memories(&self, filter: MemoryFilter) -> Result<Vec<Memory>, StorageError> {
        let mut query = "SELECT data FROM memories".to_string();
        let mut conditions = Vec::new();

        if filter.entity.is_some() {
            conditions.push("data LIKE ?".to_string());
        }

        if !conditions.is_empty() {
            query.push_str(" WHERE ");
            query.push_str(&conditions.join(" AND "));
        }

        query.push_str(" ORDER BY created_at DESC");

        if let Some(limit) = filter.limit {
            query.push_str(&format!(" LIMIT {}", limit));
        }
        if let Some(offset) = filter.offset {
            query.push_str(&format!(" OFFSET {}", offset));
        }

        let mut q = sqlx::query(&query);
        if let Some(entity) = filter.entity {
            q = q.bind(format!("%{}%", entity));
        }

        let rows = q.fetch_all(&self.pool).await?;
        let mut result = Vec::new();
        for row in rows {
            let data: String = row.get("data");
            let mem: Memory = serde_json::from_str(&data)?;

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
        let result = sqlx::query("DELETE FROM memories WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    async fn index_model(&self, _model: &Model) -> Result<(), StorageError> {
        // FTS5 virtual table for full-text search
        // Would be implemented with a separate FTS table
        Ok(())
    }

    async fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchResult>, StorageError> {
        // Use FTS5 if available, fallback to LIKE
        let search_term = format!("%{}%", query.to_lowercase());

        let rows = sqlx::query(
            "SELECT name, datasource, data FROM models 
             WHERE name LIKE ? OR data LIKE ? 
             LIMIT ?",
        )
        .bind(&search_term)
        .bind(&search_term)
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;

        let mut results = Vec::new();
        for row in rows {
            let name: String = row.get("name");
            let datasource: String = row.get("datasource");
            let data: String = row.get("data");
            let model: Model = serde_json::from_str(&data)?;

            results.push(SearchResult {
                model_name: name,
                datasource,
                score: 1.0, // Simplified scoring
                matched_fields: vec![],
                snippet: model.description.unwrap_or_default(),
            });
        }
        Ok(results)
    }
}
