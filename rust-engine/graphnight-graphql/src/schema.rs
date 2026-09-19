use async_graphql::*;
use graphnight_core::models::{
    AggregationType as CoreAggregationType, FilterOperator as CoreFilterOperator,
    JoinType as CoreJoinType, Measure, OrderBy, Query as CoreQuery, SourceSpec, TimeDimension,
    TimeGranularity as CoreTimeGranularity,
};
pub use serde_json::Value as JsonValue;
use std::collections::HashMap;

/// Aggregation type enum for GraphQL
#[derive(Enum, Copy, Clone, Eq, PartialEq, Debug)]
pub enum AggregationType {
    Sum,
    Avg,
    Count,
    Min,
    Max,
    CountDistinct,
}

impl From<AggregationType> for CoreAggregationType {
    fn from(v: AggregationType) -> Self {
        match v {
            AggregationType::Sum => CoreAggregationType::Sum,
            AggregationType::Avg => CoreAggregationType::Avg,
            AggregationType::Count => CoreAggregationType::Count,
            AggregationType::Min => CoreAggregationType::Min,
            AggregationType::Max => CoreAggregationType::Max,
            AggregationType::CountDistinct => CoreAggregationType::CountDistinct,
        }
    }
}

impl From<CoreAggregationType> for AggregationType {
    fn from(v: CoreAggregationType) -> Self {
        match v {
            CoreAggregationType::Sum => AggregationType::Sum,
            CoreAggregationType::Avg => AggregationType::Avg,
            CoreAggregationType::Count => AggregationType::Count,
            CoreAggregationType::Min => AggregationType::Min,
            CoreAggregationType::Max => AggregationType::Max,
            CoreAggregationType::CountDistinct => AggregationType::CountDistinct,
            CoreAggregationType::Custom(s) => {
                panic!("Custom aggregation not supported in GraphQL: {}", s)
            }
        }
    }
}

/// Time granularity enum for GraphQL
#[derive(Enum, Copy, Clone, Eq, PartialEq, Debug)]
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

impl From<TimeGranularity> for CoreTimeGranularity {
    fn from(v: TimeGranularity) -> Self {
        match v {
            TimeGranularity::Second => CoreTimeGranularity::Second,
            TimeGranularity::Minute => CoreTimeGranularity::Minute,
            TimeGranularity::Hour => CoreTimeGranularity::Hour,
            TimeGranularity::Day => CoreTimeGranularity::Day,
            TimeGranularity::Week => CoreTimeGranularity::Week,
            TimeGranularity::Month => CoreTimeGranularity::Month,
            TimeGranularity::Quarter => CoreTimeGranularity::Quarter,
            TimeGranularity::Year => CoreTimeGranularity::Year,
        }
    }
}

impl From<CoreTimeGranularity> for TimeGranularity {
    fn from(v: CoreTimeGranularity) -> Self {
        match v {
            CoreTimeGranularity::Second => TimeGranularity::Second,
            CoreTimeGranularity::Minute => TimeGranularity::Minute,
            CoreTimeGranularity::Hour => TimeGranularity::Hour,
            CoreTimeGranularity::Day => TimeGranularity::Day,
            CoreTimeGranularity::Week => TimeGranularity::Week,
            CoreTimeGranularity::Month => TimeGranularity::Month,
            CoreTimeGranularity::Quarter => TimeGranularity::Quarter,
            CoreTimeGranularity::Year => TimeGranularity::Year,
        }
    }
}

/// Filter operator enum for GraphQL
#[derive(Enum, Copy, Clone, Eq, PartialEq, Debug)]
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

impl From<FilterOperator> for CoreFilterOperator {
    fn from(v: FilterOperator) -> Self {
        match v {
            FilterOperator::Eq => CoreFilterOperator::Eq,
            FilterOperator::Neq => CoreFilterOperator::Neq,
            FilterOperator::Gt => CoreFilterOperator::Gt,
            FilterOperator::Gte => CoreFilterOperator::Gte,
            FilterOperator::Lt => CoreFilterOperator::Lt,
            FilterOperator::Lte => CoreFilterOperator::Lte,
            FilterOperator::Like => CoreFilterOperator::Like,
            FilterOperator::ILike => CoreFilterOperator::ILike,
            FilterOperator::In => CoreFilterOperator::In,
            FilterOperator::NotIn => CoreFilterOperator::NotIn,
            FilterOperator::IsNull => CoreFilterOperator::IsNull,
            FilterOperator::IsNotNull => CoreFilterOperator::IsNotNull,
            FilterOperator::Between => CoreFilterOperator::Between,
            FilterOperator::NotBetween => CoreFilterOperator::NotBetween,
        }
    }
}

/// Join type enum for GraphQL
#[derive(Enum, Copy, Clone, Eq, PartialEq, Debug)]
pub enum JoinType {
    Inner,
    Left,
    Right,
    Full,
}

impl From<JoinType> for CoreJoinType {
    fn from(v: JoinType) -> Self {
        match v {
            JoinType::Inner => CoreJoinType::Inner,
            JoinType::Left => CoreJoinType::Left,
            JoinType::Right => CoreJoinType::Right,
            JoinType::Full => CoreJoinType::Full,
        }
    }
}

/// Source specification input
#[derive(InputObject, Clone)]
pub struct SourceSpecInput {
    pub model: String,
    pub datasource: Option<String>,
    pub alias: Option<String>,
}

impl From<SourceSpecInput> for SourceSpec {
    fn from(v: SourceSpecInput) -> Self {
        SourceSpec {
            model: v.model,
            datasource: v.datasource,
            alias: v.alias,
        }
    }
}

/// Measure input
#[derive(InputObject, Clone)]
pub struct MeasureInput {
    pub formula: String,
    pub label: Option<String>,
    pub format: Option<String>,
    pub aggregation: AggregationType,
}

impl From<MeasureInput> for Measure {
    fn from(v: MeasureInput) -> Self {
        Measure {
            formula: graphnight_core::models::Formula {
                expression: v.formula,
                label: v.label,
                format: v.format,
            },
            aggregation: v.aggregation.into(),
        }
    }
}

/// Dimension input
#[derive(InputObject, Clone)]
pub struct DimensionInput {
    pub name: String,
    pub label: Option<String>,
}

impl From<DimensionInput> for graphnight_core::models::Dimension {
    fn from(v: DimensionInput) -> Self {
        graphnight_core::models::Dimension {
            name: v.name,
            label: v.label,
        }
    }
}

/// Time dimension input
#[derive(InputObject, Clone)]
pub struct TimeDimensionInput {
    pub dimension: String,
    pub granularity: TimeGranularity,
    pub label: Option<String>,
}

impl From<TimeDimensionInput> for TimeDimension {
    fn from(v: TimeDimensionInput) -> Self {
        TimeDimension {
            dimension: v.dimension,
            granularity: v.granularity.into(),
            label: v.label,
        }
    }
}

/// Filter input
#[derive(InputObject, Clone)]
pub struct FilterInput {
    pub field: String,
    pub operator: FilterOperator,
    pub value: JsonValue,
    pub or_condition: Option<bool>,
}

impl From<FilterInput> for graphnight_core::models::Filter {
    fn from(v: FilterInput) -> Self {
        graphnight_core::models::Filter {
            field: v.field,
            operator: v.operator.into(),
            value: v.value,
            or_condition: v.or_condition.unwrap_or(false),
        }
    }
}

/// Order by input
#[derive(InputObject, Clone)]
pub struct OrderByInput {
    pub field: String,
    pub descending: Option<bool>,
}

impl From<OrderByInput> for OrderBy {
    fn from(v: OrderByInput) -> Self {
        OrderBy {
            field: v.field,
            descending: v.descending.unwrap_or(false),
        }
    }
}

/// Main query input
#[derive(InputObject, Clone, Default)]
pub struct QueryInput {
    pub name: Option<String>,
    pub source_model: Option<SourceSpecInput>,
    pub measures: Option<Vec<MeasureInput>>,
    pub dimensions: Option<Vec<DimensionInput>>,
    pub time_dimensions: Option<Vec<TimeDimensionInput>>,
    pub filters: Option<Vec<FilterInput>>,
    pub order: Option<Vec<OrderByInput>>,
    pub limit: Option<i32>,
    pub offset: Option<i32>,
    pub whole_periods_only: Option<bool>,
    pub distinct_dimension_values: Option<bool>,
}

impl From<QueryInput> for CoreQuery {
    fn from(v: QueryInput) -> Self {
        CoreQuery {
            name: v.name,
            source_model: v.source_model.map(|s| s.into()),
            measures: v
                .measures
                .unwrap_or_default()
                .into_iter()
                .map(|m| m.into())
                .collect(),
            dimensions: v
                .dimensions
                .unwrap_or_default()
                .into_iter()
                .map(|d| d.into())
                .collect(),
            time_dimensions: v
                .time_dimensions
                .unwrap_or_default()
                .into_iter()
                .map(|t| t.into())
                .collect(),
            filters: v
                .filters
                .unwrap_or_default()
                .into_iter()
                .map(|f| f.into())
                .collect(),
            order: v
                .order
                .unwrap_or_default()
                .into_iter()
                .map(|o| o.into())
                .collect(),
            limit: v.limit.map(|l| l as usize),
            offset: v.offset.map(|o| o as usize),
            whole_periods_only: v.whole_periods_only,
            distinct_dimension_values: v.distinct_dimension_values,
            stage_ref: None,
        }
    }
}

/// Field metadata output
#[derive(SimpleObject, serde::Serialize, serde::Deserialize)]
pub struct FieldMetadata {
    pub label: Option<String>,
    pub format: Option<String>,
}

/// Response attributes output
#[derive(SimpleObject)]
pub struct ResponseAttributes {
    pub dimensions: HashMap<String, FieldMetadata>,
    pub measures: HashMap<String, FieldMetadata>,
}

/// Query response output
#[derive(SimpleObject)]
pub struct QueryResponse {
    pub data: Vec<JsonValue>,
    pub columns: Vec<String>,
    pub sql: Option<String>,
    pub attributes: Option<ResponseAttributes>,
    pub population: Option<i64>,
    pub population_inferred: bool,
    pub execution_time_ms: f64,
}

/// Dry run response
#[derive(SimpleObject)]
pub struct DryRunResponse {
    pub sql: String,
    pub explained: bool,
}

/// Multi-stage query response
#[derive(SimpleObject)]
pub struct MultiStageResponse {
    pub results: Vec<QueryResponse>,
    pub execution_time_ms: f64,
}

/// Model info output
#[derive(SimpleObject)]
pub struct ModelInfo {
    pub name: String,
    pub datasource: String,
    pub description: Option<String>,
    pub measures: Vec<String>,
    pub dimensions: Vec<String>,
    pub time_dimensions: Vec<String>,
}

/// Datasource info output
#[derive(SimpleObject)]
pub struct DatasourceInfo {
    pub name: String,
    pub driver: String,
    pub description: Option<String>,
    pub models: Vec<String>,
}

/// Search result output
#[derive(SimpleObject)]
pub struct SearchResult {
    pub model_name: String,
    pub datasource: String,
    pub score: f32,
    pub matched_fields: Vec<String>,
    pub snippet: String,
}

/// Memory output
#[derive(SimpleObject)]
pub struct Memory {
    pub id: String,
    pub learning: String,
    pub linked_entities: Vec<String>,
    pub description: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// Save memory input
#[derive(InputObject)]
pub struct SaveMemoryInput {
    pub learning: String,
    pub linked_entities: Vec<String>,
    pub id: Option<String>,
    pub description: Option<String>,
}

/// Memory filter input
#[derive(InputObject, Default)]
pub struct MemoryFilter {
    pub query: Option<String>,
    pub entity: Option<String>,
    pub limit: Option<i32>,
    pub offset: Option<i32>,
}

/// Forget memory response
#[derive(SimpleObject)]
pub struct ForgetMemoryResponse {
    pub success: bool,
    pub id: String,
}

/// Ingestion report
#[derive(SimpleObject)]
pub struct IngestionReport {
    pub models_created: i32,
    pub models_updated: i32,
    pub errors: Vec<String>,
}

/// Create model input
#[derive(InputObject)]
pub struct CreateModelInput {
    pub name: String,
    pub datasource: String,
    pub description: Option<String>,
    pub measures: Vec<MeasureInput>,
    pub dimensions: Vec<DimensionInput>,
    pub time_dimensions: Vec<TimeDimensionInput>,
    pub joins: Option<Vec<JoinInput>>,
}

/// Join input
#[derive(InputObject)]
pub struct JoinInput {
    pub name: String,
    pub model: String,
    pub join_type: JoinType,
    pub on: Vec<JoinConditionInput>,
    pub alias: Option<String>,
}

/// Join condition input
#[derive(InputObject)]
pub struct JoinConditionInput {
    pub left: String,
    pub right: String,
}

/// Create datasource input
#[derive(InputObject)]
pub struct CreateDatasourceInput {
    pub name: String,
    pub driver: String,
    pub connection_string: String,
    pub description: Option<String>,
    pub pool_size: Option<i32>,
}

/// Update model input
#[derive(InputObject)]
pub struct UpdateModelInput {
    pub description: Option<String>,
    pub measures: Option<Vec<MeasureInput>>,
    pub dimensions: Option<Vec<DimensionInput>>,
    pub time_dimensions: Option<Vec<TimeDimensionInput>>,
    pub joins: Option<Vec<JoinInput>>,
}

/// Update datasource input
#[derive(InputObject)]
pub struct UpdateDatasourceInput {
    pub description: Option<String>,
    pub connection_string: Option<String>,
    pub pool_size: Option<i32>,
}

/// Model change event for subscriptions
#[derive(SimpleObject)]
pub struct ModelChangeEvent {
    pub event_type: String, // "created", "updated", "deleted"
    pub model_name: String,
    pub datasource: String,
    pub timestamp: String,
}

impl From<JoinInput> for graphnight_core::models::Join {
    fn from(v: JoinInput) -> Self {
        graphnight_core::models::Join {
            name: v.name,
            model: v.model,
            join_type: v.join_type.into(),
            on: v.on.into_iter().map(|c| (c.left, c.right)).collect(),
            alias: v.alias,
        }
    }
}

impl From<JoinConditionInput> for (String, String) {
    fn from(v: JoinConditionInput) -> Self {
        (v.left, v.right)
    }
}
