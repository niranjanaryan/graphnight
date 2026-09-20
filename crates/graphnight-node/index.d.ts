// TypeScript definitions for @graphnight/native

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
export type Granularity = 'second' | 'minute' | 'hour' | 'day' | 'week' | 'month' | 'quarter' | 'year';

/** Sort direction */
export type SortDirection = 'asc' | 'desc';

/** Join types */
export type JoinType = 'inner' | 'left' | 'right' | 'full';

/** Formula definition */
export interface FormulaInput {
  expression: string;
  label?: string;
  format?: string;
}

/** Measure definition */
export interface MeasureInput {
  formula: FormulaInput;
  aggregation: Aggregation;
}

/** Dimension definition */
export interface DimensionInput {
  name: string;
  label?: string;
}

/** Time dimension definition */
export interface TimeDimensionInput {
  dimension: string;
  granularity: Granularity;
  label?: string;
}

/** Filter definition */
export interface FilterInput {
  field: string;
  operator: FilterOperator;
  value?: string;  // JSON string
  values?: string; // JSON string array
  or_condition?: boolean;
}

/** Order by definition */
export interface OrderByInput {
  field: string;
  descending?: boolean;
}

/** Source specification for multi-model queries */
export interface SourceSpecInput {
  model: string;
  datasource?: string;
  alias?: string;
}

/** Query input */
export interface QueryInput {
  name?: string;
  source_model?: SourceSpecInput;
  measures: MeasureInput[];
  dimensions?: DimensionInput[];
  time_dimensions?: TimeDimensionInput[];
  filters?: FilterInput[];
  order?: OrderByInput[];
  limit?: number;
  offset?: number;
  whole_periods_only?: boolean;
  distinct_dimension_values?: boolean;
}

/** Model definition */
export interface ModelInput {
  name: string;
  datasource: string;
  description?: string;
  measures: MeasureInput[];
  dimensions?: DimensionInput[];
  time_dimensions?: TimeDimensionInput[];
  joins?: JoinInput[];
}

/** Join definition */
export interface JoinInput {
  name: string;
  model: string;
  join_type: JoinType;
  on: string;
  alias?: string;
}

/** DataSource definition */
export interface DataSourceInput {
  name: string;
  driver: Driver;
  connection_string: string;
  description?: string;
  models?: string[];
  pool_size?: number;
}

/** Query result */
export interface QueryResult {
  data: string;  // JSON string of array of objects
  columns: string[];
  sql: string;
  execution_time_ms: number;
  row_count: number;
}

/** Model summary */
export interface ModelSummary {
  name: string;
  datasource: string;
  description: string;
  measures: string[];
  dimensions: string[];
  time_dimensions: string[];
}

/** DataSource summary */
export interface DataSourceSummary {
  name: string;
  driver: string;
  description: string;
  models: string[];
  pool_size?: number;
}

/** Search result */
export interface SearchResult {
  model_name: string;
  datasource: string;
  score: number;
  matched_fields: string[];
  snippet: string;
}

/** Memory item */
export interface MemoryItem {
  id: string;
  learning: string;
  linked_entities: string[];
  description?: string;
  created_at: string;
  updated_at: string;
}

/** GraphNight client class */
export declare class GraphNightClient {
  constructor(storagePath?: string);
  
  // Query operations
  query(input: QueryInput): Promise<QueryResult>;
  generateSql(input: QueryInput): Promise<string>;
  dryRun(input: QueryInput): Promise<QueryResult>;
  
  // Model operations
  listModels(datasource?: string): Promise<ModelSummary[]>;
  getModel(name: string, datasource?: string): Promise<ModelSummary | null>;
  createModel(model: ModelInput): Promise<ModelSummary>;
  
  // DataSource operations
  listDataSources(): Promise<DataSourceSummary[]>;
  createDataSource(datasource: DataSourceInput): Promise<DataSourceSummary>;
  
  // Search
  search(query: string, limit?: number): Promise<SearchResult[]>;
  
  // Memory operations
  saveMemory(
    learning: string,
    linkedEntities: string[],
    id?: string,
    description?: string
  ): Promise<MemoryItem>;
  listMemories(
    query?: string,
    entity?: string,
    limit?: number,
    offset?: number
  ): Promise<MemoryItem[]>;
  deleteMemory(id: string): Promise<boolean>;
}

/** Create a new GraphNight client */
export function createClient(storagePath?: string): GraphNightClient;

/** Version */
export const VERSION: string;