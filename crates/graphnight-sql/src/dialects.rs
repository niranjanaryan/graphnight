use serde_json::Value;

/// SQL Dialect trait for database-specific SQL generation
pub trait Dialect: Send + Sync {
    fn name(&self) -> &str;

    fn quote_ident(&self, ident: &str) -> String;

    fn format_value(&self, value: &Value) -> String;

    fn generate_sql(&self, plan: &super::generator::LogicalPlan) -> anyhow::Result<String>;

    /// The bind placeholder for the parameter at `index` (1-based).
    ///
    /// Filter values are never interpolated into the statement. The generated
    /// text carries a placeholder here and the value is sent separately, so a
    /// value cannot change the shape of the statement at all.
    fn placeholder(&self, index: usize) -> String {
        let _ = index;
        "?".to_string()
    }

    /// Wrap an already-rendered geometry value in a constructor, for use as the
    /// second argument of a spatial predicate.
    ///
    /// `value_sql` is whatever the caller produced: a bind placeholder when
    /// executing, a quoted literal when displaying. Returning `None` means this
    /// dialect has no equivalent, and the generator refuses rather than emitting
    /// SQL the database would reject — so a spatial filter against MySQL or
    /// SQLite fails at query time with a clear reason instead of at the driver
    /// with a syntax error.
    fn spatial_geometry(&self, value_sql: &str, from_wkt: bool) -> Option<String> {
        let _ = (value_sql, from_wkt);
        None
    }

    /// Render a geometry column as GeoJSON for a result set, or `None` where
    /// the dialect has no spatial support.
    ///
    /// A geometry column has no JSON representation the driver can decode, so
    /// the conversion has to happen in SQL. `ST_AsGeoJSON` returns text, which
    /// the extraction path already handles, and exists with the same name on
    /// both PostGIS and DuckDB spatial. MySQL and SQLite return `None` here, and
    /// the generator refuses a geometry column rather than emitting SQL that
    /// would fail at the driver.
    fn geometry_as_geojson(&self, column_sql: &str) -> Option<String> {
        let _ = column_sql;
        None
    }

    /// Render the FROM source. A plain table name is quoted as an identifier;
    /// a model's custom SQL override is inlined as a subquery. The alias is
    /// appended by the caller, so it is deliberately not added here.
    fn render_source(&self, plan: &super::generator::LogicalPlan) -> String {
        if plan.source_is_sql {
            let trimmed = plan.source_table.trim().trim_end_matches(';');
            return format!("({})", trimmed);
        }
        self.quote_ident(&plan.source_table)
    }

    fn current_timestamp(&self) -> String;

    fn date_trunc(&self, unit: &str, column: &str) -> String;

    fn cast(&self, expr: &str, target_type: &str) -> String;

    fn limit_offset(&self, sql: &str, limit: Option<usize>, offset: Option<usize>) -> String;

    fn supports_cte(&self) -> bool {
        true
    }

    fn supports_window_functions(&self) -> bool {
        true
    }

    fn supports_lateral_join(&self) -> bool {
        false
    }

    fn array_agg(&self, expr: &str) -> String;

    fn json_extract(&self, column: &str, path: &str) -> String;
}

/// PostgreSQL dialect
/// Geometry constructor rendering shared by the dialects that implement
/// OGC-style spatial predicates: PostGIS and DuckDB spatial.
///
/// The `CAST` is load-bearing rather than defensive. With a bare prepared
/// parameter DuckDB fails to resolve the constructor's overload and reports
/// `ST_GeomFromText requires a string argument`; naming the type fixes it, and
/// is harmless on Postgres where the function signature already pins the
/// argument. Both forms were run against a real DuckDB engine.
fn ogc_geometry_ctor(value_sql: &str, from_wkt: bool) -> String {
    let ctor = if from_wkt {
        "ST_GeomFromText"
    } else {
        "ST_GeomFromGeoJSON"
    };
    format!("{ctor}(CAST({value_sql} AS VARCHAR))")
}

pub struct PostgresDialect;

impl Dialect for PostgresDialect {
    fn spatial_geometry(&self, value_sql: &str, from_wkt: bool) -> Option<String> {
        Some(ogc_geometry_ctor(value_sql, from_wkt))
    }

    fn geometry_as_geojson(&self, column_sql: &str) -> Option<String> {
        Some(format!("ST_AsGeoJSON({column_sql})"))
    }

    fn name(&self) -> &str {
        "postgres"
    }

    fn quote_ident(&self, ident: &str) -> String {
        format!("\"{}\"", ident.replace("\"", "\"\""))
    }

    fn placeholder(&self, index: usize) -> String {
        format!("${index}")
    }

    fn format_value(&self, value: &Value) -> String {
        match value {
            Value::Null => "NULL".to_string(),
            Value::Bool(b) => if *b { "TRUE" } else { "FALSE" }.to_string(),
            Value::Number(n) => n.to_string(),
            Value::String(s) => format!("'{}'", s.replace("'", "''")),
            Value::Array(arr) => {
                let items: Vec<String> = arr.iter().map(|v| self.format_value(v)).collect();
                format!("ARRAY[{}]", items.join(", "))
            }
            Value::Object(_) => format!("'{}'", value.to_string().replace("'", "''")),
        }
    }

    fn generate_sql(&self, plan: &super::generator::LogicalPlan) -> anyhow::Result<String> {
        let mut sql = String::new();

        // WITH clause for CTEs if needed
        // (would be added for multi-stage queries)

        // SELECT
        sql.push_str("SELECT ");
        if plan.distinct {
            sql.push_str("DISTINCT ");
        }

        let select_strs: Vec<String> = plan
            .select_items
            .iter()
            .map(|item| match &item.alias {
                Some(alias) => format!("{} AS {}", item.expression, self.quote_ident(alias)),
                None => item.expression.clone(),
            })
            .collect();
        sql.push_str(&select_strs.join(", "));

        // FROM
        sql.push_str(&format!(" FROM {} ", self.render_source(plan)));
        sql.push_str(&format!("{} ", self.quote_ident(&plan.source_alias)));

        // JOINs
        for join in &plan.joins {
            sql.push_str(&format!(
                " {} {} {} ON {} ",
                join.join_type,
                self.quote_ident(&join.table),
                self.quote_ident(&join.alias),
                join.on
            ));
        }

        // WHERE
        if let Some(where_clause) = &plan.where_clause {
            sql.push_str(&format!(" WHERE {} ", where_clause));
        }

        // GROUP BY
        if !plan.group_by.is_empty() {
            sql.push_str(&format!(" GROUP BY {} ", plan.group_by.join(", ")));
        }

        // ORDER BY
        if !plan.order_by.is_empty() {
            let order_strs: Vec<String> = plan
                .order_by
                .iter()
                .map(|item| {
                    format!(
                        "{} {}",
                        item.column,
                        if item.descending { "DESC" } else { "ASC" }
                    )
                })
                .collect();
            sql.push_str(&format!(" ORDER BY {} ", order_strs.join(", ")));
        }

        // LIMIT/OFFSET
        sql = self.limit_offset(&sql, plan.limit, plan.offset);

        Ok(sql.trim().to_string())
    }

    fn current_timestamp(&self) -> String {
        "NOW()".to_string()
    }

    fn date_trunc(&self, unit: &str, column: &str) -> String {
        format!("DATE_TRUNC('{}', {})", unit, column)
    }

    fn cast(&self, expr: &str, target_type: &str) -> String {
        format!("{}::{}", expr, target_type)
    }

    fn limit_offset(&self, sql: &str, limit: Option<usize>, offset: Option<usize>) -> String {
        let mut result = sql.to_string();
        if let Some(lim) = limit {
            result.push_str(&format!(" LIMIT {}", lim));
        }
        if let Some(off) = offset {
            result.push_str(&format!(" OFFSET {}", off));
        }
        result
    }

    fn supports_lateral_join(&self) -> bool {
        true
    }

    fn array_agg(&self, expr: &str) -> String {
        format!("ARRAY_AGG({})", expr)
    }

    fn json_extract(&self, column: &str, path: &str) -> String {
        format!("{} ->> '{}'", column, path)
    }
}

/// MySQL dialect
pub struct MySqlDialect;

impl MySqlDialect {
    /// Escape a MySQL string literal.
    ///
    /// Backslash first, then the quote. The order is the whole point: MySQL
    /// treats `\` as an escape character inside string literals by default, so
    /// a value of `\'` would otherwise become an escaped quote, and the *next*
    /// quote would close the literal early. Escaping the backslash first means
    /// `\'` reaches the server as a literal backslash followed by a quote that
    /// is then itself doubled.
    ///
    /// A session with `NO_BACKSLASH_ESCAPES` enabled is also safe: there the
    /// doubled quote is the correct escape, and the extra backslash is literal
    /// data rather than an escape. That is the better of the two failure modes.
    fn escape_literal(s: &str) -> String {
        s.replace('\\', "\\\\").replace('\'', "''")
    }
}

impl Dialect for MySqlDialect {
    fn name(&self) -> &str {
        "mysql"
    }

    fn quote_ident(&self, ident: &str) -> String {
        format!("`{}`", ident.replace("`", "``"))
    }

    fn format_value(&self, value: &Value) -> String {
        match value {
            Value::Null => "NULL".to_string(),
            Value::Bool(b) => {
                if *b {
                    "TRUE".to_string()
                } else {
                    "FALSE".to_string()
                }
            }
            Value::Number(n) => n.to_string(),
            Value::String(s) => format!("'{}'", Self::escape_literal(s)),
            Value::Array(arr) => {
                let items: Vec<String> = arr.iter().map(|v| self.format_value(v)).collect();
                format!("JSON_ARRAY({})", items.join(", "))
            }
            Value::Object(_) => format!("'{}'", Self::escape_literal(&value.to_string())),
        }
    }

    fn generate_sql(&self, plan: &super::generator::LogicalPlan) -> anyhow::Result<String> {
        let mut sql = String::new();

        sql.push_str("SELECT ");
        if plan.distinct {
            sql.push_str("DISTINCT ");
        }

        let select_strs: Vec<String> = plan
            .select_items
            .iter()
            .map(|item| match &item.alias {
                Some(alias) => format!("{} AS {}", item.expression, self.quote_ident(alias)),
                None => item.expression.clone(),
            })
            .collect();
        sql.push_str(&select_strs.join(", "));

        sql.push_str(&format!(" FROM {} ", self.render_source(plan)));
        sql.push_str(&format!("{} ", self.quote_ident(&plan.source_alias)));

        for join in &plan.joins {
            sql.push_str(&format!(
                " {} {} {} ON {} ",
                join.join_type,
                self.quote_ident(&join.table),
                self.quote_ident(&join.alias),
                join.on
            ));
        }

        if let Some(where_clause) = &plan.where_clause {
            sql.push_str(&format!(" WHERE {} ", where_clause));
        }

        if !plan.group_by.is_empty() {
            sql.push_str(&format!(" GROUP BY {} ", plan.group_by.join(", ")));
        }

        if !plan.order_by.is_empty() {
            let order_strs: Vec<String> = plan
                .order_by
                .iter()
                .map(|item| {
                    format!(
                        "{} {}",
                        item.column,
                        if item.descending { "DESC" } else { "ASC" }
                    )
                })
                .collect();
            sql.push_str(&format!(" ORDER BY {} ", order_strs.join(", ")));
        }

        sql = self.limit_offset(&sql, plan.limit, plan.offset);

        Ok(sql.trim().to_string())
    }

    fn current_timestamp(&self) -> String {
        "NOW()".to_string()
    }

    fn date_trunc(&self, unit: &str, column: &str) -> String {
        // MySQL doesn't have DATE_TRUNC, use DATE_FORMAT
        let format = match unit {
            "second" => "%Y-%m-%d %H:%i:%s",
            "minute" => "%Y-%m-%d %H:%i:00",
            "hour" => "%Y-%m-%d %H:00:00",
            "day" => "%Y-%m-%d",
            "week" => "%Y-%u",
            "month" => "%Y-%m",
            "quarter" => "QUARTER",
            "year" => "%Y",
            _ => "%Y-%m-%d",
        };
        if format == "QUARTER" {
            format!("QUARTER({})", column)
        } else {
            format!("DATE_FORMAT({}, '{}')", column, format)
        }
    }

    fn cast(&self, expr: &str, target_type: &str) -> String {
        format!("CAST({} AS {})", expr, target_type)
    }

    fn limit_offset(&self, sql: &str, limit: Option<usize>, offset: Option<usize>) -> String {
        let mut result = sql.to_string();
        if let Some(lim) = limit {
            if let Some(off) = offset {
                result.push_str(&format!(" LIMIT {} OFFSET {}", off, lim));
            } else {
                result.push_str(&format!(" LIMIT {}", lim));
            }
        } else if let Some(off) = offset {
            result.push_str(&format!(
                " LIMIT {} OFFSET {}",
                18446744073709551615u64, off
            ));
        }
        result
    }

    fn supports_lateral_join(&self) -> bool {
        false
    }

    fn array_agg(&self, expr: &str) -> String {
        format!("JSON_ARRAYAGG({})", expr)
    }

    fn json_extract(&self, column: &str, path: &str) -> String {
        format!("JSON_UNQUOTE(JSON_EXTRACT({}, '$.{}'))", column, path)
    }
}

/// SQLite dialect
pub struct SqliteDialect;

impl Dialect for SqliteDialect {
    fn name(&self) -> &str {
        "sqlite"
    }

    fn quote_ident(&self, ident: &str) -> String {
        format!("\"{}\"", ident.replace("\"", "\"\""))
    }

    fn format_value(&self, value: &Value) -> String {
        match value {
            Value::Null => "NULL".to_string(),
            Value::Bool(b) => if *b { "1" } else { "0" }.to_string(),
            Value::Number(n) => n.to_string(),
            Value::String(s) => format!("'{}'", s.replace("'", "''")),
            Value::Array(arr) => {
                let items: Vec<String> = arr.iter().map(|v| self.format_value(v)).collect();
                format!("JSON_GROUP_ARRAY({})", items.join(","))
            }
            Value::Object(_) => format!("'{}'", value.to_string().replace("'", "''")),
        }
    }

    fn generate_sql(&self, plan: &super::generator::LogicalPlan) -> anyhow::Result<String> {
        let mut sql = String::new();

        sql.push_str("SELECT ");
        if plan.distinct {
            sql.push_str("DISTINCT ");
        }

        let select_strs: Vec<String> = plan
            .select_items
            .iter()
            .map(|item| match &item.alias {
                Some(alias) => format!("{} AS {}", item.expression, self.quote_ident(alias)),
                None => item.expression.clone(),
            })
            .collect();
        sql.push_str(&select_strs.join(", "));

        sql.push_str(&format!(" FROM {} ", self.render_source(plan)));
        sql.push_str(&format!("{} ", self.quote_ident(&plan.source_alias)));

        for join in &plan.joins {
            sql.push_str(&format!(
                " {} {} {} ON {} ",
                join.join_type,
                self.quote_ident(&join.table),
                self.quote_ident(&join.alias),
                join.on
            ));
        }

        if let Some(where_clause) = &plan.where_clause {
            sql.push_str(&format!(" WHERE {} ", where_clause));
        }

        if !plan.group_by.is_empty() {
            sql.push_str(&format!(" GROUP BY {} ", plan.group_by.join(", ")));
        }

        if !plan.order_by.is_empty() {
            let order_strs: Vec<String> = plan
                .order_by
                .iter()
                .map(|item| {
                    format!(
                        "{} {}",
                        item.column,
                        if item.descending { "DESC" } else { "ASC" }
                    )
                })
                .collect();
            sql.push_str(&format!(" ORDER BY {} ", order_strs.join(", ")));
        }

        sql = self.limit_offset(&sql, plan.limit, plan.offset);

        Ok(sql.trim().to_string())
    }

    fn current_timestamp(&self) -> String {
        "datetime('now')".to_string()
    }

    fn date_trunc(&self, unit: &str, column: &str) -> String {
        let format = match unit {
            "second" => "%Y-%m-%d %H:%M:%S",
            "minute" => "%Y-%m-%d %H:%M:00",
            "hour" => "%Y-%m-%d %H:00:00",
            "day" => "%Y-%m-%d",
            "week" => "%Y-%W",
            "month" => "%Y-%m",
            "quarter" => "((CAST(strftime('%m', {}) AS INTEGER) - 1) / 3) + 1",
            "year" => "%Y",
            _ => "%Y-%m-%d",
        };
        if format.contains("quarter") {
            format.replace("{}", column).to_string()
        } else {
            format!("strftime('{}', {})", format, column)
        }
    }

    fn cast(&self, expr: &str, target_type: &str) -> String {
        format!("CAST({} AS {})", expr, target_type)
    }

    fn limit_offset(&self, sql: &str, limit: Option<usize>, offset: Option<usize>) -> String {
        let mut result = sql.to_string();
        if let Some(lim) = limit {
            result.push_str(&format!(" LIMIT {}", lim));
        }
        if let Some(off) = offset {
            result.push_str(&format!(" OFFSET {}", off));
        }
        result
    }

    fn supports_lateral_join(&self) -> bool {
        false
    }

    fn array_agg(&self, expr: &str) -> String {
        format!("JSON_GROUP_ARRAY({})", expr)
    }

    fn json_extract(&self, column: &str, path: &str) -> String {
        format!("JSON_EXTRACT({}, '$.{}')", column, path)
    }
}

/// DuckDB dialect
pub struct DuckDbDialect;

impl Dialect for DuckDbDialect {
    fn spatial_geometry(&self, value_sql: &str, from_wkt: bool) -> Option<String> {
        Some(ogc_geometry_ctor(value_sql, from_wkt))
    }

    fn geometry_as_geojson(&self, column_sql: &str) -> Option<String> {
        Some(format!("ST_AsGeoJSON({column_sql})"))
    }

    fn name(&self) -> &str {
        "duckdb"
    }

    fn quote_ident(&self, ident: &str) -> String {
        format!("\"{}\"", ident.replace("\"", "\"\""))
    }

    fn format_value(&self, value: &Value) -> String {
        match value {
            Value::Null => "NULL".to_string(),
            Value::Bool(b) => if *b { "TRUE" } else { "FALSE" }.to_string(),
            Value::Number(n) => n.to_string(),
            Value::String(s) => format!("'{}'", s.replace("'", "''")),
            Value::Array(arr) => {
                let items: Vec<String> = arr.iter().map(|v| self.format_value(v)).collect();
                format!("[{}]", items.join(", "))
            }
            Value::Object(_) => format!("'{}'", value.to_string().replace("'", "''")),
        }
    }

    fn generate_sql(&self, plan: &super::generator::LogicalPlan) -> anyhow::Result<String> {
        let mut sql = String::new();

        sql.push_str("SELECT ");
        if plan.distinct {
            sql.push_str("DISTINCT ");
        }

        let select_strs: Vec<String> = plan
            .select_items
            .iter()
            .map(|item| match &item.alias {
                Some(alias) => format!("{} AS {}", item.expression, self.quote_ident(alias)),
                None => item.expression.clone(),
            })
            .collect();
        sql.push_str(&select_strs.join(", "));

        sql.push_str(&format!(" FROM {} ", self.render_source(plan)));
        sql.push_str(&format!("{} ", self.quote_ident(&plan.source_alias)));

        for join in &plan.joins {
            sql.push_str(&format!(
                " {} {} {} ON {} ",
                join.join_type,
                self.quote_ident(&join.table),
                self.quote_ident(&join.alias),
                join.on
            ));
        }

        if let Some(where_clause) = &plan.where_clause {
            sql.push_str(&format!(" WHERE {} ", where_clause));
        }

        if !plan.group_by.is_empty() {
            sql.push_str(&format!(" GROUP BY {} ", plan.group_by.join(", ")));
        }

        if !plan.order_by.is_empty() {
            let order_strs: Vec<String> = plan
                .order_by
                .iter()
                .map(|item| {
                    format!(
                        "{} {}",
                        item.column,
                        if item.descending { "DESC" } else { "ASC" }
                    )
                })
                .collect();
            sql.push_str(&format!(" ORDER BY {} ", order_strs.join(", ")));
        }

        sql = self.limit_offset(&sql, plan.limit, plan.offset);

        Ok(sql.trim().to_string())
    }

    fn current_timestamp(&self) -> String {
        "CURRENT_TIMESTAMP".to_string()
    }

    fn date_trunc(&self, unit: &str, column: &str) -> String {
        if unit == "day" {
            // Day buckets should carry DATE, not TIMESTAMP (SQLite's
            // strftime-based day bucket is an ISO date string too).
            format!("CAST(DATE_TRUNC('day', {}) AS DATE)", column)
        } else {
            format!("DATE_TRUNC('{}', {})", unit, column)
        }
    }

    fn cast(&self, expr: &str, target_type: &str) -> String {
        format!("{}::{}", expr, target_type)
    }

    fn limit_offset(&self, sql: &str, limit: Option<usize>, offset: Option<usize>) -> String {
        let mut result = sql.to_string();
        if let Some(lim) = limit {
            result.push_str(&format!(" LIMIT {}", lim));
        }
        if let Some(off) = offset {
            result.push_str(&format!(" OFFSET {}", off));
        }
        result
    }

    fn supports_lateral_join(&self) -> bool {
        true
    }

    fn array_agg(&self, expr: &str) -> String {
        format!("LIST({})", expr)
    }

    fn json_extract(&self, column: &str, path: &str) -> String {
        format!("{}.{}", column, path)
    }
}

/// Get dialect by name
pub fn get_dialect(name: &str) -> Box<dyn Dialect> {
    match name.to_lowercase().as_str() {
        "postgres" | "postgresql" | "pg" => Box::new(PostgresDialect),
        "mysql" | "mariadb" => Box::new(MySqlDialect),
        "sqlite" | "sqlite3" => Box::new(SqliteDialect),
        "duckdb" => Box::new(DuckDbDialect),
        _ => Box::new(PostgresDialect), // default
    }
}
