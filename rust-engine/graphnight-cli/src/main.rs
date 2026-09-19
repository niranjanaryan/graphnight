use clap::{Parser, Subcommand};
use graphnight_core::models::{DataSource, Model, Query as CoreQuery};
use graphnight_sql::{
    dialects::get_dialect,
    executor::{ConnectionManager, QueryExecutor},
    SqlEngine,
};
use graphnight_storage::{Memory, MemoryFilter, StorageBackend, YamlStorage};
use std::sync::Arc;
use tabled::{Table, Tabled};
use tracing::{info, Level};
use tracing_subscriber::FmtSubscriber;

#[derive(Parser, Debug)]
#[command(
    name = "graphnight",
    version,
    about = "GraphNight CLI - Semantic Layer for AI Agents"
)]
struct Cli {
    /// Configuration file
    #[arg(short, long, default_value = "graphnight.toml")]
    config: String,

    /// Storage path
    #[arg(long, default_value = "./graphnight_data")]
    storage_path: String,

    /// Log level
    #[arg(long, default_value = "info")]
    log_level: String,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Query commands
    Query {
        #[command(subcommand)]
        action: QueryCommands,
    },
    /// Model management
    Model {
        #[command(subcommand)]
        action: ModelCommands,
    },
    /// Datasource management
    Datasource {
        #[command(subcommand)]
        action: DatasourceCommands,
    },
    /// Memory management
    Memory {
        #[command(subcommand)]
        action: MemoryCommands,
    },
    /// Search models
    Search {
        /// Search query
        query: String,
        /// Limit results
        #[arg(short, long, default_value = "10")]
        limit: usize,
    },
    /// Generate SQL from query
    Sql {
        /// Query file (JSON)
        #[arg(short, long)]
        file: String,
    },
    /// Start the server
    Serve {
        /// Host
        #[arg(long, default_value = "0.0.0.0")]
        host: String,
        /// Port
        #[arg(long, default_value = "8080")]
        port: u16,
    },
}

#[derive(Subcommand, Debug)]
enum QueryCommands {
    /// Execute a query from JSON file
    Run {
        /// Query file (JSON)
        #[arg(short, long)]
        file: String,
        /// Output format
        #[arg(short, long, default_value = "table")]
        format: String,
    },
    /// Dry run - generate SQL only
    DryRun {
        /// Query file (JSON)
        #[arg(short, long)]
        file: String,
    },
}

#[derive(Subcommand, Debug)]
enum ModelCommands {
    /// List models
    List {
        /// Filter by datasource
        #[arg(short, long)]
        datasource: Option<String>,
    },
    /// Get a model
    Get {
        /// Model name
        name: String,
        /// Datasource
        #[arg(short, long)]
        datasource: Option<String>,
    },
    /// Create a model
    Create {
        /// Model file (YAML)
        #[arg(short, long)]
        file: String,
    },
    /// Delete a model
    Delete {
        /// Model name
        name: String,
        /// Datasource
        #[arg(short, long)]
        datasource: Option<String>,
    },
}

#[derive(Subcommand, Debug)]
enum DatasourceCommands {
    /// List datasources
    List,
    /// Get a datasource
    Get {
        /// Datasource name
        name: String,
    },
    /// Create a datasource
    Create {
        /// Datasource file (YAML)
        #[arg(short, long)]
        file: String,
    },
    /// Delete a datasource
    Delete {
        /// Datasource name
        name: String,
    },
}

#[derive(Subcommand, Debug)]
enum MemoryCommands {
    /// List memories
    List {
        /// Search query
        #[arg(short, long)]
        query: Option<String>,
        /// Filter by entity
        #[arg(short, long)]
        entity: Option<String>,
        /// Limit results
        #[arg(short, long, default_value = "10")]
        limit: usize,
    },
    /// Get a memory
    Get {
        /// Memory ID
        id: String,
    },
    /// Save a memory
    Save {
        /// Learning text
        learning: String,
        /// Linked entities (comma-separated)
        #[arg(short, long)]
        entities: String,
        /// Memory ID (optional)
        #[arg(short, long)]
        id: Option<String>,
        /// Description (optional)
        #[arg(short, long)]
        description: Option<String>,
    },
    /// Delete a memory
    Delete {
        /// Memory ID
        id: String,
    },
}

#[derive(Tabled)]
struct ModelRow {
    name: String,
    datasource: String,
    description: String,
    measures: usize,
    dimensions: usize,
}

#[derive(Tabled)]
struct DatasourceRow {
    name: String,
    driver: String,
    description: String,
    models: usize,
}

#[derive(Tabled)]
struct MemoryRow {
    id: String,
    learning: String,
    entities: String,
    created_at: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    // Initialize tracing
    let log_level = cli.log_level.parse::<Level>().unwrap_or(Level::INFO);
    let subscriber = FmtSubscriber::builder().with_max_level(log_level).finish();
    tracing::subscriber::set_global_default(subscriber)?;

    // Initialize storage
    let storage = Arc::new(YamlStorage::new(&cli.storage_path)?);
    storage.load().await?;
    let storage_backend: Arc<dyn StorageBackend> = storage.clone();

    // Initialize SQL engine with models from storage
    let dialect = get_dialect("postgres");
    let conn_manager = Arc::new(ConnectionManager::new());
    let executor = Arc::new(QueryExecutor::new(conn_manager));
    let models = storage_backend.list_models(None).await?;
    let sql_engine = Arc::new(SqlEngine::new(dialect, executor)?.with_models(models));

    match cli.command {
        Commands::Query { action } => handle_query(action, &sql_engine, &storage_backend).await?,
        Commands::Model { action } => handle_model(action, &storage_backend).await?,
        Commands::Datasource { action } => handle_datasource(action, &storage_backend).await?,
        Commands::Memory { action } => handle_memory(action, &storage_backend).await?,
        Commands::Search { query, limit } => handle_search(&storage_backend, &query, limit).await?,
        Commands::Sql { file } => handle_sql(&sql_engine, &file).await?,
        Commands::Serve { host, port } => handle_serve(&host, port).await?,
    }

    Ok(())
}

async fn handle_query(
    action: QueryCommands,
    sql_engine: &Arc<SqlEngine>,
    storage: &Arc<dyn StorageBackend>,
) -> anyhow::Result<()> {
    match action {
        QueryCommands::Run { file, format } => {
            let content = tokio::fs::read_to_string(&file).await?;
            let query: CoreQuery = serde_json::from_str(&content)?;

            let model_name = query
                .name
                .as_ref()
                .or_else(|| query.source_model.as_ref().map(|s| &s.model))
                .ok_or_else(|| anyhow::anyhow!("Query must have name or source_model"))?;

            let model = storage
                .get_model(model_name, None)
                .await?
                .ok_or_else(|| anyhow::anyhow!("Model not found: {}", model_name))?;

            let datasource = storage
                .get_datasource(&model.datasource)
                .await?
                .ok_or_else(|| anyhow::anyhow!("Datasource not found: {}", model.datasource))?;

            let sql = sql_engine.generate_sql(&query)?;
            let results = sql_engine.execute_sqlx(&datasource, &sql).await?;

            match format.as_str() {
                "json" => println!("{}", serde_json::to_string_pretty(&results)?),
                "table" => {
                    if !results.is_empty() {
                        // Convert to table format
                        let headers: Vec<String> = results[0].keys().cloned().collect();
                        println!("{}", headers.join("\t"));
                        for row in results {
                            let values: Vec<String> = headers
                                .iter()
                                .map(|h| row.get(h).map(|v| v.to_string()).unwrap_or_default())
                                .collect();
                            println!("{}", values.join("\t"));
                        }
                    }
                }
                _ => println!("Unknown format: {}", format),
            }
        }
        QueryCommands::DryRun { file } => {
            let content = tokio::fs::read_to_string(&file).await?;
            let query: CoreQuery = serde_json::from_str(&content)?;
            let sql = sql_engine.generate_sql(&query)?;
            println!("{}", sql);
        }
    }
    Ok(())
}

async fn handle_model(
    action: ModelCommands,
    storage: &Arc<dyn StorageBackend>,
) -> anyhow::Result<()> {
    match action {
        ModelCommands::List { datasource } => {
            let models = storage.list_models(datasource.as_deref()).await?;
            let rows: Vec<ModelRow> = models
                .into_iter()
                .map(|m| ModelRow {
                    name: m.name,
                    datasource: m.datasource,
                    description: m.description.unwrap_or_default(),
                    measures: m.measures.len(),
                    dimensions: m.dimensions.len(),
                })
                .collect();
            println!("{}", Table::new(rows));
        }
        ModelCommands::Get { name, datasource } => {
            let model = storage.get_model(&name, datasource.as_deref()).await?;
            if let Some(m) = model {
                println!("{}", serde_yaml::to_string(&m)?);
            } else {
                println!("Model not found: {}", name);
            }
        }
        ModelCommands::Create { file } => {
            let content = tokio::fs::read_to_string(&file).await?;
            let model: Model = serde_yaml::from_str(&content)?;
            let created = storage.create_model(model).await?;
            println!("Created model: {}", created.name);
        }
        ModelCommands::Delete { name, datasource } => {
            let deleted = storage.delete_model(&name, datasource.as_deref()).await?;
            if deleted {
                println!("Deleted model: {}", name);
            } else {
                println!("Model not found: {}", name);
            }
        }
    }
    Ok(())
}

async fn handle_datasource(
    action: DatasourceCommands,
    storage: &Arc<dyn StorageBackend>,
) -> anyhow::Result<()> {
    match action {
        DatasourceCommands::List => {
            let datasources = storage.list_datasources().await?;
            let rows: Vec<DatasourceRow> = datasources
                .into_iter()
                .map(|d| DatasourceRow {
                    name: d.name,
                    driver: d.driver,
                    description: d.description.unwrap_or_default(),
                    models: d.models.len(),
                })
                .collect();
            println!("{}", Table::new(rows));
        }
        DatasourceCommands::Get { name } => {
            let ds = storage.get_datasource(&name).await?;
            if let Some(d) = ds {
                println!("{}", serde_yaml::to_string(&d)?);
            } else {
                println!("Datasource not found: {}", name);
            }
        }
        DatasourceCommands::Create { file } => {
            let content = tokio::fs::read_to_string(&file).await?;
            let ds: DataSource = serde_yaml::from_str(&content)?;
            let created = storage.create_datasource(ds).await?;
            println!("Created datasource: {}", created.name);
        }
        DatasourceCommands::Delete { name } => {
            let deleted = storage.delete_datasource(&name).await?;
            if deleted {
                println!("Deleted datasource: {}", name);
            } else {
                println!("Datasource not found: {}", name);
            }
        }
    }
    Ok(())
}

async fn handle_memory(
    action: MemoryCommands,
    storage: &Arc<dyn StorageBackend>,
) -> anyhow::Result<()> {
    match action {
        MemoryCommands::List {
            query,
            entity,
            limit,
        } => {
            let filter = MemoryFilter {
                query,
                entity,
                limit: Some(limit),
                offset: None,
            };
            let memories = storage.list_memories(filter).await?;
            let rows: Vec<MemoryRow> = memories
                .into_iter()
                .map(|m| MemoryRow {
                    id: m.id,
                    learning: m.learning,
                    entities: m.linked_entities.join(", "),
                    created_at: m.created_at.to_rfc3339(),
                })
                .collect();
            println!("{}", Table::new(rows));
        }
        MemoryCommands::Get { id } => {
            let memory = storage.get_memory(&id).await?;
            if let Some(m) = memory {
                println!("{}", serde_yaml::to_string(&m)?);
            } else {
                println!("Memory not found: {}", id);
            }
        }
        MemoryCommands::Save {
            learning,
            entities,
            id,
            description,
        } => {
            let memory = Memory {
                id: id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
                learning,
                linked_entities: entities.split(',').map(|s| s.trim().to_string()).collect(),
                description,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
                meta: std::collections::HashMap::new(),
            };
            let saved = storage.save_memory(memory).await?;
            println!("Saved memory: {}", saved.id);
        }
        MemoryCommands::Delete { id } => {
            let deleted = storage.delete_memory(&id).await?;
            if deleted {
                println!("Deleted memory: {}", id);
            } else {
                println!("Memory not found: {}", id);
            }
        }
    }
    Ok(())
}

async fn handle_search(
    storage: &Arc<dyn StorageBackend>,
    query: &str,
    limit: usize,
) -> anyhow::Result<()> {
    let results = storage.search(query, limit).await?;
    for r in results {
        println!("Model: {} (score: {:.2})", r.model_name, r.score);
        println!("  Datasource: {}", r.datasource);
        println!("  Matched: {}", r.matched_fields.join(", "));
        println!("  Snippet: {}", r.snippet);
        println!();
    }
    Ok(())
}

async fn handle_sql(sql_engine: &Arc<SqlEngine>, file: &str) -> anyhow::Result<()> {
    let content = tokio::fs::read_to_string(file).await?;
    let query: CoreQuery = serde_json::from_str(&content)?;
    let sql = sql_engine.generate_sql(&query)?;
    println!("{}", sql);
    Ok(())
}

async fn handle_serve(host: &str, port: u16) -> anyhow::Result<()> {
    info!("Starting server on {}:{}", host, port);
    // This would start the full server - for now just print
    println!("Server would start on {}:{}", host, port);
    println!("Use 'graphnight-server' binary for production server");
    Ok(())
}
