//! Importer tests.
//!
//! Each case pins a decision that is easy to get wrong later: that a dbt model
//! with no declared measures warns instead of guessing, that an unknown Cube
//! aggregation is dropped rather than interpolated into SQL, and that import
//! order is stable.

use graphnight_core::importers::{import_cube_str, import_dbt_str};
use graphnight_core::models::AggregationType;

const CUBE_SCHEMA: &str = r#"
cubes:
  - name: orders
    description: One row per order
    sql_table: public.orders
    measures:
      - name: total_amount
        sql: sum(amount_usd)
      - name: order_count
        type: count
    dimensions:
      - name: status
      - name: customer_id
      - name: created_at
        type: time
    joins:
      - name: customers
        sql: "{CUBE}.customer_id = {customers}.id"
        relationship: many_to_one

  - name: customers
    description: One row per customer
    measures:
      - name: lifetime_value
        sql: "sum(total_spend)"
    dimensions:
      - name: id
      - name: segment
"#;

fn cube_report() -> graphnight_core::importers::ImportReport {
    import_cube_str(CUBE_SCHEMA, "warehouse").expect("cube schema should import")
}

#[test]
fn cube_measures_split_into_aggregation_and_formula() {
    let report = cube_report();
    let orders = report
        .models
        .iter()
        .find(|m| m.name == "orders")
        .expect("orders cube should import");

    let total = orders
        .measures
        .iter()
        .find(|m| m.formula.label.as_deref() == Some("total_amount"))
        .expect("total_amount measure should import");
    // `sum(amount_usd)` becomes aggregation=sum, expression=amount_usd — the
    // shape GraphNight needs to generate SQL.
    assert_eq!(total.aggregation, AggregationType::Sum);
    assert_eq!(total.formula.expression, "amount_usd");
}

#[test]
fn cube_count_type_becomes_a_count_measure() {
    let orders = cube_report()
        .models
        .into_iter()
        .find(|m| m.name == "orders")
        .unwrap();
    let count = orders
        .measures
        .iter()
        .find(|m| m.aggregation == AggregationType::Count)
        .expect("a count measure should be produced from `type: count`");
    assert_eq!(count.formula.expression, "*");
}

#[test]
fn cube_time_dimension_becomes_a_time_dimension() {
    let orders = cube_report()
        .models
        .into_iter()
        .find(|m| m.name == "orders")
        .unwrap();
    assert!(
        orders
            .time_dimensions
            .iter()
            .any(|t| t.dimension == "created_at"),
        "a `type: time` dimension should not land in plain dimensions"
    );
    assert!(
        !orders.dimensions.iter().any(|d| d.name == "created_at"),
        "a time dimension must not also appear as a plain dimension"
    );
}

#[test]
fn cube_join_becomes_a_column_pair() {
    let orders = cube_report()
        .models
        .into_iter()
        .find(|m| m.name == "orders")
        .unwrap();
    let join = orders.joins.first().expect("join should import");
    assert_eq!(join.name, "customers");
    assert_eq!(join.on, vec![("customer_id".to_string(), "id".to_string())]);
    // many_to_one from the fact side is a left join: it keeps the fact grain.
    assert_eq!(join.join_type, graphnight_core::models::JoinType::Left);
}

#[test]
fn cube_sql_table_becomes_the_model_sql() {
    let orders = cube_report()
        .models
        .into_iter()
        .find(|m| m.name == "orders")
        .unwrap();
    assert_eq!(orders.sql.as_deref(), Some("public.orders"));
}

#[test]
fn cube_custom_aggregation_functions_are_allowed() {
    // A plain function name is a legitimate GraphNight aggregation, so
    // warehouse-specific measures survive the migration.
    let schema = r#"
cubes:
  - name: t
    measures:
      - name: p50
        sql: percentile_cont(amount, 0.5)
    dimensions:
      - name: id
"#;
    let report = import_cube_str(schema, "warehouse").unwrap();
    let model = &report.models[0];
    assert_eq!(model.measures.len(), 1);
    assert_eq!(
        model.measures[0].aggregation,
        AggregationType::Custom("percentile_cont".to_string())
    );
    assert_eq!(model.measures[0].formula.expression, "amount, 0.5");
}

#[test]
fn cube_measure_pointing_at_an_expression_is_flagged() {
    // Injection is not the risk here: the generator quotes every field name, so
    // this becomes a missing column and an error, not executed SQL. The useful
    // behaviour is to warn, because the measure will not work as written.
    let schema = r#"
cubes:
  - name: t
    measures:
      - name: odd
        sql: "sum(x)); DROP TABLE users; -- ("
    dimensions:
      - name: id
"#;
    let report = import_cube_str(schema, "warehouse").unwrap();
    let model = &report.models[0];
    assert!(
        !model.measures.is_empty(),
        "the measure should still import; the generator quotes the field"
    );
    assert!(
        report
            .warnings
            .iter()
            .any(|w| w.contains("not a plain column reference")),
        "an expression that is not a column should be flagged: {:?}",
        report.warnings
    );
}

#[test]
fn cube_aggregation_that_is_not_an_identifier_is_dropped() {
    // The aggregation becomes a function name in generated SQL, so unlike the
    // expression it *is* a boundary. It has to be a plain identifier.
    let schema = r#"
cubes:
  - name: t
    measures:
      - name: evil
        sql: "sum amount"
    dimensions:
      - name: id
"#;
    let report = import_cube_str(schema, "warehouse").unwrap();
    assert!(report.models[0].measures.is_empty());
    assert!(report.warnings.iter().any(|w| w.contains("evil")));
}

#[test]
fn cube_measure_without_a_usable_aggregation_warns() {
    let schema = r#"
cubes:
  - name: t
    measures:
      - name: ambiguous
        sql: some_expression
    dimensions:
      - name: id
"#;
    let report = import_cube_str(schema, "warehouse").unwrap();
    assert!(report.models[0].measures.is_empty());
    assert!(report.warnings.iter().any(|w| w.contains("ambiguous")));
}

#[test]
fn cube_model_without_measures_is_imported_but_flagged() {
    let schema = r#"
cubes:
  - name: lookup
    dimensions:
      - name: id
"#;
    let report = import_cube_str(schema, "warehouse").unwrap();
    assert_eq!(report.models.len(), 1);
    assert!(report.warnings.iter().any(|w| w.contains("no measures")));
}

#[test]
fn cube_sql_dimension_warns_that_the_expression_was_dropped() {
    let schema = r#"
cubes:
  - name: t
    dimensions:
      - name: bucket
        sql: "CASE WHEN x > 1 THEN 'a' ELSE 'b' END"
"#;
    let report = import_cube_str(schema, "warehouse").unwrap();
    assert_eq!(report.models[0].dimensions[0].name, "bucket");
    assert!(
        report.warnings.iter().any(|w| w.contains("SQL expression")),
        "silently dropping a SQL expression would change query results: {:?}",
        report.warnings
    );
}

#[test]
fn case_only_colliding_columns_are_reported() {
    let schema = r#"
cubes:
  - name: t
    dimensions:
      - name: UserId
      - name: userid
"#;
    let report = import_cube_str(schema, "warehouse").unwrap();
    assert!(report
        .warnings
        .iter()
        .any(|w| w.contains("differ only in case")));
}

#[test]
fn unparseable_cube_schema_is_an_error_not_a_panic() {
    let err = import_cube_str("cubes: [oops", "warehouse").unwrap_err();
    assert!(
        err.to_string().contains("Cube schema"),
        "error should name the file type: {err}"
    );
}

const DBT_MANIFEST: &str = r#"{
  "nodes": {
    "model.analytics.orders": {
      "name": "orders",
      "resource_type": "model",
      "description": "Cleaned orders",
      "config": {"materialized": "table"},
      "columns": {
        "amount_usd": {"name": "amount_usd", "data_type": "numeric"},
        "status": {"name": "status", "data_type": "text"},
        "created_at": {"name": "created_at", "data_type": "timestamp"}
      },
      "meta": {
        "graphnight_measures": [
          {"name": "revenue", "column": "amount_usd", "aggregation": "sum", "label": "Revenue"}
        ]
      }
    },
    "model.analytics.stg_orders": {
      "name": "stg_orders",
      "resource_type": "model",
      "config": {"materialized": "ephemeral"},
      "columns": {"id": {"name": "id"}}
    },
    "model.analytics.off": {
      "name": "off",
      "resource_type": "model",
      "config": {"enabled": false},
      "columns": {"id": {"name": "id"}}
    },
    "test.analytics.not_null_orders": {
      "name": "not_null_orders",
      "resource_type": "test"
    }
  }
}"#;

fn dbt_report() -> graphnight_core::importers::ImportReport {
    import_dbt_str(DBT_MANIFEST, "warehouse").expect("manifest should import")
}

#[test]
fn dbt_columns_become_dimensions() {
    let orders = dbt_report()
        .models
        .into_iter()
        .find(|m| m.name == "orders")
        .unwrap();
    let names: Vec<&str> = orders.dimensions.iter().map(|d| d.name.as_str()).collect();
    assert!(names.contains(&"status"));
    assert!(names.contains(&"amount_usd"));
}

#[test]
fn dbt_timestamp_columns_become_time_dimensions() {
    let orders = dbt_report()
        .models
        .into_iter()
        .find(|m| m.name == "orders")
        .unwrap();
    assert!(
        orders
            .time_dimensions
            .iter()
            .any(|t| t.dimension == "created_at"),
        "a timestamp column should become a time dimension"
    );
    assert!(!orders.dimensions.iter().any(|d| d.name == "created_at"));
}

#[test]
fn dbt_declared_measures_are_imported() {
    let orders = dbt_report()
        .models
        .into_iter()
        .find(|m| m.name == "orders")
        .unwrap();
    let measure = orders
        .measures
        .iter()
        .find(|m| m.formula.label.as_deref() == Some("Revenue"))
        .expect("declared measure should import");
    assert_eq!(measure.aggregation, AggregationType::Sum);
    assert_eq!(measure.formula.expression, "amount_usd");
}

#[test]
fn dbt_ephemeral_and_disabled_nodes_are_skipped() {
    let report = dbt_report();
    assert!(
        !report.models.iter().any(|m| m.name == "stg_orders"),
        "an ephemeral model is a CTE, not a table"
    );
    assert!(report.skipped.iter().any(|s| s.contains("stg_orders")));
    assert!(report.skipped.iter().any(|s| s.contains("off")));
}

#[test]
fn dbt_tests_are_not_models() {
    let report = dbt_report();
    assert!(!report.models.iter().any(|m| m.name == "not_null_orders"));
}

#[test]
fn dbt_model_without_measures_warns_instead_of_guessing() {
    // The central design decision. Inventing `sum(amount_usd)` here would
    // produce a semantic layer that silently answers the wrong question.
    let manifest = r#"{"nodes": {"model.p.orders": {
        "name": "orders", "resource_type": "model",
        "columns": {"amount_usd": {"name": "amount_usd"}}
    }}}"#;
    let report = import_dbt_str(manifest, "warehouse").unwrap();
    assert!(report.models[0].measures.is_empty());
    assert!(
        report
            .warnings
            .iter()
            .any(|w| w.contains("dimensions-only")),
        "a measure-less dbt model should be flagged: {:?}",
        report.warnings
    );
}

#[test]
fn dbt_measure_with_a_bad_aggregation_is_skipped_with_a_warning() {
    let manifest = r#"{"nodes": {"model.p.orders": {
        "name": "orders", "resource_type": "model",
        "columns": {"amount_usd": {"name": "amount_usd"}},
        "meta": {"graphnight_measures": [
            {"name": "bad", "column": "amount_usd", "aggregation": "sum(x); DROP TABLE t"}
        ]}
    }}}"#;
    let report = import_dbt_str(manifest, "warehouse").unwrap();
    assert!(report.models[0].measures.is_empty());
    assert!(report.warnings.iter().any(|w| w.contains("bad")));
}

#[test]
fn dbt_measure_missing_fields_is_skipped_with_a_warning() {
    let manifest = r#"{"nodes": {"model.p.orders": {
        "name": "orders", "resource_type": "model",
        "columns": {"amount_usd": {"name": "amount_usd"}},
        "meta": {"graphnight_measures": [{"name": "incomplete"}]}
    }}}"#;
    let report = import_dbt_str(manifest, "warehouse").unwrap();
    assert!(report.models[0].measures.is_empty());
    assert!(report
        .warnings
        .iter()
        .any(|w| w.contains("graphnight_measure")));
}

#[test]
fn dbt_import_order_is_stable() {
    // A manifest is a JSON object, so iteration order varies. An import that
    // reshuffles produces a diff on every run.
    let first: Vec<String> = dbt_report().models.into_iter().map(|m| m.name).collect();
    let second: Vec<String> = dbt_report().models.into_iter().map(|m| m.name).collect();
    assert_eq!(first, second);
}

#[test]
fn unparseable_manifest_is_an_error_not_a_panic() {
    let err = import_dbt_str("{not json", "warehouse").unwrap_err();
    assert!(
        err.to_string().contains("manifest.json"),
        "error should name the file: {err}"
    );
}
