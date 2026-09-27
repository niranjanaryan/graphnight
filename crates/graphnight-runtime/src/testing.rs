//! Fixtures for testing governed execution and the agent loop.
//!
//! Public on purpose: an integration test in another crate needs to build a
//! `QueryService` against real storage, and re-implementing a model fixture in
//! every test module is how fixtures drift apart.
//!
//! Everything here is in-memory or a temp directory. Nothing in this module
//! reaches a real warehouse, which is the point — a test must not be able to
//! accidentally query production because a fixture was wired up eagerly.

use crate::governance::{CallContext, QueryService, ServiceError};
use graphnight_core::models::{
    AggregationType, DataSource, Dimension, Filter, FilterOperator, Formula, Join, JoinType,
    Measure, Model, TimeDimension, TimeGranularity,
};
use graphnight_core::security::SessionPolicy;
use graphnight_sql::SqlEngine;
use graphnight_storage::{MemoryFilter, StorageBackend};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

/// The datasource every fixture model points at.
pub const TEST_DATASOURCE: &str = "warehouse";

/// A datasource that exists but cannot actually connect.
///
/// Dry runs and metadata tools work against it; a real execution correctly
/// fails, which is what keeps a test from reaching a network.
pub fn test_datasource() -> DataSource {
    DataSource {
        name: TEST_DATASOURCE.to_string(),
        driver: "postgres".to_string(),
        connection_string: "postgres://invalid:invalid@127.0.0.1:1/none".to_string(),
        description: Some("Test datasource".to_string()),
        models: vec!["orders".to_string()],
        pool_size: Some(5),
        meta: HashMap::new(),
    }
}

/// An `orders` model with one measure, one dimension and one time dimension.
///
/// Deliberately small: a fixture with every feature makes failures hard to read.
pub fn orders_model() -> Model {
    Model {
        name: "orders".to_string(),
        datasource: TEST_DATASOURCE.to_string(),
        description: Some("One row per order".to_string()),
        measures: vec![Measure {
            formula: Formula {
                expression: "revenue".to_string(),
                label: Some("Revenue".to_string()),
                format: None,
            },
            aggregation: AggregationType::Sum,
        }],
        dimensions: vec![
            Dimension {
                name: "status".to_string(),
                label: Some("Status".to_string()),
            },
            Dimension {
                name: "region".to_string(),
                label: Some("Region".to_string()),
            },
        ],
        time_dimensions: vec![TimeDimension {
            dimension: "created_at".to_string(),
            granularity: TimeGranularity::Day,
            label: Some("Created at".to_string()),
        }],
        joins: vec![],
        sql: None,
        meta: HashMap::new(),
    }
}

/// A `customers` model, so join and multi-model paths have something to work with.
pub fn customers_model() -> Model {
    Model {
        name: "customers".to_string(),
        datasource: TEST_DATASOURCE.to_string(),
        description: Some("One row per customer".to_string()),
        measures: vec![Measure {
            formula: Formula {
                expression: "*".to_string(),
                label: Some("Customers".to_string()),
                format: None,
            },
            aggregation: AggregationType::Count,
        }],
        dimensions: vec![Dimension {
            name: "country".to_string(),
            label: Some("Country".to_string()),
        }],
        time_dimensions: vec![TimeDimension {
            dimension: "signup_date".to_string(),
            granularity: TimeGranularity::Month,
            label: None,
        }],
        joins: vec![Join {
            name: "orders_customer".to_string(),
            model: "customers".to_string(),
            join_type: JoinType::Left,
            on: vec![("customer_id".to_string(), "id".to_string())],
            alias: None,
        }],
        sql: None,
        meta: HashMap::new(),
    }
}

/// Every fixture model.
pub fn test_models() -> Vec<Model> {
    vec![orders_model(), customers_model()]
}

/// A `SqlEngine` wired to the fixture models and datasource.
///
/// The executor is a real one over an empty connection manager: generation and
/// policy paths work, and an actual execution fails to connect, which is what
/// keeps these fixtures off any real database.
pub fn test_sql_engine() -> SqlEngine {
    let dialect = graphnight_sql::dialects::get_dialect("postgres");
    let executor = Arc::new(graphnight_sql::executor::QueryExecutor::new(Arc::new(
        graphnight_sql::executor::ConnectionManager::new(),
    )));
    SqlEngine::new(dialect, executor)
        .expect("build test engine")
        .with_models(test_models())
        .with_datasources(vec![test_datasource()])
}

/// An in-memory storage backend seeded with the fixtures.
///
/// Pure in-memory rather than `YamlStorage` in a temp directory: it needs no
/// filesystem, no cleanup, and no blocking executor just to seed it, so tests
/// stay fast and cannot interfere with each other. `StubSearch` stands in for
/// the Tantivy index, which is what `search_models` reads.
pub fn test_storage() -> Arc<dyn StorageBackend> {
    Arc::new(StubStorage {
        models: RwLock::new(test_models()),
        datasources: RwLock::new(vec![test_datasource()]),
        memories: RwLock::new(vec![]),
    })
}

/// A `QueryService` over the fixtures, ready to use in a test.
pub fn test_service() -> Arc<QueryService> {
    Arc::new(QueryService::new(
        Arc::new(test_sql_engine()),
        test_storage(),
    ))
}

/// An authenticated, unrestricted, non-admin context.
pub fn open_context() -> CallContext {
    CallContext {
        user_id: Some("test-user".to_string()),
        tenant_id: Some("test-tenant".to_string()),
        is_admin: false,
        auth_required: true,
        policy: Some(SessionPolicy::new()),
        principal_label: Some("test".to_string()),
    }
}

/// An authenticated admin, for semantic-layer mutation tests.
pub fn admin_context() -> CallContext {
    CallContext {
        user_id: Some("root".into()),
        is_admin: true,
        auth_required: true,
        ..CallContext::default()
    }
}

pub fn orders_only_context() -> CallContext {
    CallContext {
        user_id: Some("restricted".to_string()),
        tenant_id: Some("test-tenant".to_string()),
        is_admin: false,
        auth_required: true,
        policy: Some(SessionPolicy::new().with_allowed_models(vec!["orders".to_string()])),
        principal_label: None,
    }
}

/// A context whose row filter forces `status = 'completed'`.
pub fn completed_only_context() -> CallContext {
    CallContext {
        user_id: Some("rls-user".to_string()),
        tenant_id: Some("test-tenant".to_string()),
        is_admin: false,
        auth_required: true,
        policy: Some(SessionPolicy::new().with_row_filter(Filter {
            field: "status".to_string(),
            operator: FilterOperator::Eq,
            value: Value::String("completed".to_string()),
            or_condition: false,
        })),
        principal_label: None,
    }
}

struct StubStorage {
    models: RwLock<Vec<Model>>,
    datasources: RwLock<Vec<DataSource>>,
    memories: RwLock<Vec<graphnight_storage::Memory>>,
}

#[async_trait::async_trait]
impl StorageBackend for StubStorage {
    async fn list_models(
        &self,
        datasource: Option<&str>,
    ) -> Result<Vec<Model>, graphnight_core::errors::StorageError> {
        Ok(self
            .models
            .read()
            .expect("stub models poisoned")
            .iter()
            .filter(|m| datasource.is_none_or(|d| m.datasource == d))
            .cloned()
            .collect())
    }

    async fn get_model(
        &self,
        name: &str,
        _datasource: Option<&str>,
    ) -> Result<Option<Model>, graphnight_core::errors::StorageError> {
        Ok(self
            .models
            .read()
            .expect("stub models poisoned")
            .iter()
            .find(|m| m.name == name)
            .cloned())
    }

    async fn create_model(
        &self,
        model: Model,
    ) -> Result<Model, graphnight_core::errors::StorageError> {
        self.models
            .write()
            .expect("stub models poisoned")
            .push(model.clone());
        Ok(model)
    }

    async fn update_model(
        &self,
        name: &str,
        model: Model,
    ) -> Result<Model, graphnight_core::errors::StorageError> {
        let mut models = self.models.write().expect("stub models poisoned");
        if let Some(existing) = models.iter_mut().find(|m| m.name == name) {
            *existing = model.clone();
        }
        Ok(model)
    }

    async fn delete_model(
        &self,
        name: &str,
        _datasource: Option<&str>,
    ) -> Result<bool, graphnight_core::errors::StorageError> {
        let mut models = self.models.write().expect("stub models poisoned");
        let before = models.len();
        models.retain(|m| m.name != name);
        Ok(models.len() != before)
    }

    async fn list_datasources(
        &self,
    ) -> Result<Vec<DataSource>, graphnight_core::errors::StorageError> {
        Ok(self
            .datasources
            .read()
            .expect("stub datasources poisoned")
            .clone())
    }

    async fn get_datasource(
        &self,
        name: &str,
    ) -> Result<Option<DataSource>, graphnight_core::errors::StorageError> {
        Ok(self
            .datasources
            .read()
            .expect("stub datasources poisoned")
            .iter()
            .find(|d| d.name == name)
            .cloned())
    }

    async fn create_datasource(
        &self,
        ds: DataSource,
    ) -> Result<DataSource, graphnight_core::errors::StorageError> {
        self.datasources
            .write()
            .expect("stub datasources poisoned")
            .push(ds.clone());
        Ok(ds)
    }

    async fn update_datasource(
        &self,
        _name: &str,
        _ds: DataSource,
    ) -> Result<DataSource, graphnight_core::errors::StorageError> {
        Err(graphnight_core::errors::StorageError::BackendError(
            "stub cannot update datasources".to_string(),
        ))
    }

    async fn delete_datasource(
        &self,
        name: &str,
    ) -> Result<bool, graphnight_core::errors::StorageError> {
        let mut dss = self.datasources.write().expect("stub datasources poisoned");
        let before = dss.len();
        dss.retain(|d| d.name != name);
        Ok(dss.len() != before)
    }

    async fn get_datasource_priority(
        &self,
    ) -> Result<Vec<String>, graphnight_core::errors::StorageError> {
        Ok(vec![TEST_DATASOURCE.to_string()])
    }

    async fn set_datasource_priority(
        &self,
        _priority: Vec<String>,
    ) -> Result<(), graphnight_core::errors::StorageError> {
        Ok(())
    }

    async fn save_memory(
        &self,
        memory: graphnight_storage::Memory,
    ) -> Result<graphnight_storage::Memory, graphnight_core::errors::StorageError> {
        self.memories
            .write()
            .expect("stub memories poisoned")
            .push(memory.clone());
        Ok(memory)
    }

    async fn get_memory(
        &self,
        id: &str,
    ) -> Result<Option<graphnight_storage::Memory>, graphnight_core::errors::StorageError> {
        Ok(self
            .memories
            .read()
            .expect("stub memories poisoned")
            .iter()
            .find(|m| m.id == id)
            .cloned())
    }

    async fn list_memories(
        &self,
        filter: MemoryFilter,
    ) -> Result<Vec<graphnight_storage::Memory>, graphnight_core::errors::StorageError> {
        let memories = self
            .memories
            .read()
            .expect("stub memories poisoned")
            .clone();
        let mut out: Vec<graphnight_storage::Memory> = memories
            .into_iter()
            .filter(|m| {
                filter
                    .entity
                    .as_ref()
                    .is_none_or(|e| m.linked_entities.iter().any(|le| le == e))
            })
            .collect();
        if let Some(limit) = filter.limit {
            out.truncate(limit);
        }
        Ok(out)
    }

    async fn delete_memory(&self, id: &str) -> Result<bool, graphnight_core::errors::StorageError> {
        let mut memories = self.memories.write().expect("stub memories poisoned");
        let before = memories.len();
        memories.retain(|m| m.id != id);
        Ok(memories.len() != before)
    }

    async fn index_model(
        &self,
        _model: &Model,
    ) -> Result<(), graphnight_core::errors::StorageError> {
        Ok(())
    }

    async fn search(
        &self,
        _query: &str,
        limit: usize,
    ) -> Result<Vec<graphnight_storage::SearchResult>, graphnight_core::errors::StorageError> {
        // Honours `limit` like a real index would, so a test that depends on
        // the service over-fetching cannot accidentally pass.
        Ok(self
            .models
            .read()
            .expect("stub models poisoned")
            .iter()
            .map(|m| graphnight_storage::SearchResult {
                model_name: m.name.clone(),
                datasource: m.datasource.clone(),
                score: 1.0,
                matched_fields: vec!["name".to_string()],
                snippet: m.description.clone().unwrap_or_default(),
            })
            .take(limit)
            .collect())
    }
}

/// Convenience for asserting a governance error.
pub fn expect_policy_error(result: Result<impl Sized, ServiceError>) -> ServiceError {
    match result {
        Ok(_) => panic!("expected a policy violation"),
        Err(e) => e,
    }
}
