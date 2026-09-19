use graphnight_core::formula::{FormulaParser, FormulaRegistry};
use graphnight_core::join::{JoinGraph, JoinWalker};
use graphnight_core::models::*;
use graphnight_core::security::{masks, AuditEntry, AuditLogger, PolicyEnforcer, SessionPolicy};
use pretty_assertions::assert_eq;
use serde_json::json;

#[test]
fn test_aggregation_type_sql_function() {
    assert_eq!(AggregationType::Sum.sql_function(), "SUM");
    assert_eq!(AggregationType::Avg.sql_function(), "AVG");
    assert_eq!(AggregationType::Count.sql_function(), "COUNT");
    assert_eq!(AggregationType::Min.sql_function(), "MIN");
    assert_eq!(AggregationType::Max.sql_function(), "MAX");
    assert_eq!(
        AggregationType::CountDistinct.sql_function(),
        "COUNT(DISTINCT "
    );
    assert!(AggregationType::CountDistinct.needs_closing_paren());
    assert!(!AggregationType::Sum.needs_closing_paren());
}

#[test]
fn test_time_granularity_date_trunc() {
    assert_eq!(TimeGranularity::Day.date_trunc_unit(), "day");
    assert_eq!(TimeGranularity::Month.date_trunc_unit(), "month");
    assert_eq!(TimeGranularity::Year.date_trunc_unit(), "year");
}

#[test]
fn test_filter_operator_sql() {
    assert_eq!(FilterOperator::Eq.sql_operator(), "=");
    assert_eq!(FilterOperator::Neq.sql_operator(), "!=");
    assert_eq!(FilterOperator::Gt.sql_operator(), ">");
    assert_eq!(FilterOperator::Like.sql_operator(), "LIKE");
    assert!(FilterOperator::Eq.needs_value());
    assert!(!FilterOperator::IsNull.needs_value());
    assert!(FilterOperator::Between.needs_two_values());
    assert!(!FilterOperator::In.needs_two_values());
}

#[test]
fn test_formula_parse_shorthand() {
    let f = Formula::parse_shorthand("revenue:sum");
    assert_eq!(f.expression, "revenue");
    assert_eq!(f.label, Some("sum".to_string()));

    let f = Formula::parse_shorthand("status");
    assert_eq!(f.expression, "status");
    assert_eq!(f.label, None);
}

#[test]
fn test_measure_builder() {
    let m = Measure::simple("revenue", AggregationType::Sum);
    assert_eq!(m.formula.expression, "revenue");
    assert_eq!(m.aggregation, AggregationType::Sum);

    let m = Measure::new(
        Formula::new("amount").with_label("Revenue"),
        AggregationType::Sum,
    );
    assert_eq!(m.label(), "Revenue");
}

#[test]
fn test_dimension_builder() {
    let d = Dimension::new("status");
    assert_eq!(d.name, "status");
    assert_eq!(d.label(), "status");

    let d = Dimension::new("status").with_label("Order Status");
    assert_eq!(d.label(), "Order Status");
}

#[test]
fn test_time_dimension_sql_expression() {
    let td = TimeDimension::new("created_at", TimeGranularity::Day);
    assert_eq!(td.sql_expression(None), "DATE_TRUNC('day', created_at)");
    assert_eq!(
        td.sql_expression(Some("t")),
        "DATE_TRUNC('day', t.created_at)"
    );
}

#[test]
fn test_query_builder() {
    let q = Query::new()
        .with_name("orders")
        .add_measure(Measure::simple("revenue", AggregationType::Sum))
        .add_dimension(Dimension::new("status"))
        .add_time_dimension(TimeDimension::new("created_at", TimeGranularity::Day))
        .add_filter(Filter::new(
            "status",
            FilterOperator::Eq,
            json!("completed"),
        ))
        .add_order(OrderBy::desc("revenue"))
        .with_limit(100);

    assert_eq!(q.name, Some("orders".to_string()));
    assert_eq!(q.measures.len(), 1);
    assert_eq!(q.dimensions.len(), 1);
    assert_eq!(q.time_dimensions.len(), 1);
    assert_eq!(q.filters.len(), 1);
    assert_eq!(q.order.len(), 1);
    assert_eq!(q.limit, Some(100));
}

#[test]
fn test_formula_parser_simple() {
    let parser = FormulaParser::new().unwrap();

    let m = parser.parse_measure("revenue:sum").unwrap();
    assert_eq!(m.formula.expression, "revenue");
    assert_eq!(m.aggregation, AggregationType::Sum);

    let m = parser.parse_measure("orders:count").unwrap();
    assert_eq!(m.aggregation, AggregationType::Count);

    let m = parser.parse_measure("amount:avg").unwrap();
    assert_eq!(m.aggregation, AggregationType::Avg);
}

#[test]
fn test_formula_parser_rejects_unknown_aggregation() {
    let parser = FormulaParser::new().unwrap();
    let err = parser
        .parse_measure("revenue:nope")
        .unwrap_err()
        .to_string();
    assert!(err.contains("unknown aggregation"));
    assert!(parser.parse_measure("").is_err());
    assert!(FormulaParser::needs_parse("revenue:sum"));
    assert!(!FormulaParser::needs_parse("amount_usd"));
}

#[test]
fn test_formula_parser_time_shift() {
    let parser = FormulaParser::new().unwrap();

    let m = parser
        .parse_measure("time_shift(revenue:sum, -1, 'month')")
        .unwrap();
    assert!(m.formula.expression.contains("LAG"));
    assert!(m.formula.label.as_ref().unwrap().contains("shift"));
    assert_eq!(m.aggregation, AggregationType::Sum);
}

#[test]
fn test_formula_parser_ratio() {
    let parser = FormulaParser::new().unwrap();

    let m = parser
        .parse_measure("ratio(revenue:sum, cost:sum)")
        .unwrap();
    assert!(m.formula.expression.contains("/"));
    assert!(m.formula.label.as_ref().unwrap().contains("per"));
    assert_eq!(m.aggregation, AggregationType::Avg);
}

#[test]
fn test_formula_parser_granularity() {
    let parser = FormulaParser::new().unwrap();

    assert_eq!(
        parser.parse_granularity("day").unwrap(),
        TimeGranularity::Day
    );
    assert_eq!(
        parser.parse_granularity("month").unwrap(),
        TimeGranularity::Month
    );
    assert_eq!(
        parser.parse_granularity("year").unwrap(),
        TimeGranularity::Year
    );
    assert!(parser.parse_granularity("invalid").is_err());
}

#[test]
fn test_formula_parser_operator() {
    let parser = FormulaParser::new().unwrap();

    assert_eq!(parser.parse_operator("eq").unwrap(), FilterOperator::Eq);
    assert_eq!(parser.parse_operator(">").unwrap(), FilterOperator::Gt);
    assert_eq!(parser.parse_operator("like").unwrap(), FilterOperator::Like);
    assert_eq!(
        parser.parse_operator("is_null").unwrap(),
        FilterOperator::IsNull
    );
    assert!(parser.parse_operator("invalid").is_err());
}

#[test]
fn test_formula_registry() {
    let mut registry = FormulaRegistry::new().unwrap();

    registry
        .register_str("total_revenue", "revenue:sum")
        .unwrap();
    let m = registry.resolve("total_revenue").unwrap();
    assert_eq!(m.formula.expression, "revenue");

    registry
        .register_str("avg_order", "ratio(revenue:sum, orders:count)")
        .unwrap();
    let m = registry.resolve("avg_order").unwrap();
    assert!(m.formula.expression.contains("/"));

    // Unknown expression gets parsed
    let m = registry.resolve("new_field:sum").unwrap();
    assert_eq!(m.formula.expression, "new_field");
}

#[test]
fn test_join_graph() {
    let orders = Model {
        name: "orders".to_string(),
        datasource: "postgres".to_string(),
        description: None,
        measures: vec![],
        dimensions: vec![],
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
        dimensions: vec![],
        time_dimensions: vec![],
        joins: vec![],
        sql: None,
        meta: Default::default(),
    };

    let graph = JoinGraph::new(vec![orders, customers]);
    let path = graph.find_path("orders", "customers").unwrap();
    assert_eq!(path.len(), 1);
    assert_eq!(path[0].target_model, "customers");
    assert_eq!(path[0].join.join_type, JoinType::Left);
}

#[test]
fn test_join_walker() {
    let orders = Model {
        name: "orders".to_string(),
        datasource: "postgres".to_string(),
        description: None,
        measures: vec![],
        dimensions: vec![],
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
        dimensions: vec![],
        time_dimensions: vec![],
        joins: vec![],
        sql: None,
        meta: Default::default(),
    };

    let walker = JoinWalker::new(vec![orders, customers]);
    let clauses = walker
        .build_join_clauses("orders", &["customers".to_string()], "o")
        .unwrap();
    assert_eq!(clauses.len(), 1);
    assert_eq!(clauses[0].join_type, "LEFT JOIN");
    assert_eq!(clauses[0].table, "customers");
    assert_eq!(clauses[0].alias, "cust");
    assert!(clauses[0].on.contains("customer_id"));
}

#[test]
fn test_session_policy() {
    let policy = SessionPolicy::new()
        .with_forced_filter(Filter::new(
            "tenant_id",
            FilterOperator::Eq,
            json!("tenant_1"),
        ))
        .with_allowed_models(vec!["orders".to_string(), "customers".to_string()])
        .with_max_rows(5000);

    assert_eq!(policy.forced_filters.len(), 1);
    assert_eq!(policy.allowed_models.as_ref().unwrap().len(), 2);
    assert_eq!(policy.max_rows, Some(5000));
}

#[test]
fn test_policy_enforcer() {
    let policy = SessionPolicy::new()
        .with_allowed_models(vec!["orders".to_string()])
        .with_max_rows(100);

    let enforcer = PolicyEnforcer::new(policy);

    // Allowed model
    assert!(enforcer.check_model_access("orders").is_ok());

    // Denied model
    assert!(enforcer.check_model_access("products").is_err());

    // Row limit enforcement
    let mut q = Query::new().with_limit(200);
    enforcer.enforce_row_limit(&mut q);
    assert_eq!(q.limit, Some(100));

    let mut q = Query::new();
    enforcer.enforce_row_limit(&mut q);
    assert_eq!(q.limit, Some(100));
}

#[test]
fn test_builtin_masks() {
    assert_eq!(masks::email_mask("user@example.com"), "u***r@example.com");
    assert_eq!(masks::email_mask("a@b.com"), "***@b.com");
    assert_eq!(masks::phone_mask("1234567890"), "(***) ***-7890");
    assert_eq!(masks::ssn_mask("123-45-6789"), "***-**-****");
    assert_eq!(
        masks::credit_card_mask("1234-5678-9012-3456"),
        "**** **** **** 3456"
    );
    assert!(masks::hash_mask("test").starts_with("hash_"));
    assert_eq!((masks::truncate_mask(3))("hello"), "hel...");
    assert_eq!((masks::truncate_mask(10))("hi"), "hi");
}

#[test]
fn test_audit_logger() {
    let mut logger = AuditLogger::new(2);

    let entry = AuditEntry {
        timestamp: chrono::Utc::now(),
        user_id: Some("user1".to_string()),
        tenant_id: Some("tenant1".to_string()),
        action: "query".to_string(),
        model: Some("orders".to_string()),
        query_hash: Some("abc123".to_string()),
        row_count: Some(10),
        duration_ms: 50,
        success: true,
        error: None,
    };

    logger.log(entry);
    assert_eq!(logger.get_entries().len(), 1);

    // Test max entries
    logger.log(AuditEntry {
        timestamp: chrono::Utc::now(),
        user_id: None,
        tenant_id: None,
        action: "query".to_string(),
        model: None,
        query_hash: None,
        row_count: None,
        duration_ms: 10,
        success: true,
        error: None,
    });

    logger.log(AuditEntry {
        timestamp: chrono::Utc::now(),
        user_id: None,
        tenant_id: None,
        action: "query".to_string(),
        model: None,
        query_hash: None,
        row_count: None,
        duration_ms: 10,
        success: true,
        error: None,
    });

    assert_eq!(logger.get_entries().len(), 2); // Max 2 entries
}

#[test]
fn test_model_serialization() {
    let model = Model {
        name: "orders".to_string(),
        datasource: "postgres".to_string(),
        description: Some("Order facts".to_string()),
        measures: vec![Measure::simple("revenue", AggregationType::Sum)],
        dimensions: vec![Dimension::new("status")],
        time_dimensions: vec![TimeDimension::new("created_at", TimeGranularity::Day)],
        joins: vec![],
        sql: None,
        meta: Default::default(),
    };

    let yaml = serde_yaml::to_string(&model).unwrap();
    assert!(yaml.contains("orders"));
    assert!(yaml.contains("revenue"));

    let deserialized: Model = serde_yaml::from_str(&yaml).unwrap();
    assert_eq!(deserialized.name, "orders");
    assert_eq!(deserialized.measures.len(), 1);
}

#[test]
fn test_datasource_serialization() {
    let ds = DataSource {
        name: "postgres_primary".to_string(),
        driver: "postgres".to_string(),
        connection_string: "postgresql://localhost/db".to_string(),
        description: Some("Primary DB".to_string()),
        models: vec!["orders".to_string()],
        pool_size: Some(10),
        meta: Default::default(),
    };

    let yaml = serde_yaml::to_string(&ds).unwrap();
    assert!(yaml.contains("postgres_primary"));

    let deserialized: DataSource = serde_yaml::from_str(&yaml).unwrap();
    assert_eq!(deserialized.name, "postgres_primary");
    assert_eq!(deserialized.pool_size, Some(10));
}
