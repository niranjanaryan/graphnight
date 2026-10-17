use crate::models::{
    is_plain_identifier, AggregationType, FilterOperator, Formula, Measure, TimeGranularity,
};
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
            agg_regex: Regex::new(r"^([\w*]+):(\w+)$")?,
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
    ///
    /// Delegates to [`FilterOperator::parse_strict`] so the core parser and the
    /// bindings that used to be lenient share one definition of what an
    /// operator may be.
    pub fn parse_operator(&self, s: &str) -> Result<FilterOperator> {
        FilterOperator::parse_strict(s).map_err(anyhow::Error::msg)
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

/// An argument to an allowlisted formula function.
#[derive(Debug, Clone, PartialEq)]
pub enum FunctionArg {
    /// A column reference, optionally qualified with a table or model alias.
    Column(String),
    /// A numeric literal, kept as its original text.
    ///
    /// The text is validated against a strict numeric pattern and re-emitted
    /// verbatim. Round-tripping through `f64` would risk turning a long
    /// coordinate or a high-precision scale factor into different digits, which
    /// is a correctness problem for exactly the geospatial values this exists
    /// to serve.
    Number(String),
}

/// A single-level call to an allowlisted function.
#[derive(Debug, Clone, PartialEq)]
pub struct FunctionCall {
    pub name: String,
    pub args: Vec<FunctionArg>,
}

/// Functions a measure expression may call.
///
/// This is an allowlist, not a blocklist, matching the stance already taken for
/// aggregations: an unrecognised name is refused rather than passed to the
/// database. A blocklist cannot be made safe here, because the expression
/// becomes SQL and a blocklist has to anticipate every dangerous construction
/// rather than only admit known-good ones.
///
/// Every entry must be deterministic and side-effect free. Nothing that writes,
/// mutates, or reaches outside the query belongs here.
const ALLOWED_FUNCTIONS: &[&str] = &[
    // String
    "lower",
    "upper",
    "length",
    "char_length",
    "trim",
    "btrim",
    "ltrim",
    "rtrim",
    "substring",
    "substr",
    "replace",
    "concat",
    "left",
    "right",
    // Numeric
    "abs",
    "round",
    "ceil",
    "ceiling",
    "floor",
    "trunc",
    "sqrt",
    "power",
    "pow",
    "sign",
    "mod",
    "greatest",
    "least",
    // Date / time
    "date_trunc",
    "extract",
    "date_part",
    "to_char",
    "age",
    // Spatial: deterministic scalar functions
    "st_area",
    "st_perimeter",
    "st_length",
    "st_npoints",
    "st_numgeometries",
    "st_numinteriorrings",
    "st_geometrytype",
    "st_srid",
    "st_isvalid",
    "st_isempty",
    "st_ismalformed",
    "st_dimension",
    "st_zmflag",
    "st_x",
    "st_y",
    "st_z",
    "st_m",
    "st_xmin",
    "st_xmax",
    "st_ymin",
    "st_ymax",
    "st_mindim",
    "st_maxdim",
    "st_envelope",
    "st_expand",
    "st_buffer",
    "st_simplify",
    "st_transform",
    "st_astext",
    "st_aswkt",
    "st_asbinary",
    "st_asgml",
    "st_asgeojson",
    "st_geohash",
    "st_point",
    "st_makepoint",
    "st_centroid",
    "st_contains",
    "st_within",
    "st_intersects",
    "st_disjoint",
    "st_touches",
    "st_overlaps",
    "st_crosses",
    "st_equals",
    "st_dwithin",
    "st_distance",
    "st_distancesphere",
    "st_distancewithin",
    "st_hausdorffdistance",
    "st_lineintersects",
    "st_linelocatepoint",
    "st_linelength",
    "st_azimuth",
    "st_boundingbox",
    "st_remap",
];

/// True when `name` is a function a measure expression may call.
pub fn is_allowed_function(name: &str) -> bool {
    ALLOWED_FUNCTIONS
        .iter()
        .any(|f| f.eq_ignore_ascii_case(name))
}

/// Whether `expr` has the shape of a function call, i.e. it names a function
/// and opens a parenthesis.
///
/// Used to decide whether an expression containing `(` is a call that must be
/// parsed or a bare column name that must be quoted. Getting this wrong in one
/// direction mangles a call into an identifier; in the other it refuses a
/// column whose name happens to contain a parenthesis.
fn looks_like_function_call(expr: &str) -> bool {
    match expr.find('(') {
        Some(open) => {
            let name = expr[..open].trim();
            !name.is_empty() && name.split('.').all(is_plain_identifier) && !name.contains(' ')
        }
        None => false,
    }
}

/// A strict numeric literal: optional sign, digits with optional fraction and
/// optional exponent. No hex, no `inf`, no `nan`, no arithmetic.
fn is_numeric_literal(s: &str) -> bool {
    let body = s.strip_prefix(['+', '-']).unwrap_or(s);
    let (mantissa, exponent) = match body.find(['e', 'E']) {
        Some(idx) => (&body[..idx], Some(&body[idx + 1..])),
        None => (body, None),
    };
    let mut mantissa = mantissa.split('.');
    let int_part = mantissa.next().unwrap_or("");
    let frac_part = mantissa.next().unwrap_or("");
    if mantissa.next().is_some() {
        return false;
    }
    let digits_ok = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if !digits_ok(int_part) || !(frac_part.is_empty() || digits_ok(frac_part)) {
        return false;
    }
    match exponent {
        None => true,
        Some(e) => {
            let e = e.strip_prefix(['+', '-']).unwrap_or(e);
            digits_ok(e)
        }
    }
}

/// Parse a measure expression as a call to an allowlisted function.
///
/// Returns `Ok(None)` when the expression is not a function call at all, so
/// the caller can fall back to treating it as a column name. An expression that
/// *does* look like a call but cannot be honoured is an error rather than a
/// silent fallback: quoting `ST_Distance(a, b)` as an identifier produces SQL
/// that fails far from the model that caused it, and "unknown function" is a
/// far better message than `column "ST_Distance(a, b)" does not exist`.
///
/// Only single-level calls are supported. Nesting, string literals, operators
/// and subqueries are refused, which keeps the set of things this can produce
/// small enough to reason about.
pub fn parse_function_call(expr: &str) -> Result<Option<FunctionCall>, String> {
    let expr = expr.trim();
    if !looks_like_function_call(expr) {
        return Ok(None);
    }
    let open = expr.find('(').expect("checked by looks_like_function_call");
    let name = expr[..open].trim().to_string();
    let body = &expr[open + 1..];
    let close = body
        .rfind(')')
        .ok_or_else(|| format!("unclosed '(' in function call '{expr}'"))?;
    let args_src = &body[..close];
    if !body[close + 1..].trim().is_empty() {
        return Err(format!("trailing text after ')' in function call '{expr}'"));
    }
    if !is_allowed_function(&name) {
        return Err(format!(
            "'{name}' is not an allowed function. Allowed functions are a fixed list; \
             see the GraphNight docs for how to add one."
        ));
    }

    let mut args = Vec::new();
    if !args_src.trim().is_empty() {
        for raw in args_src.split(',') {
            let part = raw.trim();
            if part.is_empty() {
                return Err(format!("empty argument in function call '{expr}'"));
            }
            if is_numeric_literal(part) {
                args.push(FunctionArg::Number(part.to_string()));
            } else if part.split('.').count() <= 2 && part.split('.').all(is_plain_identifier) {
                args.push(FunctionArg::Column(part.to_string()));
            } else {
                return Err(format!(
                    "argument '{part}' in '{expr}' is not a column reference or a number. \
                     Nested calls, string literals, operators and subqueries are not supported."
                ));
            }
        }
    }
    Ok(Some(FunctionCall { name, args }))
}
