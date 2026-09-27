use criterion::{black_box, criterion_group, criterion_main, Criterion};
use graphnight_core::models::*;
use graphnight_core::security::{PolicyEnforcer, SessionPolicy};
use graphnight_sql::dialects::PostgresDialect;
use graphnight_sql::generator::SqlGenerator;

fn base_models() -> Vec<Model> {
    vec![Model {
        name: "orders".to_string(),
        datasource: "postgres".to_string(),
        description: None,
        measures: vec![Measure::simple("amount_usd", AggregationType::Sum)],
        dimensions: vec![
            Dimension::new("status"),
            Dimension::new("customer_id"),
            Dimension::new("tenant_id"),
        ],
        time_dimensions: vec![TimeDimension::new("created_at", TimeGranularity::Day)],
        joins: vec![],
        sql: None,
        meta: Default::default(),
    }]
}

fn base_query() -> Query {
    Query::new()
        .with_name("orders")
        .add_measure(Measure::simple("amount_usd", AggregationType::Sum))
        .add_dimension(Dimension::new("status"))
        .add_filter(Filter::new(
            "status",
            FilterOperator::Eq,
            serde_json::json!("completed"),
        ))
        .with_limit(100)
}

/// Enforce a representative governed-at-compile policy: an allow-list of
/// `n` models plus a tenant forced filter, RLS predicate, and row cap.
fn enforce(enforcer: &PolicyEnforcer) {
    let mut q = base_query();
    enforcer.check_model_access("orders").unwrap();
    enforcer.check_datasource_access("postgres").unwrap();
    enforcer.apply_forced_filters(&mut q);
    enforcer.apply_rls(&mut q);
    enforcer.enforce_row_limit(&mut q);
    black_box(q.limit);
}

fn group_allowlist_scale(c: &mut Criterion) {
    for n in [1usize, 100, 1000, 10_000] {
        // `orders` matches last for a worst-case linear scan.
        let mut allow: Vec<String> = (0..n.saturating_sub(1))
            .map(|i| format!("model_{i}"))
            .collect();
        allow.push("orders".to_string());
        let policy = SessionPolicy::new()
            .with_allowed_models(allow)
            .with_forced_filter(Filter::new(
                "tenant_id",
                FilterOperator::Eq,
                serde_json::json!("tenant-7"),
            ))
            .with_row_filter(Filter::new(
                "status",
                FilterOperator::Neq,
                serde_json::json!("internal"),
            ))
            .with_max_rows(500);
        let enforcer = PolicyEnforcer::new(policy.clone());

        c.bench_function(&format!("e3_enforce_allowlist_{n}"), |b| {
            b.iter(|| enforce(&enforcer))
        });
    }
}

/// Headline E3 number: fraction of end-to-end plan compilation that policy
/// attachment adds, at a realistic 1000-entry allow-list.
fn group_compile_with_policy(c: &mut Criterion) {
    let generator = SqlGenerator::new(Box::new(PostgresDialect)).with_models(base_models());
    let query = base_query();

    let mut allow: Vec<String> = (0..999).map(|i| format!("model_{i}")).collect();
    allow.push("orders".to_string());
    let policy = SessionPolicy::new()
        .with_allowed_models(allow)
        .with_forced_filter(Filter::new(
            "tenant_id",
            FilterOperator::Eq,
            serde_json::json!("tenant-7"),
        ))
        .with_row_filter(Filter::new(
            "status",
            FilterOperator::Neq,
            serde_json::json!("internal"),
        ))
        .with_max_rows(500);
    let enforcer = PolicyEnforcer::new(policy);

    c.bench_function("e3_compile_governed", |b| {
        b.iter(|| {
            let mut q = base_query();
            enforcer.check_model_access("orders").unwrap();
            enforcer.check_datasource_access("postgres").unwrap();
            enforcer.apply_forced_filters(&mut q);
            enforcer.apply_rls(&mut q);
            enforcer.enforce_row_limit(&mut q);
            let sql = generator.generate(black_box(&q)).unwrap();
            black_box(sql);
        })
    });

    c.bench_function("e3_compile_ungoverned", |b| {
        b.iter(|| {
            let sql = generator.generate(black_box(&query)).unwrap();
            black_box(sql);
        })
    });
}

criterion_group!(
    policy_benches,
    group_allowlist_scale,
    group_compile_with_policy
);
criterion_main!(policy_benches);
