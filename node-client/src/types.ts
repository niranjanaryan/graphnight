// Type definitions for GraphNight HTTP Client

/** Supported database drivers */
export type Driver = 'postgres' | 'mysql' | 'sqlite' | 'duckdb';

/** Supported aggregation functions */
export type Aggregation = 'sum' | 'count' | 'avg' | 'min' | 'max' | 'count_distinct';

/** Supported filter operators */
export type FilterOperator = 
  | 'eq' | 'neq' 
  | 'gt' | 'gte' | 'lt' | 'lte' 
  | 'in' | 'not_in' 
  | 'like' | 'ilike' | 'not_like' 
  | 'is_null' | 'is_not_null'
  | 'between';

/** Supported time granularities */
export type Granularity = 'HOUR' | 'DAY' | 'WEEK' | 'MONTH' | 'QUARTER' | 'YEAR';

/** Sort direction */
export type SortDirection = 'asc' | 'desc';

/** Join types */
export type JoinType = 'inner' | 'left' | 'right' | 'full';

/** Formula definition */
export interface Formula {
  expression: string;
  label?: string;
}

/** Measure definition */
export interface Measure {
  formula: Formula;
  aggregation: Aggregation;
}

/** Dimension definition */
export interface Dimension {
  name: string;
  label?: string;
}

/** Time dimension definition */
export interface TimeDimension {
  dimension: string;
  granularity: Granularity;
}

/** Join definition */
export interface Join {
  name: string;
  model: string;
  join_type: JoinType;
  on: string;
  alias?: string;
}

/** Filter definition */
export interface Filter {
  field: string;
  operator: FilterOperator;
  value?: unknown;
  values?: unknown[];
  or_condition?: boolean;
}

/** Order by definition */
export interface OrderBy {
  field: string;
  direction?: SortDirection;
}

/** Query input */
export interface QueryInput {
  name: string;
  measures: Measure[];
  dimensions?: Dimension[];
  time_dimensions?: TimeDimension[];
  filters?: Filter[];
  order?: OrderBy[];
  limit?: number;
  offset?: number;
  stage_ref?: string; // For multi-stage queries
}

/** Multi-stage query input */
export interface MultiStageQueryInput {
  stages: QueryInput[];
}

/** Model definition */
export interface Model {
  name: string;
  datasource: string;
  description?: string;
  measures: Measure[];
  dimensions?: Dimension[];
  time_dimensions?: TimeDimension[];
  joins?: Join[];
}

/** DataSource definition */
export interface DataSource {
  name: string;
  driver: Driver;
  connection_string: string;
  description?: string;
  models?: string[];
  pool_size?: number;
}

/** Query result */
export interface QueryResult {
  data: Record<string, unknown>[];
  columns: string[];
  sql: string;
  execution_time_ms: number;
  row_count: number;
}

/** Multi-stage query result */
export interface MultiStageQueryResult {
  results: QueryResult[];
  execution_time_ms: number;
}

/** Dry run result */
export interface DryRunResult {
  sql: string;
}

/** Session policy for security */
export interface SessionPolicy {
  user_id: string;
  roles?: string[];
  row_level_filters?: Record<string, Filter[]>;
  column_masks?: Record<string, string[]>;
  query_timeout_secs?: number;
}

/** Ingestion report */
export interface IngestionReport {
  models_created: number;
  models_updated: number;
  errors: string[];
}

/** GraphNight client configuration */
export interface GraphNightClientConfig {
  url: string; // GraphQL server URL (required)
  headers?: Record<string, string>; // Auth headers
  timeout?: number; // Request timeout in ms
}

/** Model list response */
export interface ModelListResponse {
  models: Model[];
  total: number;
}

/** DataSource list response */
export interface DataSourceListResponse {
  datasources: DataSource[];
  total: number;
}

/** Search result */
export interface SearchResult {
  model: Model;
  score: number;
  matches: string[];
}

/** Search response */
export interface SearchResponse {
  results: SearchResult[];
  total: number;
}