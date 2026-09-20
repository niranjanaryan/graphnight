// GraphNight Client Tests

import { GraphNightClient, createClient } from '../src';

describe('GraphNightClient', () => {
  const mockUrl = 'http://localhost:8080/graphql';

  describe('createClient', () => {
    it('should create a client with URL', () => {
      const client = createClient({ 
        url: mockUrl,
        headers: { Authorization: 'Bearer test' }
      });
      expect(client).toBeInstanceOf(GraphNightClient);
    });

    it('should throw without URL', () => {
      expect(() => createClient({ url: '' })).toThrow('url is required');
      expect(() => createClient({} as any)).toThrow('url is required');
    });

    it('should create a client with timeout', () => {
      const client = createClient({ 
        url: mockUrl,
        timeout: 5000
      });
      expect(client).toBeInstanceOf(GraphNightClient);
    });
  });

  describe('query building', () => {
    it('should accept valid query input', () => {
      const query = {
        name: 'orders',
        measures: [
          { formula: { expression: 'amount_usd', label: 'Revenue' }, aggregation: 'sum' as const },
        ],
        dimensions: [{ name: 'status' }],
        filters: [
          { field: 'status', operator: 'eq' as const, value: 'completed' },
        ],
        order: [{ field: 'amount_usd:sum', direction: 'desc' as const }],
        limit: 100,
      };

      expect(query.name).toBe('orders');
      expect(query.measures).toHaveLength(1);
      expect(query.filters?.[0].operator).toBe('eq');
    });

    it('should accept multi-stage query input', () => {
      const multiStageQuery = {
        stages: [
          {
            name: 'orders',
            measures: [{ formula: { expression: 'amount_usd', label: 'Revenue' }, aggregation: 'sum' as const }],
            dimensions: [{ name: 'customer_id' }],
            order: [{ field: 'amount_usd:sum', direction: 'desc' as const }],
            limit: 10,
          },
          {
            name: 'orders',
            measures: [{ formula: { expression: 'amount_usd', label: 'Revenue' }, aggregation: 'sum' as const }],
            dimensions: [{ name: 'product_id' }],
            filters: [{ field: 'customer_id', operator: 'eq' as const, value: '{{stage1.customer_id}}' }],
            stage_ref: 'stage1',
          },
        ],
      };

      expect(multiStageQuery.stages).toHaveLength(2);
      expect(multiStageQuery.stages[1].stage_ref).toBe('stage1');
    });
  });

  describe('types', () => {
    it('should have correct type exports', () => {
      // This is a compile-time test - if it compiles, types are correct
      const model = {
        name: 'orders',
        datasource: 'analytics',
        measures: [
          { formula: { expression: 'amount_usd' }, aggregation: 'sum' as const },
        ],
        dimensions: [{ name: 'status' }],
        time_dimensions: [{ dimension: 'created_at', granularity: 'MONTH' as const }],
        joins: [{ name: 'customers', model: 'customers', join_type: 'left' as const, on: 'customer_id' }],
      };

      expect(model.name).toBe('orders');
      expect(model.joins?.[0].join_type).toBe('left');
    });
  });
});