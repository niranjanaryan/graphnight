use graphnight_core::models::{DataSource, Model};
use graphnight_storage::{
    Memory, MemoryFilter, PostgresMetadataStorage, StorageBackend, YamlStorage,
    METADATA_DATABASE_URL_ENV,
};
use pretty_assertions::assert_eq;
use std::sync::Arc;
use tempfile::tempdir;

/// Compile-time smoke: Postgres backend implements `StorageBackend`.
#[test]
fn postgres_metadata_storage_implements_trait() {
    fn assert_backend<T: StorageBackend>() {}
    assert_backend::<PostgresMetadataStorage>();
}

#[test]
fn postgres_resolve_url_path_env_and_fallback() {
    // Sequential: process env is shared across parallel tests.
    std::env::set_var(
        "GRAPHNIGHT_TEST_META_URL",
        "postgresql://localhost/graphnight_meta",
    );
    let url = PostgresMetadataStorage::resolve_url(Some("env:GRAPHNIGHT_TEST_META_URL")).unwrap();
    assert_eq!(url, "postgresql://localhost/graphnight_meta");
    std::env::remove_var("GRAPHNIGHT_TEST_META_URL");

    let direct =
        PostgresMetadataStorage::resolve_url(Some("postgresql://user:pass@db:5432/meta")).unwrap();
    assert_eq!(direct, "postgresql://user:pass@db:5432/meta");

    std::env::set_var(
        METADATA_DATABASE_URL_ENV,
        "postgresql://user:pass@db:5432/meta",
    );
    let from_env = PostgresMetadataStorage::resolve_url(None).unwrap();
    assert_eq!(from_env, "postgresql://user:pass@db:5432/meta");
    std::env::remove_var(METADATA_DATABASE_URL_ENV);

    let err = PostgresMetadataStorage::resolve_url(None).unwrap_err();
    assert!(err.to_string().contains(METADATA_DATABASE_URL_ENV));
}

/// Integration smoke against a live Postgres.
///
/// Run with Docker compose profile `ha` or any Postgres, then:
/// `GRAPHNIGHT_METADATA_DATABASE_URL=postgresql://graphnight:graphnight@127.0.0.1:5433/graphnight_meta \
///    cargo test -p graphnight-storage postgres_metadata_storage_crud -- --ignored --nocapture`
///
/// Search is ILIKE-only (not Tantivy). Prefer testcontainers later when CI has Docker-in-Docker.
#[tokio::test]
#[ignore = "requires Postgres; set GRAPHNIGHT_METADATA_DATABASE_URL or start docker compose --profile ha"]
async fn postgres_metadata_storage_crud() {
    let url = PostgresMetadataStorage::resolve_url(None).unwrap_or_else(|_| {
        "postgresql://graphnight:graphnight@127.0.0.1:5433/graphnight_meta".to_string()
    });
    let storage = Arc::new(
        PostgresMetadataStorage::new(&url)
            .await
            .expect("connect postgres"),
    );

    let suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let model_name = format!("orders_{suffix}");
    let ds_name = format!("ds_{suffix}");

    let ds = DataSource {
        name: ds_name.clone(),
        driver: "postgres".to_string(),
        connection_string: "postgresql://localhost/db".to_string(),
        description: Some("test".to_string()),
        models: vec![],
        pool_size: Some(2),
        meta: Default::default(),
    };
    storage.create_datasource(ds.clone()).await.unwrap();
    assert!(storage.get_datasource(&ds_name).await.unwrap().is_some());

    let model = Model {
        name: model_name.clone(),
        datasource: ds_name.clone(),
        description: Some("Order facts".to_string()),
        measures: vec![],
        dimensions: vec![],
        time_dimensions: vec![],
        joins: vec![],
        sql: None,
        meta: Default::default(),
    };
    storage.create_model(model.clone()).await.unwrap();
    let got = storage
        .get_model(&model_name, Some(&ds_name))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(got.description, Some("Order facts".to_string()));

    let results = storage.search(&model_name, 5).await.unwrap();
    assert_eq!(results.len(), 1);

    let mem_id = format!("mem_{suffix}");
    storage
        .save_memory(Memory {
            id: mem_id.clone(),
            learning: "spike".to_string(),
            linked_entities: vec!["revenue".to_string()],
            description: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            meta: Default::default(),
        })
        .await
        .unwrap();
    assert!(storage.get_memory(&mem_id).await.unwrap().is_some());

    assert!(storage.delete_memory(&mem_id).await.unwrap());
    assert!(storage
        .delete_model(&model_name, Some(&ds_name))
        .await
        .unwrap());
    assert!(storage.delete_datasource(&ds_name).await.unwrap());
}

#[tokio::test]
async fn test_yaml_storage_models() {
    let dir = tempdir().unwrap();
    let storage = Arc::new(YamlStorage::new(dir.path()).unwrap());

    let model = Model {
        name: "orders".to_string(),
        datasource: "postgres".to_string(),
        description: Some("Order facts".to_string()),
        measures: vec![],
        dimensions: vec![],
        time_dimensions: vec![],
        joins: vec![],
        sql: None,
        meta: Default::default(),
    };

    // Create
    let created = storage.create_model(model.clone()).await.unwrap();
    assert_eq!(created.name, "orders");

    // Get
    let got = storage.get_model("orders", None).await.unwrap().unwrap();
    assert_eq!(got.name, "orders");
    assert_eq!(got.datasource, "postgres");

    // List
    let models = storage.list_models(None).await.unwrap();
    assert_eq!(models.len(), 1);

    // Update
    let mut updated = got.clone();
    updated.description = Some("Updated".to_string());
    let updated = storage.update_model("orders", updated).await.unwrap();
    assert_eq!(updated.description, Some("Updated".to_string()));

    // Delete
    let deleted = storage.delete_model("orders", None).await.unwrap();
    assert!(deleted);

    let got = storage.get_model("orders", None).await.unwrap();
    assert!(got.is_none());
}

#[tokio::test]
async fn test_yaml_storage_datasources() {
    let dir = tempdir().unwrap();
    let storage = Arc::new(YamlStorage::new(dir.path()).unwrap());

    let ds = DataSource {
        name: "postgres_primary".to_string(),
        driver: "postgres".to_string(),
        connection_string: "postgresql://localhost/db".to_string(),
        description: Some("Primary".to_string()),
        models: vec![],
        pool_size: Some(10),
        meta: Default::default(),
    };

    // Create
    let created = storage.create_datasource(ds.clone()).await.unwrap();
    assert_eq!(created.name, "postgres_primary");

    // Get
    let got = storage
        .get_datasource("postgres_primary")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(got.driver, "postgres");

    // List
    let datasources = storage.list_datasources().await.unwrap();
    assert_eq!(datasources.len(), 1);

    // Update
    let mut updated = got.clone();
    updated.description = Some("Updated".to_string());
    let updated = storage
        .update_datasource("postgres_primary", updated)
        .await
        .unwrap();
    assert_eq!(updated.description, Some("Updated".to_string()));

    // Delete
    let deleted = storage.delete_datasource("postgres_primary").await.unwrap();
    assert!(deleted);
}

#[tokio::test]
async fn test_yaml_storage_memories() {
    let dir = tempdir().unwrap();
    let storage = Arc::new(YamlStorage::new(dir.path()).unwrap());

    let memory = Memory {
        id: "mem_1".to_string(),
        learning: "Revenue spikes on Black Friday".to_string(),
        linked_entities: vec!["revenue:sum".to_string()],
        description: Some("Seasonal pattern".to_string()),
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        meta: Default::default(),
    };

    // Save
    let saved = storage.save_memory(memory.clone()).await.unwrap();
    assert_eq!(saved.id, "mem_1");

    // Get
    let got = storage.get_memory("mem_1").await.unwrap().unwrap();
    assert_eq!(got.learning, "Revenue spikes on Black Friday");

    // List
    let memories = storage
        .list_memories(MemoryFilter::default())
        .await
        .unwrap();
    assert_eq!(memories.len(), 1);

    // Filter by query
    let filtered = storage
        .list_memories(MemoryFilter {
            query: Some("Black Friday".to_string()),
            entity: None,
            limit: None,
            offset: None,
        })
        .await
        .unwrap();
    assert_eq!(filtered.len(), 1);

    // Filter by entity
    let filtered = storage
        .list_memories(MemoryFilter {
            query: None,
            entity: Some("revenue:sum".to_string()),
            limit: None,
            offset: None,
        })
        .await
        .unwrap();
    assert_eq!(filtered.len(), 1);

    // Pagination
    let limited = storage
        .list_memories(MemoryFilter {
            query: None,
            entity: None,
            limit: Some(1),
            offset: Some(0),
        })
        .await
        .unwrap();
    assert_eq!(limited.len(), 1);

    // Delete
    let deleted = storage.delete_memory("mem_1").await.unwrap();
    assert!(deleted);

    let got = storage.get_memory("mem_1").await.unwrap();
    assert!(got.is_none());
}

#[tokio::test]
async fn test_yaml_storage_search() {
    let dir = tempdir().unwrap();
    let storage = Arc::new(YamlStorage::new(dir.path()).unwrap());

    let model1 = Model {
        name: "orders".to_string(),
        datasource: "postgres".to_string(),
        description: Some("Order facts table".to_string()),
        measures: vec![
            Measure::simple("revenue", AggregationType::Sum),
            Measure::simple("orders", AggregationType::Count),
        ],
        dimensions: vec![Dimension::new("status")],
        time_dimensions: vec![],
        joins: vec![],
        sql: None,
        meta: Default::default(),
    };

    let model2 = Model {
        name: "customers".to_string(),
        datasource: "postgres".to_string(),
        description: Some("Customer dimension".to_string()),
        measures: vec![],
        dimensions: vec![Dimension::new("name")],
        time_dimensions: vec![],
        joins: vec![],
        sql: None,
        meta: Default::default(),
    };

    storage.create_model(model1).await.unwrap();
    storage.create_model(model2).await.unwrap();

    // Search by name
    let results = storage.search("orders", 10).await.unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].model_name, "orders");

    // Search by description
    let results = storage.search("facts", 10).await.unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].model_name, "orders");

    // Search by measure
    let results = storage.search("revenue", 10).await.unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].model_name, "orders");

    // Search with limit
    let results = storage.search("order", 1).await.unwrap();
    assert_eq!(results.len(), 1);
}

#[tokio::test]
async fn test_yaml_storage_priority() {
    let dir = tempdir().unwrap();
    let storage = Arc::new(YamlStorage::new(dir.path()).unwrap());

    let priority = vec!["ds1".to_string(), "ds2".to_string()];
    storage
        .set_datasource_priority(priority.clone())
        .await
        .unwrap();

    let got = storage.get_datasource_priority().await.unwrap();
    assert_eq!(got, priority);
}

#[tokio::test]
async fn test_yaml_storage_persistence() {
    let dir = tempdir().unwrap();
    let path = dir.path().to_path_buf();

    {
        let storage = Arc::new(YamlStorage::new(&path).unwrap());
        let model = Model {
            name: "orders".to_string(),
            datasource: "postgres".to_string(),
            description: Some("Order facts".to_string()),
            measures: vec![],
            dimensions: vec![],
            time_dimensions: vec![],
            joins: vec![],
            sql: None,
            meta: Default::default(),
        };
        storage.create_model(model).await.unwrap();
    }

    // Reload from disk
    {
        let storage = Arc::new(YamlStorage::new(&path).unwrap());
        storage.load().await.unwrap();
        let model = storage.get_model("orders", None).await.unwrap().unwrap();
        assert_eq!(model.name, "orders");
        assert_eq!(model.description, Some("Order facts".to_string()));
    }
}

#[tokio::test]
async fn test_yaml_storage_model_exists_error() {
    let dir = tempdir().unwrap();
    let storage = Arc::new(YamlStorage::new(dir.path()).unwrap());

    let model = Model {
        name: "orders".to_string(),
        datasource: "postgres".to_string(),
        description: None,
        measures: vec![],
        dimensions: vec![],
        time_dimensions: vec![],
        joins: vec![],
        sql: None,
        meta: Default::default(),
    };

    storage.create_model(model.clone()).await.unwrap();
    let result = storage.create_model(model).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_yaml_storage_model_not_found_error() {
    let dir = tempdir().unwrap();
    let storage = Arc::new(YamlStorage::new(dir.path()).unwrap());

    let model = Model {
        name: "orders".to_string(),
        datasource: "postgres".to_string(),
        description: None,
        measures: vec![],
        dimensions: vec![],
        time_dimensions: vec![],
        joins: vec![],
        sql: None,
        meta: Default::default(),
    };

    let result = storage.update_model("orders", model).await;
    assert!(result.is_err());

    let result = storage.delete_model("orders", None).await.unwrap();
    assert!(!result);
}

#[tokio::test]
async fn test_memory_filter_pagination() {
    let dir = tempdir().unwrap();
    let storage = Arc::new(YamlStorage::new(dir.path()).unwrap());

    for i in 0..10 {
        let memory = Memory {
            id: format!("mem_{}", i),
            learning: format!("Learning {}", i),
            linked_entities: vec![],
            description: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            meta: Default::default(),
        };
        storage.save_memory(memory).await.unwrap();
    }

    // Test offset and limit
    let page1 = storage
        .list_memories(MemoryFilter {
            query: None,
            entity: None,
            limit: Some(3),
            offset: Some(0),
        })
        .await
        .unwrap();
    assert_eq!(page1.len(), 3);

    let page2 = storage
        .list_memories(MemoryFilter {
            query: None,
            entity: None,
            limit: Some(3),
            offset: Some(3),
        })
        .await
        .unwrap();
    assert_eq!(page2.len(), 3);

    // Test offset beyond length
    let empty = storage
        .list_memories(MemoryFilter {
            query: None,
            entity: None,
            limit: Some(3),
            offset: Some(100),
        })
        .await
        .unwrap();
    assert_eq!(empty.len(), 0);
}

use graphnight_core::models::*;
