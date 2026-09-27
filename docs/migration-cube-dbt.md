# Migrating from Cube or dbt

Both importers produce GraphNight models you can read, review and edit. Neither
is a blind conversion, and the difference in how much survives is worth
understanding before you start.

| | Cube | dbt |
|---|---|---|
| Source | schema file (YAML) | `target/manifest.json` |
| Measures | carried over | **must be declared** |
| Dimensions | carried over | from columns |
| Joins | carried over | not inferred |
| Time dimensions | carried over | inferred from column names/types |
| Fidelity | close to 1:1 | columns only |

## Cube

Cube is a semantic layer, so the mapping is nearly one-to-one and most schemas
convert without editing:

```bash
# Review first — prints YAML, writes nothing
graphnight import cube --file cube/schema.yml --datasource warehouse

# Then store it
graphnight import cube --file cube/schema.yml --datasource warehouse --apply
```

What maps directly:

- `measures[].sql: sum(amount)` → aggregation `sum` over `amount`. The function
  name has to be a plain identifier, so `sum`, `avg`, `count_distinct` and
  warehouse-specific functions like `percentile_cont` all work.
- `measures[].type: count` → a count measure.
- `dimensions[]` → dimensions; `type: time` → time dimensions at day grain.
- `joins[].sql: "{CUBE}.customer_id = {customers}.id"` → the column pair, with
  `many_to_one` becoming a left join so your fact grain is preserved.
- `sql_table` → the model SQL override.

Two things are reported rather than guessed:

- A measure whose SQL is not a plain column (`CASE WHEN … END`, say) imports as
  a field name and warns. The generator quotes every field name, so this cannot
  become injected SQL — but it will fail at query time unless a column of that
  name exists.
- Dimensions that differ only in case warn. Warehouses disagree about whether
  `userId` and `userid` are the same column, and this importer will not pick for
  you.

## dbt

dbt transforms data; it does not define metrics. There is no honest way to infer
that `amount_usd` should be summed rather than averaged, and guessing would
produce a semantic layer that quietly answers the wrong question.

So dbt import gives you **columns**, and you declare the measures:

```bash
graphnight import dbt --file target/manifest.json --datasource warehouse
```

Every model without declared measures imports as dimensions-only and says so.
To add measures, annotate the dbt model:

```yaml
# models/orders.yml
version: 2

models:
  - name: fct_orders
    description: One row per order
    config:
      meta:
        graphnight_measures:
          - name: revenue
            column: amount_usd
            aggregation: sum
            label: Revenue
          - name: order_count
            column: "*"
            aggregation: count
          - name: aov
            column: amount_usd
            aggregation: avg
            label: Average order value
```

Then re-run the import and the measures appear. The `meta` block lives in your
dbt project, so it is version-controlled and reviewable like everything else.

### Which dbt models are skipped

- `materialized: ephemeral` — these are CTEs, not tables. They print as
  `skipped:` so you can see what was left out.
- `config.enabled: false`
- Anything whose `resource_type` is not `model` (tests, seeds, snapshots,
  exposures).

### Time dimensions

A column becomes a time dimension when its name ends in `_at`, `_date`, `_time`,
`_ts`, `_timestamp`, `_day`, `_month` or `_year`, or is one of `date`, `time`,
`timestamp`, `day`, `month`, `year`, `created`, `updated`; or when dbt records
its type as a date/timestamp.

A wrong guess is harmless — a time dimension still groups correctly, the query
just cannot apply date truncation to something that is not a date. Edit
`granularity` after import if the default of `day` is wrong for your grain.

### Joins

Not inferred. dbt knows the dependency graph, but a dependency is not a join
condition, and guessing `customer_id = id` from two similarly named columns
produces wrong numbers rather than an error. Add joins to the model after
import:

```yaml
joins:
  - name: customers
    model: dim_customers
    join_type: left
    on:
      - [customer_id, id]
```

## After importing

Imported models are ordinary models. Review them before trusting them:

```bash
# What came across
graphnight model list

# Check the SQL a query actually produces
graphnight sql --file query.json
```

Two things are worth checking on any import:

1. **Measure semantics.** Confirm each aggregation matches the business
   definition. This is the part an importer cannot verify.
2. **Row counts against the source.** A model that returns plausible-looking
   numbers with the wrong grain is the failure mode that matters, and it will
   not raise an error.

## Round trip

The importers are one-way. GraphNight models are not exported back to Cube or
dbt, so keep the converted YAML under version control and edit it there — it is
a normal model file, and `graphnight model create --file` takes it directly.

## Troubleshooting

**"not a readable Cube schema"** — the file is not the Cube schema YAML. In a JS
Cube project the schema lives in JavaScript, which this importer does not
execute. Export the schema from the Cube API, or hand-write the YAML; the
importer only needs `cubes[].name`, `measures`, `dimensions` and `joins`.

**"not a readable dbt manifest.json"** — run `dbt compile` first. The manifest
is a build artifact and is not in version control.

**A model imports with no measures** — expected for dbt without `meta`, and for
any Cube model whose measures use a SQL form this importer cannot resolve. The
warning names the model; fix the source and re-import.
