use super::backend::{Memory, MemoryFilter, SearchResult, StorageBackend};
use graphnight_core::errors::StorageError;
use graphnight_core::models::{DataSource, Model};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::info;

pub struct YamlStorage {
    base_path: PathBuf,
    models: Arc<RwLock<HashMap<String, Model>>>,
    datasources: Arc<RwLock<HashMap<String, DataSource>>>,
    memories: Arc<RwLock<HashMap<String, Memory>>>,
    priority: Arc<RwLock<Vec<String>>>,
}

impl YamlStorage {
    pub fn new(base_path: impl Into<PathBuf>) -> Result<Self, StorageError> {
        let base_path = base_path.into();
        std::fs::create_dir_all(&base_path)?;

        let storage = Self {
            base_path: base_path.clone(),
            models: Arc::new(RwLock::new(HashMap::new())),
            datasources: Arc::new(RwLock::new(HashMap::new())),
            memories: Arc::new(RwLock::new(HashMap::new())),
            priority: Arc::new(RwLock::new(Vec::new())),
        };

        Ok(storage)
    }

    pub async fn load(&self) -> Result<(), StorageError> {
        // Load models
        if self.models_path().exists() {
            let content = tokio::fs::read_to_string(self.models_path()).await?;
            if !content.trim().is_empty() {
                let models: Vec<Model> = serde_yaml::from_str(&content)?;
                let mut map = self.models.write().await;
                for model in models {
                    map.insert(model.name.clone(), model);
                }
            }
        }

        // Load datasources
        if self.datasources_path().exists() {
            let content = tokio::fs::read_to_string(self.datasources_path()).await?;
            if !content.trim().is_empty() {
                let datasources: Vec<DataSource> = serde_yaml::from_str(&content)?;
                let mut map = self.datasources.write().await;
                for ds in datasources {
                    map.insert(ds.name.clone(), ds);
                }
            }
        }

        // Load memories
        if self.memories_path().exists() {
            let content = tokio::fs::read_to_string(self.memories_path()).await?;
            if !content.trim().is_empty() {
                let memories: Vec<Memory> = serde_yaml::from_str(&content)?;
                let mut map = self.memories.write().await;
                for mem in memories {
                    map.insert(mem.id.clone(), mem);
                }
            }
        }

        // Load priority
        if self.priority_path().exists() {
            let content = tokio::fs::read_to_string(self.priority_path()).await?;
            if !content.trim().is_empty() {
                let priority: Vec<String> = serde_yaml::from_str(&content)?;
                *self.priority.write().await = priority;
            }
        }

        info!("Loaded YAML storage from {:?}", self.base_path);
        Ok(())
    }

    fn models_path(&self) -> PathBuf {
        self.base_path.join("models.yaml")
    }

    fn datasources_path(&self) -> PathBuf {
        self.base_path.join("datasources.yaml")
    }

    fn memories_path(&self) -> PathBuf {
        self.base_path.join("memories.yaml")
    }

    fn priority_path(&self) -> PathBuf {
        self.base_path.join("priority.yaml")
    }

    async fn persist_models(&self) -> Result<(), StorageError> {
        let models = self.models.read().await;
        let vec: Vec<Model> = models.values().cloned().collect();
        let content = serde_yaml::to_string(&vec)?;
        tokio::fs::write(self.models_path(), content).await?;
        Ok(())
    }

    async fn persist_datasources(&self) -> Result<(), StorageError> {
        let datasources = self.datasources.read().await;
        let vec: Vec<DataSource> = datasources.values().cloned().collect();
        let content = serde_yaml::to_string(&vec)?;
        tokio::fs::write(self.datasources_path(), content).await?;
        Ok(())
    }

    async fn persist_memories(&self) -> Result<(), StorageError> {
        let memories = self.memories.read().await;
        let vec: Vec<Memory> = memories.values().cloned().collect();
        let content = serde_yaml::to_string(&vec)?;
        tokio::fs::write(self.memories_path(), content).await?;
        Ok(())
    }

    async fn persist_priority(&self) -> Result<(), StorageError> {
        let priority = self.priority.read().await;
        let content = serde_yaml::to_string(&*priority)?;
        tokio::fs::write(self.priority_path(), content).await?;
        Ok(())
    }

    /// Backup all metadata to a JSON file
    pub async fn backup(&self, path: &PathBuf) -> Result<(), StorageError> {
        use crate::backend::StorageBackup;
        use chrono::Utc;

        let models = self.models.read().await;
        let datasources = self.datasources.read().await;
        let memories = self.memories.read().await;
        let priority = self.priority.read().await;

        let backup = StorageBackup {
            version: "1".to_string(),
            created_at: Utc::now(),
            models: models.values().cloned().collect(),
            datasources: datasources.values().cloned().collect(),
            memories: memories.values().cloned().collect(),
            datasource_priority: priority.clone(),
        };

        let content = serde_json::to_string_pretty(&backup)?;
        tokio::fs::write(path, content).await?;
        Ok(())
    }

    /// Restore metadata from a JSON file
    pub async fn restore(&self, path: &PathBuf) -> Result<(), StorageError> {
        use crate::backend::StorageBackup;

        let content = tokio::fs::read_to_string(path).await?;
        let backup: StorageBackup = serde_json::from_str(&content)?;

        if backup.version != "1" {
            return Err(StorageError::BackendError(format!(
                "Unsupported backup version: {}",
                backup.version
            )));
        }

        // Restore models
        {
            let mut models = self.models.write().await;
            models.clear();
            for model in backup.models {
                models.insert(model.name.clone(), model);
            }
            self.persist_models().await?;
        }

        // Restore datasources
        {
            let mut datasources = self.datasources.write().await;
            datasources.clear();
            for ds in backup.datasources {
                datasources.insert(ds.name.clone(), ds);
            }
            self.persist_datasources().await?;
        }

        // Restore memories
        {
            let mut memories = self.memories.write().await;
            memories.clear();
            for memory in backup.memories {
                memories.insert(memory.id.clone(), memory);
            }
            self.persist_memories().await?;
        }

        // Restore priority
        {
            let mut priority = self.priority.write().await;
            *priority = backup.datasource_priority;
            self.persist_priority().await?;
        }

        Ok(())
    }
}

#[async_trait::async_trait]
impl StorageBackend for YamlStorage {
    async fn list_models(&self, datasource: Option<&str>) -> Result<Vec<Model>, StorageError> {
        let models = self.models.read().await;
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
        let models = self.models.read().await;
        if let Some(model) = models.get(name) {
            if let Some(ds) = datasource {
                if model.datasource != ds {
                    return Ok(None);
                }
            }
            Ok(Some(model.clone()))
        } else {
            Ok(None)
        }
    }

    async fn create_model(&self, model: Model) -> Result<Model, StorageError> {
        let mut models = self.models.write().await;
        if models.contains_key(&model.name) {
            return Err(StorageError::ModelExists(model.name));
        }
        models.insert(model.name.clone(), model.clone());
        drop(models);
        self.persist_models().await?;
        Ok(model)
    }

    async fn update_model(&self, name: &str, model: Model) -> Result<Model, StorageError> {
        let mut models = self.models.write().await;
        if !models.contains_key(name) {
            return Err(StorageError::ModelNotFound(name.to_string()));
        }
        models.insert(name.to_string(), model.clone());
        drop(models);
        self.persist_models().await?;
        Ok(model)
    }

    async fn delete_model(
        &self,
        name: &str,
        datasource: Option<&str>,
    ) -> Result<bool, StorageError> {
        let mut models = self.models.write().await;
        let exists = if let Some(ds) = datasource {
            models
                .get(name)
                .map(|m| m.datasource == ds)
                .unwrap_or(false)
        } else {
            models.contains_key(name)
        };
        if exists {
            models.remove(name);
            drop(models);
            self.persist_models().await?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    async fn list_datasources(&self) -> Result<Vec<DataSource>, StorageError> {
        let datasources = self.datasources.read().await;
        Ok(datasources.values().cloned().collect())
    }

    async fn get_datasource(&self, name: &str) -> Result<Option<DataSource>, StorageError> {
        let datasources = self.datasources.read().await;
        Ok(datasources.get(name).cloned())
    }

    async fn create_datasource(&self, ds: DataSource) -> Result<DataSource, StorageError> {
        let mut datasources = self.datasources.write().await;
        if datasources.contains_key(&ds.name) {
            return Err(StorageError::DatasourceExists(ds.name));
        }
        datasources.insert(ds.name.clone(), ds.clone());
        drop(datasources);
        self.persist_datasources().await?;
        Ok(ds)
    }

    async fn update_datasource(
        &self,
        name: &str,
        ds: DataSource,
    ) -> Result<DataSource, StorageError> {
        let mut datasources = self.datasources.write().await;
        if !datasources.contains_key(name) {
            return Err(StorageError::DatasourceNotFound(name.to_string()));
        }
        datasources.insert(name.to_string(), ds.clone());
        drop(datasources);
        self.persist_datasources().await?;
        Ok(ds)
    }

    async fn delete_datasource(&self, name: &str) -> Result<bool, StorageError> {
        let mut datasources = self.datasources.write().await;
        if datasources.remove(name).is_some() {
            drop(datasources);
            self.persist_datasources().await?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    async fn get_datasource_priority(&self) -> Result<Vec<String>, StorageError> {
        let priority = self.priority.read().await;
        Ok(priority.clone())
    }

    async fn set_datasource_priority(&self, priority: Vec<String>) -> Result<(), StorageError> {
        *self.priority.write().await = priority;
        self.persist_priority().await
    }

    async fn save_memory(&self, memory: Memory) -> Result<Memory, StorageError> {
        let mut memories = self.memories.write().await;
        memories.insert(memory.id.clone(), memory.clone());
        drop(memories);
        self.persist_memories().await?;
        Ok(memory)
    }

    async fn get_memory(&self, id: &str) -> Result<Option<Memory>, StorageError> {
        let memories = self.memories.read().await;
        Ok(memories.get(id).cloned())
    }

    async fn list_memories(&self, filter: MemoryFilter) -> Result<Vec<Memory>, StorageError> {
        let memories = self.memories.read().await;
        let mut result: Vec<Memory> = memories.values().cloned().collect();

        if let Some(query) = filter.query {
            let q = query.to_lowercase();
            result.retain(|m| m.learning.to_lowercase().contains(&q));
        }

        if let Some(entity) = filter.entity {
            result.retain(|m| m.linked_entities.contains(&entity));
        }

        if let Some(offset) = filter.offset {
            if offset < result.len() {
                result = result[offset..].to_vec();
            } else {
                result.clear();
            }
        }

        if let Some(limit) = filter.limit {
            result.truncate(limit);
        }

        Ok(result)
    }

    async fn delete_memory(&self, id: &str) -> Result<bool, StorageError> {
        let mut memories = self.memories.write().await;
        if memories.remove(id).is_some() {
            drop(memories);
            self.persist_memories().await?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    async fn index_model(&self, _model: &Model) -> Result<(), StorageError> {
        // YAML storage doesn't have full-text search
        // This would be implemented in TantivyStorage
        Ok(())
    }

    async fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchResult>, StorageError> {
        let models = self.models.read().await;
        let q = query.to_lowercase();
        let mut results = Vec::new();

        for model in models.values() {
            let mut matched = Vec::new();
            let mut score = 0.0;

            if model.name.to_lowercase().contains(&q) {
                matched.push("name".to_string());
                score += 10.0;
            }
            if model
                .description
                .as_ref()
                .is_some_and(|d| d.to_lowercase().contains(&q))
            {
                matched.push("description".to_string());
                score += 5.0;
            }
            for m in &model.measures {
                if m.formula.expression.to_lowercase().contains(&q) {
                    matched.push(format!("measure:{}", m.formula.expression));
                    score += 3.0;
                }
            }
            for d in &model.dimensions {
                if d.name.to_lowercase().contains(&q) {
                    matched.push(format!("dimension:{}", d.name));
                    score += 2.0;
                }
            }

            if !matched.is_empty() {
                results.push(SearchResult {
                    model_name: model.name.clone(),
                    datasource: model.datasource.clone(),
                    score,
                    matched_fields: matched,
                    snippet: model.description.clone().unwrap_or_default(),
                });
            }
        }

        results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap());
        results.truncate(limit);
        Ok(results)
    }

    async fn backup(&self, path: &PathBuf) -> Result<(), StorageError> {
        self.backup(path).await
    }

    async fn restore(&self, path: &PathBuf) -> Result<(), StorageError> {
        self.restore(path).await
    }
}
