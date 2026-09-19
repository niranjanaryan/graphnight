use criterion::{black_box, criterion_group, criterion_main, Criterion};
use graphnight_core::models::*;
use graphnight_sql::dialects::PostgresDialect;
use graphnight_sql::generator::SqlGenerator;

fn sample_models() -> Vec<Model> {
    let orders = Model {
        name: "orders".to_string(),
        datasource: "postgres".to_string(),
        description: None,
        measures: vec![Measure::simple("amount_usd", AggregationType::Sum)],
        dimensions: vec![Dimension::new("status"), Dimension::new("customer_id")],
        time_dimensions: vec![TimeDimension::new("created_at", TimeGranularity::Day)],
        joins: vec![Join {
            name: "customers".to_string(),
            model: "customers".to_string(),
            join_type: JoinType::Left,
            on: vec![("customer_id".to_string(), "id".to_string())],
            alias: Some("cust".to_string()),
        }],
        sql: None,
        meta: Default::default(),
    };
    let customers = Model {
        name: "customers".to_string(),
        datasource: "postgres".to_string(),
        description: None,
        measures: vec![],
        dimensions: vec![Dimension::new("id"), Dimension::new("segment")],
        time_dimensions: vec![],
        joins: vec![],
        sql: None,
        meta: Default::default(),
    };
    vec![orders, customers]
}

fn bench_generate(c: &mut Criterion) {
    let generator = SqlGenerator::new(Box::new(PostgresDialect)).with_models(sample_models());
    let query = Query::new()
        .with_name("orders")
        .add_measure(Measure::simple("amount_usd", AggregationType::Sum))
        .add_measure(Measure::new(
            Formula::new("ratio(amount_usd:sum, *:count)"),
            AggregationType::Avg,
        ))
        .add_dimension(Dimension::new("status"))
        .with_limit(100);

    c.bench_function("sql_generate_orders", |b| {
        b.iter(|| {
            let sql = generator.generate(black_box(&query)).unwrap();
            black_box(sql);
        })
    });
}

fn bench_plan_cache(c: &mut Criterion) {
    use graphnight_sql::executor::{ConnectionManager, QueryExecutor};
    use graphnight_sql::SqlEngine;
    use std::sync::Arc;

    let dialect = Box::new(PostgresDialect);
    let executor = Arc::new(QueryExecutor::new(Arc::new(ConnectionManager::new())));
    let engine = SqlEngine::new(dialect, executor)
        .unwrap()
        .with_models(sample_models());
    let query = Query::new()
        .with_name("orders")
        .add_measure(Measure::simple("amount_usd", AggregationType::Sum))
        .add_dimension(Dimension::new("status"));

    // Warm cache
    let _ = engine.generate_sql(&query).unwrap();

    c.bench_function("sql_plan_cache_hit", |b| {
        b.iter(|| {
            let sql = engine.generate_sql(black_box(&query)).unwrap();
            black_box(sql);
        })
    });
}

criterion_group!(benches, bench_generate, bench_plan_cache);
criterion_main!(benches);
