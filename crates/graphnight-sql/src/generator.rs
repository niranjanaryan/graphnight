use anyhow::{anyhow, Result};
use graphnight_core::formula::FormulaParser;
use graphnight_core::join::JoinWalker;
use graphnight_core::models::{AggregationType, Measure, Model, Query};
use std::collections::HashMap;
use std::sync::OnceLock;

use crate::dialects::Dialect;

fn formula_parser() -> &'static FormulaParser {
    static PARSER: OnceLock<FormulaParser> = OnceLock::new();
    PARSER.get_or_init(|| FormulaParser::new().expect("FormulaParser"))
}

/// SQL Generator - converts semantic Query to dialect-specific SQL
pub struct SqlGenerator {
    dialect: Box<dyn Dialect>,
    model_registry: HashMap<String, Model>,
}

impl SqlGenerator {
    pub fn new(dialect: Box<dyn Dialect>) -> Self {
        Self {
            dialect,
            model_registry: HashMap::new(),
        }
    }

    pub fn with_models(mut self, models: Vec<Model>) -> Self {
        for model in models {
            self.model_registry.insert(model.name.clone(), model);
        }
        self
    }

    pub fn register_model(&mut self, model: Model) {
        self.model_registry.insert(model.name.clone(), model);
    }

    pub fn dialect_name(&self) -> &str {
        self.dialect.name()
    }

    /// Generate SQL for a query
    pub fn generate(&self, query: &Query) -> Result<String> {
        let plan = self.build_logical_plan(query)?;
        self.dialect.generate_sql(&plan)
    }

    /// Build logical plan from query
    fn build_logical_plan(&self, query: &Query) -> Result<LogicalPlan> {
        // Determine the source table
        let (source_table, source_alias) = self.resolve_source(query)?;

        // Build SELECT clause
        let select_items = self.build_select_items(query, &source_alias)?;

        // Build WHERE clause
        let where_clause = self.build_where_clause(query, &source_alias)?;

        // Build GROUP BY clause
        let group_by = self.build_group_by(query, &source_alias)?;

        // Build ORDER BY clause
        let order_by = self.build_order_by(query, &source_alias)?;

        // Build LIMIT/OFFSET
        let limit = query.limit;
        let offset = query.offset;

        // Handle joins
        let joins = self.build_joins(query, &source_table, &source_alias)?;

        Ok(LogicalPlan {
            source_table,
            source_alias,
            select_items,
            where_clause,
            group_by,
            order_by,
            limit,
            offset,
            joins,
            distinct: query.distinct_dimension_values.unwrap_or(false),
        })
    }

    fn resolve_source(&self, query: &Query) -> Result<(String, String)> {
        if let Some(source) = &query.source_model {
            let model = self
                .model_registry
                .get(&source.model)
                .ok_or_else(|| anyhow!("Model not found: {}", source.model))?;

            let alias = source.alias.clone().unwrap_or_else(|| source.model.clone());
            Ok((model.name.clone(), alias))
        } else if let Some(name) = &query.name {
            let model = self
                .model_registry
                .get(name)
                .ok_or_else(|| anyhow!("Model not found: {}", name))?;
            Ok((model.name.clone(), name.clone()))
        } else {
            Err(anyhow!("Query must have either name or source_model"))
        }
    }

    fn build_select_items(&self, query: &Query, source_alias: &str) -> Result<Vec<SelectItem>> {
        let mut items = Vec::new();

        // Add measures
        for measure in &query.measures {
            let expr = self.build_measure_expression(measure, source_alias)?;
            let alias = measure
                .formula
                .label
                .clone()
                .unwrap_or_else(|| measure.formula.expression.clone());
            items.push(SelectItem {
                expression: expr,
                alias: Some(alias),
            });
        }

        // Add dimensions
        for dim in &query.dimensions {
            let expr = format!("{}.{}", source_alias, self.dialect.quote_ident(&dim.name));
            let alias = dim.label.clone().unwrap_or_else(|| dim.name.clone());
            items.push(SelectItem {
                expression: expr,
                alias: Some(alias),
            });
        }

        // Add time dimensions
        for td in &query.time_dimensions {
            let expr = td.sql_expression(Some(source_alias));
            let alias = td.label.clone().unwrap_or_else(|| td.dimension.clone());
            items.push(SelectItem {
                expression: expr,
                alias: Some(alias),
            });
        }

        if items.is_empty() {
            items.push(SelectItem {
                expression: format!("{}. *", source_alias),
                alias: None,
            });
        }

        Ok(items)
    }

    fn build_measure_expression(&self, measure: &Measure, source_alias: &str) -> Result<String> {
        let expr = measure.formula.expression.trim();
        let parser = formula_parser();

        if FormulaParser::needs_parse(expr) {
            if expr.starts_with("time_shift(") {
                return self.build_time_shift_expression(expr, source_alias);
            }
            if expr.starts_with("ratio(") {
                return self.build_ratio_expression(expr, source_alias);
            }
            if expr.starts_with("pct_change(") {
                return self.build_pct_change_expression(expr, source_alias);
            }
            if expr.starts_with("running_total(") {
                return self.build_running_total_expression(expr, source_alias);
            }

            // Shorthand like "revenue:sum"
            let parsed = parser
                .parse_measure(expr)
                .map_err(|e| anyhow!("Invalid formula '{}': {}", expr, e))?;
            return self.build_standard_aggregation(&parsed, source_alias);
        }

        self.build_standard_aggregation(measure, source_alias)
    }

    fn build_standard_aggregation(&self, measure: &Measure, source_alias: &str) -> Result<String> {
        let formula = &measure.formula;
        let agg = &measure.aggregation;

        let column = if formula.expression == "*" {
            "*".to_string()
        } else {
            format!(
                "{}.{}",
                source_alias,
                self.dialect.quote_ident(&formula.expression)
            )
        };

        let agg_fn = agg.sql_function();
        if agg.needs_closing_paren() {
            Ok(format!("{}{})", agg_fn, column))
        } else {
            Ok(format!("{}({})", agg_fn, column))
        }
    }

    fn aggregate_column(
        &self,
        field: &str,
        aggregation: &AggregationType,
        source_alias: &str,
    ) -> String {
        let column = if field == "*" {
            "*".to_string()
        } else {
            format!("{}.{}", source_alias, self.dialect.quote_ident(field))
        };
        let agg_fn = aggregation.sql_function();
        if aggregation.needs_closing_paren() {
            format!("{}{})", agg_fn, column)
        } else {
            format!("{}({})", agg_fn, column)
        }
    }

    fn build_time_shift_expression(&self, expr: &str, source_alias: &str) -> Result<String> {
        let parser = formula_parser();
        let re = regex::Regex::new(r"time_shift\(([^,]+),\s*(-?\d+),\s*'(\w+)'\)")
            .map_err(|e| anyhow!(e))?;
        let caps = re
            .captures(expr)
            .ok_or_else(|| anyhow!("Invalid formula: malformed time_shift expression"))?;
        let inner = caps.get(1).unwrap().as_str().trim();
        let inner_measure = parser.parse_measure(inner)?;
        let inner_sql = self.aggregate_column(
            &inner_measure.formula.expression,
            &inner_measure.aggregation,
            source_alias,
        );
        // ORDER BY uses the source alias as a stable key until time dims are required
        Ok(format!(
            "LAG({}) OVER (ORDER BY {})",
            inner_sql, source_alias
        ))
    }

    fn build_ratio_expression(&self, expr: &str, source_alias: &str) -> Result<String> {
        let parser = formula_parser();
        let re = regex::Regex::new(r"ratio\(([^,]+),\s*([^)]+)\)").map_err(|e| anyhow!(e))?;
        let caps = re
            .captures(expr)
            .ok_or_else(|| anyhow!("Invalid formula: malformed ratio expression"))?;
        let num = parser.parse_measure(caps.get(1).unwrap().as_str().trim())?;
        let denom = parser.parse_measure(caps.get(2).unwrap().as_str().trim())?;
        let num_sql =
            self.aggregate_column(&num.formula.expression, &num.aggregation, source_alias);
        let denom_sql =
            self.aggregate_column(&denom.formula.expression, &denom.aggregation, source_alias);
        Ok(format!("({} / NULLIF({}, 0))", num_sql, denom_sql))
    }

    fn build_pct_change_expression(&self, expr: &str, source_alias: &str) -> Result<String> {
        let parser = formula_parser();
        let inner = expr
            .strip_prefix("pct_change(")
            .and_then(|s| s.strip_suffix(')'))
            .ok_or_else(|| anyhow!("Invalid formula: malformed pct_change expression"))?
            .trim();
        let measure = parser.parse_measure(inner)?;
        let current = self.aggregate_column(
            &measure.formula.expression,
            &measure.aggregation,
            source_alias,
        );
        Ok(format!(
            "(({current} - LAG({current}) OVER (ORDER BY {source_alias})) / NULLIF(LAG({current}) OVER (ORDER BY {source_alias}), 0)) * 100"
        ))
    }

    fn build_running_total_expression(&self, expr: &str, source_alias: &str) -> Result<String> {
        let parser = formula_parser();
        let inner = expr
            .strip_prefix("running_total(")
            .and_then(|s| s.strip_suffix(')'))
            .ok_or_else(|| anyhow!("Invalid formula: malformed running_total expression"))?
            .trim();
        let measure = parser.parse_measure(inner)?;
        let inner_sql = self.aggregate_column(
            &measure.formula.expression,
            &measure.aggregation,
            source_alias,
        );
        Ok(format!(
            "SUM({inner_sql}) OVER (ORDER BY {source_alias} ROWS UNBOUNDED PRECEDING)"
        ))
    }

    fn build_where_clause(&self, query: &Query, source_alias: &str) -> Result<Option<String>> {
        if query.filters.is_empty() {
            return Ok(None);
        }

        let mut conditions = Vec::new();
        for (i, filter) in query.filters.iter().enumerate() {
            let prefix = if filter.or_condition && i > 0 {
                "OR "
            } else {
                ""
            };
            let column = format!(
                "{}.{}",
                source_alias,
                self.dialect.quote_ident(&filter.field)
            );
            let op = filter.operator.sql_operator();

            let condition = if filter.operator.needs_value() {
                if filter.operator.needs_two_values() {
                    // BETWEEN
                    let vals = filter
                        .value
                        .as_array()
                        .ok_or_else(|| anyhow!("BETWEEN requires array of two values"))?;
                    format!(
                        "{} {} BETWEEN {} AND {}",
                        prefix,
                        column,
                        self.dialect.format_value(&vals[0]),
                        self.dialect.format_value(&vals[1])
                    )
                } else {
                    format!(
                        "{} {} {} {}",
                        prefix,
                        column,
                        op,
                        self.dialect.format_value(&filter.value)
                    )
                }
            } else {
                format!("{} {} {}", prefix, column, op)
            };
            conditions.push(condition);
        }

        Ok(Some(conditions.join(" ")))
    }

    fn build_group_by(&self, query: &Query, source_alias: &str) -> Result<Vec<String>> {
        let mut group_by = Vec::new();

        for dim in &query.dimensions {
            group_by.push(format!(
                "{}.{}",
                source_alias,
                self.dialect.quote_ident(&dim.name)
            ));
        }

        for td in &query.time_dimensions {
            group_by.push(td.sql_expression(Some(source_alias)));
        }

        Ok(group_by)
    }

    fn build_order_by(&self, query: &Query, source_alias: &str) -> Result<Vec<OrderByItem>> {
        let mut items = Vec::new();
        for order in &query.order {
            let column = if order.field.contains(".") {
                order.field.clone()
            } else {
                format!(
                    "{}.{}",
                    source_alias,
                    self.dialect.quote_ident(&order.field)
                )
            };
            items.push(OrderByItem {
                column,
                descending: order.descending,
            });
        }
        Ok(items)
    }

    fn build_joins(
        &self,
        query: &Query,
        source_table: &str,
        source_alias: &str,
    ) -> Result<Vec<JoinClause>> {
        let base_model_name = query
            .source_model
            .as_ref()
            .map(|s| s.model.as_str())
            .or(query.name.as_deref())
            .unwrap_or(source_table);

        let Some(model) = self.model_registry.get(base_model_name) else {
            return Ok(vec![]);
        };

        if model.joins.is_empty() {
            return Ok(vec![]);
        }

        let required: Vec<String> = model.joins.iter().map(|j| j.model.clone()).collect();
        let models: Vec<Model> = self.model_registry.values().cloned().collect();
        let walker = JoinWalker::new(models);
        let clauses = walker
            .build_join_clauses(base_model_name, &required, source_alias)
            .map_err(|e| anyhow!("Join error: {}", e))?;

        Ok(clauses
            .into_iter()
            .map(|c| JoinClause {
                join_type: c.join_type,
                table: c.table,
                alias: c.alias,
                on: self.quote_join_on(&c.on),
            })
            .collect())
    }

    /// Best-effort quoting for identifiers inside walker ON clauses (`a.b = c.d`).
    fn quote_join_on(&self, on: &str) -> String {
        on.split(" AND ")
            .map(|cond| {
                let parts: Vec<&str> = cond.split('=').map(|s| s.trim()).collect();
                if parts.len() != 2 {
                    return cond.to_string();
                }
                format!(
                    "{} = {}",
                    self.quote_qualified(parts[0]),
                    self.quote_qualified(parts[1])
                )
            })
            .collect::<Vec<_>>()
            .join(" AND ")
    }

    fn quote_qualified(&self, qualified: &str) -> String {
        let mut parts = qualified.split('.');
        match (parts.next(), parts.next()) {
            (Some(alias), Some(col)) => {
                format!("{}.{}", alias, self.dialect.quote_ident(col))
            }
            _ => self.dialect.quote_ident(qualified),
        }
    }
}

/// Logical plan representation
#[derive(Debug, Clone)]
pub struct LogicalPlan {
    pub source_table: String,
    pub source_alias: String,
    pub select_items: Vec<SelectItem>,
    pub where_clause: Option<String>,
    pub group_by: Vec<String>,
    pub order_by: Vec<OrderByItem>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
    pub joins: Vec<JoinClause>,
    pub distinct: bool,
}

#[derive(Debug, Clone)]
pub struct SelectItem {
    pub expression: String,
    pub alias: Option<String>,
}

#[derive(Debug, Clone)]
pub struct OrderByItem {
    pub column: String,
    pub descending: bool,
}

#[derive(Debug, Clone)]
pub struct JoinClause {
    pub join_type: String,
    pub table: String,
    pub alias: String,
    pub on: String,
}
