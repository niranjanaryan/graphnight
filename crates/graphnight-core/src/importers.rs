//! Import a semantic model from a tool that already has one.
//!
//! Two very different starting points are supported, and the difference
//! matters more than the code suggests:
//!
//! - **Cube** already *is* a semantic layer: cubes with measures, dimensions
//!   and joins. The mapping is close to one-to-one, so a Cube schema converts
//!   to GraphNight with little loss.
//! - **dbt** is a transformation tool. Its models are tables and columns, not
//!   metrics. Columns become dimensions, but *measures have to be declared* —
//!   there is no honest way to guess that `amount_usd` should be summed rather
//!   than averaged, and silently picking one would produce a semantic layer
//!   that quietly answers the wrong question.
//!
//! So for dbt, measures come from `meta` that the project already declares, and
//! anything left over is reported as a warning rather than guessed at. The
//! alternative — inventing measures — is how a migration ends up trusted and
//! wrong.

use std::collections::HashMap;
use std::path::Path;

use serde_json::Value;

use crate::errors::Result;
use crate::models::{
    AggregationType, Dimension, Formula, Join, JoinType, Measure, Model, TimeDimension,
    TimeGranularity,
};

/// What an import produced, plus what it could not.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ImportReport {
    /// Models converted successfully.
    pub models: Vec<Model>,
    /// Non-fatal problems worth a human's attention.
    ///
    /// A model with no measures imports as dimensions-only, which is valid but
    /// rarely what someone wants. Saying so beats letting them find out at
    /// query time.
    pub warnings: Vec<String>,
    /// Source entries deliberately skipped (ephemeral models, disabled entries).
    pub skipped: Vec<String>,
}

impl ImportReport {
    fn warn(&mut self, message: impl Into<String>) {
        self.warnings.push(message.into());
    }
}

/// Whether an expression is a bare column reference.
///
/// `*`, `column`, and `schema.column` qualify. Anything with spaces,
/// parentheses, quotes or punctuation does not, which is the signal that the
/// value came from somewhere other than a column list.
fn is_plain_column(expr: &str) -> bool {
    let expr = expr.trim();
    if expr == "*" {
        return true;
    }
    !expr.is_empty()
        && expr
            .split('.')
            .all(|part| !part.is_empty() && part.chars().all(|c| c.is_alphanumeric() || c == '_'))
}

/// How names that differ only in case should be treated.
///
/// dbt is case-insensitive about column names while most warehouses are not,
/// so two columns can collide after folding. Silently dropping one loses a
/// dimension; a warning lets the operator rename it.
fn warn_on_case_collisions(name: &str, fields: &[String], report: &mut ImportReport) {
    let mut seen: HashMap<String, &String> = HashMap::new();
    for field in fields {
        let folded = field.to_lowercase();
        if let Some(first) = seen.insert(folded, field) {
            report.warn(format!(
                "{name}: `{first}` and `{field}` differ only in case; \
                 warehouse identifiers may not. Both were imported — rename one if queries fail."
            ));
        }
    }
}

// ---------------------------------------------------------------------------
// Cube
// ---------------------------------------------------------------------------

/// Minimal view of a Cube schema file.
///
/// Only the fields that map onto GraphNight are modelled, and every one is
/// optional: Cube files carry joins, segments, pre-aggregations and refresh
/// rules that have no GraphNight equivalent, and rejecting a file for having
/// them would make the importer useless on real schemas.
#[derive(Debug, serde::Deserialize)]
struct CubeSchema {
    #[serde(default)]
    cubes: Vec<Cube>,
}

#[derive(Debug, serde::Deserialize)]
struct Cube {
    name: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    description: Option<String>,
    /// Cube allows a bare string; treat it as a model name.
    #[serde(rename = "sql_table", default)]
    sql_table: Option<Value>,
    #[serde(default)]
    measures: Vec<CubeMeasure>,
    #[serde(default)]
    dimensions: Vec<CubeDimension>,
    #[serde(default)]
    joins: Vec<CubeJoin>,
}

#[derive(Debug, serde::Deserialize)]
struct CubeMeasure {
    name: String,
    #[serde(default)]
    title: Option<String>,
    /// `{ sql: "sum(amount)" }` or a bare string.
    #[serde(default)]
    sql: Option<Value>,
    #[serde(rename = "type", default)]
    measure_type: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
struct CubeDimension {
    name: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    sql: Option<Value>,
    #[serde(rename = "type", default)]
    dimension_type: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
struct CubeJoin {
    name: String,
    #[serde(default)]
    sql: Option<Value>,
    #[serde(rename = "relationship", default)]
    relationship: Option<String>,
}

/// Read a string that Cube allows to be either a scalar or `{ sql: ... }`.
fn cube_sql(value: &Option<Value>) -> Option<String> {
    match value {
        Some(Value::String(s)) => Some(s.clone()),
        Some(Value::Object(map)) => map.get("sql").and_then(Value::as_str).map(str::to_string),
        _ => None,
    }
}

/// Convert a Cube `sql:` expression into a GraphNight measure.
///
/// Cube writes the *inner* aggregate (`sum(amount)`), which is exactly a
/// GraphNight formula plus aggregation, so the pair splits cleanly. The
/// function name is validated rather than trusted: it ends up in generated SQL,
/// and a schema file is untrusted input even when it sits in the same repo.
fn cube_measure_to_measure(
    raw: &CubeMeasure,
    model: &str,
    report: &mut ImportReport,
) -> Option<Measure> {
    let sql = cube_sql(&raw.sql).unwrap_or_else(|| raw.name.clone());

    // `count` often appears as a measure type with no SQL at all.
    if let Some(kind) = &raw.measure_type {
        if kind.eq_ignore_ascii_case("count") {
            // Keep the label: every other imported measure carries its source
            // name, and a blank one here is an inconsistency someone will
            // notice in the UI.
            return Some(Measure::new(
                Formula {
                    expression: "*".to_string(),
                    label: raw.title.clone().or(Some(raw.name.clone())),
                    format: None,
                },
                AggregationType::Count,
            ));
        }
    }

    let Some((func, inner)) = split_aggregate(&sql) else {
        report.warn(format!(
            "{model}.{}: `{sql}` is not an aggregate call, so no aggregation could be \
             determined; measure skipped. Use `sql: sum(<column>)`.",
            raw.name
        ));
        return None;
    };
    let aggregation = AggregationType::parse_strict(func).ok().or_else(|| {
        report.warn(format!(
            "{model}.{}: `{func}` is not a GraphNight aggregation; measure skipped",
            raw.name
        ));
        None
    })?;

    let expression = match inner {
        Some(expr) => {
            // Not a security boundary: the generator quotes every field name,
            // so a hostile expression becomes a (missing) column rather than
            // injected SQL. This is about usefulness — a measure pointing at
            // something that is not a column will fail at query time, so say so
            // now rather than letting someone discover it in production.
            if !is_plain_column(expr) {
                report.warn(format!(
                    "{model}.{}: `{expr}` is not a plain column reference; imported as a \
                     field name and will fail unless a column with that name exists.",
                    raw.name
                ));
            }
            expr
        }
        // No parentheses: Cube allows `sql: amount_usd` with the type carrying
        // the aggregation, or a bare column. Treat it as a plain column and
        // fall back to the declared type.
        None => match raw
            .measure_type
            .as_deref()
            .and_then(|t| AggregationType::parse_strict(t).ok())
        {
            Some(agg) => {
                return Some(Measure::new(
                    Formula {
                        expression: sql.clone(),
                        label: raw.title.clone().or(Some(raw.name.clone())),
                        format: None,
                    },
                    agg,
                ));
            }
            None => {
                report.warn(format!(
                    "{model}.{}: could not determine an aggregation from `{sql}`; \
                     measure skipped. Use `sql: sum(<column>)`.",
                    raw.name
                ));
                return None;
            }
        },
    };

    let formula = Formula {
        expression: expression.to_string(),
        label: raw.title.clone().or(Some(raw.name.clone())),
        format: None,
    };
    Some(Measure::new(formula, aggregation))
}

/// Split `sum(amount)` into `("sum", Some("amount"))`.
///
/// Returns `None` when the text is not a function call at all, which is the
/// signal to warn: a bare expression such as `CASE WHEN … END` carries no
/// aggregation, and assuming one would be a guess.
fn split_aggregate(sql: &str) -> Option<(&str, Option<&str>)> {
    let trimmed = sql.trim();
    let open = trimmed.find('(')?;
    let name = trimmed[..open].trim();
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        // Something other than a plain function name sits before the paren, so
        // this is a nested expression rather than an aggregate.
        return None;
    }
    let close = trimmed.rfind(')')?;
    if close <= open {
        return None;
    }
    let inner = trimmed[open + 1..close].trim();
    if inner.is_empty() {
        return Some((name, None));
    }
    Some((name, Some(inner)))
}

/// Convert a Cube dimension.
fn cube_dimension_to_dimension(
    raw: &CubeDimension,
    model: &str,
    report: &mut ImportReport,
) -> Option<Dimension> {
    // A dimension defined by a SQL expression has no field name GraphNight can
    // group by. Keep the name so the model stays queryable, and be explicit
    // that the expression was not carried over.
    if let Some(sql) = cube_sql(&raw.sql) {
        if sql != raw.name && !sql.contains(&raw.name) {
            report.warn(format!(
                "{model}.{}: dimension is a SQL expression (`{sql}`); \
                 imported as a plain field named `{}`. Rewrite it as a model SQL override \
                 if you need the expression.",
                raw.name, raw.name
            ));
        }
    }

    // Cube time dimensions become GraphNight time dimensions, which is the one
    // place the two models line up exactly.
    if raw
        .dimension_type
        .as_deref()
        .is_some_and(|t| t.eq_ignore_ascii_case("time"))
    {
        return None; // handled separately by the caller
    }

    Some(Dimension {
        name: raw.name.clone(),
        label: raw.title.clone(),
    })
}

fn cube_join_to_join(raw: &CubeJoin, report: &mut ImportReport) -> Option<Join> {
    let sql = cube_sql(&raw.sql)?;
    // Cube: `cube.customer_id = ${CUBE}.customer_id`.
    // GraphNight: `("customer_id", "customer_id")`.
    let (left, right) = sql.split_once('=')?;
    let left = clean_join_side(left, raw);
    let right = clean_join_side(right, raw);
    if left.is_empty() || right.is_empty() {
        report.warn(format!(
            "join `{}`: could not read `{}` as a column pair; join skipped",
            raw.name, sql
        ));
        return None;
    }
    let relationship = raw.relationship.as_deref().unwrap_or("many_to_one");
    let join_type = match relationship {
        r if r.contains("one_to_one") => JoinType::Inner,
        // GraphNight has no "belongs to" distinction: a many-to-one from the
        // fact side is a left join, which preserves the fact grain.
        _ => JoinType::Left,
    };
    Some(Join {
        name: raw.name.clone(),
        model: String::new(),
        join_type,
        on: vec![(left, right)],
        alias: None,
    })
}

/// Strip a Cube member reference down to a bare column name.
///
/// Cube writes `{CUBE}.customer_id` in YAML and `${CUBE}.customer_id` in JS,
/// optionally quoting the whole side. Order matters: the prefixes go first, and
/// only then is the qualifier dot removed, so a remaining dot would be part of
/// a real name rather than a leftover marker.
fn clean_join_side(side: &str, raw: &CubeJoin) -> String {
    let side = side.trim().trim_matches('"').trim();
    let side = side
        .replace("${CUBE}", "")
        .replace("{CUBE}", "")
        .replace(&format!("${{{}}}", raw.name), "")
        .replace(&format!("{{{}}}", raw.name), "");
    // Drop a leading qualifier dot left behind by removing the prefix.
    let side = side.strip_prefix('.').unwrap_or(&side);
    // Anything still dotted is a schema-qualified column; GraphNight addresses
    // fields by name, so keep the last segment.
    let side = side.rsplit('.').next().unwrap_or(side);
    side.trim().to_string()
}

/// Import a Cube schema.
///
/// `datasource` is the GraphNight datasource every cube attaches to; a Cube
/// project is normally bound to one warehouse connection, and the schema file
/// does not carry credentials.
pub fn import_cube(path: &Path, datasource: &str) -> Result<ImportReport> {
    let text = std::fs::read_to_string(path).map_err(|e| {
        crate::errors::CoreError::InvalidQuery(format!("cannot read {}: {e}", path.display()))
    })?;
    import_cube_str(&text, datasource)
}

/// Import a Cube schema from a string, for tests and stdin.
pub fn import_cube_str(text: &str, datasource: &str) -> Result<ImportReport> {
    let schema: CubeSchema = serde_yaml::from_str(text).map_err(|e| {
        crate::errors::CoreError::InvalidQuery(format!("not a readable Cube schema: {e}"))
    })?;
    let mut report = ImportReport::default();

    for cube in schema.cubes {
        let model_name = cube.name.clone();
        let mut measures = Vec::new();
        let mut time_dimensions = Vec::new();
        let mut dimensions = Vec::new();

        for raw in &cube.measures {
            if let Some(measure) = cube_measure_to_measure(raw, &model_name, &mut report) {
                measures.push(measure);
            }
        }

        for raw in &cube.dimensions {
            if raw
                .dimension_type
                .as_deref()
                .is_some_and(|t| t.eq_ignore_ascii_case("time"))
            {
                time_dimensions.push(TimeDimension {
                    dimension: raw.name.clone(),
                    // GraphNight requires an explicit grain; day is the least
                    // surprising default and the query can always widen it.
                    granularity: TimeGranularity::Day,
                    label: raw.title.clone(),
                });
                continue;
            }
            if let Some(dimension) = cube_dimension_to_dimension(raw, &model_name, &mut report) {
                dimensions.push(dimension);
            }
        }

        let mut joins: Vec<Join> = cube
            .joins
            .iter()
            .filter_map(|j| cube_join_to_join(j, &mut report))
            .collect();
        // A Cube join names the *other* cube, so the target is recoverable from
        // the join name in the common `{ cube: 'customers' }` form.
        for join in &mut joins {
            if join.model.is_empty() {
                join.model = join.name.clone();
            }
        }

        if measures.is_empty() {
            report.warn(format!(
                "{model_name}: no measures were imported, so this model can only \
                 group and filter. Add measures in the Cube schema or edit the model after import."
            ));
        }

        let field_names: Vec<String> = dimensions
            .iter()
            .map(|d| d.name.clone())
            .chain(time_dimensions.iter().map(|t| t.dimension.clone()))
            .collect();
        warn_on_case_collisions(&model_name, &field_names, &mut report);

        let sql_table = cube_sql(&cube.sql_table);
        // GraphNight derives the table name from the model name, so a cube
        // pointing at a different physical table needs a SQL override. Store
        // the raw table name and let the generator decide how to wrap it.
        let sql_override = sql_table;
        if sql_override.is_some() {
            report.warnings.push(format!(
                "cube '{model_name}': sql_table set to '{}'; the generator will use it as the model's SQL source",
                sql_override.as_deref().unwrap_or("(none)")
            ));
        }

        report.models.push(Model {
            name: model_name,
            datasource: datasource.to_string(),
            description: cube.description.clone().or_else(|| cube.title.clone()),
            measures,
            dimensions,
            time_dimensions,
            joins,
            sql: sql_override,
            meta: HashMap::new(),
        });
    }

    Ok(report)
}

// ---------------------------------------------------------------------------
// dbt
// ---------------------------------------------------------------------------

/// The subset of dbt's `manifest.json` needed to build models.
#[derive(Debug, serde::Deserialize)]
struct DbtManifest {
    nodes: HashMap<String, DbtNode>,
}

#[derive(Debug, serde::Deserialize)]
struct DbtNode {
    name: String,
    #[serde(default)]
    resource_type: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    columns: HashMap<String, DbtColumn>,
    #[serde(default)]
    meta: HashMap<String, Value>,
    #[serde(default)]
    config: DbtConfig,
}

#[derive(Debug, serde::Deserialize, Default)]
struct DbtConfig {
    #[serde(default)]
    materialized: Option<String>,
    #[serde(default)]
    enabled: Option<bool>,
}

#[derive(Debug, serde::Deserialize)]
struct DbtColumn {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    data_type: Option<String>,
}

/// Import a dbt project from its `target/manifest.json`.
///
/// # Important
/// dbt does not model metrics. Every model therefore arrives with dimensions
/// from its columns and only the measures the project declared in
/// `meta.graphnight_measures`. See [`ImportReport::warnings`].
pub fn import_dbt(path: &Path, datasource: &str) -> Result<ImportReport> {
    let text = std::fs::read_to_string(path).map_err(|e| {
        crate::errors::CoreError::InvalidQuery(format!("cannot read {}: {e}", path.display()))
    })?;
    import_dbt_str(&text, datasource)
}

/// Import a dbt manifest from a string, for tests and stdin.
pub fn import_dbt_str(text: &str, datasource: &str) -> Result<ImportReport> {
    let manifest: DbtManifest = serde_json::from_str(text).map_err(|e| {
        crate::errors::CoreError::InvalidQuery(format!("not a readable dbt manifest.json: {e}"))
    })?;
    let mut report = ImportReport::default();

    // Deterministic order: a manifest is a hash map, and an import that reorders
    // itself between runs produces a diff every time.
    let mut node_ids: Vec<&String> = manifest.nodes.keys().collect();
    node_ids.sort();

    for node_id in node_ids {
        let node = &manifest.nodes[node_id];
        if node.resource_type != "model" {
            continue;
        }

        if node.config.enabled == Some(false) {
            report.skipped.push(format!("{} (disabled)", node.name));
            continue;
        }
        // An ephemeral model is a CTE, not a table. There is nothing to query.
        if node
            .config
            .materialized
            .as_deref()
            .is_some_and(|m| m == "ephemeral")
        {
            report.skipped.push(format!("{} (ephemeral)", node.name));
            continue;
        }

        let mut column_ids: Vec<&String> = node.columns.keys().collect();
        column_ids.sort();

        let mut dimensions = Vec::new();
        let mut time_dimensions = Vec::new();
        let mut column_names = Vec::new();

        for column_id in column_ids {
            let column = &node.columns[column_id];
            let column_name = column.name.clone().unwrap_or_else(|| column_id.clone());
            column_names.push(column_name.clone());

            if is_time_column(&column_name, column.data_type.as_deref()) {
                time_dimensions.push(TimeDimension {
                    dimension: column_name,
                    granularity: TimeGranularity::Day,
                    label: column.description.clone(),
                });
            } else {
                dimensions.push(Dimension {
                    name: column_name,
                    label: column.description.clone(),
                });
            }
        }

        let measures = dbt_measures(node, &mut report);
        if measures.is_empty() {
            report.warn(format!(
                "{}: imported as dimensions-only. dbt does not define metrics, so add \
                 `meta: {{graphnight_measures: [...]}}` to the model to get aggregations. \
                 See docs/migration-dbt.md.",
                node.name
            ));
        }

        warn_on_case_collisions(&node.name, &column_names, &mut report);

        report.models.push(Model {
            name: node.name.clone(),
            datasource: datasource.to_string(),
            description: node.description.clone(),
            measures,
            dimensions,
            time_dimensions,
            joins: Vec::new(),
            sql: None,
            meta: HashMap::new(),
        });
    }

    Ok(report)
}

/// Read measures declared in a dbt node's `meta.graphnight_measures`.
fn dbt_measures(node: &DbtNode, report: &mut ImportReport) -> Vec<Measure> {
    let Some(Value::Array(items)) = node.meta.get("graphnight_measures") else {
        return Vec::new();
    };

    let mut measures = Vec::new();
    for item in items {
        let name = item.get("name").and_then(Value::as_str);
        let column = item.get("column").and_then(Value::as_str);
        let aggregation = item.get("aggregation").and_then(Value::as_str);
        let label = item.get("label").and_then(Value::as_str);

        let (Some(name), Some(column), Some(aggregation)) = (name, column, aggregation) else {
            report.warn(format!(
                "{}: a graphnight_measure entry needs `name`, `column` and `aggregation`; \
                 entry skipped: {item}",
                node.name
            ));
            continue;
        };

        let aggregation = match AggregationType::parse_strict(aggregation) {
            Ok(a) => a,
            Err(e) => {
                report.warn(format!("{}.{name}: {e}; measure skipped", node.name));
                continue;
            }
        };

        measures.push(Measure::new(
            Formula {
                expression: column.to_string(),
                label: label.map(str::to_string).or_else(|| Some(name.to_string())),
                format: item
                    .get("format")
                    .and_then(Value::as_str)
                    .map(str::to_string),
            },
            aggregation,
        ));
    }
    measures
}

/// Guess whether a column holds a timestamp.
///
/// Name-based, because that is what is available without warehouse access.
/// A wrong guess is not harmful: a time dimension still groups correctly, the
/// query just cannot apply date truncation to something that is not a date.
fn is_time_column(name: &str, data_type: Option<&str>) -> bool {
    let name = name.to_lowercase();
    let by_name = [
        "_at",
        "_date",
        "_time",
        "_ts",
        "_timestamp",
        "_day",
        "_month",
        "_year",
    ]
    .iter()
    .any(|suffix| name.ends_with(suffix))
        || matches!(
            name.as_str(),
            "date" | "time" | "timestamp" | "day" | "month" | "year" | "created" | "updated"
        );
    if by_name {
        return true;
    }
    data_type.is_some_and(|t| {
        let t = t.to_lowercase();
        t.contains("timestamp") || t.contains("date") || t.contains("datetime")
    })
}
