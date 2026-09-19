use crate::models::{AggregationType, FilterOperator, Formula, Measure, TimeGranularity};
use anyhow::{anyhow, Result};
use regex::Regex;
use std::collections::HashMap;

/// Parses and validates formula expressions
pub struct FormulaParser {
    // Cache of compiled regexes
    agg_regex: Regex,
    time_shift_regex: Regex,
    ratio_regex: Regex,
}

impl FormulaParser {
    pub fn new() -> Result<Self> {
        Ok(Self {
            agg_regex: Regex::new(r"^(\w+):(\w+)$")?,
            time_shift_regex: Regex::new(r"time_shift\(([^,]+),\s*(-?\d+),\s*'(\w+)'\)")?,
            ratio_regex: Regex::new(r"ratio\(([^,]+),\s*([^)]+)\)")?,
        })
    }

    /// True when the expression uses shorthand or formula functions that need parsing.
    pub fn needs_parse(expr: &str) -> bool {
        let expr = expr.trim();
        if expr.is_empty() {
            return false;
        }
        expr.contains(':')
            || expr.starts_with("time_shift(")
            || expr.starts_with("ratio(")
            || expr.starts_with("pct_change(")
            || expr.starts_with("running_total(")
    }

    /// Parse a measure expression like "revenue:sum" or "time_shift(revenue:sum, -1, 'year')"
    pub fn parse_measure(&self, expr: &str) -> Result<Measure> {
        let expr = expr.trim();
        if expr.is_empty() {
            return Err(anyhow!("Invalid formula: expression is empty"));
        }

        // Check for time_shift
        if let Some(caps) = self.time_shift_regex.captures(expr) {
            let inner = caps.get(1).unwrap().as_str();
            let offset: i32 = caps.get(2).unwrap().as_str().parse()?;
            let granularity = caps.get(3).unwrap().as_str();

            let inner_measure = self.parse_measure(inner)?;
            return Ok(Measure::new(
                Formula::new(format!(
                    "LAG({}::{}) OVER (ORDER BY {})",
                    inner_measure.formula.expression, inner_measure.aggregation, granularity
                ))
                .with_label(format!("{}_shift_{}", inner_measure.label(), offset)),
                inner_measure.aggregation,
            ));
        }

        // Check for ratio
        if let Some(caps) = self.ratio_regex.captures(expr) {
            let num = caps.get(1).unwrap().as_str();
            let denom = caps.get(2).unwrap().as_str();

            let num_measure = self.parse_measure(num)?;
            let denom_measure = self.parse_measure(denom)?;

            return Ok(Measure::new(
                Formula::new(format!(
                    "({}::{}) / NULLIF({}::{}, 0)",
                    num_measure.formula.expression,
                    num_measure.aggregation,
                    denom_measure.formula.expression,
                    denom_measure.aggregation
                ))
                .with_label(format!(
                    "{}_per_{}",
                    num_measure.label(),
                    denom_measure.label()
                )),
                AggregationType::Avg, // Ratios are typically averaged
            ));
        }

        // Simple aggregation like "revenue:sum"
        if let Some(caps) = self.agg_regex.captures(expr) {
            let field = caps.get(1).unwrap().as_str();
            let agg_str = caps.get(2).unwrap().as_str();
            let aggregation = self.parse_aggregation(agg_str)?;
            return Ok(Measure::simple(field, aggregation));
        }

        // Default to sum
        Ok(Measure::simple(expr, AggregationType::Sum))
    }

    fn parse_aggregation(&self, s: &str) -> Result<AggregationType> {
        match s.to_lowercase().as_str() {
            "sum" => Ok(AggregationType::Sum),
            "avg" | "average" => Ok(AggregationType::Avg),
            "count" => Ok(AggregationType::Count),
            "min" => Ok(AggregationType::Min),
            "max" => Ok(AggregationType::Max),
            "count_distinct" | "countd" => Ok(AggregationType::CountDistinct),
            other => Err(anyhow!("Invalid formula: unknown aggregation '{}'", other)),
        }
    }

    /// True when a parsed measure expression is already dialect SQL (window/ratio).
    pub fn is_compiled_sql(expression: &str) -> bool {
        expression.contains(" OVER ") || expression.contains("NULLIF(") || expression.contains("::")
    }

    /// Parse time granularity from string
    pub fn parse_granularity(&self, s: &str) -> Result<TimeGranularity> {
        match s.to_lowercase().as_str() {
            "second" | "seconds" | "sec" => Ok(TimeGranularity::Second),
            "minute" | "minutes" | "min" => Ok(TimeGranularity::Minute),
            "hour" | "hours" | "hr" => Ok(TimeGranularity::Hour),
            "day" | "days" | "d" => Ok(TimeGranularity::Day),
            "week" | "weeks" | "w" => Ok(TimeGranularity::Week),
            "month" | "months" | "mon" => Ok(TimeGranularity::Month),
            "quarter" | "quarters" | "qtr" => Ok(TimeGranularity::Quarter),
            "year" | "years" | "yr" => Ok(TimeGranularity::Year),
            other => Err(anyhow!("Unknown time granularity: {}", other)),
        }
    }

    /// Parse filter operator
    pub fn parse_operator(&self, s: &str) -> Result<FilterOperator> {
        match s.to_lowercase().as_str() {
            "eq" | "=" | "==" => Ok(FilterOperator::Eq),
            "neq" | "!=" | "<>" => Ok(FilterOperator::Neq),
            "gt" | ">" => Ok(FilterOperator::Gt),
            "gte" | ">=" => Ok(FilterOperator::Gte),
            "lt" | "<" => Ok(FilterOperator::Lt),
            "lte" | "<=" => Ok(FilterOperator::Lte),
            "like" => Ok(FilterOperator::Like),
            "ilike" => Ok(FilterOperator::ILike),
            "in" => Ok(FilterOperator::In),
            "not_in" | "nin" => Ok(FilterOperator::NotIn),
            "is_null" | "isnull" => Ok(FilterOperator::IsNull),
            "is_not_null" | "isnotnull" => Ok(FilterOperator::IsNotNull),
            "between" => Ok(FilterOperator::Between),
            "not_between" => Ok(FilterOperator::NotBetween),
            other => Err(anyhow!("Unknown filter operator: {}", other)),
        }
    }
}

impl Default for FormulaParser {
    fn default() -> Self {
        Self::new().expect("Failed to create FormulaParser")
    }
}

/// Formula registry for managing computed measures
pub struct FormulaRegistry {
    parser: FormulaParser,
    formulas: HashMap<String, Measure>,
}

impl FormulaRegistry {
    pub fn new() -> Result<Self> {
        Ok(Self {
            parser: FormulaParser::new()?,
            formulas: HashMap::new(),
        })
    }

    pub fn register(&mut self, name: impl Into<String>, formula: Measure) {
        self.formulas.insert(name.into(), formula);
    }

    pub fn register_str(&mut self, name: impl Into<String>, expr: &str) -> Result<()> {
        let measure = self.parser.parse_measure(expr)?;
        self.register(name, measure);
        Ok(())
    }

    pub fn get(&self, name: &str) -> Option<&Measure> {
        self.formulas.get(name)
    }

    pub fn resolve(&self, expr: &str) -> Result<Measure> {
        // Check if it's a registered formula
        if let Some(measure) = self.formulas.get(expr) {
            return Ok(measure.clone());
        }
        // Otherwise parse as expression
        self.parser.parse_measure(expr)
    }
}

impl Default for FormulaRegistry {
    fn default() -> Self {
        Self::new().expect("Failed to create FormulaRegistry")
    }
}
