use anyhow::{anyhow, Result};
use async_stream::try_stream;
use futures::Stream;
use futures::StreamExt;
use graphnight_core::models::DataSource;
use graphnight_core::resolve_connection_string;
use serde_json::Value;
use sqlx::{Column, MySql, Pool, Postgres, Row, Sqlite};
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tracing::{debug, info};

/// Default statement timeout applied on Postgres/MySQL connections when possible.
const DEFAULT_STATEMENT_TIMEOUT: Duration = Duration::from_secs(300);

/// Connection pool manager for multiple datasources
pub struct ConnectionManager {
    pg_pools: Arc<RwLock<HashMap<String, Pool<Postgres>>>>,
    mysql_pools: Arc<RwLock<HashMap<String, Pool<MySql>>>>,
    sqlite_pools: Arc<RwLock<HashMap<String, Pool<Sqlite>>>>,
    statement_timeout: Duration,
}

impl ConnectionManager {
    pub fn new() -> Self {
        Self {
            pg_pools: Arc::new(RwLock::new(HashMap::new())),
            mysql_pools: Arc::new(RwLock::new(HashMap::new())),
            sqlite_pools: Arc::new(RwLock::new(HashMap::new())),
            statement_timeout: DEFAULT_STATEMENT_TIMEOUT,
        }
    }

    pub fn with_statement_timeout(mut self, timeout: Duration) -> Self {
        self.statement_timeout = timeout;
        self
    }

    fn resolved_connection_string(ds: &DataSource) -> Result<String> {
        resolve_connection_string(&ds.connection_string).map_err(|e| anyhow!(e))
    }

    /// Get or create a PostgreSQL pool
    pub async fn get_pg_pool(&self, ds: &DataSource) -> Result<Pool<Postgres>> {
        let mut pools = self.pg_pools.write().await;
        if let Some(pool) = pools.get(&ds.name) {
            return Ok(pool.clone());
        }

        let conn = Self::resolved_connection_string(ds)?;
        let pool_size = ds.pool_size.unwrap_or(10);
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(pool_size)
            .acquire_timeout(Duration::from_secs(30))
            .idle_timeout(Duration::from_secs(600))
            .connect(&conn)
            .await?;

        // Apply statement timeout for this session defaults via SET on first use in execute.
        pools.insert(ds.name.clone(), pool.clone());
        info!("Created PostgreSQL pool for datasource: {}", ds.name);
        Ok(pool)
    }

    /// Get or create a MySQL pool
    pub async fn get_mysql_pool(&self, ds: &DataSource) -> Result<Pool<MySql>> {
        let mut pools = self.mysql_pools.write().await;
        if let Some(pool) = pools.get(&ds.name) {
            return Ok(pool.clone());
        }

        let conn = Self::resolved_connection_string(ds)?;
        let pool_size = ds.pool_size.unwrap_or(10);
        let pool = sqlx::mysql::MySqlPoolOptions::new()
            .max_connections(pool_size)
            .acquire_timeout(Duration::from_secs(30))
            .idle_timeout(Duration::from_secs(600))
            .connect(&conn)
            .await?;

        pools.insert(ds.name.clone(), pool.clone());
        info!("Created MySQL pool for datasource: {}", ds.name);
        Ok(pool)
    }

    /// Get or create a SQLite pool
    pub async fn get_sqlite_pool(&self, ds: &DataSource) -> Result<Pool<Sqlite>> {
        let mut pools = self.sqlite_pools.write().await;
        if let Some(pool) = pools.get(&ds.name) {
            return Ok(pool.clone());
        }

        let conn = Self::resolved_connection_string(ds)?;
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .acquire_timeout(Duration::from_secs(30))
            .connect(&conn)
            .await?;

        pools.insert(ds.name.clone(), pool.clone());
        info!("Created SQLite pool for datasource: {}", ds.name);
        Ok(pool)
    }

    pub fn statement_timeout(&self) -> Duration {
        self.statement_timeout
    }

    /// Counts of currently open pools per driver (lazy-created on first query).
    pub async fn pool_counts(&self) -> PoolCounts {
        let postgres = self.pg_pools.read().await.len();
        let mysql = self.mysql_pools.read().await.len();
        let sqlite = self.sqlite_pools.read().await.len();
        PoolCounts {
            postgres,
            mysql,
            sqlite,
        }
    }

    /// Close all pools
    pub async fn close_all(&self) {
        let pg_pools = self.pg_pools.write().await;
        for (name, pool) in pg_pools.iter() {
            pool.close().await;
            info!("Closed PostgreSQL pool: {}", name);
        }
    }
}

/// Snapshot of open connection pools for health / ops endpoints.
#[derive(Debug, Clone, Copy, serde::Serialize)]
pub struct PoolCounts {
    pub postgres: usize,
    pub mysql: usize,
    pub sqlite: usize,
}

impl Default for ConnectionManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Query executor using connection pools
pub struct QueryExecutor {
    connection_manager: Arc<ConnectionManager>,
}

impl QueryExecutor {
    pub fn new(connection_manager: Arc<ConnectionManager>) -> Self {
        Self { connection_manager }
    }

    /// Execute a query and return results as JSON (buffered).
    pub async fn execute(&self, ds: &DataSource, sql: &str) -> Result<Vec<HashMap<String, Value>>> {
        self.execute_streaming_collect(ds, sql).await
    }

    /// Fetch rows via streaming API and collect into a Vec (avoids fetch_all bulk path).
    pub async fn execute_streaming_collect(
        &self,
        ds: &DataSource,
        sql: &str,
    ) -> Result<Vec<HashMap<String, Value>>> {
        let mut rows = Vec::new();
        let mut stream = self.execute_stream(ds, sql.to_string());
        while let Some(item) = stream.next().await {
            rows.push(item?);
        }
        Ok(rows)
    }

    /// Stream result rows one at a time.
    pub fn execute_stream<'a>(
        &'a self,
        ds: &'a DataSource,
        sql: String,
    ) -> Pin<Box<dyn Stream<Item = Result<HashMap<String, Value>>> + Send + 'a>> {
        let driver = ds.driver.clone();
        let ds_name = ds.name.clone();
        let timeout = self.connection_manager.statement_timeout();
        let cm = self.connection_manager.clone();

        Box::pin(try_stream! {
            debug!("Streaming SQL on {}: {}", ds_name, sql);
            match driver.as_str() {
                "postgres" | "postgresql" | "pg" => {
                    let pool = cm.get_pg_pool(ds).await?;
                    let mut conn = pool.acquire().await?;
                    let ms = timeout.as_millis();
                    sqlx::query(&format!("SET statement_timeout = {ms}"))
                        .execute(&mut *conn)
                        .await
                        .ok();
                    let mut rows = sqlx::query(&sql).fetch(&mut *conn);
                    while let Some(row) = rows.next().await {
                        let row = row?;
                        yield pg_row_to_map(&row)?;
                    }
                }
                "mysql" | "mariadb" => {
                    let pool = cm.get_mysql_pool(ds).await?;
                    let mut conn = pool.acquire().await?;
                    let ms = timeout.as_millis();
                    sqlx::query(&format!("SET SESSION max_execution_time = {ms}"))
                        .execute(&mut *conn)
                        .await
                        .ok();
                    let mut rows = sqlx::query(&sql).fetch(&mut *conn);
                    while let Some(row) = rows.next().await {
                        let row = row?;
                        yield mysql_row_to_map(&row)?;
                    }
                }
                "sqlite" | "sqlite3" => {
                    let pool = cm.get_sqlite_pool(ds).await?;
                    let mut rows = sqlx::query(&sql).fetch(&pool);
                    while let Some(row) = rows.next().await {
                        let row = row?;
                        yield sqlite_row_to_map(&row)?;
                    }
                }
                other => {
                    Err(anyhow!("Unsupported driver: {other}"))?;
                }
            }
        })
    }
}

fn pg_row_to_map(row: &sqlx::postgres::PgRow) -> Result<HashMap<String, Value>> {
    let mut map = HashMap::new();
    for col in row.columns() {
        let name = col.name().to_string();
        map.insert(name.clone(), pg_value_to_json(row, &name)?);
    }
    Ok(map)
}

fn mysql_row_to_map(row: &sqlx::mysql::MySqlRow) -> Result<HashMap<String, Value>> {
    let mut map = HashMap::new();
    for col in row.columns() {
        let name = col.name().to_string();
        map.insert(name.clone(), mysql_value_to_json(row, &name)?);
    }
    Ok(map)
}

fn sqlite_row_to_map(row: &sqlx::sqlite::SqliteRow) -> Result<HashMap<String, Value>> {
    let mut map = HashMap::new();
    for col in row.columns() {
        let name = col.name().to_string();
        map.insert(name.clone(), sqlite_value_to_json(row, &name)?);
    }
    Ok(map)
}

fn pg_value_to_json(row: &sqlx::postgres::PgRow, name: &str) -> Result<Value> {
    if let Ok(v) = row.try_get::<Option<String>, _>(name) {
        return Ok(match v {
            Some(s) => Value::String(s),
            None => Value::Null,
        });
    }
    if let Ok(v) = row.try_get::<Option<i64>, _>(name) {
        return Ok(match v {
            Some(n) => Value::Number(n.into()),
            None => Value::Null,
        });
    }
    if let Ok(v) = row.try_get::<Option<f64>, _>(name) {
        return Ok(match v {
            Some(num) => serde_json::Number::from_f64(num)
                .map(Value::Number)
                .unwrap_or(Value::Null),
            None => Value::Null,
        });
    }
    if let Ok(v) = row.try_get::<Option<bool>, _>(name) {
        return Ok(match v {
            Some(b) => Value::Bool(b),
            None => Value::Null,
        });
    }
    if let Ok(v) = row.try_get::<Option<serde_json::Value>, _>(name) {
        return Ok(v.unwrap_or(Value::Null));
    }
    if let Ok(v) = row.try_get::<Option<chrono::DateTime<chrono::Utc>>, _>(name) {
        return Ok(match v {
            Some(dt) => Value::String(dt.to_rfc3339()),
            None => Value::Null,
        });
    }
    Ok(Value::Null)
}

fn mysql_value_to_json(row: &sqlx::mysql::MySqlRow, name: &str) -> Result<Value> {
    if let Ok(v) = row.try_get::<Option<String>, _>(name) {
        return Ok(match v {
            Some(s) => Value::String(s),
            None => Value::Null,
        });
    }
    if let Ok(v) = row.try_get::<Option<i64>, _>(name) {
        return Ok(match v {
            Some(n) => Value::Number(n.into()),
            None => Value::Null,
        });
    }
    if let Ok(v) = row.try_get::<Option<f64>, _>(name) {
        return Ok(match v {
            Some(num) => serde_json::Number::from_f64(num)
                .map(Value::Number)
                .unwrap_or(Value::Null),
            None => Value::Null,
        });
    }
    if let Ok(v) = row.try_get::<Option<bool>, _>(name) {
        return Ok(match v {
            Some(b) => Value::Bool(b),
            None => Value::Null,
        });
    }
    Ok(Value::Null)
}

fn sqlite_value_to_json(row: &sqlx::sqlite::SqliteRow, name: &str) -> Result<Value> {
    if let Ok(v) = row.try_get::<Option<String>, _>(name) {
        return Ok(match v {
            Some(s) => Value::String(s),
            None => Value::Null,
        });
    }
    if let Ok(v) = row.try_get::<Option<i64>, _>(name) {
        return Ok(match v {
            Some(n) => Value::Number(n.into()),
            None => Value::Null,
        });
    }
    if let Ok(v) = row.try_get::<Option<f64>, _>(name) {
        return Ok(match v {
            Some(num) => serde_json::Number::from_f64(num)
                .map(Value::Number)
                .unwrap_or(Value::Null),
            None => Value::Null,
        });
    }
    if let Ok(v) = row.try_get::<Option<bool>, _>(name) {
        return Ok(match v {
            Some(b) => Value::Bool(b),
            None => Value::Null,
        });
    }
    Ok(Value::Null)
}
