use anyhow::{anyhow, Result};
use graphnight_core::models::DataSource;
use serde_json::Value;
use sqlx::{MySql, Pool, Postgres, Row, Sqlite};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info};

/// Connection pool manager for multiple datasources
pub struct ConnectionManager {
    pg_pools: Arc<RwLock<HashMap<String, Pool<Postgres>>>>,
    mysql_pools: Arc<RwLock<HashMap<String, Pool<MySql>>>>,
    sqlite_pools: Arc<RwLock<HashMap<String, Pool<Sqlite>>>>,
}

impl ConnectionManager {
    pub fn new() -> Self {
        Self {
            pg_pools: Arc::new(RwLock::new(HashMap::new())),
            mysql_pools: Arc::new(RwLock::new(HashMap::new())),
            sqlite_pools: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Get or create a PostgreSQL pool
    pub async fn get_pg_pool(&self, ds: &DataSource) -> Result<Pool<Postgres>> {
        let mut pools = self.pg_pools.write().await;
        if let Some(pool) = pools.get(&ds.name) {
            return Ok(pool.clone());
        }

        let pool_size = ds.pool_size.unwrap_or(10) as u32;
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(pool_size)
            .connect(&ds.connection_string)
            .await?;

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

        let pool_size = ds.pool_size.unwrap_or(10) as u32;
        let pool = sqlx::mysql::MySqlPoolOptions::new()
            .max_connections(pool_size)
            .connect(&ds.connection_string)
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

        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect(&ds.connection_string)
            .await?;

        pools.insert(ds.name.clone(), pool.clone());
        info!("Created SQLite pool for datasource: {}", ds.name);
        Ok(pool)
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

/// Query executor using connection pools
pub struct QueryExecutor {
    connection_manager: Arc<ConnectionManager>,
}

impl QueryExecutor {
    pub fn new(connection_manager: Arc<ConnectionManager>) -> Self {
        Self { connection_manager }
    }

    /// Execute a query and return results as JSON
    pub async fn execute(&self, ds: &DataSource, sql: &str) -> Result<Vec<HashMap<String, Value>>> {
        debug!("Executing SQL on {}: {}", ds.name, sql);

        match ds.driver.as_str() {
            "postgres" | "postgresql" | "pg" => {
                let pool = self.connection_manager.get_pg_pool(ds).await?;
                self.execute_pg(&pool, sql).await
            }
            "mysql" | "mariadb" => {
                let pool = self.connection_manager.get_mysql_pool(ds).await?;
                self.execute_mysql(&pool, sql).await
            }
            "sqlite" | "sqlite3" => {
                let pool = self.connection_manager.get_sqlite_pool(ds).await?;
                self.execute_sqlite(&pool, sql).await
            }
            _ => Err(anyhow!("Unsupported driver: {}", ds.driver)),
        }
    }

    /// Execute and stream results (for large result sets)
    pub async fn execute_stream(
        &self,
        ds: &DataSource,
        sql: &str,
    ) -> Result<Vec<HashMap<String, Value>>> {
        // For now, fall back to executing and collecting all results
        // Proper streaming would require more complex lifetime management
        self.execute(ds, sql).await
    }

    // Streaming disabled for now - would need async stream with proper lifetimes
    // fn stream_pg(&self, pool: Pool<Postgres>, sql: String) -> Box<dyn Stream<Item = Result<HashMap<String, Value>>> + Send + Unpin + 'static> {
    //     let stream = sqlx::query(&sql).fetch(pool);
    //     Box::new(stream.map(move |row_result| {
    //         match row_result {
    //             Ok(row) => {
    //                 let mut map = HashMap::new();
    //                 for i in 0..row.len() {
    //                     if let Ok(name) = row.try_get::<String, _>(i) {
    //                         let value = self.pg_value_to_json(&row, &name)?;
    //                         map.insert(name, value);
    //                     }
    //                 }
    //                 Ok(map)
    //             }
    //             Err(e) => Err(anyhow!(e)),
    //         }
    //     }))
    // }

    async fn execute_pg(
        &self,
        pool: &Pool<Postgres>,
        sql: &str,
    ) -> Result<Vec<HashMap<String, Value>>> {
        let rows = sqlx::query(sql).fetch_all(pool).await?;
        let mut results = Vec::new();

        for row in rows {
            let mut map = HashMap::new();
            for i in 0..row.len() {
                if let Ok(name) = row.try_get::<String, _>(i) {
                    let value = self.pg_value_to_json(&row, &name)?;
                    map.insert(name, value);
                }
            }
            results.push(map);
        }

        Ok(results)
    }

    async fn execute_mysql(
        &self,
        pool: &Pool<MySql>,
        sql: &str,
    ) -> Result<Vec<HashMap<String, Value>>> {
        let rows = sqlx::query(sql).fetch_all(pool).await?;
        let mut results = Vec::new();

        for row in rows {
            let mut map = HashMap::new();
            for i in 0..row.len() {
                if let Ok(name) = row.try_get::<String, _>(i) {
                    let value = self.mysql_value_to_json(&row, &name)?;
                    map.insert(name, value);
                }
            }
            results.push(map);
        }

        Ok(results)
    }

    async fn execute_sqlite(
        &self,
        pool: &Pool<Sqlite>,
        sql: &str,
    ) -> Result<Vec<HashMap<String, Value>>> {
        let rows = sqlx::query(sql).fetch_all(pool).await?;
        let mut results = Vec::new();

        for row in rows {
            let mut map = HashMap::new();
            for i in 0..row.len() {
                if let Ok(name) = row.try_get::<String, _>(i) {
                    let value = self.sqlite_value_to_json(&row, &name)?;
                    map.insert(name, value);
                }
            }
            results.push(map);
        }

        Ok(results)
    }

    // Streaming disabled for now - would need async stream with proper lifetimes
    // fn stream_pg(&self, pool: Pool<Postgres>, sql: String) -> Box<dyn Stream<Item = Result<HashMap<String, Value>>> + Send + Unpin + 'static> {
    //     let stream = sqlx::query(&sql).fetch(pool);
    //     Box::new(stream.map(move |row_result| {
    //         match row_result {
    //             Ok(row) => {
    //                 let mut map = HashMap::new();
    //                 for i in 0..row.len() {
    //                     if let Ok(name) = row.try_get::<String, _>(i) {
    //                         let value = self.pg_value_to_json(&row, &name)?;
    //                         map.insert(name, value);
    //                     }
    //                 }
    //                 Ok(map)
    //             }
    //             Err(e) => Err(anyhow!(e)),
    //         }
    //     }))
    // }

    fn pg_value_to_json(&self, row: &sqlx::postgres::PgRow, name: &str) -> Result<Value> {
        use sqlx::Row;
        if let Ok(v) = row.try_get::<Option<String>, _>(name) {
            return Ok(Value::String(v.unwrap_or_default()));
        }
        if let Ok(v) = row.try_get::<Option<i64>, _>(name) {
            return Ok(Value::Number(serde_json::Number::from(v.unwrap_or(0))));
        }
        if let Ok(v) = row.try_get::<Option<f64>, _>(name) {
            if let Some(num) = v {
                return Ok(serde_json::Number::from_f64(num)
                    .map(Value::Number)
                    .unwrap_or(Value::Null));
            }
            return Ok(Value::Null);
        }
        if let Ok(v) = row.try_get::<Option<bool>, _>(name) {
            return Ok(Value::Bool(v.unwrap_or(false)));
        }
        if let Ok(v) = row.try_get::<Option<serde_json::Value>, _>(name) {
            return Ok(v.unwrap_or(Value::Null));
        }
        if let Ok(v) = row.try_get::<Option<chrono::DateTime<chrono::Utc>>, _>(name) {
            return Ok(Value::String(
                v.map(|dt| dt.to_rfc3339()).unwrap_or_default(),
            ));
        }
        Ok(Value::Null)
    }

    fn mysql_value_to_json(&self, row: &sqlx::mysql::MySqlRow, name: &str) -> Result<Value> {
        use sqlx::Row;
        if let Ok(v) = row.try_get::<Option<String>, _>(name) {
            return Ok(Value::String(v.unwrap_or_default()));
        }
        if let Ok(v) = row.try_get::<Option<i64>, _>(name) {
            return Ok(Value::Number(serde_json::Number::from(v.unwrap_or(0))));
        }
        if let Ok(v) = row.try_get::<Option<f64>, _>(name) {
            if let Some(num) = v {
                return Ok(serde_json::Number::from_f64(num)
                    .map(Value::Number)
                    .unwrap_or(Value::Null));
            }
            return Ok(Value::Null);
        }
        if let Ok(v) = row.try_get::<Option<bool>, _>(name) {
            return Ok(Value::Bool(v.unwrap_or(false)));
        }
        Ok(Value::Null)
    }

    fn sqlite_value_to_json(&self, row: &sqlx::sqlite::SqliteRow, name: &str) -> Result<Value> {
        use sqlx::Row;
        if let Ok(v) = row.try_get::<Option<String>, _>(name) {
            return Ok(Value::String(v.unwrap_or_default()));
        }
        if let Ok(v) = row.try_get::<Option<i64>, _>(name) {
            return Ok(Value::Number(serde_json::Number::from(v.unwrap_or(0))));
        }
        if let Ok(v) = row.try_get::<Option<f64>, _>(name) {
            if let Some(num) = v {
                return Ok(serde_json::Number::from_f64(num)
                    .map(Value::Number)
                    .unwrap_or(Value::Null));
            }
            return Ok(Value::Null);
        }
        if let Ok(v) = row.try_get::<Option<bool>, _>(name) {
            return Ok(Value::Bool(v.unwrap_or(false)));
        }
        Ok(Value::Null)
    }
}
