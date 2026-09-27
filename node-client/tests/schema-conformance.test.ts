// Schema conformance: every GraphQL document the client sends must validate
// against the server's real schema.
//
// `schema.graphql` is generated from the Rust schema by
//   GRAPHNIGHT_WRITE_SDL=1 cargo test -p graphnight-graphql
// and committed. The previous revision of this client shipped documents for a
// schema that never existed (nested `measures { formula { ... } }`, a
// `connectionString` field, `query` sent as a mutation), and nothing caught it
// because the old tests only asserted TypeScript types.

import { readFileSync } from 'fs';
import { join } from 'path';
import { buildSchema, parse, validate, print, DocumentNode } from 'graphql';
import { GraphNightClient, createClient } from '../src';

const SDL_PATH = join(__dirname, 'schema.graphql');

function loadSchema() {
  try {
    return buildSchema(readFileSync(SDL_PATH, 'utf8'));
  } catch (e) {
    throw new Error(
      `Missing or unreadable ${SDL_PATH}. Regenerate with:\n` +
        `  GRAPHNIGHT_WRITE_SDL=1 cargo test -p graphnight-graphql\n` +
        `Original error: ${e}`
    );
  }
}

const schema = loadSchema();

/**
 * Run `invoke` against a client whose transport is stubbed, and return the
 * GraphQL document it tried to send. Returns an empty string when the call
 * failed before reaching the transport.
 */
async function captureDocument(invoke: (client: GraphNightClient) => Promise<unknown>): Promise<string> {
  const client = createClient({ url: 'http://unused.invalid/graphql' });
  let captured = '';
  (client as any).client = {
    request: async (doc: DocumentNode | string) => {
      // graphql-request's `gql` yields a string; be tolerant of both forms.
      captured = typeof doc === 'string' ? doc : print(doc);
      return {};
    },
  };
  try {
    await invoke(client);
  } catch {
    /* argument validation may fail before transport; captured stays as-is */
  }
  return captured;
}

function assertValid(label: string, doc: string) {
  if (!doc) {
    throw new Error(`${label}: no GraphQL document reached the transport (bad arguments?)`);
  }
  const errors = validate(schema, parse(doc));
  if (errors.length > 0) {
    const detail = errors.map((e) => `  - ${e.message}`).join('\n');
    throw new Error(`${label} is invalid against the server schema:\n${detail}\n\nDocument:\n${doc}`);
  }
}

describe('schema snapshot', () => {
  it('loads the committed server SDL', () => {
    expect(schema.getQueryType()?.name).toBe('QueryRoot');
    expect(schema.getMutationType()?.name).toBe('MutationRoot');
  });

  it('exposes the resolvers the client depends on', () => {
    const q = schema.getQueryType()!.getFields();
    const m = schema.getMutationType()!.getFields();

    // `query`, `multiStageQuery` and `dryRun` are queries, not mutations.
    for (const f of ['query', 'multiStageQuery', 'dryRun', 'datasource', 'search', 'memories', 'memory', 'models', 'model', 'inspect']) {
      expect(q[f]).toBeDefined();
    }
    for (const f of ['createModel', 'updateModel', 'deleteModel', 'createDatasource', 'updateDatasource', 'deleteDatasource', 'ingestModels', 'saveMemory', 'forgetMemory']) {
      expect(m[f]).toBeDefined();
    }
  });

  it('never exposes datasource connection strings', () => {
    const fields = (schema.getType('DatasourceInfo') as any).getFields();
    expect(fields['connectionString']).toBeUndefined();
    expect(fields['connection_string']).toBeUndefined();
    expect(fields['poolSize']).toBeDefined();
  });

  it('ModelInfo carries structured details including joins', () => {
    const fields = (schema.getType('ModelInfo') as any).getFields();
    for (const f of [
      'measures',
      'dimensions',
      'timeDimensions',
      'measureDetails',
      'dimensionDetails',
      'timeDimensionDetails',
      'joins',
    ]) {
      expect(fields[f]).toBeDefined();
    }
  });
});

describe('client documents conform to the server schema', () => {
  const cases: Array<[string, (c: GraphNightClient) => Promise<unknown>]> = [
    ['listModels', (c) => c.listModels()],
    ['getModel', (c) => c.getModel('orders')],
    ['inspectModel', (c) => c.inspectModel('orders')],
    ['createModel', (c) => c.createModel({ name: 'm', datasource: 'd', measures: [] })],
    ['updateModel', (c) => c.updateModel('orders', { description: 'x' })],
    ['deleteModel', (c) => c.deleteModel('orders')],
    ['listDataSources', (c) => c.listDataSources()],
    ['getDataSource', (c) => c.getDataSource('pg')],
    ['createDataSource', (c) =>
      c.createDataSource({ name: 'd', driver: 'postgres', connection_string: 'postgres://localhost/x' })],
    ['updateDataSource', (c) => c.updateDataSource('d', { pool_size: 4 })],
    ['deleteDataSource', (c) => c.deleteDataSource('d')],
    ['query', (c) =>
      c.query({
        name: 'orders',
        measures: [{ formula: { expression: 'revenue:sum' }, aggregation: 'SUM' }],
        dimensions: [{ name: 'status' }],
        filters: [{ field: 'status', operator: 'EQ', value: 'completed' }],
        order: [{ field: 'revenue:sum', descending: true }],
        limit: 10,
      })],
    ['dryRun', (c) => c.dryRun({ name: 'orders', measures: [] })],
    ['multiStageQuery', (c) => c.multiStageQuery({ stages: [{ name: 'orders' }] })],
    ['ingestModels', (c) => c.ingestModels('pg')],
    ['search', (c) => c.search('revenue')],
    ['listMemories', (c) => c.listMemories({ limit: 5 })],
    ['getMemory', (c) => c.getMemory('m1')],
    ['saveMemory', (c) => c.saveMemory({ learning: 'revenue spikes', linkedEntities: ['revenue:sum'] })],
    ['forgetMemory', (c) => c.forgetMemory('m1')],
  ];

  it.each(cases)('%s sends a valid document', async (_name, invoke) => {
    assertValid(_name, await captureDocument(invoke));
  });
});

describe('setHeaders', () => {
  it('replaces headers for subsequent requests', async () => {
    const client = createClient({ url: 'http://unused.invalid/graphql', headers: { 'X-API-Key': 'one' } });
    expect(client.getHeaders()['X-API-Key']).toBe('one');

    client.setHeaders({ Authorization: 'Bearer two' });
    expect(client.getHeaders()['Authorization']).toBe('Bearer two');
    expect(client.getHeaders()['X-API-Key']).toBeUndefined();
  });

  it('does not mutate the config object it was given', () => {
    const config = { url: 'http://unused.invalid/graphql', headers: { 'X-API-Key': 'one' } };
    const client = createClient(config);
    client.setHeaders({ 'X-API-Key': 'two' });
    expect(config.headers['X-API-Key']).toBe('one');
  });
});
