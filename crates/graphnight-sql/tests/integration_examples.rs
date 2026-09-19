use graphnight_core::models::Query;
use graphnight_sql::dialects::PostgresDialect;
use graphnight_sql::generator::SqlGenerator;
use graphnight_storage::{StorageBackend, YamlStorage};
use std::path::PathBuf;

fn examples_data_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/data")
}

#[tokio::test]
async fn examples_yaml_loads_and_dry_run_sql() {
    let data_dir = examples_data_dir();
    assert!(
        data_dir.join("models.yaml").exists(),
        "expected examples at {:?}",
        data_dir
    );

    let storage = YamlStorage::new(&data_dir).unwrap();
    storage.load().await.unwrap();
    let models = storage.list_models(None).await.unwrap();
    assert!(models.iter().any(|m| m.name == "orders"));
    assert!(models.iter().any(|m| m.name == "customers"));

    let query_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/query.json");
    let query: Query = serde_json::from_str(&std::fs::read_to_string(query_path).unwrap()).unwrap();

    let generator = SqlGenerator::new(Box::new(PostgresDialect)).with_models(models);
    let sql = generator.generate(&query).unwrap();

    assert!(sql.contains("SUM"));
    assert!(sql.contains("COUNT"));
    assert!(sql.contains("\"amount_usd\"") || sql.contains("amount_usd"));
    assert!(sql.contains("GROUP BY"));
    assert!(sql.contains("LIMIT 100"));
}
