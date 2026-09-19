# Concepts

## Model

A semantic table (or SQL view) with measures, dimensions, time dimensions, and optional joins. Stored in YAML (`models.yaml`) or via GraphQL `createModel`.

## Measure

An aggregation over a field or formula:

- Structured: expression `amount_usd` + aggregation `sum`
- Shorthand formula string: `revenue:sum`, `*:count`
- Functions: `ratio(revenue:sum, cost:sum)`, `time_shift(revenue:sum, -1, 'month')`

Invalid aggregations (e.g. `revenue:nope`) fail at SQL generation time.

## Dimension / time dimension

Group-by fields. Time dimensions use `DATE_TRUNC` with a granularity (`day`, `month`, …).

## Join

Declared on a model. The SQL generator uses the join graph walker to emit `JOIN` clauses for declared targets (multi-hop paths when models are registered together).

## Query

JSON or GraphQL input selecting a model (`name` or `sourceModel`), measures, dimensions, filters, limit. **Dry-run** returns SQL only.

## Unsupported in alpha

| Feature | Status |
|---------|--------|
| `ingestModels` (DB introspection) | Error — use YAML / `createModel` |
| `multiStageQuery` DAG | Error — not implemented |
| Live subscriptions | Stub / empty streams |
| RLS / forced filters on request path | Library only — not wired |
| Result / plan cache | Missing |
| Python on PyPI | Experimental bindings only |
