use anyhow::{anyhow, Result};
use graphnight_core::formula::{self, FormulaParser};
use graphnight_core::join::JoinWalker;
use graphnight_core::models::{
    AggregationType, DataSource, FilterOperator, Measure, Model, Query, TimeDimension,
};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, OnceLock, RwLock};

use crate::dialects::Dialect;

fn formula_parser() -> &'static FormulaParser {
    static PARSER: OnceLock<FormulaParser> = OnceLock::new();
    PARSER.get_or_init(|| FormulaParser::new().expect("FormulaParser"))
}

/// SQL Generator - converts semantic Query to dialect-specific SQL.
///
/// A workspace may hold models on datasources with different drivers, so the
/// dialect is resolved per query from the source model's datasource rather than
/// fixed for the whole generator. `default_dialect` is used whenever a model has
/// no registered datasource, keeping single-datasource behaviour unchanged.
pub struct SqlGenerator {
    default_dialect: Box<dyn Dialect>,
    dialects: HashMap<String, Box<dyn Dialect>>,
    /// Interior-mutable so a model created or updated while the process is
    /// running can be registered without rebuilding the engine. A `HashMap`
    /// behind a plain field would freeze the registry at construction time and
    /// leave runtime-created models unqueryable.
    model_registry: Arc<RwLock<HashMap<String, Model>>>,
    datasource_drivers: HashMap<String, String>,
}

impl SqlGenerator {
    pub fn new(dialect: Box<dyn Dialect>) -> Self {
        Self {
            default_dialect: dialect,
            dialects: HashMap::new(),
            model_registry: Arc::new(RwLock::new(HashMap::new())),
            datasource_drivers: HashMap::new(),
        }
    }

    pub fn with_models(self, models: Vec<Model>) -> Self {
        {
            let mut registry = self.model_registry.write().expect("model registry lock");
            for model in models {
                registry.insert(model.name.clone(), model);
            }
        }
        self
    }

    /// Look up a model by name.
    ///
    /// Returns an owned clone because the registry is behind a lock: handing out
    /// a borrow would not be possible without holding the guard for the whole
    /// generation, and model metadata is small.
    fn model(&self, name: &str) -> Option<Model> {
        self.model_registry
            .read()
            .expect("model registry lock")
            .get(name)
            .cloned()
    }

    /// Register datasources so queries resolve the correct driver dialect.
    pub fn with_datasources(mut self, datasources: Vec<DataSource>) -> Self {
        for ds in datasources {
            self.register_datasource(ds);
        }
        self
    }

    pub fn register_datasource(&mut self, ds: DataSource) {
        self.datasource_drivers
            .insert(ds.name.clone(), ds.driver.clone());
        // The default dialect is kept as-is; only foreign drivers need an entry.
        if ds.driver != self.default_dialect.name() && !self.dialects.contains_key(&ds.driver) {
            self.dialects
                .insert(ds.driver.clone(), crate::dialects::get_dialect(&ds.driver));
        }
    }

    /// Register or replace a model at runtime.
    ///
    /// Takes `&self` so a shared, already-constructed engine can learn about
    /// models created through the admin API after startup.
    pub fn register_model(&self, model: Model) {
        self.model_registry
            .write()
            .expect("model registry lock")
            .insert(model.name.clone(), model);
    }

    /// Drop a model, e.g. after it is deleted.
    pub fn unregister_model(&self, name: &str) {
        self.model_registry
            .write()
            .expect("model registry lock")
            .remove(name);
    }

    pub fn dialect_name(&self) -> &str {
        self.default_dialect.name()
    }

    /// The dialect that will be used to generate SQL for `model_name`.
    pub fn dialect_for_model(&self, model_name: &str) -> &dyn Dialect {
        self.dialect_for(model_name)
    }

    /// Dialect name resolved for the model a query reads from. Used as part of
    /// the plan-cache key so two models on different drivers never share a
    /// cached statement.
    pub fn resolved_dialect_name(&self, query: &Query) -> String {
        match self.resolve_source(query) {
            Ok((model, _)) => self.dialect_for(&model).name().to_string(),
            Err(_) => self.default_dialect.name().to_string(),
        }
    }

    fn dialect_for(&self, model_name: &str) -> &dyn Dialect {
        let driver = self
            .model(model_name)
            .and_then(|m| self.datasource_drivers.get(&m.datasource).cloned());
        match driver {
            Some(d) if d != self.default_dialect.name() => self
                .dialects
                .get(&d)
                .map(|b| b.as_ref())
                .unwrap_or(self.default_dialect.as_ref()),
            _ => self.default_dialect.as_ref(),
        }
    }

    /// Generate SQL for a query.
    ///
    /// Produces both forms from the same plan builder: `sql` is safe to execute
    /// because every filter value is a placeholder, and `display` renders the
    /// same statement with the values inlined for a human. Building both from
    /// one code path is deliberate — a second, hand-written display renderer
    /// would drift from what actually runs.
    pub fn generate(&self, query: &Query) -> Result<GeneratedSql> {
        let (source_table, source_alias) = self.resolve_source(query)?;
        let dialect = self.dialect_for(&source_table);

        let bound = self.build_logical_plan(
            query,
            dialect,
            &source_table,
            &source_alias,
            ValueMode::Placeholder,
        )?;
        let params = bound.params.clone();
        let sql = dialect.generate_sql(&bound)?;

        let shown = self.build_logical_plan(
            query,
            dialect,
            &source_table,
            &source_alias,
            ValueMode::Literal,
        )?;
        let display = dialect.generate_sql(&shown)?;

        Ok(GeneratedSql {
            sql,
            params,
            display,
        })
    }

    /// Build logical plan from query
    fn build_logical_plan(
        &self,
        query: &Query,
        dialect: &dyn Dialect,
        source_table: &str,
        source_alias: &str,
        mode: ValueMode,
    ) -> Result<LogicalPlan> {
        let model = self.model(source_table);
        let base_model_name = model
            .as_ref()
            .map(|m| m.name.clone())
            .unwrap_or_else(|| source_table.to_string());
        // A model may override its own source with SQL. This is the only way a
        // model can be backed by something other than a table of the same name,
        // and it is what the importers emit for a Cube `sql_table`.
        let source_override = model
            .as_ref()
            .and_then(|m| m.sql.as_ref())
            .map(|sql| sql.trim().to_string())
            .filter(|sql| !sql.is_empty());

        // Build SELECT clause
        let select_items =
            self.build_select_items(query, dialect, source_alias, &base_model_name)?;

        // Build WHERE clause
        let mut params: Vec<Value> = Vec::new();
        let where_clause = self.build_where_clause(
            query,
            dialect,
            source_alias,
            &base_model_name,
            mode,
            &mut params,
        )?;

        // Build GROUP BY clause
        let group_by = self.build_group_by(query, dialect, source_alias, &base_model_name)?;

        // Build ORDER BY clause
        let order_by = self.build_order_by(query, dialect, source_alias)?;

        // Build LIMIT/OFFSET
        let limit = query.limit;
        let offset = query.offset;

        // Handle joins
        let joins = self.build_joins(query, dialect, source_table, source_alias)?;

        Ok(LogicalPlan {
            source_table: match &source_override {
                Some(sql) => sql.clone(),
                None => source_table.to_string(),
            },
            source_is_sql: source_override.is_some(),
            source_alias: source_alias.to_string(),
            select_items,
            where_clause,
            group_by,
            order_by,
            limit,
            offset,
            joins,
            distinct: query.distinct_dimension_values.unwrap_or(false),
            params,
        })
    }

    fn resolve_source(&self, query: &Query) -> Result<(String, String)> {
        if let Some(source) = &query.source_model {
            let model = self
                .model(&source.model)
                .ok_or_else(|| anyhow!("Model not found: {}", source.model))?;

            let alias = source.alias.clone().unwrap_or_else(|| source.model.clone());
            Ok((model.name, alias))
        } else if let Some(name) = &query.name {
            let model = self
                .model(name)
                .ok_or_else(|| anyhow!("Model not found: {}", name))?;
            Ok((model.name, name.clone()))
        } else {
            Err(anyhow!("Query must have either name or source_model"))
        }
    }

    fn build_select_items(
        &self,
        query: &Query,
        dialect: &dyn Dialect,
        source_alias: &str,
        base_model_name: &str,
    ) -> Result<Vec<SelectItem>> {
        let mut items = Vec::new();

        // Add measures
        for measure in &query.measures {
            let expr =
                self.build_measure_expression(measure, dialect, source_alias, base_model_name)?;
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
            let mut expr = self.qualify_field(dialect, base_model_name, &dim.name, source_alias);
            if self.field_is_geometry(base_model_name, &dim.name) {
                // A geometry column has no JSON form the driver can decode, so
                // it is converted in SQL. A dialect without the function is
                // refused here with a reason instead of emitting SQL the driver
                // would fail on.
                expr = dialect.geometry_as_geojson(&expr).ok_or_else(|| {
                    anyhow!(
                        "`{}` is a geometry column, and the {} dialect has no spatial support \
                         to return it as GeoJSON",
                        dim.name,
                        dialect.name()
                    )
                })?;
            }
            let alias = dim.label.clone().unwrap_or_else(|| dim.name.clone());
            items.push(SelectItem {
                expression: expr,
                alias: Some(alias),
            });
        }

        // Add time dimensions
        for td in &query.time_dimensions {
            items.push(SelectItem {
                expression: self.time_dim_expr(dialect, td, Some(source_alias)),
                alias: Some(td.label.clone().unwrap_or_else(|| {
                    format!("{}_{}", td.dimension, td.granularity.date_trunc_unit())
                })),
            });
        }

        Ok(items)
    }

    fn build_measure_expression(
        &self,
        measure: &Measure,
        dialect: &dyn Dialect,
        source_alias: &str,
        base_model_name: &str,
    ) -> Result<String> {
        let expr = measure.formula.expression.trim();
        let parser = formula_parser();

        if FormulaParser::needs_parse(expr) {
            if expr.starts_with("time_shift(") {
                return self.build_time_shift_expression(
                    expr,
                    dialect,
                    source_alias,
                    base_model_name,
                );
            }
            if expr.starts_with("ratio(") {
                return self.build_ratio_expression(expr, dialect, source_alias, base_model_name);
            }
            if expr.starts_with("pct_change(") {
                return self.build_pct_change_expression(
                    expr,
                    dialect,
                    source_alias,
                    base_model_name,
                );
            }
            if expr.starts_with("running_total(") {
                return self.build_running_total_expression(
                    expr,
                    dialect,
                    source_alias,
                    base_model_name,
                );
            }

            // Shorthand like "revenue:sum"
            let parsed = parser
                .parse_measure(expr)
                .map_err(|e| anyhow!("Invalid formula '{}': {}", expr, e))?;
            return self.build_standard_aggregation(
                &parsed,
                dialect,
                source_alias,
                base_model_name,
            );
        }

        self.build_standard_aggregation(measure, dialect, source_alias, base_model_name)
    }

    /// Resolve a measure's expression to SQL.
    ///
    /// A bare name becomes a qualified column. An expression shaped like a call
    /// to an allowlisted function becomes that call, with every column argument
    /// re-qualified through the dialect. The output is assembled from validated
    /// parts and never copied from the input text, so a function name or
    /// argument cannot carry SQL of its own into the statement.
    fn resolve_expression(
        &self,
        dialect: &dyn Dialect,
        base_model_name: &str,
        expr: &str,
        source_alias: &str,
    ) -> Result<String> {
        let call = formula::parse_function_call(expr)
            .map_err(|e| anyhow!("Invalid formula '{}': {}", expr, e))?;
        let Some(call) = call else {
            return Ok(self.qualify_field(dialect, base_model_name, expr, source_alias));
        };

        let mut rendered = Vec::with_capacity(call.args.len());
        for arg in &call.args {
            rendered.push(match arg {
                formula::FunctionArg::Column(col) => {
                    self.qualify_field(dialect, base_model_name, col, source_alias)
                }
                formula::FunctionArg::Number(n) => n.clone(),
            });
        }
        // The function name passed the allowlist, which only admits plain
        // identifiers, so it needs no further quoting.
        Ok(format!("{}({})", call.name, rendered.join(", ")))
    }

    fn build_standard_aggregation(
        &self,
        measure: &Measure,
        dialect: &dyn Dialect,
        source_alias: &str,
        base_model_name: &str,
    ) -> Result<String> {
        let formula = &measure.formula;
        let agg = &measure.aggregation;

        let column =
            self.resolve_expression(dialect, base_model_name, &formula.expression, source_alias)?;

        let agg_fn = agg.sql_function();
        if agg.needs_closing_paren() {
            Ok(format!("{}{})", agg_fn, column))
        } else {
            Ok(format!("{}({})", agg_fn, column))
        }
    }

    fn aggregate_column(
        &self,
        dialect: &dyn Dialect,
        field: &str,
        aggregation: &AggregationType,
        source_alias: &str,
        base_model_name: &str,
    ) -> String {
        let column = self.qualify_field(dialect, base_model_name, field, source_alias);
        let agg_fn = aggregation.sql_function();
        if aggregation.needs_closing_paren() {
            format!("{}{})", agg_fn, column)
        } else {
            format!("{}({})", agg_fn, column)
        }
    }

    fn build_time_shift_expression(
        &self,
        expr: &str,
        dialect: &dyn Dialect,
        source_alias: &str,
        base_model_name: &str,
    ) -> Result<String> {
        let parser = formula_parser();
        let re = regex::Regex::new(r"time_shift\(([^,]+),\s*(-?\d+),\s*'(\w+)'\)")
            .map_err(|e| anyhow!(e))?;
        let caps = re
            .captures(expr)
            .ok_or_else(|| anyhow!("Invalid formula: malformed time_shift expression"))?;
        let inner = caps.get(1).unwrap().as_str().trim();
        let inner_measure = parser.parse_measure(inner)?;
        let inner_sql = self.aggregate_column(
            dialect,
            &inner_measure.formula.expression,
            &inner_measure.aggregation,
            source_alias,
            base_model_name,
        );
        // ORDER BY uses the source alias as a stable key until time dims are required
        Ok(format!(
            "LAG({}) OVER (ORDER BY {})",
            inner_sql, source_alias
        ))
    }

    fn build_ratio_expression(
        &self,
        expr: &str,
        dialect: &dyn Dialect,
        source_alias: &str,
        base_model_name: &str,
    ) -> Result<String> {
        let parser = formula_parser();
        let re = regex::Regex::new(r"ratio\(([^,]+),\s*([^)]+)\)").map_err(|e| anyhow!(e))?;
        let caps = re
            .captures(expr)
            .ok_or_else(|| anyhow!("Invalid formula: malformed ratio expression"))?;
        let num = parser.parse_measure(caps.get(1).unwrap().as_str().trim())?;
        let denom = parser.parse_measure(caps.get(2).unwrap().as_str().trim())?;
        let num_sql = self.aggregate_column(
            dialect,
            &num.formula.expression,
            &num.aggregation,
            source_alias,
            base_model_name,
        );
        let denom_sql = self.aggregate_column(
            dialect,
            &denom.formula.expression,
            &denom.aggregation,
            source_alias,
            base_model_name,
        );
        Ok(format!("({} / NULLIF({}, 0))", num_sql, denom_sql))
    }

    fn build_pct_change_expression(
        &self,
        expr: &str,
        dialect: &dyn Dialect,
        source_alias: &str,
        base_model_name: &str,
    ) -> Result<String> {
        let parser = formula_parser();
        let inner = expr
            .strip_prefix("pct_change(")
            .and_then(|s| s.strip_suffix(')'))
            .ok_or_else(|| anyhow!("Invalid formula: malformed pct_change expression"))?
            .trim();
        let measure = parser.parse_measure(inner)?;
        let current = self.aggregate_column(
            dialect,
            &measure.formula.expression,
            &measure.aggregation,
            source_alias,
            base_model_name,
        );
        Ok(format!(
            "(({current} - LAG({current}) OVER (ORDER BY {source_alias})) / NULLIF(LAG({current}) OVER (ORDER BY {source_alias}), 0)) * 100"
        ))
    }

    fn build_running_total_expression(
        &self,
        expr: &str,
        dialect: &dyn Dialect,
        source_alias: &str,
        base_model_name: &str,
    ) -> Result<String> {
        let parser = formula_parser();
        let inner = expr
            .strip_prefix("running_total(")
            .and_then(|s| s.strip_suffix(')'))
            .ok_or_else(|| anyhow!("Invalid formula: malformed running_total expression"))?
            .trim();
        let measure = parser.parse_measure(inner)?;
        let inner_sql = self.aggregate_column(
            dialect,
            &measure.formula.expression,
            &measure.aggregation,
            source_alias,
            base_model_name,
        );
        Ok(format!(
            "SUM({inner_sql}) OVER (ORDER BY {source_alias} ROWS UNBOUNDED PRECEDING)"
        ))
    }

    /// Does the model expose `field` as a plain column (dimension, time column,
    /// or bare-formula measure — not the `*` count shorthand)?
    fn model_has_plain_field(model: &Model, field: &str) -> bool {
        if field == "*" {
            return false;
        }
        if model.dimensions.iter().any(|d| d.name == field) {
            return true;
        }
        if model.time_dimensions.iter().any(|td| td.dimension == field) {
            return true;
        }
        model.measures.iter().any(|m| {
            let e = m.formula.expression.trim();
            e == field && !FormulaParser::needs_parse(e)
        })
    }

    /// Qualify `field` against the base model first, then any directly joined
    /// models (using their declared alias), falling back to the source alias so
    /// joined-model columns render as `alias.column` instead of `source.column`.
    fn qualify_field(
        &self,
        dialect: &dyn Dialect,
        base_model_name: &str,
        field: &str,
        source_alias: &str,
    ) -> String {
        if field == "*" {
            return String::from("*");
        }
        let quoted = dialect.quote_ident(field);

        let Some(base_model) = self.model(base_model_name) else {
            return format!("{source_alias}.{quoted}");
        };

        if Self::model_has_plain_field(&base_model, field) {
            return format!("{source_alias}.{quoted}");
        }

        for join in &base_model.joins {
            if let Some(joined) = self.model(&join.model) {
                if Self::model_has_plain_field(&joined, field) {
                    let alias = join.alias.clone().unwrap_or_else(|| join.name.clone());
                    return format!("{alias}.{quoted}");
                }
            }
        }

        // Qualified spelling: `alias.field` or `model.field`. Without this,
        // `customers.name` would be quoted whole into `orders."customers.name"`,
        // a three-part identifier that no dialect accepts. The prefix is
        // matched against known models and join aliases only, so this emits
        // the same two-part identifiers as every branch above.
        if let Some((prefix, column)) = field.rsplit_once('.') {
            let plain = dialect.quote_ident(column);
            if prefix == base_model_name {
                return format!("{source_alias}.{plain}");
            }
            for join in &base_model.joins {
                let Some(joined) = self.model(&join.model) else {
                    continue;
                };
                let alias = join.alias.as_deref().unwrap_or(join.name.as_str());
                if (prefix == alias || prefix == join.model)
                    && Self::model_has_plain_field(&joined, column)
                {
                    return format!("{alias}.{plain}");
                }
            }
        }

        format!("{source_alias}.{quoted}")
    }

    /// Whether `field` resolves to a geometry column on the model that owns it.
    ///
    /// Mirrors the resolution in [`Self::qualify_field`] so a plain name, a
    /// joined name, and a `alias.field` spelling all consult the same model.
    /// Only models that recorded their geometry columns (introspection does)
    /// answer true; a hand-written model without that metadata is treated as
    /// having none, which leaves its behaviour exactly as before.
    fn field_is_geometry(&self, base_model_name: &str, field: &str) -> bool {
        if field == "*" {
            return false;
        }
        let Some(base_model) = self.model(base_model_name) else {
            return false;
        };

        if Self::model_has_plain_field(&base_model, field) {
            return base_model.is_geometry_column(field);
        }

        for join in &base_model.joins {
            if let Some(joined) = self.model(&join.model) {
                if Self::model_has_plain_field(&joined, field) {
                    return joined.is_geometry_column(field);
                }
            }
        }

        if let Some((prefix, column)) = field.rsplit_once('.') {
            if prefix == base_model_name {
                return base_model.is_geometry_column(column);
            }
            for join in &base_model.joins {
                let Some(joined) = self.model(&join.model) else {
                    continue;
                };
                let alias = join.alias.as_deref().unwrap_or(join.name.as_str());
                if (prefix == alias || prefix == join.model)
                    && Self::model_has_plain_field(&joined, column)
                {
                    return joined.is_geometry_column(column);
                }
            }
        }

        false
    }

    /// Render a time dimension through the active dialect (e.g. SQLite
    /// strftime / MySQL DATE_FORMAT vs Postgres/DuckDB DATE_TRUNC).
    fn time_dim_expr(
        &self,
        dialect: &dyn Dialect,
        td: &TimeDimension,
        source_alias: Option<&str>,
    ) -> String {
        let column = if let Some(alias) = source_alias {
            format!("{alias}.{}", td.dimension)
        } else {
            td.dimension.clone()
        };
        dialect.date_trunc(td.granularity.date_trunc_unit(), &column)
    }

    fn build_where_clause(
        &self,
        query: &Query,
        dialect: &dyn Dialect,
        source_alias: &str,
        base_model_name: &str,
        mode: ValueMode,
        params: &mut Vec<Value>,
    ) -> Result<Option<String>> {
        if query.filters.is_empty() {
            return Ok(None);
        }

        let mut conditions = Vec::new();
        for (i, filter) in query.filters.iter().enumerate() {
            let separator = if i == 0 {
                ""
            } else if filter.or_condition {
                " OR "
            } else {
                " AND "
            };
            let column = self.qualify_field(dialect, base_model_name, &filter.field, source_alias);

            // Spatial predicates are function calls, not comparisons, and the
            // value has to become a geometry before it can be compared at all.
            // Handled before `sql_operator`, which is `None` for these.
            let condition = if let Some(function) = filter.operator.spatial_function() {
                let from_wkt = match &filter.value {
                    // An object is GeoJSON, a string is taken as WKT. Both are
                    // bound, so neither can alter the statement's shape.
                    serde_json::Value::String(_) => true,
                    serde_json::Value::Object(_) => false,
                    other => {
                        return Err(anyhow!(
                            "{function} on `{}` needs a geometry: a GeoJSON object or a WKT string, got {other}",
                            filter.field
                        ))
                    }
                };
                let value = render_value(dialect, mode, params, &filter.value);
                let geometry = dialect.spatial_geometry(&value, from_wkt).ok_or_else(|| {
                    anyhow!(
                        "{function} is not supported on the {} dialect: it has no spatial predicates, \
                         so there is no way to build this filter",
                        dialect.name()
                    )
                })?;
                format!("{function}({column}, {geometry})")
            } else {
                let op = filter.operator.sql_operator().ok_or_else(|| {
                    anyhow!("filter operator {:?} has no SQL rendering", filter.operator)
                })?;

                if filter.operator == FilterOperator::In || filter.operator == FilterOperator::NotIn
                {
                    // Membership lists render portably as parenthesized value lists.
                    let arr = filter.value.as_array().ok_or_else(|| {
                        anyhow!("{op} requires an array of values, got {}", filter.value)
                    })?;
                    let items: Vec<String> = arr
                        .iter()
                        .map(|v| render_value(dialect, mode, params, v))
                        .collect();
                    format!("{column} {op} ({})", items.join(", "))
                } else if filter.operator.needs_value() && filter.operator.needs_two_values() {
                    // BETWEEN
                    let vals = filter
                        .value
                        .as_array()
                        .ok_or_else(|| anyhow!("BETWEEN requires array of two values"))?;
                    format!(
                        "{column} {} BETWEEN {} AND {}",
                        op,
                        render_value(dialect, mode, params, &vals[0]),
                        render_value(dialect, mode, params, &vals[1])
                    )
                } else if filter.operator.needs_value() {
                    format!(
                        "{column} {op} {}",
                        render_value(dialect, mode, params, &filter.value)
                    )
                } else {
                    format!("{column} {op}")
                }
            };
            conditions.push(format!("{separator}{condition}"));
        }

        Ok(Some(conditions.concat()))
    }

    fn build_group_by(
        &self,
        query: &Query,
        dialect: &dyn Dialect,
        source_alias: &str,
        base_model_name: &str,
    ) -> Result<Vec<String>> {
        let mut group_by = Vec::new();

        for dim in &query.dimensions {
            group_by.push(self.qualify_field(dialect, base_model_name, &dim.name, source_alias));
        }

        for td in &query.time_dimensions {
            group_by.push(self.time_dim_expr(dialect, td, Some(source_alias)));
        }

        Ok(group_by)
    }

    fn build_order_by(
        &self,
        query: &Query,
        dialect: &dyn Dialect,
        source_alias: &str,
    ) -> Result<Vec<OrderByItem>> {
        let mut items = Vec::new();
        for order in &query.order {
            let column = if order.field.contains(".") {
                order.field.clone()
            } else {
                format!("{}.{}", source_alias, dialect.quote_ident(&order.field))
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
        dialect: &dyn Dialect,
        source_table: &str,
        source_alias: &str,
    ) -> Result<Vec<JoinClause>> {
        let base_model_name = query
            .source_model
            .as_ref()
            .map(|s| s.model.as_str())
            .or(query.name.as_deref())
            .unwrap_or(source_table);

        let Some(model) = self.model(base_model_name) else {
            return Ok(vec![]);
        };

        if model.joins.is_empty() {
            return Ok(vec![]);
        }

        let required: Vec<String> = model.joins.iter().map(|j| j.model.clone()).collect();
        let models: Vec<Model> = self
            .model_registry
            .read()
            .expect("model registry lock")
            .values()
            .cloned()
            .collect();
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
                on: Self::quote_join_on(dialect, &c.on),
            })
            .collect())
    }

    /// Best-effort quoting for identifiers inside walker ON clauses (`a.b = c.d`).
    fn quote_join_on(dialect: &dyn Dialect, on: &str) -> String {
        on.split(" AND ")
            .map(|cond| {
                let parts: Vec<&str> = cond.split('=').map(|s| s.trim()).collect();
                if parts.len() != 2 {
                    return cond.to_string();
                }
                format!(
                    "{} = {}",
                    Self::quote_qualified(dialect, parts[0]),
                    Self::quote_qualified(dialect, parts[1])
                )
            })
            .collect::<Vec<_>>()
            .join(" AND ")
    }

    fn quote_qualified(dialect: &dyn Dialect, qualified: &str) -> String {
        let mut parts = qualified.split('.');
        match (parts.next(), parts.next()) {
            (Some(alias), Some(col)) => format!("{}.{}", alias, dialect.quote_ident(col)),
            _ => dialect.quote_ident(qualified),
        }
    }
}

/// Render one filter value into a statement.
///
/// In [`ValueMode::Placeholder`] the value is pushed onto `params` and a bind
/// placeholder is returned, so the value never becomes part of the statement
/// text. In [`ValueMode::Literal`] the dialect's escaping is used, which is
/// only ever used for display.
fn render_value(
    dialect: &dyn Dialect,
    mode: ValueMode,
    params: &mut Vec<Value>,
    value: &Value,
) -> String {
    match mode {
        ValueMode::Placeholder => {
            params.push(value.clone());
            dialect.placeholder(params.len())
        }
        ValueMode::Literal => dialect.format_value(value),
    }
}

/// How a filter value is rendered into a statement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ValueMode {
    /// Emit a bind placeholder and collect the value. Used for execution.
    Placeholder,
    /// Inline the value with the dialect's escaping. Display only.
    Literal,
}

/// A generated statement in both the form that runs and the form a human reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedSql {
    /// Placeholder form. Send this with `params`; it is the only form that
    /// should reach a database.
    pub sql: String,
    /// Values to bind, in the order their placeholders appear in `sql`.
    pub params: Vec<Value>,
    /// The same statement with values inlined, for dry runs, audit logs and
    /// anything shown to a user. Never executed.
    pub display: String,
}

impl GeneratedSql {
    /// The statement to execute, with placeholders still in place.
    pub fn executable(&self) -> &str {
        &self.sql
    }

    /// A statement with nothing to bind, for GraphNight's own internal SQL.
    pub fn literal(sql: &str) -> Self {
        Self {
            sql: sql.to_string(),
            params: Vec::new(),
            display: sql.to_string(),
        }
    }
}

impl std::fmt::Display for GeneratedSql {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Reading a `GeneratedSql` as a string gives the readable form, which
        // is what nearly every caller wants; the executable form has to be
        // asked for by name.
        f.write_str(&self.display)
    }
}

impl std::ops::Deref for GeneratedSql {
    type Target = str;

    fn deref(&self) -> &str {
        &self.display
    }
}

/// Logical plan representation
#[derive(Debug, Clone)]
pub struct LogicalPlan {
    /// Either a table name (quoted as an identifier) or, when
    /// `source_is_sql` is set, a raw SQL expression to inline.
    pub source_table: String,
    /// The model declares a custom SQL override, so `source_table` holds SQL
    /// rather than a table name.
    pub source_is_sql: bool,
    pub source_alias: String,
    pub select_items: Vec<SelectItem>,
    pub where_clause: Option<String>,
    pub group_by: Vec<String>,
    pub order_by: Vec<OrderByItem>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
    pub joins: Vec<JoinClause>,
    pub distinct: bool,
    /// Filter values, in placeholder order, when the plan was built in
    /// [`ValueMode::Placeholder`].
    pub params: Vec<Value>,
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
