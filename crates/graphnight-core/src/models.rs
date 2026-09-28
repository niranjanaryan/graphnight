use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum AggregationType {
    Sum,
    Avg,
    Count,
    Min,
    Max,
    CountDistinct,
    /// Custom aggregation function
    Custom(String),
}

impl AggregationType {
    /// Build an aggregation from user input, rejecting anything that is not a
    /// plain SQL identifier.
    ///
    /// `Custom` interpolates its name straight into the statement as a function
    /// name, so accepting arbitrary text here would let a request author inject
    /// SQL. A function name is `[A-Za-z_][A-Za-z0-9_$]*` and nothing else --
    /// optionally schema-qualified with a dot, which is still only identifiers.
    /// Anything containing a space, quote, paren or comment marker is refused.
    pub fn parse_strict(input: &str) -> Result<Self, String> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Err("aggregation must not be empty".to_string());
        }
        let upper = trimmed.to_ascii_uppercase();
        let known = match upper.as_str() {
            "SUM" => Some(Self::Sum),
            "AVG" | "AVERAGE" => Some(Self::Avg),
            "COUNT" => Some(Self::Count),
            "MIN" => Some(Self::Min),
            "MAX" => Some(Self::Max),
            "COUNT_DISTINCT" | "COUNTDISTINCT" => Some(Self::CountDistinct),
            _ => None,
        };
        if let Some(known) = known {
            return Ok(known);
        }
        if !trimmed.split('.').all(is_plain_identifier) {
            return Err(format!(
                "{input:?} is not a valid aggregation. Use one of SUM, AVG, COUNT, MIN, \
                 MAX, COUNT_DISTINCT, or a plain function name such as percentile_cont"
            ));
        }
        Ok(Self::Custom(trimmed.to_string()))
    }

    pub fn sql_function(&self) -> &str {
        match self {
            AggregationType::Sum => "SUM",
            AggregationType::Avg => "AVG",
            AggregationType::Count => "COUNT",
            AggregationType::Min => "MIN",
            AggregationType::Max => "MAX",
            AggregationType::CountDistinct => "COUNT(DISTINCT ",
            AggregationType::Custom(name) => name,
        }
    }

    pub fn needs_closing_paren(&self) -> bool {
        matches!(self, AggregationType::CountDistinct)
    }
}

/// Whether `part` is a bare SQL identifier: letter or underscore, then
/// alphanumerics, underscore or `$`. Deliberately strict.
/// A bare identifier: ASCII letters/digits/underscore, not starting with a digit.
///
/// Deliberately narrow. This is the single definition of "safe to pass to a
/// dialect's `quote_ident`", shared by the aggregation check and the formula
/// function-call parser, so the two cannot drift into disagreeing about which
/// names are safe.
pub(crate) fn is_plain_identifier(part: &str) -> bool {
    let mut chars = part.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
}

impl std::fmt::Display for AggregationType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.sql_function())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum TimeGranularity {
    Second,
    Minute,
    Hour,
    Day,
    Week,
    Month,
    Quarter,
    Year,
}

impl TimeGranularity {
    pub fn date_trunc_unit(&self) -> &str {
        match self {
            TimeGranularity::Second => "second",
            TimeGranularity::Minute => "minute",
            TimeGranularity::Hour => "hour",
            TimeGranularity::Day => "day",
            TimeGranularity::Week => "week",
            TimeGranularity::Month => "month",
            TimeGranularity::Quarter => "quarter",
            TimeGranularity::Year => "year",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum JoinType {
    Inner,
    Left,
    Right,
    Full,
}

impl JoinType {
    pub fn sql_keyword(&self) -> &str {
        match self {
            JoinType::Inner => "INNER JOIN",
            JoinType::Left => "LEFT JOIN",
            JoinType::Right => "RIGHT JOIN",
            JoinType::Full => "FULL JOIN",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum FilterOperator {
    Eq,
    Neq,
    Gt,
    Gte,
    Lt,
    Lte,
    Like,
    ILike,
    In,
    NotIn,
    IsNull,
    IsNotNull,
    Between,
    NotBetween,
    /// Spatial: the row's geometry intersects the supplied geometry.
    Intersects,
    /// Spatial: the row's geometry lies within the supplied geometry.
    Within,
    /// Spatial: the row's geometry contains the supplied geometry.
    Contains,
}

impl FilterOperator {
    /// The infix SQL operator, or `None` for operators that are not infix.
    ///
    /// Returning `None` rather than a placeholder string means a spatial
    /// operator cannot be rendered as if it were a comparison: the caller is
    /// forced to deal with it, so `ST_Intersects` can never reach a database
    /// as a bare infix operator between two values.
    pub fn sql_operator(&self) -> Option<&str> {
        Some(match self {
            FilterOperator::Eq => "=",
            FilterOperator::Neq => "!=",
            FilterOperator::Gt => ">",
            FilterOperator::Gte => ">=",
            FilterOperator::Lt => "<",
            FilterOperator::Lte => "<=",
            FilterOperator::Like => "LIKE",
            FilterOperator::ILike => "ILIKE",
            FilterOperator::In => "IN",
            FilterOperator::NotIn => "NOT IN",
            FilterOperator::IsNull => "IS NULL",
            FilterOperator::IsNotNull => "IS NOT NULL",
            FilterOperator::Between => "BETWEEN",
            FilterOperator::NotBetween => "NOT BETWEEN",
            FilterOperator::Intersects | FilterOperator::Within | FilterOperator::Contains => {
                return None
            }
        })
    }

    /// The `ST_*` predicate for a spatial operator.
    ///
    /// The names are shared rather than per-dialect because PostGIS and DuckDB
    /// spatial implement the same functions with the same argument order:
    /// `f(a, b)` reads as "a relative to b", so the row's geometry is always
    /// the first argument. Verified against DuckDB rather than assumed —
    /// `ST_Within(point, polygon)` is true and `ST_Within(polygon, point)` is
    /// not, and swapping them would have silently inverted every result.
    ///
    /// Support itself is gated per dialect by
    /// [`Dialect::spatial_geometry`][geometry], which returns `None` where
    /// there is no equivalent.
    pub fn spatial_function(&self) -> Option<&'static str> {
        match self {
            FilterOperator::Intersects => Some("ST_Intersects"),
            FilterOperator::Within => Some("ST_Within"),
            FilterOperator::Contains => Some("ST_Contains"),
            _ => None,
        }
    }

    /// True when the operator takes no value at all.
    pub fn is_valueless(&self) -> bool {
        matches!(self, FilterOperator::IsNull | FilterOperator::IsNotNull)
    }

    /// True when the operator takes a value, which spatial operators do.
    pub fn needs_value(&self) -> bool {
        !self.is_valueless()
    }

    pub fn needs_two_values(&self) -> bool {
        matches!(self, FilterOperator::Between | FilterOperator::NotBetween)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Formula {
    pub expression: String,
    pub label: Option<String>,
    pub format: Option<String>,
}

impl Formula {
    pub fn new(expression: impl Into<String>) -> Self {
        Self {
            expression: expression.into(),
            label: None,
            format: None,
        }
    }

    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn with_format(mut self, format: impl Into<String>) -> Self {
        self.format = Some(format.into());
        self
    }

    /// Parse shorthand like "revenue:sum" or "status"
    pub fn parse_shorthand(s: &str) -> Self {
        if let Some((expr, agg)) = s.split_once(':') {
            Self::new(expr).with_label(agg)
        } else {
            Self::new(s)
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Measure {
    pub formula: Formula,
    pub aggregation: AggregationType,
}

impl Measure {
    pub fn new(formula: Formula, aggregation: AggregationType) -> Self {
        Self {
            formula,
            aggregation,
        }
    }

    pub fn simple(expression: impl Into<String>, aggregation: AggregationType) -> Self {
        Self::new(Formula::new(expression), aggregation)
    }

    pub fn label(&self) -> &str {
        self.formula
            .label
            .as_deref()
            .unwrap_or(&self.formula.expression)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Dimension {
    pub name: String,
    pub label: Option<String>,
}

impl Dimension {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            label: None,
        }
    }

    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn label(&self) -> &str {
        self.label.as_deref().unwrap_or(&self.name)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TimeDimension {
    pub dimension: String,
    pub granularity: TimeGranularity,
    pub label: Option<String>,
}

impl TimeDimension {
    pub fn new(dimension: impl Into<String>, granularity: TimeGranularity) -> Self {
        Self {
            dimension: dimension.into(),
            granularity,
            label: None,
        }
    }

    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn label(&self) -> &str {
        self.label.as_deref().unwrap_or(&self.dimension)
    }

    pub fn sql_expression(&self, table_alias: Option<&str>) -> String {
        let column = if let Some(alias) = table_alias {
            format!("{}.{}", alias, self.dimension)
        } else {
            self.dimension.clone()
        };
        format!(
            "DATE_TRUNC('{}', {})",
            self.granularity.date_trunc_unit(),
            column
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Filter {
    pub field: String,
    pub operator: FilterOperator,
    pub value: serde_json::Value,
    /// Join this filter to the previous one with OR instead of AND.
    #[serde(default)]
    pub or_condition: bool,
}

impl Filter {
    pub fn new(
        field: impl Into<String>,
        operator: FilterOperator,
        value: serde_json::Value,
    ) -> Self {
        Self {
            field: field.into(),
            operator,
            value,
            or_condition: false,
        }
    }

    pub fn or(mut self) -> Self {
        self.or_condition = true;
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OrderBy {
    pub field: String,
    #[serde(default)]
    pub descending: bool,
}

impl OrderBy {
    pub fn new(field: impl Into<String>, descending: bool) -> Self {
        Self {
            field: field.into(),
            descending,
        }
    }

    pub fn asc(field: impl Into<String>) -> Self {
        Self::new(field, false)
    }

    pub fn desc(field: impl Into<String>) -> Self {
        Self::new(field, true)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SourceSpec {
    pub model: String,
    pub datasource: Option<String>,
    pub alias: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Query {
    pub name: Option<String>,
    pub source_model: Option<SourceSpec>,
    // The collections default to empty so a hand-written query file only has to
    // state the parts it cares about, instead of every key including empty
    // lists. This only widens what deserialization accepts.
    #[serde(default)]
    pub measures: Vec<Measure>,
    #[serde(default)]
    pub dimensions: Vec<Dimension>,
    #[serde(default)]
    pub time_dimensions: Vec<TimeDimension>,
    #[serde(default)]
    pub filters: Vec<Filter>,
    #[serde(default)]
    pub order: Vec<OrderBy>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
    pub whole_periods_only: Option<bool>,
    pub distinct_dimension_values: Option<bool>,
    /// For multi-stage queries - reference to previous stage
    pub stage_ref: Option<String>,
}

impl Query {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    pub fn with_source_model(mut self, source: SourceSpec) -> Self {
        self.source_model = Some(source);
        self
    }

    pub fn add_measure(mut self, measure: Measure) -> Self {
        self.measures.push(measure);
        self
    }

    pub fn add_dimension(mut self, dimension: Dimension) -> Self {
        self.dimensions.push(dimension);
        self
    }

    pub fn add_time_dimension(mut self, td: TimeDimension) -> Self {
        self.time_dimensions.push(td);
        self
    }

    pub fn add_filter(mut self, filter: Filter) -> Self {
        self.filters.push(filter);
        self
    }

    pub fn add_order(mut self, order: OrderBy) -> Self {
        self.order.push(order);
        self
    }

    pub fn with_limit(mut self, limit: usize) -> Self {
        self.limit = Some(limit);
        self
    }

    pub fn with_offset(mut self, offset: usize) -> Self {
        self.offset = Some(offset);
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Model {
    pub name: String,
    pub datasource: String,
    pub description: Option<String>,
    pub measures: Vec<Measure>,
    pub dimensions: Vec<Dimension>,
    pub time_dimensions: Vec<TimeDimension>,
    pub joins: Vec<Join>,
    pub sql: Option<String>, // Custom SQL override
    pub meta: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Join {
    pub name: String,
    pub model: String,
    pub join_type: JoinType,
    pub on: Vec<(String, String)>, // (left_field, right_field)
    pub alias: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DataSource {
    pub name: String,
    pub driver: String, // postgres, mysql, sqlite, etc.
    pub connection_string: String,
    pub description: Option<String>,
    pub models: Vec<String>,
    pub pool_size: Option<u32>,
    pub meta: HashMap<String, serde_json::Value>,
}
