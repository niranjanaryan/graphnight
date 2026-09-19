use graphnight_core::models::*;
use graphnight_sql::dialects::*;
use graphnight_sql::generator::SqlGenerator;
use pretty_assertions::assert_eq;
use serde_json::json;

fn create_test_model() -> Model {
    Model {
        name: "orders".to_string(),
        datasource: "postgres".to_string(),
        description: None,
        measures: vec![
            Measure::simple("revenue", AggregationType::Sum),
            Measure::simple("order_count", AggregationType::Count),
        ],
        dimensions: vec![Dimension::new("status"), Dimension::new("store_id")],
        time_dimensions: vec![TimeDimension::new("created_at", TimeGranularity::Day)],
        joins: vec![],
        sql: None,
        meta: Default::default(),
    }
}

#[test]
fn test_postgres_dialect_basic_select() {
    let model = create_test_model();
    let dialect = PostgresDialect;
    let generator = SqlGenerator::new(Box::new(dialect)).with_models(vec![model]);

    let query = Query::new()
        .with_name("orders")
        .add_measure(Measure::simple("revenue", AggregationType::Sum))
        .add_dimension(Dimension::new("status"))
        .add_filter(Filter::new(
            "status",
            FilterOperator::Eq,
            json!("completed"),
        ))
        .add_order(OrderBy::desc("revenue"))
        .with_limit(10);

    let sql = generator.generate(&query).unwrap();
    println!("PostgreSQL: {}", sql);

    assert!(sql.contains("SELECT"));
    assert!(sql.contains("FROM"));
    assert!(sql.contains("\"orders\""));
    assert!(sql.contains("\"revenue\""));
    assert!(sql.contains("SUM"));
    assert!(sql.contains("WHERE"));
    assert!(sql.contains("status"));
    assert!(sql.contains("'completed'"));
    assert!(sql.contains("GROUP BY"));
    assert!(sql.contains("ORDER BY"));
    assert!(sql.contains("LIMIT 10"));
}

#[test]
fn test_mysql_dialect_basic_select() {
    let model = create_test_model();
    let dialect = MySqlDialect;
    let generator = SqlGenerator::new(Box::new(dialect)).with_models(vec![model]);

    let query = Query::new()
        .with_name("orders")
        .add_measure(Measure::simple("revenue", AggregationType::Sum))
        .add_dimension(Dimension::new("status"))
        .with_limit(10);

    let sql = generator.generate(&query).unwrap();
    println!("MySQL: {}", sql);

    assert!(sql.contains("SELECT"));
    assert!(sql.contains("`orders`"));
    assert!(sql.contains("SUM"));
    assert!(sql.contains("LIMIT 10"));
}

#[test]
fn test_sqlite_dialect_basic_select() {
    let model = create_test_model();
    let dialect = SqliteDialect;
    let generator = SqlGenerator::new(Box::new(dialect)).with_models(vec![model]);

    let query = Query::new()
        .with_name("orders")
        .add_measure(Measure::simple("revenue", AggregationType::Sum))
        .add_dimension(Dimension::new("status"))
        .with_limit(10);

    let sql = generator.generate(&query).unwrap();
    println!("SQLite: {}", sql);

    assert!(sql.contains("SELECT"));
    assert!(sql.contains("\"orders\""));
    assert!(sql.contains("SUM"));
    assert!(sql.contains("LIMIT 10"));
}

#[test]
fn test_duckdb_dialect_basic_select() {
    let model = create_test_model();
    let dialect = DuckDbDialect;
    let generator = SqlGenerator::new(Box::new(dialect)).with_models(vec![model]);

    let query = Query::new()
        .with_name("orders")
        .add_measure(Measure::simple("revenue", AggregationType::Sum))
        .add_dimension(Dimension::new("status"))
        .with_limit(10);

    let sql = generator.generate(&query).unwrap();
    println!("DuckDB: {}", sql);

    assert!(sql.contains("SELECT"));
    assert!(sql.contains("\"orders\""));
    assert!(sql.contains("SUM"));
    assert!(sql.contains("LIMIT 10"));
}

#[test]
fn test_time_dimension_in_query() {
    let model = create_test_model();
    let dialect = PostgresDialect;
    let generator = SqlGenerator::new(Box::new(dialect)).with_models(vec![model]);

    let query = Query::new()
        .with_name("orders")
        .add_measure(Measure::simple("revenue", AggregationType::Sum))
        .add_time_dimension(TimeDimension::new("created_at", TimeGranularity::Month))
        .add_filter(Filter::new(
            "created_at",
            FilterOperator::Gte,
            json!("2024-01-01"),
        ))
        .with_limit(100);

    let sql = generator.generate(&query).unwrap();
    println!("Time dimension SQL: {}", sql);

    assert!(sql.contains("DATE_TRUNC"));
    assert!(sql.contains("month"));
    assert!(sql.contains("GROUP BY"));
    assert!(sql.contains("DATE_TRUNC"));
}

#[test]
fn test_multiple_measures() {
    let model = create_test_model();
    let dialect = PostgresDialect;
    let generator = SqlGenerator::new(Box::new(dialect)).with_models(vec![model]);

    let query = Query::new()
        .with_name("orders")
        .add_measure(Measure::simple("revenue", AggregationType::Sum))
        .add_measure(Measure::simple("order_count", AggregationType::Count))
        .add_dimension(Dimension::new("status"));

    let sql = generator.generate(&query).unwrap();
    println!("Multiple measures: {}", sql);

    assert!(sql.contains("SUM"));
    assert!(sql.contains("COUNT"));
}

#[test]
fn test_all_aggregation_types() {
    let model = create_test_model();
    let dialect = PostgresDialect;
    let generator = SqlGenerator::new(Box::new(dialect)).with_models(vec![model]);

    let aggs = vec![
        (AggregationType::Sum, "SUM"),
        (AggregationType::Avg, "AVG"),
        (AggregationType::Count, "COUNT"),
        (AggregationType::Min, "MIN"),
        (AggregationType::Max, "MAX"),
    ];

    for (agg, expected) in aggs {
        let query = Query::new()
            .with_name("orders")
            .add_measure(Measure::simple("revenue", agg.clone()));

        let sql = generator.generate(&query).unwrap();
        assert!(
            sql.contains(expected),
            "Expected {} in SQL for {:?}",
            expected,
            agg
        );
    }
}

#[test]
fn test_count_distinct() {
    let model = create_test_model();
    let dialect = PostgresDialect;
    let generator = SqlGenerator::new(Box::new(dialect)).with_models(vec![model]);

    let query = Query::new()
        .with_name("orders")
        .add_measure(Measure::simple(
            "customer_id",
            AggregationType::CountDistinct,
        ));

    let sql = generator.generate(&query).unwrap();
    println!("Count distinct: {}", sql);

    assert!(sql.contains("COUNT(DISTINCT"));
}

#[test]
fn test_filter_operators() {
    let model = create_test_model();
    let dialect = PostgresDialect;
    let generator = SqlGenerator::new(Box::new(dialect)).with_models(vec![model]);

    let filters = vec![
        (FilterOperator::Eq, "="),
        (FilterOperator::Neq, "!="),
        (FilterOperator::Gt, ">"),
        (FilterOperator::Gte, ">="),
        (FilterOperator::Lt, "<"),
        (FilterOperator::Lte, "<="),
        (FilterOperator::Like, "LIKE"),
        (FilterOperator::ILike, "ILIKE"),
        (FilterOperator::IsNull, "IS NULL"),
        (FilterOperator::IsNotNull, "IS NOT NULL"),
    ];

    for (op, expected) in filters {
        let query = Query::new()
            .with_name("orders")
            .add_measure(Measure::simple("revenue", AggregationType::Sum))
            .add_filter(Filter::new("status", op.clone(), json!("test")));

        let sql = generator.generate(&query).unwrap();
        assert!(
            sql.contains(expected),
            "Expected {} in SQL for {:?}",
            expected,
            op
        );
    }
}

#[test]
fn test_between_filter() {
    let model = create_test_model();
    let dialect = PostgresDialect;
    let generator = SqlGenerator::new(Box::new(dialect)).with_models(vec![model]);

    let query = Query::new()
        .with_name("orders")
        .add_measure(Measure::simple("revenue", AggregationType::Sum))
        .add_filter(Filter::new(
            "created_at",
            FilterOperator::Between,
            json!(["2024-01-01", "2024-12-31"]),
        ));

    let sql = generator.generate(&query).unwrap();
    println!("Between filter: {}", sql);

    assert!(sql.contains("BETWEEN"));
}

#[test]
fn test_in_filter() {
    let model = create_test_model();
    let dialect = PostgresDialect;
    let generator = SqlGenerator::new(Box::new(dialect)).with_models(vec![model]);

    let query = Query::new()
        .with_name("orders")
        .add_measure(Measure::simple("revenue", AggregationType::Sum))
        .add_filter(Filter::new(
            "status",
            FilterOperator::In,
            json!(["completed", "pending"]),
        ));

    let sql = generator.generate(&query).unwrap();
    println!("In filter: {}", sql);

    assert!(sql.contains("IN"));
}

#[test]
fn test_or_condition() {
    let model = create_test_model();
    let dialect = PostgresDialect;
    let generator = SqlGenerator::new(Box::new(dialect)).with_models(vec![model]);

    let query = Query::new()
        .with_name("orders")
        .add_measure(Measure::simple("revenue", AggregationType::Sum))
        .add_filter(Filter::new(
            "status",
            FilterOperator::Eq,
            json!("completed"),
        ))
        .add_filter(Filter::new("status", FilterOperator::Eq, json!("pending")).or());

    let sql = generator.generate(&query).unwrap();
    println!("OR condition: {}", sql);

    assert!(sql.contains("OR"));
}

#[test]
fn test_join_generation() {
    let orders = Model {
        name: "orders".to_string(),
        datasource: "postgres".to_string(),
        description: None,
        measures: vec![Measure::simple("revenue", AggregationType::Sum)],
        dimensions: vec![Dimension::new("status")],
        time_dimensions: vec![],
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
        dimensions: vec![Dimension::new("name")],
        time_dimensions: vec![],
        joins: vec![],
        sql: None,
        meta: Default::default(),
    };

    let dialect = PostgresDialect;
    let generator = SqlGenerator::new(Box::new(dialect)).with_models(vec![orders, customers]);

    let query = Query::new()
        .with_source_model(SourceSpec {
            model: "orders".to_string(),
            datasource: None,
            alias: None,
        })
        .add_measure(Measure::simple("revenue", AggregationType::Sum))
        .add_dimension(Dimension::new("status"))
        .add_dimension(Dimension::new("name")); // From joined model

    let sql = generator.generate(&query).unwrap();
    println!("Join SQL: {}", sql);

    assert!(sql.contains("LEFT JOIN"));
    assert!(sql.contains("customers"));
    assert!(sql.contains("cust"));
}

#[test]
fn test_dialect_quote_ident() {
    let pg = PostgresDialect;
    assert_eq!(pg.quote_ident("test"), "\"test\"");
    assert_eq!(pg.quote_ident("test\"quote"), "\"test\"\"quote\"");

    let mysql = MySqlDialect;
    assert_eq!(mysql.quote_ident("test"), "`test`");
    assert_eq!(mysql.quote_ident("test`quote"), "`test``quote`");

    let sqlite = SqliteDialect;
    assert_eq!(sqlite.quote_ident("test"), "\"test\"");
}

#[test]
fn test_dialect_format_value() {
    let pg = PostgresDialect;
    assert_eq!(pg.format_value(&json!(null)), "NULL");
    assert_eq!(pg.format_value(&json!(true)), "TRUE");
    assert_eq!(pg.format_value(&json!(42)), "42");
    assert_eq!(pg.format_value(&json!("test")), "'test'");
    assert_eq!(pg.format_value(&json!({"a": 1})), "'{\"a\":1}'");
}

#[test]
fn test_dialect_date_trunc() {
    let pg = PostgresDialect;
    assert_eq!(
        pg.date_trunc("day", "created_at"),
        "DATE_TRUNC('day', created_at)"
    );

    let mysql = MySqlDialect;
    assert!(mysql
        .date_trunc("day", "created_at")
        .contains("DATE_FORMAT"));

    let sqlite = SqliteDialect;
    assert!(sqlite.date_trunc("day", "created_at").contains("strftime"));
}

#[test]
fn test_dialect_limit_offset() {
    let pg = PostgresDialect;
    let sql = "SELECT * FROM test";
    assert_eq!(
        pg.limit_offset(sql, Some(10), Some(5)),
        "SELECT * FROM test LIMIT 10 OFFSET 5"
    );
    assert_eq!(
        pg.limit_offset(sql, Some(10), None),
        "SELECT * FROM test LIMIT 10"
    );
    assert_eq!(
        pg.limit_offset(sql, None, Some(5)),
        "SELECT * FROM test OFFSET 5"
    );
}

#[test]
fn test_get_dialect() {
    assert_eq!(get_dialect("postgres").name(), "postgres");
    assert_eq!(get_dialect("POSTGRESQL").name(), "postgres");
    assert_eq!(get_dialect("mysql").name(), "mysql");
    assert_eq!(get_dialect("sqlite").name(), "sqlite");
    assert_eq!(get_dialect("duckdb").name(), "duckdb");
    assert_eq!(get_dialect("unknown").name(), "postgres"); // default
}
