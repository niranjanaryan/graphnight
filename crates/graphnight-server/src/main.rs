use clap::Parser;
use graphnight_core::models::DataSource;
use graphnight_graphql::build_schema;
use graphnight_sql::{
    dialects::get_dialect,
    executor::{ConnectionManager, QueryExecutor},
    SqlEngine,
};
use graphnight_storage::{StorageBackend, YamlStorage};
use std::sync::Arc;
use tracing::{info, Level};
use tracing_subscriber::FmtSubscriber;

#[derive(Parser, Debug)]
#[command(
    name = "graphnight-server",
    version,
    about = "GraphNight Semantic Layer Server"
)]
struct Args {
    /// Configuration file path
    #[arg(short, long, default_value = "graphnight.toml")]
    config: String,

    /// Host to bind to
    #[arg(long, default_value = "0.0.0.0")]
    host: String,

    /// Port to bind to
    #[arg(long, default_value = "8080")]
    port: u16,

    /// Storage path
    #[arg(long, default_value = "./graphnight_data")]
    storage_path: String,

    /// Log level
    #[arg(long, default_value = "info")]
    log_level: String,
}

#[derive(serde::Deserialize, Debug)]
struct Config {
    server: ServerConfig,
    storage: StorageConfig,
    datasources: Option<std::collections::HashMap<String, DataSourceConfig>>,
}

#[derive(serde::Deserialize, Debug)]
struct ServerConfig {
    host: Option<String>,
    port: Option<u16>,
    _workers: Option<usize>,
}

#[derive(serde::Deserialize, Debug)]
struct StorageConfig {
    #[serde(rename = "type")]
    storage_type: Option<String>,
    path: Option<String>,
}

#[derive(serde::Deserialize, Debug)]
struct DataSourceConfig {
    driver: String,
    connection_string: String,
    description: Option<String>,
    pool_size: Option<u32>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    // Initialize tracing
    let log_level = args.log_level.parse::<Level>().unwrap_or(Level::INFO);
    let subscriber = FmtSubscriber::builder().with_max_level(log_level).finish();
    tracing::subscriber::set_global_default(subscriber)?;

    info!("Starting GraphNight server...");

    // Load config
    let config = load_config(&args.config).await?;

    // Determine host and port
    let host = config.server.host.as_deref().unwrap_or(&args.host);
    let port = config.server.port.unwrap_or(args.port);

    // Initialize storage
    let storage_path = config.storage.path.as_deref().unwrap_or(&args.storage_path);
    let storage: Arc<dyn StorageBackend> =
        match config.storage.storage_type.as_deref().unwrap_or("yaml") {
            "yaml" => {
                let storage = Arc::new(YamlStorage::new(storage_path)?);
                storage.load().await?;
                storage
            }
            "sqlite" => Arc::new(graphnight_storage::SqliteStorage::new(storage_path).await?),
            _ => {
                let storage = Arc::new(YamlStorage::new(storage_path)?);
                storage.load().await?;
                storage
            }
        };

    // Initialize connection manager
    let conn_manager = Arc::new(ConnectionManager::new());

    // Register datasources from config before building the SQL engine
    if let Some(datasources) = config.datasources {
        for (name, ds_config) in datasources {
            if storage.get_datasource(&name).await?.is_some() {
                info!(
                    "Datasource '{}' already present; skipping config create",
                    name
                );
                continue;
            }
            let ds = DataSource {
                name,
                driver: ds_config.driver,
                connection_string: ds_config.connection_string,
                description: ds_config.description,
                models: vec![],
                pool_size: ds_config.pool_size,
                meta: std::collections::HashMap::new(),
            };
            storage.create_datasource(ds).await?;
        }
    }

    // Create SQL engine with models from storage
    let dialect = get_dialect("postgres");
    let executor = Arc::new(QueryExecutor::new(conn_manager.clone()));
    let models = storage.list_models(None).await?;
    let sql_engine = Arc::new(SqlEngine::new(dialect, executor)?.with_models(models));

    // Build GraphQL schema
    let schema = build_schema(sql_engine.clone(), storage.clone());

    // Create Tide app
    let mut app = tide::with_state(schema.clone());

    // GraphQL endpoint
    app.at("/graphql")
        .post(async_graphql_tide::graphql(schema.clone()));
    // GraphQL Playground not available in async-graphql-tide 7.x
    // app.at("/graphql").get(async_graphql_tide::graphql_playground(...));

    // Health check
    app.at("/health").get(|_| async { Ok("OK") });

    // Metrics endpoint
    app.at("/metrics")
        .get(|_| async { Ok("Metrics not implemented yet") });

    // Start server
    let addr = format!("{}:{}", host, port);
    info!("Server listening on {}", addr);
    app.listen(addr).await?;

    Ok(())
}

async fn load_config(path: &str) -> anyhow::Result<Config> {
    if !std::path::Path::new(path).exists() {
        // Return default config
        return Ok(Config {
            server: ServerConfig {
                host: Some("0.0.0.0".to_string()),
                port: Some(8080),
                _workers: Some(4),
            },
            storage: StorageConfig {
                storage_type: Some("yaml".to_string()),
                path: Some("./graphnight_data".to_string()),
            },
            datasources: None,
        });
    }

    let content = tokio::fs::read_to_string(path).await?;
    let config: Config = toml::from_str(&content)?;
    Ok(config)
}
