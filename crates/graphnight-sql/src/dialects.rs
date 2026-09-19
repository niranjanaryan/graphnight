use serde_json::Value;

/// SQL Dialect trait for database-specific SQL generation
pub trait Dialect: Send + Sync {
    fn name(&self) -> &str;

    fn quote_ident(&self, ident: &str) -> String;

    fn format_value(&self, value: &Value) -> String;

    fn generate_sql(&self, plan: &super::generator::LogicalPlan) -> anyhow::Result<String>;

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
pub struct PostgresDialect;

impl Dialect for PostgresDialect {
    fn name(&self) -> &str {
        "postgres"
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
        sql.push_str(&format!(" FROM {} ", self.quote_ident(&plan.source_table)));
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
            Value::Bool(b) => if *b { "TRUE" } else { "FALSE" }.to_string(),
            Value::Number(n) => n.to_string(),
            Value::String(s) => format!("'{}'", s.replace("'", "''")),
            Value::Array(arr) => {
                let items: Vec<String> = arr.iter().map(|v| self.format_value(v)).collect();
                format!("JSON_ARRAY({})", items.join(", "))
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

        sql.push_str(&format!(" FROM {} ", self.quote_ident(&plan.source_table)));
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

        sql.push_str(&format!(" FROM {} ", self.quote_ident(&plan.source_table)));
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
            format!("{}", format.replace("{}", column))
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

        sql.push_str(&format!(" FROM {} ", self.quote_ident(&plan.source_table)));
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
