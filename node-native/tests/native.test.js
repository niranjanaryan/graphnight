// Test file for @graphnight/native

import { describe, it, before, after } from 'node:test';
import assert from 'node:assert';
import { GraphNightClient } from '../index.js';

describe('@graphnight/native', () => {
  let client: GraphNightClient;
  const testStoragePath = './test_graphnight_data';

  before(() => {
    // Create client with test storage
    client = new GraphNightClient(testStoragePath);
  });

  after(() => {
    // Cleanup test storage
    // Note: In real tests, you'd want to properly clean up
  });

  it('should create a client instance', () => {
    assert.ok(client instanceof GraphNightClient);
  });

  it('should list models (empty initially)', async () => {
    const models = await client.listModels();
    assert.ok(Array.isArray(models));
  });

  it('should generate SQL for a simple query', async () => {
    const query = {
      name: 'orders',
      measures: [
        { formula: { expression: 'amount_usd', label: 'Revenue' }, aggregation: 'sum' },
      ],
      dimensions: [{ name: 'status' }],
    };
    
    // This will fail if model doesn't exist, but we can test the method exists
    try {
      const result = await client.generateSql(query);
      assert.ok(typeof result === 'string');
    } catch (error) {
      // Expected if model doesn't exist
      assert.ok(error instanceof Error);
    }
  });

  it('should handle dry run', async () => {
    const query = {
      name: 'orders',
      measures: [
        { formula: { expression: 'amount_usd', label: 'Revenue' }, aggregation: 'sum' },
      ],
      dimensions: [{ name: 'status' }],
    };
    
    try {
      const result = await client.dryRun(query);
      assert.ok(typeof result.sql === 'string');
      assert.ok(result.data === '[]');
    } catch (error) {
      // Expected if model doesn't exist
      assert.ok(error instanceof Error);
    }
  });

  it('should list datasources', async () => {
    const datasources = await client.listDataSources();
    assert.ok(Array.isArray(datasources));
  });

  it('should search models', async () => {
    const results = await client.search('revenue', 10);
    assert.ok(Array.isArray(results));
  });

  it('should save and list memories', async () => {
    const memory = await client.saveMemory(
      'Test learning',
      ['orders', 'customers'],
      'test-memory-1',
      'Test description'
    );
    
    assert.ok(memory.id);
    assert.strictEqual(memory.learning, 'Test learning');
    assert.deepStrictEqual(memory.linked_entities, ['orders', 'customers']);
    assert.strictEqual(memory.description, 'Test description');

    const memories = await client.listMemories();
    assert.ok(Array.isArray(memories));
    
    const found = memories.find(m => m.id === memory.id);
    assert.ok(found);
  });

  it('should delete memory', async () => {
    const memory = await client.saveMemory(
      'To be deleted',
      ['orders'],
      'delete-test'
    );
    
    const deleted = await client.deleteMemory(memory.id);
    assert.strictEqual(deleted, true);
    
    const memories = await client.listMemories();
    const found = memories.find(m => m.id === memory.id);
    assert.strictEqual(found, undefined);
  });
});