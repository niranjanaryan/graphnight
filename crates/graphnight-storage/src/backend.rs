use async_trait::async_trait;
use chrono::{DateTime, Utc};
use graphnight_core::errors::StorageError;
use graphnight_core::models::{DataSource, Model};
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;

/// Backup data structure
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StorageBackup {
    pub version: String,
    pub created_at: DateTime<Utc>,
    pub models: Vec<Model>,
    pub datasources: Vec<DataSource>,
    pub memories: Vec<Memory>,
    pub datasource_priority: Vec<String>,
}

#[async_trait]
pub trait StorageBackend: Send + Sync {
    async fn list_models(&self, datasource: Option<&str>) -> Result<Vec<Model>, StorageError>;
    async fn get_model(
        &self,
        name: &str,
        datasource: Option<&str>,
    ) -> Result<Option<Model>, StorageError>;
    async fn create_model(&self, model: Model) -> Result<Model, StorageError>;
    async fn update_model(&self, name: &str, model: Model) -> Result<Model, StorageError>;
    async fn delete_model(
        &self,
        name: &str,
        datasource: Option<&str>,
    ) -> Result<bool, StorageError>;

    async fn list_datasources(&self) -> Result<Vec<DataSource>, StorageError>;
    async fn get_datasource(&self, name: &str) -> Result<Option<DataSource>, StorageError>;
    async fn create_datasource(&self, ds: DataSource) -> Result<DataSource, StorageError>;
    async fn update_datasource(
        &self,
        name: &str,
        ds: DataSource,
    ) -> Result<DataSource, StorageError>;
    async fn delete_datasource(&self, name: &str) -> Result<bool, StorageError>;

    async fn get_datasource_priority(&self) -> Result<Vec<String>, StorageError>;
    async fn set_datasource_priority(&self, priority: Vec<String>) -> Result<(), StorageError>;

    // Memory operations
    async fn save_memory(&self, memory: Memory) -> Result<Memory, StorageError>;
    async fn get_memory(&self, id: &str) -> Result<Option<Memory>, StorageError>;
    async fn list_memories(&self, filter: MemoryFilter) -> Result<Vec<Memory>, StorageError>;
    async fn delete_memory(&self, id: &str) -> Result<bool, StorageError>;

    // Search index
    async fn index_model(&self, model: &Model) -> Result<(), StorageError>;
    async fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchResult>, StorageError>;

    /// Backup all metadata to a JSON file
    async fn backup(&self, path: &PathBuf) -> Result<(), StorageError> {
        let _ = path;
        Err(StorageError::BackendError(
            "backup not implemented for this storage backend".to_string(),
        ))
    }

    /// Restore metadata from a JSON file
    async fn restore(&self, path: &PathBuf) -> Result<(), StorageError> {
        let _ = path;
        Err(StorageError::BackendError(
            "restore not implemented for this storage backend".to_string(),
        ))
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Memory {
    pub id: String,
    pub learning: String,
    pub linked_entities: Vec<String>,
    pub description: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub meta: HashMap<String, Value>,
}

#[derive(Debug, Clone, Default)]
pub struct MemoryFilter {
    pub query: Option<String>,
    pub entity: Option<String>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SearchResult {
    pub model_name: String,
    pub datasource: String,
    pub score: f32,
    pub matched_fields: Vec<String>,
    pub snippet: String,
}
