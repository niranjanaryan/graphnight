use super::backend::{Memory, MemoryFilter, SearchResult, StorageBackend};
use graphnight_core::errors::StorageError;
use graphnight_core::models::{DataSource, Model};
use std::path::Path;
use std::sync::Arc;
use tantivy::{collector::TopDocs, doc, schema::*, Index, IndexWriter, ReloadPolicy};
use tokio::sync::RwLock;
use tracing::info;

/// Tantivy-based search storage with full-text search capabilities
pub struct TantivyStorage {
    base_path: std::path::PathBuf,
    index: Index,
    schema: Schema,
    writer: Arc<RwLock<IndexWriter>>,
    // In-memory caches
    models: Arc<RwLock<std::collections::HashMap<String, Model>>>,
    datasources: Arc<RwLock<std::collections::HashMap<String, DataSource>>>,
    memories: Arc<RwLock<std::collections::HashMap<String, Memory>>>,
    priority: Arc<RwLock<Vec<String>>>,
}

impl TantivyStorage {
    pub fn new(base_path: impl AsRef<Path>) -> Result<Self, StorageError> {
        let base_path = base_path.as_ref().to_path_buf();
        std::fs::create_dir_all(&base_path)?;

        // Define Tantivy schema
        let mut schema_builder = Schema::builder();
        let _model_name = schema_builder.add_text_field("model_name", STRING | STORED);
        let _datasource_field = schema_builder.add_text_field("datasource", STRING | STORED);
        let _description = schema_builder.add_text_field("description", TEXT | STORED);
        let _measures = schema_builder.add_text_field("measures", TEXT);
        let _dimensions = schema_builder.add_text_field("dimensions", TEXT);
        let _time_dimensions = schema_builder.add_text_field("time_dimensions", TEXT);
        let _content = schema_builder.add_text_field("content", TEXT);
        let schema = schema_builder.build();

        // Create or open index
        let index_path = base_path.join("tantivy_index");
        let index = Index::create_in_dir(&index_path, schema.clone())
            .map_err(|e| StorageError::TantivyError(e.to_string()))?;

        let writer = Arc::new(RwLock::new(
            index
                .writer(50_000_000)
                .map_err(|e| StorageError::TantivyError(e.to_string()))?,
        ));

        let mut storage = Self {
            base_path: base_path.clone(),
            index,
            schema,
            writer,
            models: Arc::new(RwLock::new(std::collections::HashMap::new())),
            datasources: Arc::new(RwLock::new(std::collections::HashMap::new())),
            memories: Arc::new(RwLock::new(std::collections::HashMap::new())),
            priority: Arc::new(RwLock::new(Vec::new())),
        };

        storage.load()?;
        info!("Initialized Tantivy storage at {:?}", base_path);
        Ok(storage)
    }

    fn models_path(&self) -> std::path::PathBuf {
        self.base_path.join("models.yaml")
    }

    fn datasources_path(&self) -> std::path::PathBuf {
        self.base_path.join("datasources.yaml")
    }

    fn memories_path(&self) -> std::path::PathBuf {
        self.base_path.join("memories.yaml")
    }

    fn priority_path(&self) -> std::path::PathBuf {
        self.base_path.join("priority.yaml")
    }

    fn load(&mut self) -> Result<(), StorageError> {
        // Load models
        if self.models_path().exists() {
            let content = std::fs::read_to_string(self.models_path())?;
            if !content.trim().is_empty() {
                let models: Vec<Model> = serde_yaml::from_str(&content)?;
                let mut map = self.models.blocking_write();
                for model in models {
                    self.index_model_sync(&model)?;
                    map.insert(model.name.clone(), model);
                }
            }
        }

        // Load datasources
        if self.datasources_path().exists() {
            let content = std::fs::read_to_string(self.datasources_path())?;
            if !content.trim().is_empty() {
                let datasources: Vec<DataSource> = serde_yaml::from_str(&content)?;
                let mut map = self.datasources.blocking_write();
                for ds in datasources {
                    map.insert(ds.name.clone(), ds);
                }
            }
        }

        // Load memories
        if self.memories_path().exists() {
            let content = std::fs::read_to_string(self.memories_path())?;
            if !content.trim().is_empty() {
                let memories: Vec<Memory> = serde_yaml::from_str(&content)?;
                let mut map = self.memories.blocking_write();
                for mem in memories {
                    map.insert(mem.id.clone(), mem);
                }
            }
        }

        // Load priority
        if self.priority_path().exists() {
            let content = std::fs::read_to_string(self.priority_path())?;
            if !content.trim().is_empty() {
                let priority: Vec<String> = serde_yaml::from_str(&content)?;
                *self.priority.blocking_write() = priority;
            }
        }

        Ok(())
    }

    fn index_model_sync(&self, model: &Model) -> Result<(), StorageError> {
        let model_name_field = self.schema.get_field("model_name").unwrap();
        let datasource_field = self.schema.get_field("datasource").unwrap();
        let description_field = self.schema.get_field("description").unwrap();
        let measures_field = self.schema.get_field("measures").unwrap();
        let dimensions_field = self.schema.get_field("dimensions").unwrap();
        let time_dimensions_field = self.schema.get_field("time_dimensions").unwrap();
        let content_field = self.schema.get_field("content").unwrap();

        let mut writer = self.writer.blocking_write();

        let measures_text = model
            .measures
            .iter()
            .map(|m| m.formula.expression.clone())
            .collect::<Vec<_>>()
            .join(" ");
        let dimensions_text = model
            .dimensions
            .iter()
            .map(|d| d.name.clone())
            .collect::<Vec<_>>()
            .join(" ");
        let time_dims_text = model
            .time_dimensions
            .iter()
            .map(|t| t.dimension.clone())
            .collect::<Vec<_>>()
            .join(" ");

        let content_text = format!(
            "{} {} {} {} {}",
            model.name,
            model.description.as_ref().map(|s| s.as_str()).unwrap_or(""),
            measures_text,
            dimensions_text,
            time_dims_text
        );

        writer
            .add_document(doc!(
                model_name_field => model.name.as_str(),
                datasource_field => model.datasource.as_str(),
                description_field => model.description.as_deref().unwrap_or(""),
                measures_field => measures_text.as_str(),
                dimensions_field => dimensions_text.as_str(),
                time_dimensions_field => time_dims_text.as_str(),
                content_field => content_text.as_str(),
            ))
            .map_err(|e| StorageError::TantivyError(e.to_string()))?;

        writer
            .commit()
            .map_err(|e| StorageError::TantivyError(e.to_string()))?;
        Ok(())
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
}

#[async_trait::async_trait]
impl StorageBackend for TantivyStorage {
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

        // Index in Tantivy
        self.index_model_sync(&model)?;

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

        // Re-index
        self.index_model_sync(&model)?;

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
            // Note: Tantivy doesn't support easy deletion, would need to rebuild index
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

    async fn index_model(&self, model: &Model) -> Result<(), StorageError> {
        self.index_model_sync(model)
    }

    async fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchResult>, StorageError> {
        let reader = self
            .index
            .reader_builder()
            .reload_policy(ReloadPolicy::OnCommitWithDelay)
            .try_into()
            .map_err(|e| StorageError::Internal(format!("Failed to create reader: {}", e)))?;

        let searcher = reader.searcher();

        let model_name_field = self.schema.get_field("model_name").unwrap();
        let datasource_field = self.schema.get_field("datasource").unwrap();
        let description_field = self.schema.get_field("description").unwrap();
        let content_field = self.schema.get_field("content").unwrap();

        // Parse query
        let query_parser = tantivy::query::QueryParser::for_index(&self.index, vec![content_field]);
        let query = query_parser
            .parse_query(query)
            .map_err(|e| StorageError::QueryParserError(e.to_string()))?;

        // Search using TopDocs collector
        let collector = TopDocs::with_limit(limit).order_by_score();
        let top_docs = searcher
            .search(&query, &collector)
            .map_err(|e| StorageError::TantivyError(e.to_string()))?;

        let top_docs: Vec<(f32, tantivy::DocAddress)> = top_docs;

        let mut results = Vec::new();
        for (score, doc_address) in top_docs.into_iter() {
            let doc_address: tantivy::DocAddress = doc_address;
            let doc: tantivy::TantivyDocument = searcher
                .doc(doc_address)
                .map_err(|e| StorageError::TantivyError(e.to_string()))?;
            let model_name = doc
                .get_first(model_name_field)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let datasource = doc
                .get_first(datasource_field)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let snippet = doc
                .get_first(description_field)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            // Get matched fields (simplified)
            let matched_fields = vec!["content".to_string()];

            results.push(SearchResult {
                model_name,
                datasource,
                score,
                matched_fields,
                snippet,
            });
        }

        Ok(results)
    }
}
