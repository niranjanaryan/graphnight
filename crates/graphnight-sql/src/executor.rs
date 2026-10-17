use crate::generator::GeneratedSql;
use anyhow::{anyhow, Result};
use async_stream::try_stream;
use futures::Stream;
use futures::StreamExt;
use graphnight_core::models::DataSource;
use graphnight_core::resolve_connection_string;
use serde_json::Value;
use sqlx::Arguments as _;
use sqlx::{Column, MySql, Pool, Postgres, Row, Sqlite};
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tracing::{debug, info};

/// A stream of result rows.
///
/// Named because the expanded form is unwieldy at every use site.
pub type RowStream<'a> = Pin<Box<dyn Stream<Item = Result<HashMap<String, Value>>> + Send + 'a>>;

/// Default statement timeout applied on Postgres/MySQL connections when possible.
const DEFAULT_STATEMENT_TIMEOUT: Duration = Duration::from_secs(300);

/// Connection pool manager for multiple datasources
pub struct ConnectionManager {
    pg_pools: Arc<RwLock<HashMap<String, Pool<Postgres>>>>,
    mysql_pools: Arc<RwLock<HashMap<String, Pool<MySql>>>>,
    sqlite_pools: Arc<RwLock<HashMap<String, Pool<Sqlite>>>>,
    /// DuckDB is embedded and its `Connection` is not `Sync`, so each datasource
    /// gets a single connection behind a mutex. That serialises queries against
    /// one DuckDB database, which is the right default for an embedded engine
    /// reading a local file.
    #[cfg(feature = "duckdb")]
    duckdb_conns: Arc<RwLock<HashMap<String, Arc<tokio::sync::Mutex<duckdb::Connection>>>>>,
    statement_timeout: Duration,
}

impl ConnectionManager {
    pub fn new() -> Self {
        Self {
            pg_pools: Arc::new(RwLock::new(HashMap::new())),
            mysql_pools: Arc::new(RwLock::new(HashMap::new())),
            sqlite_pools: Arc::new(RwLock::new(HashMap::new())),
            #[cfg(feature = "duckdb")]
            duckdb_conns: Arc::new(RwLock::new(HashMap::new())),
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

    /// Get or create the DuckDB connection for a datasource.
    ///
    /// A connection string of `:memory:` (or empty) opens an in-memory database;
    /// anything else is treated as a path to a DuckDB database file, which is
    /// created if it does not exist.
    #[cfg(feature = "duckdb")]
    pub async fn get_duckdb_conn(
        &self,
        ds: &DataSource,
    ) -> Result<Arc<tokio::sync::Mutex<duckdb::Connection>>> {
        let mut conns = self.duckdb_conns.write().await;
        if let Some(conn) = conns.get(&ds.name) {
            return Ok(conn.clone());
        }

        let conn_str = Self::resolved_connection_string(ds)?;
        let conn = if conn_str.is_empty() || conn_str == ":memory:" {
            duckdb::Connection::open_in_memory()?
        } else {
            duckdb::Connection::open(conn_str.trim_start_matches("duckdb://"))?
        };

        // Best effort: enable extension auto-loading so a query that uses a
        // spatial function works when the extension can be resolved, without
        // making every query depend on network access to the extension repo. A
        // query that does not use the extension is unaffected either way.
        let _ = conn.execute_batch(
            "SET autoinstall_known_extensions = true; SET autoload_known_extensions = true;",
        );

        let conn = Arc::new(tokio::sync::Mutex::new(conn));
        conns.insert(ds.name.clone(), conn.clone());
        info!("Created DuckDB connection for datasource: {}", ds.name);
        Ok(conn)
    }

    pub fn statement_timeout(&self) -> Duration {
        self.statement_timeout
    }

    /// Counts of currently open pools per driver (lazy-created on first query).
    pub async fn pool_counts(&self) -> PoolCounts {
        let postgres = self.pg_pools.read().await.len();
        let mysql = self.mysql_pools.read().await.len();
        let sqlite = self.sqlite_pools.read().await.len();
        #[cfg(feature = "duckdb")]
        let duckdb = self.duckdb_conns.read().await.len();
        PoolCounts {
            postgres,
            mysql,
            sqlite,
            #[cfg(feature = "duckdb")]
            duckdb,
        }
    }

    /// Close all pools
    pub async fn close_all(&self) {
        let pg_pools = self.pg_pools.write().await;
        for (name, pool) in pg_pools.iter() {
            pool.close().await;
            info!("Closed PostgreSQL pool: {}", name);
        }
        drop(pg_pools);
        // Dropping the last `Arc` closes an embedded DuckDB connection; there is
        // no async `close` to call.
        #[cfg(feature = "duckdb")]
        {
            let mut duckdb_conns = self.duckdb_conns.write().await;
            for name in duckdb_conns.keys() {
                info!("Closed DuckDB connection: {}", name);
            }
            duckdb_conns.clear();
        }
    }
}

/// Snapshot of open connection pools for health / ops endpoints.
#[derive(Debug, Clone, Copy, serde::Serialize)]
pub struct PoolCounts {
    pub postgres: usize,
    pub mysql: usize,
    pub sqlite: usize,
    #[cfg(feature = "duckdb")]
    pub duckdb: usize,
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

    /// Run a statement that has no bound values.
    ///
    /// For GraphNight's own internal SQL (schema introspection), never for
    /// anything a caller supplied. Crate-private on purpose: the project's
    /// posture is that every statement reaching a database is generated by
    /// `SqlGenerator`, so an ungenerated-SQL entry point in the public API
    /// would be a hole in that property. It was public while the binding work
    /// was in flight, which is exactly when it looked harmless.
    pub(crate) async fn execute_literal(
        &self,
        ds: &DataSource,
        sql: &str,
    ) -> Result<Vec<HashMap<String, Value>>> {
        let generated = GeneratedSql::literal(sql);
        self.execute_streaming_collect(ds, &generated).await
    }

    /// Execute a query and return results as JSON (buffered).
    pub async fn execute(
        &self,
        ds: &DataSource,
        generated: &GeneratedSql,
    ) -> Result<Vec<HashMap<String, Value>>> {
        self.execute_streaming_collect(ds, generated).await
    }

    /// Fetch rows via streaming API and collect into a Vec (avoids fetch_all bulk path).
    pub async fn execute_streaming_collect(
        &self,
        ds: &DataSource,
        generated: &GeneratedSql,
    ) -> Result<Vec<HashMap<String, Value>>> {
        let mut rows = Vec::new();
        let mut stream = self.execute_stream(ds, generated.clone());
        while let Some(item) = stream.next().await {
            rows.push(item?);
        }
        Ok(rows)
    }

    /// Stream result rows one at a time.
    pub fn execute_stream<'a>(
        &'a self,
        ds: &'a DataSource,
        generated: GeneratedSql,
    ) -> RowStream<'a> {
        let sql = generated.executable().to_string();
        let params = generated.params.clone();
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
                    let mut args = sqlx::postgres::PgArguments::default();
                    args.reserve(params.len(), 0);
                    for param in &params {
                        add_pg_param(&mut args, param)?;
                    }
                    let mut rows = sqlx::query_with(&sql, args).fetch(&mut *conn);
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
                    let mut args = sqlx::mysql::MySqlArguments::default();
                    args.reserve(params.len(), 0);
                    for param in &params {
                        add_mysql_param(&mut args, param)?;
                    }
                    let mut rows = sqlx::query_with(&sql, args).fetch(&mut *conn);
                    while let Some(row) = rows.next().await {
                        let row = row?;
                        yield mysql_row_to_map(&row)?;
                    }
                }
                "sqlite" | "sqlite3" => {
                    let pool = cm.get_sqlite_pool(ds).await?;
                    let mut args = sqlx::sqlite::SqliteArguments::default();
                    args.reserve(params.len(), 0);
                    for param in &params {
                        add_sqlite_param(&mut args, param)?;
                    }
                    let mut rows = sqlx::query_with(&sql, args).fetch(&pool);
                    while let Some(row) = rows.next().await {
                        let row = row?;
                        yield sqlite_row_to_map(&row)?;
                    }
                }
                #[cfg(feature = "duckdb")]
                "duckdb" => {
                    let conn = cm.get_duckdb_conn(ds).await?;
                    let guard = conn.lock().await;
                    // Run the whole query in a synchronous helper: DuckDB's
                    // `Statement` and `Rows` hold raw pointers and are not
                    // `Send`, so they must not live across a yield in this
                    // generator. The helper returns an owned, `Send` `Vec`.
                    let collected = run_duckdb_query(&guard, &sql, &params)?;
                    drop(guard);
                    for row in collected {
                        yield row;
                    }
                }
                other => {
                    Err(anyhow!("Unsupported driver: {other}"))?;
                }
            }
        })
    }
}

/// Append one bound value to a driver's argument list.
///
/// The JSON wrapper is deliberately unwrapped: sqlx encodes a `Value::String`
/// as its JSON form, which would send `"abc"` and never match a text column.
/// Each driver gets the native scalar instead, so the parameter's type is the
/// one the column expects. Arrays and objects have no scalar equivalent and are
/// sent as their JSON text.
///
/// Written as a macro because sqlx only implements `Type`/`Encode` for the
/// concrete database types, so the arm cannot be written generically over
/// `sqlx::Database`.
macro_rules! add_param_fn {
    ($name:ident, $args:ty) => {
        fn $name(args: &mut $args, value: &Value) -> Result<()> {
            // `u64` is not encodable on every driver, so a large unsigned
            // integer is widened to i64 where it fits and only then to f64.
            let wide = match value {
                Value::Number(n) => n.as_u64().and_then(|u| i64::try_from(u).ok()),
                _ => None,
            };
            match value {
                Value::Null => args.add(Option::<String>::None).map_err(db_err)?,
                Value::Bool(b) => args.add(*b).map_err(db_err)?,
                Value::Number(n) if n.is_i64() => args
                    .add(n.as_i64().expect("guarded by is_i64"))
                    .map_err(db_err)?,
                // Only reachable for a value above i64::MAX, which no driver
                // takes as u64; send it as a float rather than fail the query.
                Value::Number(_) if wide.is_some() => args
                    .add(wide.expect("guarded by is_some"))
                    .map_err(db_err)?,
                Value::Number(n) => args.add(n.as_f64().unwrap_or(f64::NAN)).map_err(db_err)?,
                Value::String(s) => args.add(s.clone()).map_err(db_err)?,
                other => args.add(other.to_string()).map_err(db_err)?,
            }
            Ok(())
        }
    };
}

/// sqlx reports a type-incompatible bind as a boxed error; surface the text.
fn db_err(e: Box<dyn std::error::Error + Send + Sync>) -> anyhow::Error {
    anyhow!("cannot bind query parameter: {e}")
}

add_param_fn!(add_pg_param, sqlx::postgres::PgArguments);
add_param_fn!(add_mysql_param, sqlx::mysql::MySqlArguments);
add_param_fn!(add_sqlite_param, sqlx::sqlite::SqliteArguments);

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

/// Decode one column into JSON.
///
/// Written once and instantiated per driver because the three copies drifted:
/// each listed only the types its author had a model for, so a Postgres `int4`
/// or `numeric` column, a MySQL `INT`, and so on fell through every attempt and
/// came back as `null` — silently, with no error, in the main query path. One
/// definition cannot drift.
///
/// sqlx is strict about wire types: it will not widen `int4` to `i64` or narrow
/// `numeric` to `f64`, so every width a driver can return has to be named here.
/// A real SQL NULL is matched by the first attempt and returns early, which is
/// why the later attempts only ever see a present value.
/// Decode one column into JSON for the types every driver shares.
///
/// Written once and instantiated per driver because the three hand-written
/// copies had drifted: each listed only the types its author happened to have a
/// model for, so a Postgres `int4` or `numeric`, a MySQL `INT`, and so on fell
/// past every attempt and came back as `null` — silently, with no error, in the
/// main query path. One definition cannot drift.
///
/// sqlx is strict about wire types: it will not widen `int4` to `i64` or narrow
/// `numeric` to `f64`, so every width a driver can return has to be named here.
/// A real SQL NULL is matched by the first attempt and returns early, which is
/// why the later attempts only ever see a present value.
///
/// Returns `None` when nothing matched, so the caller can try the types that
/// are specific to its driver.
macro_rules! common_value_to_json {
    ($name:ident, $row:ty) => {
        fn $name(row: &$row, name: &str) -> Option<Value> {
            if let Ok(v) = row.try_get::<Option<String>, _>(name) {
                return Some(match v {
                    Some(s) => Value::String(s),
                    None => Value::Null,
                });
            }
            if let Ok(v) = row.try_get::<Option<i64>, _>(name) {
                return Some(match v {
                    Some(n) => Value::Number(n.into()),
                    None => Value::Null,
                });
            }
            if let Ok(v) = row.try_get::<Option<i32>, _>(name) {
                return Some(match v {
                    Some(n) => Value::Number(n.into()),
                    None => Value::Null,
                });
            }
            if let Ok(v) = row.try_get::<Option<i16>, _>(name) {
                return Some(match v {
                    Some(n) => Value::Number(n.into()),
                    None => Value::Null,
                });
            }
            if let Ok(v) = row.try_get::<Option<f64>, _>(name) {
                return Some(match v {
                    Some(num) => serde_json::Number::from_f64(num)
                        .map(Value::Number)
                        .unwrap_or(Value::Null),
                    None => Value::Null,
                });
            }
            if let Ok(v) = row.try_get::<Option<f32>, _>(name) {
                return Some(match v {
                    Some(num) => serde_json::Number::from_f64(num as f64)
                        .map(Value::Number)
                        .unwrap_or(Value::Null),
                    None => Value::Null,
                });
            }
            if let Ok(v) = row.try_get::<Option<bool>, _>(name) {
                return Some(match v {
                    Some(b) => Value::Bool(b),
                    None => Value::Null,
                });
            }
            if let Ok(v) = row.try_get::<Option<serde_json::Value>, _>(name) {
                return Some(v.unwrap_or(Value::Null));
            }
            if let Ok(v) = row.try_get::<Option<chrono::DateTime<chrono::Utc>>, _>(name) {
                return Some(match v {
                    Some(dt) => Value::String(dt.to_rfc3339()),
                    None => Value::Null,
                });
            }
            if let Ok(v) = row.try_get::<Option<uuid::Uuid>, _>(name) {
                return Some(match v {
                    Some(u) => Value::String(u.to_string()),
                    None => Value::Null,
                });
            }
            None
        }
    };
}

common_value_to_json!(pg_common, sqlx::postgres::PgRow);
common_value_to_json!(mysql_common, sqlx::mysql::MySqlRow);
common_value_to_json!(sqlite_common, sqlx::sqlite::SqliteRow);

/// Render an arbitrary-precision decimal as a JSON number without going via
/// f64, which would round the digits the caller asked for.
fn decimal_to_json(d: rust_decimal::Decimal) -> Value {
    serde_json::from_str::<Value>(&d.to_string()).unwrap_or_else(|_| Value::String(d.to_string()))
}

fn pg_value_to_json(row: &sqlx::postgres::PgRow, name: &str) -> Result<Value> {
    if let Some(v) = pg_common(row, name) {
        return Ok(v);
    }
    // `numeric` is arbitrary precision and is not decodable as f64 without
    // loss, so it needs its own decoder. Postgres also has a `timestamp`
    // without a zone, which is a distinct type from `timestamptz`.
    if let Ok(v) = row.try_get::<Option<rust_decimal::Decimal>, _>(name) {
        return Ok(match v {
            Some(d) => decimal_to_json(d),
            None => Value::Null,
        });
    }
    if let Ok(v) = row.try_get::<Option<chrono::NaiveDateTime>, _>(name) {
        return Ok(match v {
            Some(dt) => Value::String(dt.format("%Y-%m-%dT%H:%M:%S").to_string()),
            None => Value::Null,
        });
    }
    if let Ok(v) = row.try_get::<Option<chrono::NaiveDate>, _>(name) {
        return Ok(match v {
            Some(d) => Value::String(d.to_string()),
            None => Value::Null,
        });
    }
    // A type nothing above names. Reporting it beats returning null: a null is
    // indistinguishable from a genuine SQL NULL, so an unhandled column type
    // would quietly answer a different question than the one asked.
    anyhow::bail!("unsupported Postgres column type returned for {name}")
}

fn mysql_value_to_json(row: &sqlx::mysql::MySqlRow, name: &str) -> Result<Value> {
    if let Some(v) = mysql_common(row, name) {
        return Ok(v);
    }
    if let Ok(v) = row.try_get::<Option<rust_decimal::Decimal>, _>(name) {
        return Ok(match v {
            Some(d) => decimal_to_json(d),
            None => Value::Null,
        });
    }
    if let Ok(v) = row.try_get::<Option<chrono::NaiveDateTime>, _>(name) {
        return Ok(match v {
            Some(dt) => Value::String(dt.format("%Y-%m-%dT%H:%M:%S").to_string()),
            None => Value::Null,
        });
    }
    if let Ok(v) = row.try_get::<Option<chrono::NaiveDate>, _>(name) {
        return Ok(match v {
            Some(d) => Value::String(d.to_string()),
            None => Value::Null,
        });
    }
    anyhow::bail!("unsupported MySQL column type returned for {name}")
}

fn sqlite_value_to_json(row: &sqlx::sqlite::SqliteRow, name: &str) -> Result<Value> {
    if let Some(v) = sqlite_common(row, name) {
        return Ok(v);
    }
    // SQLite is dynamically typed, so dates and numerics usually arrive as text
    // or a real and are already handled by the common chain.
    if let Ok(v) = row.try_get::<Option<chrono::NaiveDate>, _>(name) {
        return Ok(match v {
            Some(d) => Value::String(d.to_string()),
            None => Value::Null,
        });
    }
    anyhow::bail!("unsupported SQLite column type returned for {name}")
}

/// Execute one generated statement on a DuckDB connection and collect the rows.
///
/// Synchronous on purpose: see the call site. The returned rows are owned, so
/// nothing that borrows the connection escapes into an async stream.
#[cfg(feature = "duckdb")]
fn run_duckdb_query(
    conn: &duckdb::Connection,
    sql: &str,
    params: &[Value],
) -> Result<Vec<HashMap<String, Value>>> {
    let mut stmt = conn.prepare(sql)?;
    let values: Vec<duckdb::types::Value> = params.iter().map(json_to_duckdb_value).collect();
    let mut rows = stmt.query(duckdb::params_from_iter(values.iter()))?;
    let column_names: Vec<String> = rows
        .as_ref()
        .map(|statement| statement.column_names())
        .unwrap_or_default();
    let mut out = Vec::new();
    while let Some(row) = rows.next()? {
        let mut map = HashMap::new();
        for (idx, name) in column_names.iter().enumerate() {
            map.insert(name.clone(), duckdb_value_to_json(row, idx, name)?);
        }
        out.push(map);
    }
    Ok(out)
}

/// Convert a JSON value to the DuckDB scalar it stands for.
///
/// Mirrors [`add_param_fn!`]: the JSON wrapper is unwrapped so a text parameter
/// is not compared as `'"abc"'`, and an integer larger than `i64::MAX` is sent
/// as a double because DuckDB's `BIGINT` cannot hold it either.
#[cfg(feature = "duckdb")]
fn json_to_duckdb_value(value: &Value) -> duckdb::types::Value {
    use duckdb::types::Value as DuckValue;
    match value {
        Value::Null => DuckValue::Null,
        Value::Bool(b) => DuckValue::Boolean(*b),
        Value::Number(n) if n.is_i64() => {
            DuckValue::BigInt(n.as_i64().expect("guarded by is_i64"))
        }
        Value::Number(n) => DuckValue::Double(n.as_f64().unwrap_or(f64::NAN)),
        Value::String(s) => DuckValue::Text(s.clone()),
        other => DuckValue::Text(other.to_string()),
    }
}

/// Decode one DuckDB column into JSON by index.
///
/// DuckDB is dynamically typed and duckdb-rs *widens* on decode: a `get::<i64>`
/// on a `DOUBLE` column succeeds and rounds, so a chain of `if let Ok` attempts
/// (the shape the sqlx drivers use) would silently turn `1.5` into `2`. The
/// column's own type is therefore read first, and the matching decoder is the
/// only one tried. A type not named here is an error rather than a silent null,
/// for the same reason as the other drivers: a null is indistinguishable from a
/// real SQL NULL.
#[cfg(feature = "duckdb")]
fn duckdb_value_to_json(row: &duckdb::Row<'_>, idx: usize, name: &str) -> Result<Value> {
    use duckdb::types::Type;

    let data_type = row.get_ref(idx)?.data_type();
    let value = match data_type {
        Type::Null => Value::Null,
        Type::Boolean => Value::Bool(row.get::<usize, bool>(idx)?),
        Type::TinyInt => Value::Number((row.get::<usize, i8>(idx)? as i64).into()),
        Type::SmallInt => Value::Number((row.get::<usize, i16>(idx)? as i64).into()),
        Type::Int => Value::Number((row.get::<usize, i32>(idx)? as i64).into()),
        Type::BigInt => Value::Number(row.get::<usize, i64>(idx)?.into()),
        Type::HugeInt => {
            let n = row.get::<usize, i128>(idx)?;
            i64::try_from(n)
                .map(|i| Value::Number(i.into()))
                .unwrap_or_else(|_| Value::String(n.to_string()))
        }
        Type::UTinyInt => Value::Number((row.get::<usize, u8>(idx)? as u64).into()),
        Type::USmallInt => Value::Number((row.get::<usize, u16>(idx)? as u64).into()),
        Type::UInt => Value::Number((row.get::<usize, u32>(idx)? as u64).into()),
        Type::UBigInt => Value::Number(row.get::<usize, u64>(idx)?.into()),
        Type::UHugeInt => {
            let n = row.get::<usize, u128>(idx)?;
            u64::try_from(n)
                .map(|u| Value::Number(u.into()))
                .unwrap_or_else(|_| Value::String(n.to_string()))
        }
        Type::Float => serde_json::Number::from_f64(row.get::<usize, f32>(idx)? as f64)
            .map(Value::Number)
            .unwrap_or(Value::Null),
        Type::Double => serde_json::Number::from_f64(row.get::<usize, f64>(idx)?)
            .map(Value::Number)
            .unwrap_or(Value::Null),
        Type::Decimal => decimal_to_json(row.get::<usize, rust_decimal::Decimal>(idx)?),
        Type::Timestamp => {
            let dt = row.get::<usize, chrono::NaiveDateTime>(idx)?;
            Value::String(dt.format("%Y-%m-%dT%H:%M:%S").to_string())
        }
        Type::Date32 => Value::String(row.get::<usize, chrono::NaiveDate>(idx)?.to_string()),
        Type::Time64 => {
            let t = row.get::<usize, chrono::NaiveTime>(idx)?;
            Value::String(t.format("%H:%M:%S").to_string())
        }
        Type::Text | Type::Enum => Value::String(row.get::<usize, String>(idx)?),
        // A DuckDB `GEOMETRY` column that was not wrapped by `ST_AsGeoJSON`
        // arrives as WKB. Rendering it as hex keeps the value rather than
        // dropping it.
        Type::Blob | Type::Geometry => Value::String(hex_encode(&row.get::<usize, Vec<u8>>(idx)?)),
        other => anyhow::bail!("unsupported DuckDB column type {other:?} returned for {name}"),
    };
    Ok(value)
}

/// Lower-case hex, used to carry an unwrapped binary column through JSON.
#[cfg(feature = "duckdb")]
fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

#[cfg(all(test, feature = "duckdb"))]
mod duckdb_driver_tests {
    use super::*;
    use serde_json::json;

    fn datasource() -> DataSource {
        DataSource {
            name: "duck".to_string(),
            driver: "duckdb".to_string(),
            connection_string: ":memory:".to_string(),
            description: None,
            models: vec![],
            pool_size: None,
            meta: Default::default(),
        }
    }

    fn executor() -> QueryExecutor {
        QueryExecutor::new(Arc::new(ConnectionManager::new()))
    }

    /// The driver reaches DuckDB and decodes the scalar types a result can hold
    /// without going through a model, so a regression in either the connection
    /// path or the extraction chain fails here.
    #[tokio::test]
    async fn executes_and_extracts_scalars() {
        let ds = datasource();
        let generated = GeneratedSql::literal("SELECT 1 AS n, 'x' AS s, 1.5 AS f, true AS b");
        let rows = executor().execute(&ds, &generated).await.expect("execute");

        assert_eq!(rows.len(), 1, "{rows:?}");
        let row = &rows[0];
        assert_eq!(row["n"], json!(1));
        assert_eq!(row["s"], json!("x"));
        assert_eq!(row["f"], json!(1.5));
        assert_eq!(row["b"], json!(true));
    }

    /// A bound parameter must reach DuckDB as a native scalar, not as its JSON
    /// text, otherwise a text comparison would look for `'"abc"'`.
    #[tokio::test]
    async fn binds_parameters() {
        let ds = datasource();
        let generated = GeneratedSql {
            sql: "SELECT ? AS v".to_string(),
            params: vec![json!("abc")],
            display: String::new(),
        };
        let rows = executor().execute(&ds, &generated).await.expect("execute");
        assert_eq!(rows[0]["v"], json!("abc"));
    }

    /// An unhandled column type is an error, not a silent null.
    #[tokio::test]
    async fn unsupported_type_is_an_error() {
        let ds = datasource();
        let generated = GeneratedSql::literal("SELECT [1, 2, 3] AS xs");
        let err = executor().execute(&ds, &generated).await;
        assert!(err.is_err(), "a list column must not decode: {err:?}");
    }
}
