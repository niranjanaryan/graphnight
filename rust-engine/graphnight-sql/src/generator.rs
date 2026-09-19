use anyhow::{anyhow, Result};
use graphnight_core::models::{Formula, Measure, Model, Query};
use std::collections::HashMap;

use crate::dialects::Dialect;

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
        let formula = &measure.formula;
        let agg = measure.aggregation.clone();

        // Handle special expressions like time_shift, ratio
        if formula.expression.contains("time_shift") {
            return self.build_time_shift_expression(formula, source_alias);
        }
        if formula.expression.contains("ratio") {
            return self.build_ratio_expression(formula, source_alias);
        }
        if formula.expression.contains("pct_change") {
            return self.build_pct_change_expression(formula, source_alias);
        }
        if formula.expression.contains("running_total") {
            return self.build_running_total_expression(formula, source_alias);
        }

        // Build standard aggregation
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
        let expr = if agg.needs_closing_paren() {
            format!("{}({})", agg_fn, column)
        } else {
            format!("{}({})", agg_fn, column)
        };

        Ok(expr)
    }

    fn build_time_shift_expression(&self, formula: &Formula, source_alias: &str) -> Result<String> {
        // Parse time_shift(expr, offset, 'granularity')
        // This is a simplified version - in production would use proper parsing
        Ok(format!(
            "LAG({}) OVER (ORDER BY {})",
            formula.expression, source_alias
        ))
    }

    fn build_ratio_expression(&self, formula: &Formula, _source_alias: &str) -> Result<String> {
        // Parse ratio(num, denom)
        Ok(formula.expression.replace("ratio(", "").replace(")", ""))
    }

    fn build_pct_change_expression(
        &self,
        formula: &Formula,
        _source_alias: &str,
    ) -> Result<String> {
        // (current - previous) / previous * 100
        Ok(format!("(({} - LAG({}) OVER (ORDER BY time_dim)) / NULLIF(LAG({}) OVER (ORDER BY time_dim), 0)) * 100", 
            formula.expression, formula.expression, formula.expression))
    }

    fn build_running_total_expression(
        &self,
        formula: &Formula,
        _source_alias: &str,
    ) -> Result<String> {
        Ok(format!(
            "SUM({}) OVER (ORDER BY time_dim ROWS UNBOUNDED PRECEDING)",
            formula.expression
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
        _source_table: &str,
        source_alias: &str,
    ) -> Result<Vec<JoinClause>> {
        let mut joins = Vec::new();

        // If query has a model with joins, add them
        if let Some(source) = &query.source_model {
            if let Some(model) = self.model_registry.get(&source.model) {
                for join in &model.joins {
                    let joined_model = self
                        .model_registry
                        .get(&join.model)
                        .ok_or_else(|| anyhow!("Joined model not found: {}", join.model))?;

                    let join_alias = join.alias.clone().unwrap_or_else(|| join.model.clone());
                    let join_type = join.join_type.sql_keyword();

                    let mut on_conditions = Vec::new();
                    for (left, right) in &join.on {
                        on_conditions.push(format!(
                            "{}.{} = {}.{}",
                            source_alias,
                            self.dialect.quote_ident(left),
                            join_alias,
                            self.dialect.quote_ident(right)
                        ));
                    }

                    joins.push(JoinClause {
                        join_type: join_type.to_string(),
                        table: joined_model.name.clone(),
                        alias: join_alias,
                        on: on_conditions.join(" AND "),
                    });
                }
            }
        }

        Ok(joins)
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
