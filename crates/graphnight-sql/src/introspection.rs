use anyhow::{anyhow, Result};
use graphnight_core::models::{DataSource, Join, JoinType, Measure, Model, TimeDimension};
use graphnight_core::models::{AggregationType, Dimension, Formula, TimeGranularity};
use crate::executor::QueryExecutor;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct TableInfo {
    pub name: String,
    pub columns: Vec<ColumnInfo>,
    pub primary_keys: Vec<String>,
    pub foreign_keys: Vec<ForeignKeyInfo>,
}

#[derive(Debug, Clone)]
pub struct ColumnInfo {
    pub name: String,
    pub data_type: String,
    pub is_nullable: bool,
    pub is_primary_key: bool,
    pub default: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ForeignKeyInfo {
    pub column: String,
    pub referenced_table: String,
    pub referenced_column: String,
}

#[derive(Debug, Clone)]
#[derive(Default)]
pub struct IntrospectionConfig {
    pub include_views: bool,
    pub schema_filter: Option<String>,
    pub table_filter: Option<Vec<String>>,
}


pub struct SchemaIntrospector {
    executor: Arc<QueryExecutor>,
}

impl SchemaIntrospector {
    pub fn new(executor: Arc<QueryExecutor>) -> Self {
        Self { executor }
    }

    pub async fn introspect(&self, ds: &DataSource, config: IntrospectionConfig) -> Result<Vec<TableInfo>> {
        match ds.driver.as_str() {
            "postgres" | "postgresql" | "pg" => self.introspect_postgres(ds, config).await,
            "mysql" | "mariadb" => self.introspect_mysql(ds, config).await,
            "sqlite" | "sqlite3" => self.introspect_sqlite(ds, config).await,
            other => Err(anyhow!("Unsupported driver for introspection: {}", other)),
        }
    }

    async fn introspect_postgres(&self, ds: &DataSource, config: IntrospectionConfig) -> Result<Vec<TableInfo>> {
        let schema = config.schema_filter.unwrap_or_else(|| "public".to_string());
        let table_filter = config.table_filter.map(|tables| {
            tables.iter().map(|t| format!("'{}'", t)).collect::<Vec<_>>().join(",")
        });

        let tables_sql = if let Some(filter) = table_filter {
            format!(
                r#"SELECT table_name FROM information_schema.tables 
                WHERE table_schema = '{}' AND table_type = 'BASE TABLE' AND table_name IN ({})
                ORDER BY table_name"#,
                schema, filter
            )
        } else {
            format!(
                r#"SELECT table_name FROM information_schema.tables 
                WHERE table_schema = '{}' AND table_type = 'BASE TABLE'
                ORDER BY table_name"#,
                schema
            )
        };

        let table_rows = self.executor.execute(ds, &tables_sql).await?;
        let table_names: Vec<String> = table_rows
            .into_iter()
            .filter_map(|row| row.get("table_name").and_then(|v| v.as_str()).map(String::from))
            .collect();

        let mut tables = Vec::new();
        for table_name in table_names {
            let columns = self.get_postgres_columns(ds, &schema, &table_name).await?;
            let primary_keys = self.get_postgres_primary_keys(ds, &schema, &table_name).await?;
            let foreign_keys = self.get_postgres_foreign_keys(ds, &schema, &table_name).await?;

            tables.push(TableInfo {
                name: table_name.clone(),
                columns,
                primary_keys,
                foreign_keys,
            });
        }

        Ok(tables)
    }

    async fn get_postgres_columns(
        &self,
        ds: &DataSource,
        schema: &str,
        table: &str,
    ) -> Result<Vec<ColumnInfo>> {
        let sql = format!(
            r#"SELECT column_name, data_type, is_nullable, column_default
            FROM information_schema.columns
            WHERE table_schema = '{}' AND table_name = '{}'
            ORDER BY ordinal_position"#,
            schema, table
        );

        let rows = self.executor.execute(ds, &sql).await?;
        let mut columns = Vec::new();
        for row in rows {
            let name = row.get("column_name").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let data_type = row.get("data_type").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let is_nullable = row.get("is_nullable").and_then(|v| v.as_str()).map(|s| s == "YES").unwrap_or(true);
            let default = row.get("column_default").and_then(|v| v.as_str()).map(String::from);

            columns.push(ColumnInfo {
                name,
                data_type,
                is_nullable,
                is_primary_key: false,
                default,
            });
        }
        Ok(columns)
    }

    async fn get_postgres_primary_keys(
        &self,
        ds: &DataSource,
        schema: &str,
        table: &str,
    ) -> Result<Vec<String>> {
        let sql = format!(
            r#"SELECT kcu.column_name
            FROM information_schema.table_constraints tc
            JOIN information_schema.key_column_usage kcu
                ON tc.constraint_name = kcu.constraint_name
                AND tc.table_schema = kcu.table_schema
            WHERE tc.table_schema = '{}' 
                AND tc.table_name = '{}'
                AND tc.constraint_type = 'PRIMARY KEY'
            ORDER BY kcu.ordinal_position"#,
            schema, table
        );

        let rows = self.executor.execute(ds, &sql).await?;
        Ok(rows
            .into_iter()
            .filter_map(|row| row.get("column_name").and_then(|v| v.as_str()).map(String::from))
            .collect())
    }

    async fn get_postgres_foreign_keys(
        &self,
        ds: &DataSource,
        schema: &str,
        table: &str,
    ) -> Result<Vec<ForeignKeyInfo>> {
        let sql = format!(
            r#"SELECT
                kcu.column_name,
                ccu.table_name AS foreign_table_name,
                ccu.column_name AS foreign_column_name
            FROM information_schema.table_constraints tc
            JOIN information_schema.key_column_usage kcu
                ON tc.constraint_name = kcu.constraint_name
                AND tc.table_schema = kcu.table_schema
            JOIN information_schema.constraint_column_usage ccu
                ON ccu.constraint_name = tc.constraint_name
                AND ccu.table_schema = tc.table_schema
            WHERE tc.table_schema = '{}'
                AND tc.table_name = '{}'
                AND tc.constraint_type = 'FOREIGN KEY'"#,
            schema, table
        );

        let rows = self.executor.execute(ds, &sql).await?;
        let mut fks = Vec::new();
        for row in rows {
            if let (Some(column), Some(ref_table), Some(ref_column)) = (
                row.get("column_name").and_then(|v| v.as_str()),
                row.get("foreign_table_name").and_then(|v| v.as_str()),
                row.get("foreign_column_name").and_then(|v| v.as_str()),
            ) {
                fks.push(ForeignKeyInfo {
                    column: column.to_string(),
                    referenced_table: ref_table.to_string(),
                    referenced_column: ref_column.to_string(),
                });
            }
        }
        Ok(fks)
    }

    async fn introspect_mysql(&self, ds: &DataSource, config: IntrospectionConfig) -> Result<Vec<TableInfo>> {
        let schema = config.schema_filter.unwrap_or_default();
        let table_filter = config.table_filter.map(|tables| {
            tables.iter().map(|t| format!("'{}'", t)).collect::<Vec<_>>().join(",")
        });

        let schema_clause = if schema.is_empty() {
            "".to_string()
        } else {
            format!("AND table_schema = '{}'", schema)
        };

        let tables_sql = if let Some(filter) = table_filter {
            format!(
                r#"SELECT table_name FROM information_schema.tables 
                WHERE table_type = 'BASE TABLE' {} AND table_name IN ({})
                ORDER BY table_name"#,
                schema_clause, filter
            )
        } else {
            format!(
                r#"SELECT table_name FROM information_schema.tables 
                WHERE table_type = 'BASE TABLE' {}
                ORDER BY table_name"#,
                schema_clause
            )
        };

        let table_rows = self.executor.execute(ds, &tables_sql).await?;
        let table_names: Vec<String> = table_rows
            .into_iter()
            .filter_map(|row| row.get("table_name").and_then(|v| v.as_str()).map(String::from))
            .collect();

        let mut tables = Vec::new();
        for table_name in table_names {
            let columns = self.get_mysql_columns(ds, &table_name, &schema).await?;
            let primary_keys = self.get_mysql_primary_keys(ds, &table_name, &schema).await?;
            let foreign_keys = self.get_mysql_foreign_keys(ds, &table_name, &schema).await?;

            tables.push(TableInfo {
                name: table_name.clone(),
                columns,
                primary_keys,
                foreign_keys,
            });
        }

        Ok(tables)
    }

    async fn get_mysql_columns(
        &self,
        ds: &DataSource,
        table: &str,
        schema: &str,
    ) -> Result<Vec<ColumnInfo>> {
        let schema_clause = if schema.is_empty() {
            "".to_string()
        } else {
            format!("AND table_schema = '{}'", schema)
        };

        let sql = format!(
            r#"SELECT column_name, data_type, is_nullable, column_default
            FROM information_schema.columns
            WHERE table_name = '{}' {}
            ORDER BY ordinal_position"#,
            table, schema_clause
        );

        let rows = self.executor.execute(ds, &sql).await?;
        let mut columns = Vec::new();
        for row in rows {
            let name = row.get("column_name").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let data_type = row.get("data_type").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let is_nullable = row.get("is_nullable").and_then(|v| v.as_str()).map(|s| s == "YES").unwrap_or(true);
            let default = row.get("column_default").and_then(|v| v.as_str()).map(String::from);

            columns.push(ColumnInfo {
                name,
                data_type,
                is_nullable,
                is_primary_key: false,
                default,
            });
        }
        Ok(columns)
    }

    async fn get_mysql_primary_keys(
        &self,
        ds: &DataSource,
        table: &str,
        schema: &str,
    ) -> Result<Vec<String>> {
        let schema_clause = if schema.is_empty() {
            "".to_string()
        } else {
            format!("AND table_schema = '{}'", schema)
        };

        let sql = format!(
            r#"SELECT column_name
            FROM information_schema.key_column_usage
            WHERE table_name = '{}' 
                AND constraint_name = 'PRIMARY'
                {}
            ORDER BY ordinal_position"#,
            table, schema_clause
        );

        let rows = self.executor.execute(ds, &sql).await?;
        Ok(rows
            .into_iter()
            .filter_map(|row| row.get("column_name").and_then(|v| v.as_str()).map(String::from))
            .collect())
    }

    async fn get_mysql_foreign_keys(
        &self,
        ds: &DataSource,
        table: &str,
        schema: &str,
    ) -> Result<Vec<ForeignKeyInfo>> {
        let schema_clause = if schema.is_empty() {
            "".to_string()
        } else {
            format!("AND table_schema = '{}'", schema)
        };

        let sql = format!(
            r#"SELECT
                kcu.column_name,
                kcu.referenced_table_name,
                kcu.referenced_column_name
            FROM information_schema.key_column_usage kcu
            JOIN information_schema.table_constraints tc
                ON kcu.constraint_name = tc.constraint_name
                AND kcu.table_schema = tc.table_schema
            WHERE kcu.table_name = '{}'
                AND tc.constraint_type = 'FOREIGN KEY'
                {}"#,
            table, schema_clause
        );

        let rows = self.executor.execute(ds, &sql).await?;
        let mut fks = Vec::new();
        for row in rows {
            if let (Some(column), Some(ref_table), Some(ref_column)) = (
                row.get("column_name").and_then(|v| v.as_str()),
                row.get("referenced_table_name").and_then(|v| v.as_str()),
                row.get("referenced_column_name").and_then(|v| v.as_str()),
            ) {
                fks.push(ForeignKeyInfo {
                    column: column.to_string(),
                    referenced_table: ref_table.to_string(),
                    referenced_column: ref_column.to_string(),
                });
            }
        }
        Ok(fks)
    }

    async fn introspect_sqlite(&self, ds: &DataSource, config: IntrospectionConfig) -> Result<Vec<TableInfo>> {
        let table_filter = config.table_filter.map(|tables| {
            tables.iter().map(|t| format!("'{}'", t)).collect::<Vec<_>>().join(",")
        });

        let tables_sql = if let Some(filter) = table_filter {
            format!(
                r#"SELECT name FROM sqlite_master 
                WHERE type = 'table' AND name IN ({})
                ORDER BY name"#,
                filter
            )
        } else {
            r#"SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name"#.to_string()
        };

        let table_rows = self.executor.execute(ds, &tables_sql).await?;
        let table_names: Vec<String> = table_rows
            .into_iter()
            .filter_map(|row| row.get("name").and_then(|v| v.as_str()).map(String::from))
            .filter(|name| !name.starts_with("sqlite_"))
            .collect();

        let mut tables = Vec::new();
        for table_name in table_names {
            let columns = self.get_sqlite_columns(ds, &table_name).await?;
            let primary_keys = self.get_sqlite_primary_keys(ds, &table_name).await?;
            let foreign_keys = self.get_sqlite_foreign_keys(ds, &table_name).await?;

            tables.push(TableInfo {
                name: table_name.clone(),
                columns,
                primary_keys,
                foreign_keys,
            });
        }

        Ok(tables)
    }

    async fn get_sqlite_columns(&self, ds: &DataSource, table: &str) -> Result<Vec<ColumnInfo>> {
        let sql = format!("PRAGMA table_info({})", table);
        let rows = self.executor.execute(ds, &sql).await?;
        let mut columns = Vec::new();
        for row in rows {
            let name = row.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let data_type = row.get("type").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let not_null = row.get("notnull").and_then(|v| v.as_i64()).unwrap_or(0) == 1;
            let is_pk = row.get("pk").and_then(|v| v.as_i64()).unwrap_or(0) == 1;
            let default = row.get("dflt_value").and_then(|v| v.as_str()).map(String::from);

            columns.push(ColumnInfo {
                name,
                data_type,
                is_nullable: !not_null,
                is_primary_key: is_pk,
                default,
            });
        }
        Ok(columns)
    }

    async fn get_sqlite_primary_keys(&self, ds: &DataSource, table: &str) -> Result<Vec<String>> {
        let sql = format!("PRAGMA table_info({})", table);
        let rows = self.executor.execute(ds, &sql).await?;
        Ok(rows
            .into_iter()
            .filter(|row| row.get("pk").and_then(|v| v.as_i64()).unwrap_or(0) == 1)
            .filter_map(|row| row.get("name").and_then(|v| v.as_str()).map(String::from))
            .collect())
    }

    async fn get_sqlite_foreign_keys(&self, ds: &DataSource, table: &str) -> Result<Vec<ForeignKeyInfo>> {
        let sql = format!("PRAGMA foreign_key_list({})", table);
        let rows = self.executor.execute(ds, &sql).await?;
        let mut fks = Vec::new();
        for row in rows {
            if let (Some(column), Some(ref_table), Some(ref_column)) = (
                row.get("from").and_then(|v| v.as_str()),
                row.get("table").and_then(|v| v.as_str()),
                row.get("to").and_then(|v| v.as_str()),
            ) {
                fks.push(ForeignKeyInfo {
                    column: column.to_string(),
                    referenced_table: ref_table.to_string(),
                    referenced_column: ref_column.to_string(),
                });
            }
        }
        Ok(fks)
    }
}

pub fn infer_model_from_table(
    table: &TableInfo,
    datasource_name: &str,
    all_tables: &[TableInfo],
) -> Model {
    let table_name = &table.name;
    let model_name = to_snake_case(table_name);

    let mut measures = Vec::new();
    let mut dimensions = Vec::new();
    let mut time_dimensions = Vec::new();

    for column in &table.columns {
        if column.is_primary_key {
            dimensions.push(Dimension {
                name: column.name.clone(),
                label: Some(to_title_case(&column.name)),
            });
            continue;
        }

        let col_type = column.data_type.to_lowercase();
        let is_numeric = is_numeric_type(&col_type);
        let is_temporal = is_temporal_type(&col_type);

        if is_temporal {
            time_dimensions.push(TimeDimension {
                dimension: column.name.clone(),
                granularity: TimeGranularity::Day,
                label: Some(to_title_case(&column.name)),
            });
            dimensions.push(Dimension {
                name: column.name.clone(),
                label: Some(to_title_case(&column.name)),
            });
        } else if is_numeric {
            measures.push(Measure {
                formula: Formula::new(column.name.clone()),
                aggregation: AggregationType::Sum,
            });
            dimensions.push(Dimension {
                name: column.name.clone(),
                label: Some(to_title_case(&column.name)),
            });
        } else {
            dimensions.push(Dimension {
                name: column.name.clone(),
                label: Some(to_title_case(&column.name)),
            });
        }
    }

    let mut joins = Vec::new();
    for fk in &table.foreign_keys {
        if let Some(_ref_table) = all_tables.iter().find(|t| t.name == fk.referenced_table) {
            let join_name = format!("{}_to_{}", model_name, to_snake_case(&fk.referenced_table));
            joins.push(Join {
                name: join_name,
                model: to_snake_case(&fk.referenced_table),
                join_type: JoinType::Left,
                on: vec![(fk.column.clone(), fk.referenced_column.clone())],
                alias: Some(to_snake_case(&fk.referenced_table)),
            });
        }
    }

    Model {
        name: model_name,
        datasource: datasource_name.to_string(),
        description: Some(format!("Auto-generated from table {}", table_name)),
        measures,
        dimensions,
        time_dimensions,
        joins,
        sql: None,
        meta: HashMap::new(),
    }
}

fn to_snake_case(s: &str) -> String {
    let mut result = String::new();
    let mut prev_upper = false;
    for (i, c) in s.chars().enumerate() {
        if c.is_uppercase() {
            if i > 0 && !prev_upper {
                result.push('_');
            }
            result.push(c.to_lowercase().next().unwrap());
            prev_upper = true;
        } else {
            result.push(c);
            prev_upper = false;
        }
    }
    result
}

fn to_title_case(s: &str) -> String {
    s.split('_')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().chain(chars).collect::<String>(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn is_numeric_type(data_type: &str) -> bool {
    matches!(
        data_type,
        "integer" | "int" | "bigint" | "smallint" | "tinyint" | "mediumint"
            | "decimal" | "numeric" | "float" | "double" | "real" | "double precision"
            | "money" | "serial" | "bigserial" | "smallserial"
    )
}

fn is_temporal_type(data_type: &str) -> bool {
    matches!(
        data_type,
        "date" | "time" | "timestamp" | "timestamptz" | "timestamp with time zone"
            | "timestamp without time zone" | "datetime" | "year"
    )
}