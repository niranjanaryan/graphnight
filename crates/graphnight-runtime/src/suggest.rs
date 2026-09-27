//! Shorthand query parsing and suggestion generation.
//!
//! Agents write `"revenue:sum"` and `"status:eq:completed"`, not nested JSON
//! `Measure { formula, aggregation }` structures. The shorthand is what makes
//! tool calls small enough to fit comfortably in a prompt, and the trade-off is
//! that a typo has to produce a *useful* error rather than a wall of text.
//!
//! So every parse failure carries the valid vocabulary and, where a name was
//! misspelled, the closest real one. An agent should never have to guess.

use crate::tools::{ToolError, ToolQuery};
use graphnight_core::models::{
    AggregationType, Dimension, Filter, FilterOperator, Formula, Measure, Model, OrderBy, Query,
    TimeDimension, TimeGranularity,
};
use serde_json::{json, Value};

/// Parse `"revenue:sum"` into a measure.
///
/// `*` is shorthand for a row count, the single most common agent query.
fn parse_measure(spec: &str) -> Result<Measure, ToolError> {
    let spec = spec.trim();
    if spec.is_empty() {
        return Err(ToolError::new("INVALID_MEASURE", "measure spec is empty")
            .with_hint("Use '<column>:<aggregation>', e.g. revenue:sum or *:count."));
    }
    if spec == "*" {
        return Ok(Measure {
            formula: Formula {
                expression: "*".to_string(),
                label: Some("row_count".to_string()),
                format: None,
            },
            aggregation: AggregationType::Count,
        });
    }

    let (expression, aggregation) = spec.rsplit_once(':').ok_or_else(|| {
        ToolError::new(
            "INVALID_MEASURE",
            format!("measure {spec:?} is missing an aggregation"),
        )
        .with_hint(
            "Use '<column>:<aggregation>'. Valid aggregations: \
                 sum, avg, count, min, max, count_distinct.",
        )
    })?;

    if expression.trim().is_empty() {
        return Err(
            ToolError::new("INVALID_MEASURE", format!("measure {spec:?} has no column"))
                .with_hint("Use '<column>:<aggregation>', e.g. revenue:sum."),
        );
    }

    let aggregation = match aggregation.trim().to_ascii_lowercase().as_str() {
        "sum" => AggregationType::Sum,
        "avg" | "average" | "mean" => AggregationType::Avg,
        "count" => AggregationType::Count,
        "min" => AggregationType::Min,
        "max" => AggregationType::Max,
        "count_distinct" | "countdistinct" | "distinct" => AggregationType::CountDistinct,
        other => {
            return Err(ToolError::new(
                "UNKNOWN_AGGREGATION",
                format!("unknown aggregation {other:?}"),
            )
            .with_hint(
                "Valid aggregations: sum, avg, count, min, max, count_distinct. \
                 Or use '*' alone to count rows.",
            ))
        }
    };

    Ok(Measure {
        formula: Formula {
            expression: expression.trim().to_string(),
            label: Some(aggregation.sql_function().to_string()),
            format: None,
        },
        aggregation,
    })
}

/// Parse `"created_at@month"` into a time dimension.
fn parse_time_dimension(spec: &str) -> Result<TimeDimension, ToolError> {
    let (dimension, granularity) = spec.split_once('@').ok_or_else(|| {
        ToolError::new(
            "INVALID_TIME_DIMENSION",
            format!("time dimension {spec:?} is missing a granularity"),
        )
        .with_hint("Use 'column@granularity', e.g. created_at@month. Valid granularities: second, minute, hour, day, week, month, quarter, year.")
    })?;

    let granularity = match granularity.trim().to_ascii_lowercase().as_str() {
        "second" | "sec" => TimeGranularity::Second,
        "minute" | "min" => TimeGranularity::Minute,
        "hour" | "hr" => TimeGranularity::Hour,
        "day" => TimeGranularity::Day,
        "week" | "wk" => TimeGranularity::Week,
        "month" | "mo" => TimeGranularity::Month,
        "quarter" | "qtr" => TimeGranularity::Quarter,
        "year" | "yr" => TimeGranularity::Year,
        other => {
            return Err(ToolError::new(
                "UNKNOWN_GRANULARITY",
                format!("unknown granularity {other:?}"),
            )
            .with_hint(
                "Valid granularities: second, minute, hour, day, week, month, quarter, year.",
            ))
        }
    };

    Ok(TimeDimension {
        dimension: dimension.trim().to_string(),
        granularity,
        label: None,
    })
}

/// Parse `"status:eq:completed"` into a filter.
///
/// The value may contain colons, so only the first two separators are structural.
fn parse_filter(spec: &str) -> Result<Filter, ToolError> {
    let mut parts = spec.splitn(3, ':');
    let field = parts.next().unwrap_or_default().trim().to_string();
    let operator = parts.next().unwrap_or_default().trim();
    let raw_value = parts.next().unwrap_or_default();

    if field.is_empty() {
        return Err(
            ToolError::new("INVALID_FILTER", format!("filter {spec:?} has no field"))
                .with_hint("Use 'field:operator:value', e.g. status:eq:completed."),
        );
    }
    if operator.is_empty() {
        return Err(ToolError::new(
            "INVALID_FILTER",
            format!("filter {spec:?} is missing an operator"),
        )
        .with_hint(
            "Use 'field:operator:value'. Valid operators: eq, neq, gt, gte, lt, lte, \
             in, not_in, like, ilike, not_like, is_null, is_not_null, between, not_between.",
        ));
    }

    let operator_name = operator.to_ascii_lowercase();
    let operator = match operator_name.as_str() {
        "eq" | "=" => FilterOperator::Eq,
        "neq" | "!=" | "ne" => FilterOperator::Neq,
        "gt" | ">" => FilterOperator::Gt,
        "gte" | ">=" | "ge" => FilterOperator::Gte,
        "lt" | "<" => FilterOperator::Lt,
        "lte" | "<=" | "le" => FilterOperator::Lte,
        "like" => FilterOperator::Like,
        "ilike" => FilterOperator::ILike,
        "not_like" | "nlike" => {
            return Err(ToolError::new(
                "UNKNOWN_OPERATOR",
                "not_like is not supported",
            )
            .with_hint("Use 'ilike' with a leading '%' to express a negated match, e.g. status:ilike:'%draft%' negated via neq."))
        }
        "in" => FilterOperator::In,
        "not_in" | "nin" => FilterOperator::NotIn,
        "is_null" | "isnull" => FilterOperator::IsNull,
        "is_not_null" | "isnotnull" => FilterOperator::IsNotNull,
        "between" => FilterOperator::Between,
        "not_between" | "notbetween" => FilterOperator::NotBetween,
        other => {
            return Err(ToolError::new(
                "UNKNOWN_OPERATOR",
                format!("unknown filter operator {other:?}"),
            )
            .with_hint(
                "Valid operators: eq, neq, gt, gte, lt, lte, in, not_in, like, ilike, \
                 is_null, is_not_null, between, not_between.",
            ))
        }
    };

    let value = coerce_value(&operator, raw_value);
    if operator.needs_value() && raw_value.is_empty() {
        return Err(
            ToolError::new("INVALID_FILTER", format!("filter {spec:?} needs a value")).with_hint(
                format!(
                    "{operator_name} requires a value, e.g. {field}:{operator_name}:some_value"
                ),
            ),
        );
    }

    Ok(Filter {
        field,
        operator,
        value,
        or_condition: false,
    })
}

/// Interpret a filter's textual value: numbers, booleans, nulls, lists.
///
/// Typed values matter more than they look — passing `"5"` where the column is
/// numeric makes Postgres compare text to integer and fail at execution, long
/// after the agent could have been corrected.
fn coerce_value(operator: &FilterOperator, raw: &str) -> Value {
    let raw = raw.trim();
    let unquoted = raw
        .strip_prefix('\'')
        .and_then(|r| r.strip_suffix('\''))
        .unwrap_or(raw);

    if *operator == FilterOperator::IsNull {
        return Value::Null;
    }
    if *operator == FilterOperator::IsNotNull {
        return Value::Null;
    }
    if operator.needs_two_values() {
        // `between:2024-01-01,2024-02-01`
        let (a, b) = unquoted.split_once(',').unwrap_or((unquoted, ""));
        return json!([parse_scalar(a), parse_scalar(b)]);
    }
    if matches!(operator, FilterOperator::In | FilterOperator::NotIn) {
        return Value::Array(
            unquoted
                .split(',')
                .map(|s| parse_scalar(s.trim()))
                .filter(|v| !v.is_null() || v.as_bool() == Some(false))
                .collect(),
        );
    }
    parse_scalar(unquoted)
}

fn parse_scalar(s: &str) -> Value {
    let s = s.trim();
    if s.is_empty() {
        return Value::String(String::new());
    }
    if let Ok(i) = s.parse::<i64>() {
        return json!(i);
    }
    if let Ok(f) = s.parse::<f64>() {
        return json!(f);
    }
    match s.to_ascii_lowercase().as_str() {
        "true" => return json!(true),
        "false" => return json!(false),
        "null" | "none" => return Value::Null,
        _ => {}
    }
    Value::String(s.to_string())
}

/// Convert a tool query into the core semantic query.
pub fn to_core_query(input: &ToolQuery) -> Result<Query, ToolError> {
    let measures = input
        .measures
        .iter()
        .map(|m| parse_measure(m))
        .collect::<Result<Vec<_>, _>>()?;
    let dimensions = input
        .dimensions
        .iter()
        .map(|d| {
            let name = d.trim();
            if name.is_empty() {
                return Err(ToolError::new(
                    "INVALID_DIMENSION",
                    "dimension name is empty",
                ));
            }
            Ok(Dimension {
                name: name.to_string(),
                label: None,
            })
        })
        .collect::<Result<Vec<_>, ToolError>>()?;
    let time_dimensions = input
        .time_dimensions
        .iter()
        .map(|t| parse_time_dimension(t))
        .collect::<Result<Vec<_>, _>>()?;
    let filters = input
        .filters
        .iter()
        .map(|f| parse_filter(f))
        .collect::<Result<Vec<_>, _>>()?;

    let order = input
        .order
        .iter()
        .map(|o| {
            let o = o.trim();
            let (field, descending) = match o.rsplit_once(':') {
                Some((f, d)) if d.eq_ignore_ascii_case("desc") => (f, true),
                Some((f, d)) if d.eq_ignore_ascii_case("asc") => (f, false),
                _ => {
                    return Err(ToolError::new(
                        "INVALID_ORDER",
                        format!("order {o:?} needs a direction"),
                    )
                    .with_hint("Use 'field:asc' or 'field:desc'."))
                }
            };
            if field.trim().is_empty() {
                return Err(ToolError::new(
                    "INVALID_ORDER",
                    format!("order {o:?} has no field"),
                ));
            }
            Ok(OrderBy {
                field: field.trim().to_string(),
                descending,
            })
        })
        .collect::<Result<Vec<_>, ToolError>>()?;

    if measures.is_empty() && dimensions.is_empty() && time_dimensions.is_empty() {
        return Err(
            ToolError::new("EMPTY_QUERY", "query selects nothing").with_hint(
                "Add at least one measure (e.g. \"revenue:sum\") or dimension (e.g. \"status\").",
            ),
        );
    }

    // `stage_ref` references a previous stage, never a model, so it must not be
    // used as a fallback for the model name.
    let model = input
        .name
        .clone()
        .filter(|n| !n.trim().is_empty())
        .ok_or_else(|| {
            ToolError::new("MISSING_MODEL", "query does not name a model")
                .with_hint("Set `name` to the model to query, e.g. \"orders\".")
        })?;

    Ok(Query {
        name: Some(model),
        source_model: None,
        measures,
        dimensions,
        time_dimensions,
        filters,
        order,
        limit: input.limit.map(|l| l as usize),
        offset: input.offset.map(|o| o as usize),
        whole_periods_only: None,
        distinct_dimension_values: None,
        stage_ref: input.stage_ref.clone(),
    })
}

/// Every field name a model exposes, for validation and suggestions.
pub fn field_names(model: &Model) -> Vec<&str> {
    let mut names: Vec<&str> = Vec::new();
    names.extend(model.measures.iter().map(|m| m.formula.expression.as_str()));
    names.extend(model.dimensions.iter().map(|d| d.name.as_str()));
    names.extend(model.time_dimensions.iter().map(|t| t.dimension.as_str()));
    names
}

/// Levenshtein distance, for did-you-mean suggestions.
fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.to_ascii_lowercase().chars().collect();
    let b: Vec<char> = b.to_ascii_lowercase().chars().collect();
    if a.is_empty() {
        return b.len();
    }
    if b.is_empty() {
        return a.len();
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut curr = vec![0usize; b.len() + 1];
    for i in 1..=a.len() {
        curr[0] = i;
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            curr[j] = (prev[j] + 1).min(curr[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[b.len()]
}

/// Closest known field names to `field`, best first.
pub fn closest_matches<'a>(field: &str, known: &[&'a str], limit: usize) -> Vec<&'a str> {
    let mut scored: Vec<(usize, &str)> = known
        .iter()
        .map(|k| (edit_distance(field, k), *k))
        .filter(|(d, k)| {
            // Accept a near miss, or a substring relationship in either
            // direction ("revenue" ~ "total_revenue_usd").
            *d <= (field.len().max(k.len()) / 2).max(1)
                || k.to_ascii_lowercase().contains(&field.to_ascii_lowercase())
                || field.to_ascii_lowercase().contains(&k.to_ascii_lowercase())
        })
        .collect();
    scored.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(b.1)));
    scored.into_iter().take(limit).map(|(_, k)| k).collect()
}

/// A structured "unknown field" error naming the closest real fields.
pub fn unknown_field_error(field: &str, known: &[&str], role: &str) -> ToolError {
    let suggestions = closest_matches(field, known, 3);
    let mut hint = if known.is_empty() {
        "Call get_model to see this model's fields.".to_string()
    } else if suggestions.is_empty() {
        format!("Valid {role}s on this model: {}.", known.join(", "))
    } else {
        format!("Did you mean: {}?", suggestions.join(", "))
    };
    if !known.is_empty() {
        hint.push_str(&format!(" All fields: {}.", known.join(", ")));
    }
    ToolError::new("UNKNOWN_FIELD", format!("unknown {role} {field:?}")).with_hint(hint)
}

/// Worked example queries for a model.
///
/// Emitted from the model's own shape, so they cannot drift from the schema the
/// way hardcoded examples do. This is the cheapest available reliability win:
/// a correct example in `get_model` prevents a wrong query in `run_query`.
pub fn example_queries(model: &Model) -> Vec<Value> {
    let mut examples = Vec::new();

    if let Some(measure) = model.measures.first() {
        let agg = measure.aggregation.sql_function().to_ascii_lowercase();
        let agg = if agg == "count(distinct " {
            "count_distinct".to_string()
        } else {
            agg
        };
        let column = if measure.formula.expression == "*" {
            "*".to_string()
        } else {
            measure.formula.expression.clone()
        };
        let spec = if column == "*" {
            "*".to_string()
        } else {
            format!("{column}:{agg}")
        };

        let mut example = json!({
            "description": format!("Total {} across all rows", model.name),
            "query": { "name": model.name, "measures": [spec.clone()] }
        });
        examples.push(example.take());
    }

    if let Some(dim) = model.dimensions.first() {
        examples.push(json!({
            "description": format!("{} broken down by {}", model.name, dim.name),
            "query": {
                "name": model.name,
                "measures": [model.measures.first().map(|m| m.formula.expression.clone()).unwrap_or_else(|| "*".to_string())],
                "dimensions": [dim.name],
                "order": [format!("{}:desc", dim.name)]
            }
        }));
    }

    if let Some(td) = model.time_dimensions.first() {
        examples.push(json!({
            "description": format!("{} over time by month", model.name),
            "query": {
                "name": model.name,
                "measures": [model.measures.first().map(|m| m.formula.expression.clone()).unwrap_or_else(|| "*".to_string())],
                "time_dimensions": [format!("{}@month", td.dimension)],
                "order": [format!("{}:asc", td.dimension)]
            }
        }));
    }

    if let Some(dim) = model.dimensions.first() {
        examples.push(json!({
            "description": format!("Filter {} by {}", model.name, dim.name),
            "query": {
                "name": model.name,
                "measures": ["*:count"],
                "filters": [format!("{}:eq:<value>", dim.name)],
                "dimensions": [dim.name]
            }
        }));
    }

    examples
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn model() -> Model {
        Model {
            name: "orders".to_string(),
            datasource: "warehouse".to_string(),
            description: Some("One row per order".to_string()),
            measures: vec![Measure {
                formula: Formula {
                    expression: "revenue".to_string(),
                    label: None,
                    format: None,
                },
                aggregation: AggregationType::Sum,
            }],
            dimensions: vec![Dimension {
                name: "status".to_string(),
                label: None,
            }],
            time_dimensions: vec![TimeDimension {
                dimension: "created_at".to_string(),
                granularity: TimeGranularity::Day,
                label: None,
            }],
            joins: vec![],
            sql: None,
            meta: HashMap::new(),
        }
    }

    #[test]
    fn parses_measure_shorthand() {
        let m = parse_measure("revenue:sum").unwrap();
        assert_eq!(m.formula.expression, "revenue");
        assert_eq!(m.aggregation, AggregationType::Sum);
    }

    #[test]
    fn star_is_a_row_count() {
        let m = parse_measure("*").unwrap();
        assert_eq!(m.aggregation, AggregationType::Count);
        assert_eq!(m.formula.expression, "*");
    }

    #[test]
    fn missing_aggregation_lists_the_vocabulary() {
        let err = parse_measure("revenue").unwrap_err();
        assert_eq!(err.code, "INVALID_MEASURE");
        assert!(err.hint.unwrap().contains("count_distinct"));
    }

    #[test]
    fn unknown_aggregation_is_specific() {
        let err = parse_measure("revenue:totally_bogus").unwrap_err();
        assert_eq!(err.code, "UNKNOWN_AGGREGATION");
    }

    #[test]
    fn parses_time_dimension() {
        let t = parse_time_dimension("created_at@month").unwrap();
        assert_eq!(t.dimension, "created_at");
        assert_eq!(t.granularity, TimeGranularity::Month);
    }

    #[test]
    fn time_dimension_without_granularity_is_hinted() {
        let err = parse_time_dimension("created_at").unwrap_err();
        assert_eq!(err.code, "INVALID_TIME_DIMENSION");
    }

    #[test]
    fn parses_filter_and_types_the_value() {
        let f = parse_filter("total:gt:100").unwrap();
        assert_eq!(f.field, "total");
        assert_eq!(f.operator, FilterOperator::Gt);
        assert_eq!(f.value, json!(100));
    }

    #[test]
    fn quoted_values_stay_strings() {
        let f = parse_filter("status:eq:'completed'").unwrap();
        assert_eq!(f.value, json!("completed"));
    }

    #[test]
    fn in_operator_builds_a_list() {
        let f = parse_filter("status:in:pending,completed").unwrap();
        assert_eq!(f.value, json!(["pending", "completed"]));
    }

    #[test]
    fn value_may_contain_colons() {
        let f = parse_filter("note:eq:a:b:c").unwrap();
        assert_eq!(f.value, json!("a:b:c"));
    }

    #[test]
    fn is_null_needs_no_value() {
        let f = parse_filter("deleted_at:is_null").unwrap();
        assert_eq!(f.operator, FilterOperator::IsNull);
    }

    #[test]
    fn between_takes_a_pair() {
        let f = parse_filter("created_at:between:2024-01-01,2024-02-01").unwrap();
        assert_eq!(f.operator, FilterOperator::Between);
        assert_eq!(f.value, json!(["2024-01-01", "2024-02-01"]));
    }

    #[test]
    fn empty_query_is_rejected_with_guidance() {
        let err = to_core_query(&ToolQuery {
            name: Some("orders".into()),
            ..Default::default()
        })
        .unwrap_err();
        assert_eq!(err.code, "EMPTY_QUERY");
    }

    #[test]
    fn suggests_the_intended_field() {
        let m = model();
        let known = field_names(&m);
        let err = unknown_field_error("revenu", &known, "dimension");
        assert_eq!(err.code, "UNKNOWN_FIELD");
        let hint = err.hint.unwrap();
        assert!(hint.contains("revenue"), "hint was: {hint}");
    }

    #[test]
    fn edit_distance_basics() {
        assert_eq!(edit_distance("", "abc"), 3);
        assert_eq!(edit_distance("abc", "abc"), 0);
        assert_eq!(edit_distance("kitten", "sitting"), 3);
    }

    #[test]
    fn examples_reference_real_fields() {
        let m = model();
        let examples = example_queries(&m);
        assert!(!examples.is_empty());
        let rendered = serde_json::to_string(&examples).unwrap();
        assert!(rendered.contains("revenue"));
        assert!(rendered.contains("created_at"));
    }
}
