//! The agent tool catalog.
//!
//! A tool is a governed operation exposed with a JSON Schema an LLM can read.
//! Two properties matter more than the list of tools itself:
//!
//! 1. **Every tool runs through [`QueryService`]**, so an agent is subject to
//!    the same RLS, forced filters, column masks and row caps as any other
//!    caller.
//! 2. **Errors carry a code and a hint.** An agent that misspells a field
//!    should be told the correct spelling in one turn, not left to guess. This
//!    is the single highest-leverage reliability feature in the surface.
//!
//! Mutations are deliberately *not* exposed by default: letting an LLM call
//! `create_model` is how a semantic layer gets corrupted. They require an
//! explicit opt-in on the catalog.

use crate::governance::{CallContext, ExecuteOptions, QueryService, ServiceError};
use graphnight_core::models::Query as CoreQuery;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;

/// A tool failure rendered for machine consumption.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolError {
    /// Stable, machine-readable code (e.g. `UNKNOWN_FIELD`).
    pub code: String,
    /// Human/model-readable message.
    pub message: String,
    /// What to do about it. Present whenever we can offer a concrete next step.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

impl ToolError {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_string(),
            message: message.into(),
            hint: None,
        }
    }

    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).unwrap_or_else(|_| json!({ "code": self.code }))
    }
}

impl std::fmt::Display for ToolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.hint {
            Some(hint) => write!(f, "{} [{}] ({hint})", self.message, self.code),
            None => write!(f, "{} [{}]", self.message, self.code),
        }
    }
}

impl From<ServiceError> for ToolError {
    fn from(e: ServiceError) -> Self {
        let (code, hint) = match &e {
            ServiceError::ModelNotFound(m) => (
                "MODEL_NOT_FOUND",
                Some(format!(
                    "Call search_models to find the right model, or list_models to see all available models. Unknown model: {m}"
                )),
            ),
            ServiceError::DatasourceNotFound(d) => (
                "DATASOURCE_NOT_FOUND",
                Some(format!("Call list_datasources to see registered datasources. Missing: {d}")),
            ),
            ServiceError::Unauthenticated(_) => (
                "UNAUTHENTICATED",
                Some(
                    "No valid credentials were presented. This is not a permissions \
                     problem and retrying will not help; obtain an API key first."
                        .to_string(),
                ),
            ),
            ServiceError::PolicyViolation(p) => ("POLICY_VIOLATION", {
                let _ = p;
                Some(
                    "This caller is not permitted to access that model or datasource. \
                     Do not retry; report the restriction."
                        .to_string(),
                )
            }),
            ServiceError::InvalidQuery(q) => ("INVALID_QUERY", Some(q.clone())),
            ServiceError::Timeout(s) => (
                "TIMEOUT",
                Some(format!("Query exceeded {s}s. Narrow the time range or add filters, then retry once.")),
            ),
            ServiceError::Execution(x) => ("EXECUTION_FAILED", Some(x.clone())),
            ServiceError::Storage(_) => ("STORAGE_ERROR", None),
            ServiceError::UnknownTool(t) => (
                "UNKNOWN_TOOL",
                Some(format!("Unknown tool {t}. Call tools_list to see available tools.")),
            ),
            ServiceError::Invalid(x) => ("INVALID_ARGUMENT", Some(x.clone())),
        };
        ToolError {
            code: code.to_string(),
            message: e.to_string(),
            hint,
        }
    }
}

// ---------------------------------------------------------------------------
// Tool input types (JSON Schema is derived, so it cannot drift)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct CapabilitiesInput {}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct ListModelsInput {
    /// Restrict to one datasource.
    #[serde(default)]
    pub datasource: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SearchModelsInput {
    /// Free-text query, e.g. "monthly revenue by store".
    pub q: String,
    /// Maximum results (default 10).
    #[serde(default)]
    pub limit: Option<u32>,
    /// Restrict to one datasource.
    #[serde(default)]
    pub datasource: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GetModelInput {
    /// Model name, e.g. "orders".
    pub name: String,
    #[serde(default)]
    pub datasource: Option<String>,
}

/// The semantic query an agent submits. Mirrors `Query` in graphnight-core but
/// uses the shorthand formula strings agents actually write.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct ToolQuery {
    /// Model to query, e.g. "orders".
    pub name: Option<String>,
    /// Measure formulas, e.g. `["revenue:sum", "order_count:count"]`.
    #[serde(default)]
    pub measures: Vec<String>,
    /// Dimensions to group by, e.g. `["status"]`.
    #[serde(default)]
    pub dimensions: Vec<String>,
    /// Time dimensions with granularity, e.g. `created_at@month`.
    #[serde(default)]
    pub time_dimensions: Vec<String>,
    /// Filters as `field:operator:value`, e.g. `status:eq:completed`.
    #[serde(default)]
    pub filters: Vec<String>,
    /// `field:asc` or `field:desc`.
    #[serde(default)]
    pub order: Vec<String>,
    #[serde(default)]
    pub limit: Option<u32>,
    #[serde(default)]
    pub offset: Option<u32>,
    /// Reference a previous stage in a multi-stage query.
    #[serde(default)]
    pub stage_ref: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RunQueryInput {
    pub query: ToolQuery,
    /// Validate and compile without touching the database.
    #[serde(default)]
    pub dry_run: Option<bool>,
    /// Include the generated SQL in the response.
    #[serde(default)]
    pub explain: Option<bool>,
    /// Row cap for this call; clamped by the session policy.
    #[serde(default)]
    pub max_rows: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ValidateQueryInput {
    pub query: ToolQuery,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MultiStageInput {
    /// Ordered stages; a stage may set `stage_ref` to filter on a prior stage.
    pub stages: Vec<ToolQuery>,
    #[serde(default)]
    pub dry_run: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RecallMemoriesInput {
    /// Free-text query; ranked by relevance, recency and importance.
    #[serde(default)]
    pub query: Option<String>,
    /// Only memories linked to this entity (e.g. "revenue:sum").
    #[serde(default)]
    pub entity: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RememberInput {
    /// The fact or learning to store.
    pub text: String,
    /// Entities this relates to, e.g. `["revenue:sum", "orders"]`.
    #[serde(default)]
    pub linked_entities: Vec<String>,
    /// learning | fact | episode | decision | error
    #[serde(default)]
    pub kind: Option<String>,
    /// 0.0–1.0; higher is surfaced sooner.
    #[serde(default)]
    pub importance: Option<f32>,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ToolsListInput {}

// --- semantic layer mutation inputs -------------------------------------
// These exist so an agent can author the semantic layer, but they are hidden
// and refused unless an operator turns the catalog's mutation gate on.

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CreateModelInput {
    /// Unique model name, e.g. "orders".
    pub name: String,
    /// Datasource the model reads from; must already exist.
    pub datasource: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub measures: Vec<MeasureInput>,
    #[serde(default)]
    pub dimensions: Vec<DimensionInput>,
    #[serde(default)]
    pub time_dimensions: Vec<TimeDimensionInput>,
    /// Base table or view, e.g. "public.orders". Ignored when `sql` is set.
    #[serde(default)]
    pub base_table: Option<String>,
    /// Custom SQL override; takes precedence over `base_table`.
    #[serde(default)]
    pub sql: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct UpdateModelInput {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub measures: Option<Vec<MeasureInput>>,
    #[serde(default)]
    pub dimensions: Option<Vec<DimensionInput>>,
    #[serde(default)]
    pub time_dimensions: Option<Vec<TimeDimensionInput>>,
    #[serde(default)]
    pub base_table: Option<String>,
    #[serde(default)]
    pub sql: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DeleteModelInput {
    pub name: String,
    /// Required only when two datasources define models of the same name.
    #[serde(default)]
    pub datasource: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MeasureInput {
    /// SQL expression to aggregate, or "*" to count rows.
    pub formula: String,
    pub aggregation: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub format: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DimensionInput {
    pub name: String,
    #[serde(default)]
    pub label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TimeDimensionInput {
    /// Name of a column that is also declared as a dimension.
    pub dimension: String,
    /// One of: second, minute, hour, day, week, month, quarter, year.
    pub granularity: String,
    #[serde(default)]
    pub label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CreateDatasourceInput {
    pub name: String,
    /// One of: postgres, mysql, sqlite, duckdb.
    pub driver: String,
    /// Connection URI. Treated as a secret: never echoed back to the caller.
    pub connection_string: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub pool_size: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct UpdateDatasourceInput {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub connection_string: Option<String>,
    #[serde(default)]
    pub pool_size: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DeleteDatasourceInput {
    pub name: String,
}

// ---------------------------------------------------------------------------
// Catalog
// ---------------------------------------------------------------------------

/// Static description of one tool.
#[derive(Debug, Clone)]
pub struct ToolSpec {
    pub name: &'static str,
    pub description: &'static str,
    /// Mutating tools require `allow_mutations` on the catalog.
    pub mutating: bool,
    /// Build the JSON Schema for this tool's input.
    schema_fn: fn() -> Value,
}

impl ToolSpec {
    pub fn input_schema(&self) -> Value {
        (self.schema_fn)()
    }
}

fn schema_of<T: JsonSchema>() -> Value {
    serde_json::to_value(schemars::schema_for!(T)).unwrap_or_else(|_| json!({"type": "object"}))
}

const CAPABILITIES_DESCRIPTION: &str = "Describe this server: supported SQL dialects, aggregations, time granularities, filter operators, formula functions and row limits. Call this first if unsure what is supported.";

const LIST_MODELS_DESCRIPTION: &str = "List the semantic models this caller may query. Use when you know the domain but not the model name.";

const SEARCH_MODELS_DESCRIPTION: &str = "Search models by intent, e.g. \"monthly revenue by store\". Returns ranked model names with matching fields and snippets. Start here when the model name is unknown.";

const GET_MODEL_DESCRIPTION: &str = "Get a model's full schema: measures with formulas and aggregations, dimensions, time dimensions, joins, plus worked example queries. Always call this before run_query so you use real field names.";

const RUN_QUERY_DESCRIPTION: &str = "Run a governed semantic query and return rows. Subject to the caller's row limit, forced filters, row-level security and column masks. Set dry_run=true to compile without executing.";

const VALIDATE_QUERY_DESCRIPTION: &str = "Validate and compile a query without executing it. Returns precise errors and did-you-mean suggestions. Cheaper than run_query; use it to check a query first.";

const MULTI_STAGE_DESCRIPTION: &str = "Run an ordered pipeline of queries as a DAG, where a stage may filter on a previous stage's output via stage_ref. Use for cohort and funnel analyses.";

const RECALL_DESCRIPTION: &str = "Recall stored learnings and facts relevant to a query, ranked by relevance, recency and importance. Call before answering from memory.";

const REMEMBER_DESCRIPTION: &str = "Store a durable learning, fact or episode with its linked entities, so future sessions can recall it.";

const TOOLS_LIST_DESCRIPTION: &str =
    "List the tools available to this caller, with descriptions and JSON Schemas.";

const CREATE_MODEL_DESCRIPTION: &str = "Define a new semantic model: its measures, dimensions and time dimensions over a datasource. Only available when the operator has enabled semantic layer mutations.";
const UPDATE_MODEL_DESCRIPTION: &str = "Change an existing model's measures, dimensions or SQL. Only fields you supply are changed. Only available when the operator has enabled semantic layer mutations.";
const DELETE_MODEL_DESCRIPTION: &str = "Delete a semantic model. This cannot be undone. Only available when the operator has enabled semantic layer mutations.";
const CREATE_DATASOURCE_DESCRIPTION: &str = "Register a new datasource connection. The connection string is stored but never returned. Only available when the operator has enabled semantic layer mutations.";
const UPDATE_DATASOURCE_DESCRIPTION: &str = "Update a datasource's description, connection string or pool size. Only available when the operator has enabled semantic layer mutations.";
const DELETE_DATASOURCE_DESCRIPTION: &str = "Delete a datasource. Every model on it becomes unqueryable. Only available when the operator has enabled semantic layer mutations.";

/// The tools GraphNight exposes to agents, in discovery order.
pub fn builtin_tools() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "get_capabilities",
            description: CAPABILITIES_DESCRIPTION,
            mutating: false,
            schema_fn: schema_of::<CapabilitiesInput>,
        },
        ToolSpec {
            name: "search_models",
            description: SEARCH_MODELS_DESCRIPTION,
            mutating: false,
            schema_fn: schema_of::<SearchModelsInput>,
        },
        ToolSpec {
            name: "list_models",
            description: LIST_MODELS_DESCRIPTION,
            mutating: false,
            schema_fn: schema_of::<ListModelsInput>,
        },
        ToolSpec {
            name: "get_model",
            description: GET_MODEL_DESCRIPTION,
            mutating: false,
            schema_fn: schema_of::<GetModelInput>,
        },
        ToolSpec {
            name: "validate_query",
            description: VALIDATE_QUERY_DESCRIPTION,
            mutating: false,
            schema_fn: schema_of::<ValidateQueryInput>,
        },
        ToolSpec {
            name: "run_query",
            description: RUN_QUERY_DESCRIPTION,
            mutating: false,
            schema_fn: schema_of::<RunQueryInput>,
        },
        ToolSpec {
            name: "multi_stage_query",
            description: MULTI_STAGE_DESCRIPTION,
            mutating: false,
            schema_fn: schema_of::<MultiStageInput>,
        },
        ToolSpec {
            name: "recall_memories",
            description: RECALL_DESCRIPTION,
            mutating: false,
            schema_fn: schema_of::<RecallMemoriesInput>,
        },
        ToolSpec {
            name: "remember",
            description: REMEMBER_DESCRIPTION,
            mutating: false,
            schema_fn: schema_of::<RememberInput>,
        },
        ToolSpec {
            name: "tools_list",
            description: TOOLS_LIST_DESCRIPTION,
            mutating: false,
            schema_fn: schema_of::<ToolsListInput>,
        },
        // Semantic layer mutations. Hidden unless an operator opts in.
        ToolSpec {
            name: "create_model",
            description: CREATE_MODEL_DESCRIPTION,
            mutating: true,
            schema_fn: schema_of::<CreateModelInput>,
        },
        ToolSpec {
            name: "update_model",
            description: UPDATE_MODEL_DESCRIPTION,
            mutating: true,
            schema_fn: schema_of::<UpdateModelInput>,
        },
        ToolSpec {
            name: "delete_model",
            description: DELETE_MODEL_DESCRIPTION,
            mutating: true,
            schema_fn: schema_of::<DeleteModelInput>,
        },
        ToolSpec {
            name: "create_datasource",
            description: CREATE_DATASOURCE_DESCRIPTION,
            mutating: true,
            schema_fn: schema_of::<CreateDatasourceInput>,
        },
        ToolSpec {
            name: "update_datasource",
            description: UPDATE_DATASOURCE_DESCRIPTION,
            mutating: true,
            schema_fn: schema_of::<UpdateDatasourceInput>,
        },
        ToolSpec {
            name: "delete_datasource",
            description: DELETE_DATASOURCE_DESCRIPTION,
            mutating: true,
            schema_fn: schema_of::<DeleteDatasourceInput>,
        },
    ]
}

/// The set of tools a caller may invoke, and the executor for them.
pub struct ToolCatalog {
    service: Arc<QueryService>,
    tools: HashMap<&'static str, ToolSpec>,
    order: Vec<&'static str>,
    allow_mutations: bool,
    /// Row cap applied to every `run_query` unless the call overrides it.
    default_max_rows: usize,
    embedder: Option<Arc<dyn crate::embedding::EmbeddingProvider>>,
    /// Embeddings of model descriptions, keyed by model name and reused while
    /// the description's content hash is unchanged.
    ///
    /// Hybrid retrieval has to embed every visible model on each search, because
    /// a semantically similar model may not appear in the lexical hits at all.
    /// Caching by content hash keeps that to one embedding call per model that
    /// has actually changed.
    embed_cache: std::sync::RwLock<HashMap<String, (u64, Vec<f32>)>>,
}

/// Render a model into the text that gets embedded.
///
/// Name, description and field names are included, not the formulas: an agent
/// searching for intent should match on what the model *contains*, and formula
/// bodies are noise for that purpose (and often long enough to dilute the
/// vector).
fn model_document(m: &graphnight_core::models::Model) -> String {
    let mut parts: Vec<String> = Vec::new();
    parts.push(m.name.clone());
    if let Some(d) = &m.description {
        parts.push(d.clone());
    }
    for meas in &m.measures {
        parts.push(meas.formula.expression.clone());
    }
    for dim in &m.dimensions {
        parts.push(dim.name.clone());
    }
    for td in &m.time_dimensions {
        parts.push(td.dimension.clone());
    }
    for join in &m.joins {
        parts.push(join.name.clone());
    }
    parts.join(" ")
}

/// Cheap content fingerprint, used to invalidate a cached embedding.
///
/// `DefaultHasher` is not stable across Rust releases, which is fine: a changed
/// fingerprint only costs one extra embedding call.
fn content_hash(text: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

impl ToolCatalog {
    pub fn new(service: Arc<QueryService>) -> Self {
        let mut tools = HashMap::new();
        let mut order = Vec::new();
        for spec in builtin_tools() {
            order.push(spec.name);
            tools.insert(spec.name, spec);
        }
        Self {
            service,
            tools,
            order,
            // Off by default: an LLM must not be able to author the semantic
            // layer without an operator opting in.
            allow_mutations: false,
            default_max_rows: 500,
            embedder: None,
            embed_cache: std::sync::RwLock::new(HashMap::new()),
        }
    }

    pub fn with_mutations(mut self, allow: bool) -> Self {
        self.allow_mutations = allow;
        self
    }

    pub fn with_default_max_rows(mut self, max_rows: usize) -> Self {
        self.default_max_rows = max_rows;
        self
    }

    pub fn with_embedder(
        mut self,
        embedder: Option<Arc<dyn crate::embedding::EmbeddingProvider>>,
    ) -> Self {
        self.embedder = embedder;
        self
    }

    /// Tools visible to this catalog, honouring the mutation gate.
    pub fn specs(&self) -> Vec<&ToolSpec> {
        self.order
            .iter()
            .filter_map(|name| self.tools.get(name))
            .filter(|spec| self.allow_mutations || !spec.mutating)
            .collect()
    }

    pub fn get(&self, name: &str) -> Option<&ToolSpec> {
        self.tools
            .get(name)
            .filter(|s| self.allow_mutations || !s.mutating)
    }

    /// MCP-shaped tool descriptors.
    pub fn mcp_descriptors(&self) -> Vec<Value> {
        self.specs()
            .into_iter()
            .map(|spec| {
                json!({
                    "name": spec.name,
                    "description": spec.description,
                    "inputSchema": spec.input_schema(),
                })
            })
            .collect()
    }

    /// Execute a tool by name with JSON arguments.
    pub async fn call(
        &self,
        ctx: &CallContext,
        name: &str,
        args: Value,
    ) -> Result<Value, ToolError> {
        let spec = self
            .get(name)
            .ok_or_else(|| ToolError::from(ServiceError::UnknownTool(name.to_string())))?;
        if spec.mutating && !self.allow_mutations {
            return Err(ToolError::new(
                "MUTATIONS_DISABLED",
                format!("Tool {} is disabled on this server", name),
            )
            .with_hint(
                "Model and datasource mutations are disabled for agents by default. \
                 Ask an administrator to enable them if truly required.",
            ));
        }

        let args = if args.is_null() { json!({}) } else { args };

        match name {
            "get_capabilities" => self.capabilities(ctx),
            "tools_list" => self.tools_list(),
            "list_models" => {
                let input: ListModelsInput = parse_args(args)?;
                self.list_models(ctx, input).await
            }
            "search_models" => {
                let input: SearchModelsInput = parse_args(args)?;
                self.search_models(ctx, input).await
            }
            "get_model" => {
                let input: GetModelInput = parse_args(args)?;
                self.get_model(ctx, input).await
            }
            "validate_query" => {
                let input: ValidateQueryInput = parse_args(args)?;
                self.validate_query(ctx, input).await
            }
            "run_query" => {
                let input: RunQueryInput = parse_args(args)?;
                self.run_query(ctx, input).await
            }
            "multi_stage_query" => {
                let input: MultiStageInput = parse_args(args)?;
                self.multi_stage_query(ctx, input).await
            }
            "recall_memories" => {
                let input: RecallMemoriesInput = parse_args(args)?;
                self.recall_memories(ctx, input).await
            }
            "remember" => {
                let input: RememberInput = parse_args(args)?;
                self.remember(ctx, input).await
            }
            "create_model" => {
                let input: CreateModelInput = parse_args(args)?;
                self.create_model(ctx, input).await
            }
            "update_model" => {
                let input: UpdateModelInput = parse_args(args)?;
                self.update_model(ctx, input).await
            }
            "delete_model" => {
                let input: DeleteModelInput = parse_args(args)?;
                self.delete_model(ctx, input).await
            }
            "create_datasource" => {
                let input: CreateDatasourceInput = parse_args(args)?;
                self.create_datasource(ctx, input).await
            }
            "update_datasource" => {
                let input: UpdateDatasourceInput = parse_args(args)?;
                self.update_datasource(ctx, input).await
            }
            "delete_datasource" => {
                let input: DeleteDatasourceInput = parse_args(args)?;
                self.delete_datasource(ctx, input).await
            }
            other => Err(ToolError::from(ServiceError::UnknownTool(
                other.to_string(),
            ))),
        }
    }

    fn capabilities(&self, ctx: &CallContext) -> Result<Value, ToolError> {
        let policy = ctx.session_policy();
        Ok(json!({
            "server": "graphnight",
            "version": env!("CARGO_PKG_VERSION"),
            "sql_dialects": ["postgres", "mysql", "sqlite", "duckdb"],
            "aggregations": ["sum", "avg", "count", "min", "max", "count_distinct"],
            "time_granularities": ["second", "minute", "hour", "day", "week", "month", "quarter", "year"],
            "filter_operators": [
                "eq", "neq", "gt", "gte", "lt", "lte",
                "in", "not_in", "like", "ilike", "not_like",
                "is_null", "is_not_null", "between"
            ],
            "formula_functions": [
                "time_shift(measure, -1, 'month')",
                "ratio(numerator, denominator)",
                "pct_change(measure)",
                "running_total(measure)"
            ],
            "measure_shorthand": {
                "description": "A measure is '<column>:<aggregation>', or '*' for row counts",
                "examples": ["revenue:sum", "amount_usd:sum", "*:count", "customer_id:count_distinct"]
            },
            "query_shorthand": {
                "time_dimensions": "column@granularity, e.g. created_at@month",
                "filters": "field:operator:value, e.g. status:eq:completed",
                "order": "field:asc | field:desc"
            },
            "limits": {
                "default_max_rows": self.default_max_rows,
                "policy_max_rows": policy.max_rows,
                "query_timeout_secs": policy.query_timeout_secs
            },
            "governance": {
                "row_level_security": policy.row_filter.is_some(),
                "forced_filters": policy.forced_filters.len(),
                "column_masks": policy.column_masks.len(),
                "allowed_models": policy.allowed_models,
                "allowed_datasources": policy.allowed_datasources
            },
            "mutations_enabled": self.allow_mutations,
            "recommended_flow": [
                "get_capabilities",
                "search_models",
                "get_model",
                "validate_query",
                "run_query"
            ]
        }))
    }

    fn tools_list(&self) -> Result<Value, ToolError> {
        Ok(json!({
            "tools": self.mcp_descriptors()
        }))
    }

    async fn list_models(
        &self,
        ctx: &CallContext,
        input: ListModelsInput,
    ) -> Result<Value, ToolError> {
        let models = self
            .service
            .visible_models(ctx, input.datasource.as_deref())
            .await?;
        Ok(json!({
            "models": models
                .iter()
                .map(|m| json!({
                    "name": m.name,
                    "datasource": m.datasource,
                    "description": m.description,
                    "measure_count": m.measures.len(),
                    "dimension_count": m.dimensions.len(),
                }))
                .collect::<Vec<_>>(),
            "total": models.len(),
            "next_step": "Call get_model with a name to see its full schema and example queries."
        }))
    }

    async fn search_models(
        &self,
        ctx: &CallContext,
        input: SearchModelsInput,
    ) -> Result<Value, ToolError> {
        let limit = input.limit.unwrap_or(10) as usize;
        // Over-fetch: results are dropped for policy *and* for fusion, so asking
        // for exactly `limit` would often return far fewer.
        let pool = limit.saturating_mul(4).max(20);

        let lexical = self
            .service
            .storage()
            .search(&input.q, pool)
            .await
            .map_err(|e| ToolError::new("SEARCH_FAILED", e.to_string()))?;

        let lexical_ranked: Vec<(String, f32)> = lexical
            .iter()
            .map(|h| (h.model_name.clone(), h.score))
            .collect();

        // Dense retrieval is an enhancement, not a dependency. If the embedding
        // provider is down or misconfigured, lexical results are still good, so
        // degrade to lexical rather than failing the search.
        let (dense_ranked, dense_active) = match self.dense_candidates(ctx, &input.q, pool).await {
            Ok(ranked) => {
                let active = !ranked.is_empty();
                (ranked, active)
            }
            Err(e) => {
                tracing::warn!(error = %e, "dense retrieval unavailable, using lexical only");
                (Vec::new(), false)
            }
        };

        // RRF rather than a score blend: BM25 scores are corpus-dependent and
        // cosine is bounded, so the two are not comparable without normalising
        // both. Fusing on rank avoids the question.
        let fused = if dense_ranked.is_empty() {
            lexical_ranked.clone()
        } else {
            crate::embedding::reciprocal_rank_fusion(&lexical_ranked, &dense_ranked, 60.0, 1.0, 1.0)
        };

        let lexical_by_name: HashMap<&str, &graphnight_storage::SearchResult> =
            lexical.iter().map(|h| (h.model_name.as_str(), h)).collect();

        let mut results = Vec::new();
        for (name, score) in fused {
            if results.len() >= limit {
                break;
            }
            // Re-check access on the fused path too. Fusion can promote a model
            // that never appeared in the lexical hits, so filtering only the
            // lexical list would leak it.
            let Ok(model) = self.service.get_model(ctx, &name, None).await else {
                continue;
            };
            let in_lexical = lexical_by_name.contains_key(name.as_str());
            let in_dense = dense_ranked.iter().any(|(n, _)| *n == name);
            let hit = lexical_by_name.get(name.as_str());
            results.push(json!({
                "model": name,
                "datasource": model.datasource.clone(),
                "score": score,
                "matched_fields": hit.map(|h| h.matched_fields.clone()).unwrap_or_default(),
                // A semantic-only hit has no lexical snippet, so fall back to
                // the description rather than returning an empty string.
                "snippet": hit
                    .map(|h| h.snippet.clone())
                    .or_else(|| model.description.clone())
                    .unwrap_or_default(),
                "retrieval": match (in_lexical, in_dense) {
                    (true, true) => "hybrid",
                    (true, false) => "lexical",
                    _ => "semantic",
                },
            }));
        }

        Ok(json!({
            "results": results,
            "count": results.len(),
            "retrieval": if dense_active { "hybrid" } else { "lexical" },
            "next_step": "Call get_model with the best match to see its fields before querying."
        }))
    }

    /// Rank visible models by embedding similarity to `query`.
    ///
    /// Returns an empty list when no embedder is configured, which is the
    /// default: dense retrieval costs a network call per model, so it is opt-in
    /// rather than silently degrading every search.
    async fn dense_candidates(
        &self,
        ctx: &CallContext,
        query: &str,
        pool: usize,
    ) -> Result<Vec<(String, f32)>, ToolError> {
        let Some(embedder) = self.embedder.clone() else {
            return Ok(Vec::new());
        };

        let models = self
            .service
            .visible_models(ctx, None)
            .await
            .map_err(|e| ToolError::new("SEARCH_FAILED", e.to_string()))?;
        if models.is_empty() {
            return Ok(Vec::new());
        }

        let query_vec = embedder
            .embed_one(query)
            .await
            .map_err(|e| ToolError::new("EMBEDDING_FAILED", e.to_string()))?;

        let mut texts = Vec::new();
        for m in &models {
            texts.push(model_document(m));
        }
        let vectors = self.embed_or_cached(&*embedder, &models, &texts).await?;

        let mut scored: Vec<(String, f32)> = models
            .iter()
            .zip(vectors.iter())
            .map(|(m, v)| {
                (
                    m.name.clone(),
                    crate::embedding::cosine_similarity(&query_vec, v),
                )
            })
            .collect();
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(pool);
        Ok(scored)
    }

    /// Embed model documents, reusing a cached vector while the text is unchanged.
    async fn embed_or_cached(
        &self,
        embedder: &dyn crate::embedding::EmbeddingProvider,
        models: &[graphnight_core::models::Model],
        texts: &[String],
    ) -> Result<Vec<Vec<f32>>, ToolError> {
        let mut out: Vec<Option<Vec<f32>>> = vec![None; texts.len()];
        let mut missing: Vec<usize> = Vec::new();

        {
            let cache = self.embed_cache.read().expect("embed cache lock");
            for (i, (model, text)) in models.iter().zip(texts).enumerate() {
                let hash = content_hash(text);
                if let Some((cached_hash, vec)) = cache.get(&model.name) {
                    if *cached_hash == hash {
                        out[i] = Some(vec.clone());
                        continue;
                    }
                }
                missing.push(i);
            }
        }

        if !missing.is_empty() {
            let batch: Vec<String> = missing.iter().map(|&i| texts[i].clone()).collect();
            let fresh = embedder
                .embed(&batch)
                .await
                .map_err(|e| ToolError::new("EMBEDDING_FAILED", e.to_string()))?;
            if fresh.len() != missing.len() {
                return Err(ToolError::new(
                    "EMBEDDING_FAILED",
                    format!(
                        "embedder returned {} vectors for {} inputs",
                        fresh.len(),
                        missing.len()
                    ),
                ));
            }
            let mut cache = self.embed_cache.write().expect("embed cache lock");
            for (slot, vec) in missing.iter().zip(fresh) {
                out[*slot] = Some(vec.clone());
                cache.insert(
                    models[*slot].name.clone(),
                    (content_hash(&texts[*slot]), vec),
                );
            }
        }

        Ok(out.into_iter().map(|v| v.unwrap_or_default()).collect())
    }

    async fn get_model(&self, ctx: &CallContext, input: GetModelInput) -> Result<Value, ToolError> {
        let model = match self
            .service
            .get_model(ctx, &input.name, input.datasource.as_deref())
            .await
        {
            Ok(m) => m,
            Err(e) => return Err(explain_model_error(&self.service, ctx, e, &input.name).await),
        };

        let examples = crate::suggest::example_queries(&model);

        Ok(json!({
            "name": model.name,
            "datasource": model.datasource,
            "description": model.description,
            "measures": model.measures.iter().map(|m| json!({
                "formula": m.formula.expression,
                "aggregation": m.aggregation.sql_function(),
                "label": m.formula.label,
                "format": m.formula.format,
            })).collect::<Vec<_>>(),
            "dimensions": model.dimensions.iter().map(|d| json!({
                "name": d.name,
                "label": d.label,
            })).collect::<Vec<_>>(),
            "time_dimensions": model.time_dimensions.iter().map(|t| json!({
                "dimension": t.dimension,
                "granularity": t.granularity.date_trunc_unit(),
                "label": t.label,
            })).collect::<Vec<_>>(),
            "joins": model.joins.iter().map(|j| json!({
                "name": j.name,
                "model": j.model,
                "join_type": j.join_type.sql_keyword(),
                "alias": j.alias.clone().unwrap_or_else(|| j.name.clone()),
            })).collect::<Vec<_>>(),
            "example_queries": examples,
            "next_step": "Use one of example_queries as a template; call validate_query before run_query."
        }))
    }

    async fn validate_query(
        &self,
        ctx: &CallContext,
        input: ValidateQueryInput,
    ) -> Result<Value, ToolError> {
        let core = self.compile_query(ctx, &input.query).await?;
        match core {
            Compiled::Ok(sql, dialect) => Ok(json!({
                "valid": true,
                "dialect": dialect,
                "sql": sql,
                "note": "Not executed. Call run_query to fetch rows."
            })),
            Compiled::Invalid(err) => Ok(json!({
                "valid": false,
                "error": err
            })),
        }
    }

    async fn run_query(&self, ctx: &CallContext, input: RunQueryInput) -> Result<Value, ToolError> {
        let core = crate::suggest::to_core_query(&input.query)?;
        let options = ExecuteOptions {
            dry_run: input.dry_run.unwrap_or(false),
            explain: input.explain.unwrap_or(false) || input.dry_run.unwrap_or(false),
            max_rows: Some(input.max_rows.unwrap_or(self.default_max_rows as u32) as usize),
        };

        // Surface field-level mistakes as hints before the database ever runs.
        self.check_fields(ctx, &core).await?;

        let outcome = self
            .service
            .execute(ctx, core, options)
            .await
            .map_err(explain_query_error)?;

        Ok(json!({
            "columns": outcome.columns,
            "rows": outcome.data,
            "row_count": outcome.row_count,
            "truncated": outcome.truncated,
            "next_offset": outcome.next_offset,
            "dialect": outcome.dialect,
            "sql": outcome.sql,
            "execution_time_ms": outcome.execution_time_ms,
        }))
    }

    async fn multi_stage_query(
        &self,
        ctx: &CallContext,
        input: MultiStageInput,
    ) -> Result<Value, ToolError> {
        let mut stages = Vec::new();
        for (i, sq) in input.stages.iter().enumerate() {
            // `to_core_query` rejects a stage with no model, so a stage can
            // never silently query "whatever stage_ref names".
            let core = crate::suggest::to_core_query(sq)?;
            stages.push((format!("stage_{i}"), core));
        }
        let dry_run = input.dry_run.unwrap_or(false);
        let outcomes = self
            .service
            .execute_multi_stage(ctx, stages, dry_run)
            .await?;
        Ok(json!({
            "stages": outcomes
                .iter()
                .enumerate()
                .map(|(i, o)| json!({
                    "stage": i,
                    "columns": o.columns,
                    "rows": o.data,
                    "row_count": o.row_count,
                    "sql": o.sql,
                    "dialect": o.dialect,
                }))
                .collect::<Vec<_>>(),
            "dry_run": dry_run,
        }))
    }

    async fn recall_memories(
        &self,
        ctx: &CallContext,
        input: RecallMemoriesInput,
    ) -> Result<Value, ToolError> {
        let storage = self.service.storage();
        let limit = input.limit.unwrap_or(10) as usize;
        let filter = graphnight_storage::MemoryFilter {
            query: input.query.clone(),
            entity: input.entity.clone(),
            // Over-fetch: storage filters lexically, and scope filtering happens
            // below, so asking for exactly `limit` could return fewer than the
            // caller is allowed to see.
            limit: Some(limit.saturating_mul(4).max(20)),
            offset: None,
        };
        let memories = storage
            .list_memories(filter)
            .await
            .map_err(|e| ToolError::new("MEMORY_READ_FAILED", e.to_string()))?;

        // Stored memories are the v1 shape; present them through the v2 record so
        // agents see `kind`/`importance` consistently and older rows degrade
        // gracefully instead of being invisible. `ranked` is what enforces
        // scope isolation -- a caller's own memories plus global ones, never
        // another tenant's.
        let visible = crate::memory::MemoryRecord::ranked(
            memories
                .iter()
                .map(crate::memory::MemoryRecord::from_storage)
                .collect(),
            ctx,
            input.query.as_deref(),
        );
        let visible: Vec<_> = visible.into_iter().take(limit).collect();
        Ok(json!({
            "memories": visible.iter().map(|m| m.to_json()).collect::<Vec<_>>(),
            "count": visible.len(),
        }))
    }

    async fn remember(&self, ctx: &CallContext, input: RememberInput) -> Result<Value, ToolError> {
        // Not a semantic-layer mutation, so it is not behind the operator gate,
        // but it is still a write and needs someone to attribute it to.
        ctx.require_authenticated()?;
        let record = crate::memory::MemoryRecord::from_agent_input(
            &input.text,
            &input.linked_entities,
            input.kind.as_deref(),
            input.importance,
            input.description,
            ctx,
        )
        .map_err(ToolError::from)?;
        let saved = self
            .service
            .storage()
            .save_memory(record.to_storage())
            .await
            .map_err(|e| ToolError::new("MEMORY_WRITE_FAILED", e.to_string()))?;
        Ok(json!({ "id": saved.id, "stored": true }))
    }

    // -- semantic layer mutations ---------------------------------------
    // Each requires admin, which the catalog's mutation gate alone does not
    // imply: an operator enabling the gate grants the *ability*, and the
    // caller's own privileges still decide whether they may use it.

    async fn create_model(
        &self,
        ctx: &CallContext,
        input: CreateModelInput,
    ) -> Result<Value, ToolError> {
        ctx.require_admin()?;
        if input.measures.is_empty() && input.dimensions.is_empty() {
            return Err(ToolError::new(
                "EMPTY_MODEL",
                "a model needs at least one measure or dimension".to_string(),
            )
            .with_hint(
                "Add a measure (e.g. {formula: \"amount\", aggregation: \"sum\"}) or a dimension.",
            ));
        }
        let measures = input
            .measures
            .iter()
            .map(build_measure)
            .collect::<Result<Vec<_>, _>>()?;
        let time_dimensions = input
            .time_dimensions
            .iter()
            .map(build_time_dimension)
            .collect::<Result<Vec<_>, _>>()?;

        let model = graphnight_core::models::Model {
            name: input.name.clone(),
            datasource: input.datasource.clone(),
            description: input.description,
            measures,
            dimensions: input
                .dimensions
                .iter()
                .map(|d| graphnight_core::models::Dimension {
                    name: d.name.clone(),
                    label: d.label.clone(),
                })
                .collect(),
            time_dimensions,
            joins: vec![],
            sql: input.sql.clone().or_else(|| {
                input
                    .base_table
                    .as_ref()
                    .map(|t| format!("SELECT * FROM {t}"))
            }),
            meta: Default::default(),
        };
        let saved = self
            .service
            .storage()
            .create_model(model)
            .await
            .map_err(|e| ToolError::new("MODEL_CREATE_FAILED", e.to_string()))?;
        // Register with the SQL engine too, or the new model would be listed
        // and searchable but fail to generate SQL.
        self.service
            .register_model(ctx, saved.clone())
            .map_err(|e| ToolError::new("MODEL_REGISTER_FAILED", e.to_string()))?;
        Ok(json!({ "name": saved.name, "created": true }))
    }

    async fn update_model(
        &self,
        ctx: &CallContext,
        input: UpdateModelInput,
    ) -> Result<Value, ToolError> {
        ctx.require_admin()?;
        let mut model = self
            .service
            .storage()
            .get_model(&input.name, None)
            .await
            .map_err(|e| ToolError::new("MODEL_NOT_FOUND", e.to_string()))
            .and_then(|opt| {
                opt.ok_or_else(|| {
                    ToolError::new("MODEL_NOT_FOUND", format!("no model named {}", input.name))
                        .with_hint("Call list_models to see what exists.")
                })
            })?;
        if let Some(d) = input.description {
            model.description = Some(d);
        }
        if let Some(m) = input.measures {
            model.measures = m.iter().map(build_measure).collect::<Result<Vec<_>, _>>()?;
        }
        if let Some(d) = input.dimensions {
            model.dimensions = d
                .iter()
                .map(|x| graphnight_core::models::Dimension {
                    name: x.name.clone(),
                    label: x.label.clone(),
                })
                .collect();
        }
        if let Some(t) = input.time_dimensions {
            model.time_dimensions = t
                .iter()
                .map(build_time_dimension)
                .collect::<Result<Vec<_>, _>>()?;
        }
        if input.sql.is_some() {
            model.sql = input.sql;
        } else if let Some(t) = input.base_table {
            model.sql = Some(format!("SELECT * FROM {t}"));
        }
        let saved = self
            .service
            .storage()
            .update_model(&input.name, model)
            .await
            .map_err(|e| ToolError::new("MODEL_UPDATE_FAILED", e.to_string()))?;
        // Re-register so changed measures are reflected in generated SQL.
        self.service
            .register_model(ctx, saved.clone())
            .map_err(|e| ToolError::new("MODEL_REGISTER_FAILED", e.to_string()))?;
        Ok(json!({ "name": saved.name, "updated": true }))
    }

    async fn delete_model(
        &self,
        ctx: &CallContext,
        input: DeleteModelInput,
    ) -> Result<Value, ToolError> {
        ctx.require_admin()?;
        let deleted = self
            .service
            .storage()
            .delete_model(&input.name, input.datasource.as_deref())
            .await
            .map_err(|e| ToolError::new("MODEL_DELETE_FAILED", e.to_string()))?;
        if !deleted {
            return Err(ToolError::new(
                "MODEL_NOT_FOUND",
                format!("no model named {}", input.name),
            )
            .with_hint("Call list_models to see what exists."));
        }
        self.service
            .unregister_model(ctx, &input.name)
            .map_err(|e| ToolError::new("MODEL_UNREGISTER_FAILED", e.to_string()))?;
        Ok(json!({ "name": input.name, "deleted": true }))
    }

    async fn create_datasource(
        &self,
        ctx: &CallContext,
        input: CreateDatasourceInput,
    ) -> Result<Value, ToolError> {
        ctx.require_admin()?;
        let ds = graphnight_core::models::DataSource {
            name: input.name.clone(),
            driver: input.driver.clone(),
            connection_string: input.connection_string,
            description: input.description,
            models: vec![],
            pool_size: input.pool_size,
            meta: Default::default(),
        };
        let saved = self
            .service
            .storage()
            .create_datasource(ds)
            .await
            .map_err(|e| ToolError::new("DATASOURCE_CREATE_FAILED", e.to_string()))?;
        // Never echo connection_string back.
        Ok(json!({
            "name": saved.name,
            "driver": saved.driver,
            "pool_size": saved.pool_size,
            "created": true
        }))
    }

    async fn update_datasource(
        &self,
        ctx: &CallContext,
        input: UpdateDatasourceInput,
    ) -> Result<Value, ToolError> {
        ctx.require_admin()?;
        let mut ds = self
            .service
            .storage()
            .get_datasource(&input.name)
            .await
            .map_err(|e| ToolError::new("DATASOURCE_READ_FAILED", e.to_string()))?
            .ok_or_else(|| {
                ToolError::new(
                    "DATASOURCE_NOT_FOUND",
                    format!("no datasource named {}", input.name),
                )
            })?;
        if let Some(d) = input.description {
            ds.description = Some(d);
        }
        if let Some(c) = input.connection_string {
            ds.connection_string = c;
        }
        if let Some(p) = input.pool_size {
            ds.pool_size = Some(p);
        }
        self.service
            .storage()
            .update_datasource(&input.name, ds)
            .await
            .map_err(|e| ToolError::new("DATASOURCE_UPDATE_FAILED", e.to_string()))?;
        Ok(json!({ "name": input.name, "updated": true }))
    }

    async fn delete_datasource(
        &self,
        ctx: &CallContext,
        input: DeleteDatasourceInput,
    ) -> Result<Value, ToolError> {
        ctx.require_admin()?;
        let models = self
            .service
            .storage()
            .list_models(None)
            .await
            .map_err(|e| ToolError::new("DATASOURCE_DELETE_FAILED", e.to_string()))?;
        let orphans: Vec<&str> = models
            .iter()
            .filter(|m| m.datasource == input.name)
            .map(|m| m.name.as_str())
            .collect();
        if !orphans.is_empty() {
            return Err(ToolError::new(
                "DATASOURCE_IN_USE",
                format!(
                    "{} model(s) still read from {}: {}",
                    orphans.len(),
                    input.name,
                    orphans.join(", ")
                ),
            )
            .with_hint("Delete or repoint those models first, so you do not strand them."));
        }
        let deleted = self
            .service
            .storage()
            .delete_datasource(&input.name)
            .await
            .map_err(|e| ToolError::new("DATASOURCE_DELETE_FAILED", e.to_string()))?;
        if !deleted {
            return Err(ToolError::new(
                "DATASOURCE_NOT_FOUND",
                format!("no datasource named {}", input.name),
            ));
        }
        Ok(json!({ "name": input.name, "deleted": true }))
    }

    /// Reject unknown fields with a did-you-mean hint, before executing.
    async fn check_fields(&self, ctx: &CallContext, query: &CoreQuery) -> Result<(), ToolError> {
        let Some(model_name) = query
            .name
            .as_ref()
            .or_else(|| query.source_model.as_ref().map(|s| &s.model))
        else {
            return Ok(());
        };
        let model = match self.service.get_model(ctx, model_name, None).await {
            Ok(m) => m,
            Err(e) => return Err(explain_model_error(&self.service, ctx, e, model_name).await),
        };

        if let Some(limit) = query.limit {
            let policy = ctx.session_policy();
            let cap = policy
                .max_rows
                .unwrap_or(crate::governance::DEFAULT_MAX_ROWS);
            if limit > cap {
                return Err(ToolError::new(
                    "ROW_LIMIT_EXCEEDED",
                    format!("requested limit {limit} exceeds the permitted maximum {cap}"),
                )
                .with_hint(format!(
                    "Use limit: {cap} or lower. The cap is enforced server-side regardless."
                )));
            }
        }

        let known = crate::suggest::field_names(&model);
        for f in &query.filters {
            if !known.contains(&f.field.as_str()) {
                return Err(crate::suggest::unknown_field_error(
                    &f.field, &known, "filter",
                ));
            }
        }
        for d in &query.dimensions {
            if !known.contains(&d.name.as_str()) {
                return Err(crate::suggest::unknown_field_error(
                    &d.name,
                    &known,
                    "dimension",
                ));
            }
        }
        Ok(())
    }

    /// Compile a query for validation, returning either SQL or a structured error.
    async fn compile_query(
        &self,
        ctx: &CallContext,
        query: &crate::tools::ToolQuery,
    ) -> Result<Compiled, ToolError> {
        let core = match crate::suggest::to_core_query(query) {
            Ok(c) => c,
            Err(e) => return Ok(Compiled::Invalid(e)),
        };
        if let Err(e) = self.check_fields(ctx, &core).await {
            return Ok(Compiled::Invalid(e));
        }
        match self
            .service
            .execute(ctx, core, ExecuteOptions::dry_run())
            .await
        {
            Ok(o) => Ok(Compiled::Ok(o.sql.unwrap_or_default(), o.dialect)),
            Err(e) => Ok(Compiled::Invalid(ToolError::from(e))),
        }
    }
}

/// Reclassify generation failures that the SQL layer reported generically, so
/// an agent routes to the right remedy (look up a model vs. fix a field).
fn explain_query_error(e: ServiceError) -> ToolError {
    let mut err = ToolError::from(e);
    if err.code == "INVALID_QUERY" {
        if let Some(hint) = &err.hint {
            if hint.contains("Model not found") {
                err.code = "MODEL_NOT_FOUND".to_string();
            }
        }
    }
    err
}

/// Enrich a model-access failure so the agent can self-correct.
///
/// Two rules, both deliberate:
/// - On a *not found* error we list the models this caller can see. That set is
///   already available to them via `list_models`, so it discloses nothing new,
///   and it saves a wasted turn on a typo.
/// - On a *policy violation* we never enumerate anything. The denial is the
///   answer; enumerating what was denied would turn a convenient error message
///   into a discovery oracle.
async fn explain_model_error(
    service: &QueryService,
    ctx: &CallContext,
    e: ServiceError,
    requested: &str,
) -> ToolError {
    let mut err = ToolError::from(e);
    match err.code.as_str() {
        "MODEL_NOT_FOUND" => {
            let available = service
                .visible_models(ctx, None)
                .await
                .map(|models| models.into_iter().map(|m| m.name).collect::<Vec<_>>())
                .unwrap_or_default();
            err.hint = Some(if available.is_empty() {
                format!(
                    "No models are visible to this caller. Check the spelling of {requested:?}."
                )
            } else {
                format!(
                    "Available models: {}. Call get_model with one of these names.",
                    available.join(", ")
                )
            });
        }
        "POLICY_VIOLATION" => {
            err.hint = Some(
                "This caller is not permitted to access that model. \
                 Do not retry with variations; report the restriction."
                    .to_string(),
            );
        }
        _ => {}
    }
    err
}

enum Compiled {
    Ok(String, String),
    Invalid(ToolError),
}

fn parse_args<T: serde::de::DeserializeOwned>(args: Value) -> Result<T, ToolError> {
    serde_json::from_value(args).map_err(|e| {
        ToolError::new(
            "INVALID_ARGUMENT",
            format!("could not parse arguments: {e}"),
        )
        .with_hint("Check the tool's inputSchema for required fields and types.")
    })
}

/// Build a measure, rejecting an unknown aggregation with a usable message.
fn build_measure(input: &MeasureInput) -> Result<graphnight_core::models::Measure, ToolError> {
    use graphnight_core::models::AggregationType as Agg;
    let aggregation = match input.aggregation.to_ascii_lowercase().as_str() {
        "sum" => Agg::Sum,
        "avg" | "average" => Agg::Avg,
        "count" => Agg::Count,
        "min" => Agg::Min,
        "max" => Agg::Max,
        "count_distinct" | "countdistinct" => Agg::CountDistinct,
        other => {
            return Err(ToolError::new(
                "UNKNOWN_AGGREGATION",
                format!("unknown aggregation: {other}"),
            )
            .with_hint("Use one of: sum, avg, count, min, max, count_distinct."))
        }
    };
    Ok(graphnight_core::models::Measure {
        formula: graphnight_core::models::Formula {
            expression: input.formula.clone(),
            label: input.label.clone(),
            format: input.format.clone(),
        },
        aggregation,
    })
}

fn build_time_dimension(
    input: &TimeDimensionInput,
) -> Result<graphnight_core::models::TimeDimension, ToolError> {
    use graphnight_core::models::TimeGranularity as G;
    let granularity = match input.granularity.to_ascii_lowercase().as_str() {
        "second" => G::Second,
        "minute" => G::Minute,
        "hour" => G::Hour,
        "day" => G::Day,
        "week" => G::Week,
        "month" => G::Month,
        "quarter" => G::Quarter,
        "year" => G::Year,
        other => {
            return Err(ToolError::new(
                "UNKNOWN_GRANULARITY",
                format!("unknown time granularity: {other}"),
            )
            .with_hint("Use one of: second, minute, hour, day, week, month, quarter, year."))
        }
    };
    Ok(graphnight_core::models::TimeDimension {
        dimension: input.dimension.clone(),
        granularity,
        label: input.label.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{admin_context, open_context, orders_only_context, test_service};
    use serde_json::json;

    fn catalog(allow_mutations: bool) -> ToolCatalog {
        ToolCatalog::new(test_service()).with_mutations(allow_mutations)
    }

    #[tokio::test]
    async fn semantic_mutations_are_hidden_by_default() {
        let c = catalog(false);
        let names: Vec<&str> = c.specs().iter().map(|s| s.name).collect();
        for gated in [
            "create_model",
            "update_model",
            "delete_model",
            "create_datasource",
            "update_datasource",
            "delete_datasource",
        ] {
            assert!(!names.contains(&gated), "{gated} must be hidden: {names:?}");
        }
        assert!(c.get("create_model").is_none());
    }

    #[tokio::test]
    async fn gated_tool_call_reports_mutations_disabled() {
        let c = catalog(false);
        let err = c
            .call(&admin_context(), "create_model", json!({"name": "x"}))
            .await
            .unwrap_err();
        // Hidden tools surface as UNKNOWN_TOOL so a model is never told the
        // semantic layer is writable when it is not.
        assert_eq!(err.code, "UNKNOWN_TOOL");
        assert!(err.hint.unwrap().contains("tools_list"));
    }

    #[tokio::test]
    async fn open_gate_still_requires_admin() {
        let c = catalog(true);
        let mut args = json!({"name": "widgets", "datasource": "warehouse",
                              "measures": [{"formula": "*", "aggregation": "count"}]});
        args["datasource"] = json!("warehouse");
        let err = c
            .call(&open_context(), "create_model", args)
            .await
            .unwrap_err();
        assert_eq!(err.code, "POLICY_VIOLATION");
        assert!(err.message.contains("Admin"), "{}", err.message);
    }

    #[tokio::test]
    async fn admin_can_create_a_model_and_it_becomes_queryable() {
        let service = test_service();
        let c = ToolCatalog::new(service.clone()).with_mutations(true);
        let out = c
            .call(
                &admin_context(),
                "create_model",
                json!({
                    "name": "widgets",
                    "datasource": "warehouse",
                    "description": "test",
                    "base_table": "public.widgets",
                    "measures": [{"formula": "amount", "aggregation": "sum"}],
                    "dimensions": [{"name": "region"}],
                    "time_dimensions": [{"dimension": "created_at", "granularity": "day"}]
                }),
            )
            .await
            .expect("create_model should succeed");
        assert_eq!(out["created"], true);
        assert_eq!(out["name"], "widgets");

        let stored = service.storage().get_model("widgets", None).await.unwrap();
        assert!(stored.is_some(), "model should be persisted");
        assert_eq!(stored.unwrap().dimensions[0].name, "region");
    }

    #[tokio::test]
    async fn create_model_rejects_an_empty_model() {
        let c = catalog(true);
        let err = c
            .call(
                &admin_context(),
                "create_model",
                json!({"name": "empty", "datasource": "warehouse"}),
            )
            .await
            .unwrap_err();
        assert_eq!(err.code, "EMPTY_MODEL");
    }

    #[tokio::test]
    async fn unknown_aggregation_names_the_valid_set() {
        let c = catalog(true);
        let err = c
            .call(
                &admin_context(),
                "create_model",
                json!({
                    "name": "bad", "datasource": "warehouse",
                    "measures": [{"formula": "x", "aggregation": "medianish"}]
                }),
            )
            .await
            .unwrap_err();
        assert_eq!(err.code, "UNKNOWN_AGGREGATION");
        let hint = err.hint.unwrap();
        assert!(hint.contains("count_distinct"), "{hint}");
    }

    #[tokio::test]
    async fn datasource_connection_string_is_never_echoed() {
        let c = catalog(true);
        let out = c
            .call(
                &admin_context(),
                "create_datasource",
                json!({
                    "name": "extra", "driver": "postgres",
                    "connection_string": "postgres://user:hunter2@host/db"
                }),
            )
            .await
            .expect("create_datasource should succeed");
        let rendered = out.to_string();
        assert!(!rendered.contains("hunter2"), "secret leaked: {rendered}");
        assert!(!rendered.contains("postgres://"), "uri leaked: {rendered}");
        assert_eq!(out["name"], "extra");
    }

    #[tokio::test]
    async fn deleting_a_datasource_in_use_is_refused_and_names_the_models() {
        let c = catalog(true);
        let err = c
            .call(
                &admin_context(),
                "delete_datasource",
                json!({"name": "warehouse"}),
            )
            .await
            .unwrap_err();
        assert_eq!(err.code, "DATASOURCE_IN_USE");
        // The message must tell the model what is stranded, not just fail.
        assert!(err.message.contains("orders"), "{}", err.message);
    }

    #[tokio::test]
    async fn update_model_changes_only_supplied_fields() {
        let service = test_service();
        let c = ToolCatalog::new(service.clone()).with_mutations(true);
        c.call(
            &admin_context(),
            "update_model",
            json!({"name": "orders", "description": "now documented"}),
        )
        .await
        .expect("update_model should succeed");
        let stored = service
            .storage()
            .get_model("orders", None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.description.as_deref(), Some("now documented"));
        // Measures were not in the payload, so they must survive.
        assert!(!stored.measures.is_empty(), "measures must be preserved");
    }

    #[tokio::test]
    async fn remember_requires_an_identity() {
        let c = catalog(false);
        let mut anon = CallContext::anonymous();
        anon.auth_required = true;
        let err = c
            .call(&anon, "remember", json!({"text": "x"}))
            .await
            .unwrap_err();
        assert_eq!(err.code, "UNAUTHENTICATED");
    }

    #[tokio::test]
    async fn remember_does_not_need_the_mutation_gate() {
        let service = test_service();
        let c = ToolCatalog::new(service.clone());
        assert!(
            c.get("remember").is_some(),
            "memory writes are not semantic mutations"
        );
        let mut ctx = CallContext {
            user_id: Some("alice".into()),
            auth_required: true,
            ..CallContext::default()
        };
        ctx.policy = Some(crate::testing::orders_only_context().session_policy());
        let out = c
            .call(&ctx, "remember", json!({"text": "revenue spikes in Q4"}))
            .await
            .expect("remember should succeed");
        assert_eq!(out["stored"], true);
    }

    #[tokio::test]
    async fn recall_does_not_leak_another_tenants_memories() {
        let service = test_service();
        let c = ToolCatalog::new(service.clone());
        let mut tenant_a = CallContext {
            user_id: Some("alice".into()),
            tenant_id: Some("tenant-a".into()),
            auth_required: true,
            ..CallContext::default()
        };
        tenant_a.policy = Some(crate::testing::orders_only_context().session_policy());
        c.call(
            &tenant_a,
            "remember",
            json!({"text": "tenant a secret", "importance": 10}),
        )
        .await
        .unwrap();

        let mut tenant_b = CallContext {
            user_id: Some("bob".into()),
            tenant_id: Some("tenant-b".into()),
            auth_required: true,
            ..CallContext::default()
        };
        tenant_b.policy = Some(crate::testing::orders_only_context().session_policy());
        let out = c
            .call(
                &tenant_b,
                "recall_memories",
                json!({"query": "tenant a secret"}),
            )
            .await
            .unwrap();
        let memories = out["memories"].as_array().unwrap();
        assert!(
            memories.is_empty(),
            "tenant b saw tenant a's memory: {memories:?}"
        );
    }

    /// Embedder that maps text to a vector via a caller-supplied function, so a
    /// test can make "revenue" and "turnover" land close together.
    struct FakeEmbedder {
        dims: usize,
        calls: std::sync::Arc<std::sync::atomic::AtomicUsize>,
        /// Substring -> unit-ish vector, so tests control similarity by wording.
        table: Vec<(String, Vec<f32>)>,
    }

    #[async_trait::async_trait]
    impl crate::embedding::EmbeddingProvider for FakeEmbedder {
        fn model(&self) -> &str {
            "fake"
        }
        fn dimensions(&self) -> usize {
            self.dims
        }
        async fn embed(
            &self,
            texts: &[String],
        ) -> Result<Vec<Vec<f32>>, crate::embedding::EmbeddingError> {
            self.calls
                .fetch_add(texts.len(), std::sync::atomic::Ordering::SeqCst);
            Ok(texts
                .iter()
                .map(|t| {
                    let lower = t.to_lowercase();
                    self.table
                        .iter()
                        .find(|(k, _)| lower.contains(k.as_str()))
                        .map(|(_, v)| v.clone())
                        // Unknown text gets an orthogonal vector.
                        .unwrap_or_else(|| vec![0.0; self.dims])
                })
                .collect())
        }
    }

    fn embedder_for(
        words: &[(&str, &[f32])],
    ) -> (Arc<FakeEmbedder>, Arc<std::sync::atomic::AtomicUsize>) {
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let e = Arc::new(FakeEmbedder {
            dims: 2,
            calls: calls.clone(),
            table: words
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_vec()))
                .collect(),
        });
        (e, calls)
    }

    async fn catalog_with_embedder(
        e: Option<Arc<dyn crate::embedding::EmbeddingProvider>>,
    ) -> ToolCatalog {
        let mut c = ToolCatalog::new(test_service());
        if let Some(e) = e {
            c = c.with_embedder(Some(e));
        }
        c
    }

    #[tokio::test]
    async fn search_is_lexical_only_without_an_embedder() {
        let c = catalog_with_embedder(None).await;
        let out = c
            .call(&open_context(), "search_models", json!({ "q": "orders" }))
            .await
            .expect("search should succeed");
        assert_eq!(out["retrieval"], "lexical");
        for r in out["results"].as_array().unwrap() {
            assert_eq!(r["retrieval"], "lexical");
        }
    }

    /// The point of hybrid: a model whose wording does not match the query at
    /// all, but whose meaning does, still surfaces.
    #[tokio::test]
    async fn hybrid_search_finds_a_model_lexical_search_misses() {
        let (e, _calls) = embedder_for(&[("turnover", &[1.0, 0.0]), ("orders", &[0.0, 1.0])]);
        let c = catalog_with_embedder(Some(e)).await;

        let out = c
            .call(&open_context(), "search_models", json!({ "q": "turnover" }))
            .await
            .expect("search should succeed");
        assert_eq!(out["retrieval"], "hybrid");
        let models: Vec<&str> = out["results"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["model"].as_str().unwrap())
            .collect();
        assert!(
            models.contains(&"orders"),
            "semantic match should be retrieved: {models:?}"
        );
    }

    #[tokio::test]
    async fn hybrid_search_never_returns_a_model_the_policy_denies() {
        // `orders` and `customers` are the fixtures; only `orders` is visible.
        let (e, _calls) = embedder_for(&[("orders", &[1.0, 0.0])]);
        let c = catalog_with_embedder(Some(e)).await;

        let out = c
            .call(
                &orders_only_context(),
                "search_models",
                json!({ "q": "orders" }),
            )
            .await
            .expect("search should succeed");
        for r in out["results"].as_array().unwrap() {
            assert_eq!(r["model"], "orders", "fusion leaked a denied model: {r}");
        }
    }

    #[tokio::test]
    async fn model_embeddings_are_cached_between_searches() {
        let (e, calls) = embedder_for(&[("orders", &[0.0, 1.0])]);
        let c = catalog_with_embedder(Some(e)).await;

        c.call(&open_context(), "search_models", json!({ "q": "orders" }))
            .await
            .unwrap();
        let after_first = calls.load(std::sync::atomic::Ordering::SeqCst);
        assert!(after_first > 0, "first search should embed");

        c.call(&open_context(), "search_models", json!({ "q": "orders" }))
            .await
            .unwrap();
        let after_second = calls.load(std::sync::atomic::Ordering::SeqCst);

        // Only the query vector is recomputed; the model vectors are reused.
        assert_eq!(
            after_second - after_first,
            1,
            "model embeddings should be cached, not recomputed"
        );
    }

    #[tokio::test]
    async fn a_failing_embedder_does_not_break_search() {
        let c = catalog_with_embedder(Some(Arc::new(crate::embedding::NullEmbedder))).await;
        // NullEmbedder errors, so the dense leg is skipped and lexical results
        // still come back rather than the whole tool failing.
        let out = c
            .call(&open_context(), "search_models", json!({ "q": "orders" }))
            .await
            .expect("lexical fallback should keep search working");
        assert!(!out["results"].as_array().unwrap().is_empty());
    }
}
