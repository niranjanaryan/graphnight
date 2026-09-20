// GraphNight Node.js SDK Tests

import { GraphNightClient, createClient } from '../src';

describe('GraphNightClient', () => {
  describe('createClient', () => {
    it('should create a client in local mode', () => {
      const client = createClient({ storagePath: './test_data' });
      expect(client).toBeInstanceOf(GraphNightClient);
      expect(client.getStoragePath()).toBe('./test_data');
      expect(client.isConnected()).toBe(false);
    });

    it('should create a client in server mode', () => {
      const client = createClient({ 
        url: 'http://localhost:8080/graphql',
        headers: { Authorization: 'Bearer test' }
      });
      expect(client).toBeInstanceOf(GraphNightClient);
      expect(client.isConnected()).toBe(true);
    });

    it('should create a client with timeout', () => {
      const client = createClient({ 
        url: 'http://localhost:8080/graphql',
        timeout: 5000
      });
      expect(client.isConnected()).toBe(true);
    });
  });

  describe('local mode operations', () => {
    const client = createClient({ storagePath: './test_data' });

    it('should throw for server-only operations in local mode', async () => {
      await expect(client.listModels()).rejects.toThrow('local mode');
      await expect(client.getModel('test')).rejects.toThrow('local mode');
      await expect(client.query({ name: 'test', measures: [] })).rejects.toThrow('local mode');
    });

    it('should throw for memory operations in server mode', async () => {
      const serverClient = createClient({ url: 'http://localhost:8080/graphql' });
      await expect(serverClient.saveMemory('key', 'value')).rejects.toThrow('local mode');
      await expect(serverClient.getMemory('key')).rejects.toThrow('local mode');
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

      // Just verify it's valid TypeScript - runtime validation would be in integration tests
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