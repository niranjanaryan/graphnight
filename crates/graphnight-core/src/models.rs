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
}

impl FilterOperator {
    pub fn sql_operator(&self) -> &str {
        match self {
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
        }
    }

    pub fn needs_value(&self) -> bool {
        !matches!(self, FilterOperator::IsNull | FilterOperator::IsNotNull)
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
    pub measures: Vec<Measure>,
    pub dimensions: Vec<Dimension>,
    pub time_dimensions: Vec<TimeDimension>,
    pub filters: Vec<Filter>,
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
