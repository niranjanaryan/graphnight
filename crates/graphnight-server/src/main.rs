use async_graphql::http::GraphiQLSource;
use async_graphql_axum::{GraphQLRequest, GraphQLResponse};
use axum::{
    extract::State,
    response::{Html, IntoResponse},
    routing::get,
    Router,
};
use clap::Parser;
use graphnight_core::models::DataSource;
use graphnight_graphql::{build_schema, context::GraphQLContext, AppSchema};
use graphnight_sql::{
    dialects::get_dialect,
    executor::{ConnectionManager, QueryExecutor},
    SqlEngine,
};
use graphnight_storage::{StorageBackend, YamlStorage};
use std::sync::Arc;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
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

    /// Host to bind to (overrides config when set)
    #[arg(long)]
    host: Option<String>,

    /// Port to bind to (overrides config when set)
    #[arg(long)]
    port: Option<u16>,

    /// Storage path (overrides config when set)
    #[arg(long)]
    storage_path: Option<String>,

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

/// Shared server state. Auth middleware (Track 3) will enrich per-request context
/// in `graphql_handler` without replacing this process-wide schema.
#[derive(Clone)]
struct AppState {
    schema: AppSchema,
    sql_engine: Arc<SqlEngine>,
    storage: Arc<dyn StorageBackend>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    let log_level = args.log_level.parse::<Level>().unwrap_or(Level::INFO);
    let subscriber = FmtSubscriber::builder().with_max_level(log_level).finish();
    tracing::subscriber::set_global_default(subscriber)?;

    info!("Starting GraphNight server...");

    let config = load_config(&args.config).await?;
    // Precedence: CLI flag > config file > built-in default
    let host = args
        .host
        .as_deref()
        .or(config.server.host.as_deref())
        .unwrap_or("0.0.0.0");
    let port = args.port.or(config.server.port).unwrap_or(8080);
    let default_storage = "./graphnight_data".to_string();
    let storage_path = args
        .storage_path
        .as_deref()
        .or(config.storage.path.as_deref())
        .unwrap_or(default_storage.as_str());
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

    let conn_manager = Arc::new(ConnectionManager::new());

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

    let dialect = get_dialect("postgres");
    let executor = Arc::new(QueryExecutor::new(conn_manager.clone()));
    let models = storage.list_models(None).await?;
    let sql_engine = Arc::new(SqlEngine::new(dialect, executor)?.with_models(models));

    let schema = build_schema(sql_engine.clone(), storage.clone());
    let state = AppState {
        schema,
        sql_engine,
        storage,
    };

    let app = Router::new()
        .route("/graphql", get(graphiql).post(graphql_handler))
        .route("/health", get(health))
        .route("/metrics", get(metrics))
        .layer(TraceLayer::new_for_http())
        .layer(CorsLayer::permissive())
        .with_state(state);

    let addr = format!("{host}:{port}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    info!("Server listening on http://{addr}");
    info!("GraphiQL: http://{addr}/graphql");
    axum::serve(listener, app).await?;

    Ok(())
}

/// GraphQL POST handler with a request-scoped context seam for future auth.
async fn graphql_handler(State(state): State<AppState>, req: GraphQLRequest) -> GraphQLResponse {
    // Track 3 will parse Authorization / X-API-Key here and call with_user / with_policy.
    let request_ctx = GraphQLContext::new(state.sql_engine.clone(), state.storage.clone());
    let request = req.into_inner().data(request_ctx);
    state.schema.execute(request).await.into()
}

async fn graphiql() -> impl IntoResponse {
    Html(GraphiQLSource::build().endpoint("/graphql").finish())
}

async fn health() -> &'static str {
    "OK"
}

async fn metrics() -> &'static str {
    "# GraphNight metrics placeholder\n# Prometheus exposition planned for v0.3\n"
}

async fn load_config(path: &str) -> anyhow::Result<Config> {
    if !std::path::Path::new(path).exists() {
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
