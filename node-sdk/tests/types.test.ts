// GraphNight SDK - Basic Type Tests

import { 
  GraphNightClient, 
  createClient,
  Model,
  DataSource,
  QueryInput,
  QueryResult,
  Measure,
  Dimension,
  TimeDimension,
  Filter,
  OrderBy,
  Aggregation,
  Granularity,
  Driver,
  FilterOperator,
  JoinType,
  SortDirection,
} from '../src';

describe('GraphNight SDK Types', () => {
  it('should export all types', () => {
    // This test just verifies types compile correctly
    const client = createClient({ url: 'http://localhost:8080/graphql' });
    expect(client).toBeInstanceOf(GraphNightClient);
  });

  it('should allow creating Model type', () => {
    const model: Model = {
      name: 'orders',
      datasource: 'analytics',
      description: 'Order transactions',
      measures: [
        {
          formula: { expression: 'amount_usd', label: 'Revenue' },
          aggregation: 'sum',
        },
        {
          formula: { expression: '*', label: 'Orders' },
          aggregation: 'count',
        },
      ],
      dimensions: [
        { name: 'status', label: 'Order Status' },
        { name: 'store_id', label: 'Store' },
      ],
      time_dimensions: [
        { dimension: 'created_at', granularity: 'MONTH' },
      ],
      joins: [],
    };

    expect(model.name).toBe('orders');
    expect(model.measures).toHaveLength(2);
  });

  it('should allow creating DataSource type', () => {
    const ds: DataSource = {
      name: 'analytics',
      driver: 'postgres',
      connection_string: 'postgresql://user:pass@localhost/db',
      description: 'Analytics database',
      models: ['orders', 'customers'],
      pool_size: 10,
    };

    expect(ds.driver).toBe('postgres');
    expect(ds.models).toContain('orders');
  });

  it('should allow creating QueryInput type', () => {
    const query: QueryInput = {
      name: 'orders',
      measures: [
        { formula: { expression: 'amount_usd', label: 'Revenue' }, aggregation: 'sum' },
      ],
      dimensions: [{ name: 'status' }],
      time_dimensions: [{ dimension: 'created_at', granularity: 'MONTH' }],
      filters: [
        { field: 'status', operator: 'eq', value: 'completed' },
        { field: 'amount_usd', operator: 'gte', value: 100 },
      ],
      order: [
        { field: 'amount_usd:sum', direction: 'desc' },
      ],
      limit: 100,
      offset: 0,
    };

    expect(query.filters).toHaveLength(2);
    expect(query.limit).toBe(100);
  });

  it('should support all aggregation types', () => {
    const aggregations: Aggregation[] = ['sum', 'count', 'avg', 'min', 'max', 'count_distinct'];
    expect(aggregations).toHaveLength(6);
  });

  it('should support all granularity types', () => {
    const granularities: Granularity[] = ['HOUR', 'DAY', 'WEEK', 'MONTH', 'QUARTER', 'YEAR'];
    expect(granularities).toHaveLength(6);
  });

  it('should support all driver types', () => {
    const drivers: Driver[] = ['postgres', 'mysql', 'sqlite', 'duckdb'];
    expect(drivers).toHaveLength(4);
  });

  it('should support all filter operators', () => {
    const operators: FilterOperator[] = [
      'eq', 'neq',
      'gt', 'gte', 'lt', 'lte',
      'in', 'not_in',
      'like', 'ilike', 'not_like',
      'is_null', 'is_not_null',
      'between',
    ];
    expect(operators).toHaveLength(14);
  });

  it('should support all join types', () => {
    const joinTypes: JoinType[] = ['inner', 'left', 'right', 'full'];
    expect(joinTypes).toHaveLength(4);
  });

  it('should support all sort directions', () => {
    const directions: SortDirection[] = ['asc', 'desc'];
    expect(directions).toHaveLength(2);
  });

  it('should support multi-stage queries', () => {
    const multiStage = {
      stages: [
        {
          name: 'orders',
          measures: [{ formula: { expression: 'amount_usd', label: 'Revenue' }, aggregation: 'sum' }],
          dimensions: [{ name: 'customer_id' }],
          order: [{ field: 'amount_usd:sum', direction: 'desc' }],
          limit: 10,
        },
        {
          name: 'orders',
          measures: [{ formula: { expression: 'amount_usd', label: 'Revenue' }, aggregation: 'sum' }],
          dimensions: [{ name: 'product_id' }],
          filters: [{ field: 'customer_id', operator: 'eq', value: '{{stage1.customer_id}}' }],
          stage_ref: 'stage1',
        },
      ],
    };

    expect(multiStage.stages).toHaveLength(2);
    expect(multiStage.stages[1].stage_ref).toBe('stage1');
  });
});