mod auth_config;

use async_graphql::http::GraphiQLSource;
use async_graphql_axum::{GraphQLRequest, GraphQLResponse, GraphQLSubscription};
use auth_config::{extract_api_key, extract_tenant, AuthConfig};
use axum::{
    extract::State,
    http::HeaderMap,
    response::{Html, IntoResponse},
    routing::{get, get_service},
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
use tracing::{info, warn, Level};
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

/// Shared server state. Per-request identity is injected in `graphql_handler`.
#[derive(Clone)]
struct AppState {
    schema: AppSchema,
    sql_engine: Arc<SqlEngine>,
    storage: Arc<dyn StorageBackend>,
    auth: AuthConfig,
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
    let auth = AuthConfig::from_env();
    if auth.auth_required {
        info!(
            "Auth required ({} API keys configured)",
            auth.api_keys.len()
        );
    } else {
        warn!(
            "GraphQL auth is OPEN. Set GRAPHNIGHT_API_KEYS / GRAPHNIGHT_ADMIN_KEYS \
             (or GRAPHNIGHT_AUTH_REQUIRED=1). Use GRAPHNIGHT_DEV_OPEN=1 to silence this."
        );
    }

    let state = AppState {
        schema,
        sql_engine,
        storage,
        auth,
    };

    let schema_for_ws = state.schema.clone();

    let app = Router::new()
        .route("/graphql", get(graphiql).post(graphql_handler))
        .route(
            "/graphql/ws",
            get_service(GraphQLSubscription::new(schema_for_ws)),
        )
        .route("/health", get(health))
        .route("/metrics", get(metrics))
        .layer(TraceLayer::new_for_http())
        .layer(CorsLayer::permissive())
        .with_state(state);

    let addr = format!("{host}:{port}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    info!("Server listening on http://{addr}");
    info!("GraphiQL: http://{addr}/graphql");
    info!("GraphQL WS: ws://{addr}/graphql/ws");
    axum::serve(listener, app).await?;

    Ok(())
}

/// GraphQL POST handler: resolve identity from headers into request-scoped context.
async fn graphql_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    req: GraphQLRequest,
) -> GraphQLResponse {
    let api_key = extract_api_key(&headers);
    let tenant_id = extract_tenant(&headers);
    let identity = state.auth.authenticate(api_key.as_deref(), tenant_id);

    let mut request_ctx = GraphQLContext::new(state.sql_engine.clone(), state.storage.clone())
        .with_auth_required(state.auth.auth_required)
        .with_policy(state.auth.default_policy(identity.as_ref()));

    if let Some(id) = identity {
        request_ctx = request_ctx
            .with_user(id.user_id, id.tenant_id)
            .with_admin(id.is_admin);
    }

    let request = req.into_inner().data(request_ctx);
    state.schema.execute(request).await.into()
}

async fn graphiql() -> impl IntoResponse {
    Html(GraphiQLSource::build().endpoint("/graphql").finish())
}

async fn health() -> &'static str {
    "OK"
}

async fn metrics(State(state): State<AppState>) -> String {
    state.sql_engine.metrics().render_prometheus()
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
