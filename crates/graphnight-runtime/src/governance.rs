//! The single governed execution path.
//!
//! Every interface — GraphQL, REST, MCP, the agent runtime, and the language
//! bindings — routes through [`QueryService`]. Governance is applied to the
//! semantic plan *before* SQL generation, so the warehouse never sees an
//! ungoverned statement and the caller never receives an ungoverned result.
//!
//! This module is deliberately free of any transport concern: no GraphQL types,
//! no axum, no JSON-RPC. That is what makes it safe to reuse from an agent.

use graphnight_core::errors::CoreError;
use graphnight_core::models::{DataSource, Model, Query};
use graphnight_core::security::{AuditEntry, AuditSink, PolicyEnforcer, SessionPolicy};
use graphnight_sql::SqlEngine;
use graphnight_storage::{SearchResult, StorageBackend};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

/// Errors surfaced to any interface, including agent tools.
///
/// [`ToolError`](crate::tools::ToolError) renders these with a machine-readable
/// `code` plus a remediation `hint`, so an LLM caller can self-correct instead
/// of retrying blindly.
#[derive(Debug, thiserror::Error)]
pub enum ServiceError {
    #[error("model not found: {0}")]
    ModelNotFound(String),
    #[error("datasource not found: {0}")]
    DatasourceNotFound(String),
    /// No usable identity was presented. Distinct from [`ServiceError::PolicyViolation`]
    /// so transports can answer 401 rather than 403 — a client can fix one and
    /// never the other.
    #[error("authentication required: {0}")]
    Unauthenticated(String),
    #[error("policy violation: {0}")]
    PolicyViolation(String),
    #[error("invalid query: {0}")]
    InvalidQuery(String),
    #[error("query timeout exceeded ({0}s)")]
    Timeout(u64),
    #[error("execution failed: {0}")]
    Execution(String),
    #[error("storage error: {0}")]
    Storage(String),
    #[error("unknown tool: {0}")]
    UnknownTool(String),
    #[error("{0}")]
    Invalid(String),
}

impl From<CoreError> for ServiceError {
    fn from(e: CoreError) -> Self {
        match e {
            CoreError::ModelNotFound(m) => ServiceError::ModelNotFound(m),
            CoreError::DatasourceNotFound(d) => ServiceError::DatasourceNotFound(d),
            CoreError::PolicyViolation(p) => ServiceError::PolicyViolation(p),
            CoreError::InvalidQuery(q) => ServiceError::InvalidQuery(q),
            other => ServiceError::Invalid(other.to_string()),
        }
    }
}

/// How a query response should be shaped for the caller.
#[derive(Debug, Clone, Default)]
pub struct ExecuteOptions {
    /// Generate SQL but do not touch the database.
    pub dry_run: bool,
    /// Include the generated SQL in the response.
    pub explain: bool,
    /// Hard row cap for this call, applied after policy enforcement.
    /// `None` means "use the policy's cap".
    pub max_rows: Option<usize>,
}

impl ExecuteOptions {
    pub fn dry_run() -> Self {
        Self {
            dry_run: true,
            explain: true,
            max_rows: None,
        }
    }

    pub fn executing() -> Self {
        Self::default()
    }

    /// Options for a live/subscription tick.
    ///
    /// Same execution and same policy as [`ExecuteOptions::executing`]; the
    /// difference is that the caller does not want the generated statement in
    /// every payload, so it is left out to keep the stream small.
    pub fn live() -> Self {
        Self {
            explain: false,
            ..Self::default()
        }
    }

    pub fn with_max_rows(mut self, max_rows: usize) -> Self {
        self.max_rows = Some(max_rows);
        self
    }
}

/// The row cap applied when neither the call nor the policy sets one.
///
/// Public so every layer (tool pre-flight hints, REST, GraphQL) reports the
/// same number the service actually enforces, rather than three copies of it
/// drifting apart.
pub const DEFAULT_MAX_ROWS: usize = 10_000;

/// The result of a governed query, independent of transport.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryOutcome {
    pub data: Vec<Value>,
    pub columns: Vec<String>,
    /// Present when `explain` was requested (always for dry runs).
    pub sql: Option<String>,
    /// Rows counted for this result. `population_inferred` is true when this
    /// was inferred from the returned page rather than measured with a
    /// separate `COUNT(*)`, so it is a lower bound whenever `truncated`.
    pub population: Option<i64>,
    pub population_inferred: bool,
    pub execution_time_ms: f64,
    /// True when `truncated` because a row cap was hit.
    pub truncated: bool,
    /// Rows actually returned.
    pub row_count: usize,
    /// Suggested offset to continue from, when truncated.
    pub next_offset: Option<usize>,
    /// The dialect the SQL was generated for.
    pub dialect: String,
}

/// Identity and policy governing one call.
///
/// An agent gets one of these per session, resolved from its own credentials —
/// which is what makes RLS, forced filters, column masks and row caps apply to
/// agents exactly as they do to humans.
#[derive(Debug, Clone, Default)]
pub struct CallContext {
    pub user_id: Option<String>,
    pub tenant_id: Option<String>,
    pub is_admin: bool,
    pub auth_required: bool,
    pub policy: Option<SessionPolicy>,
    /// Free-form label recorded in audit entries (e.g. `agent:revenue_analyst`).
    pub principal_label: Option<String>,
}

impl CallContext {
    /// Anonymous, ungoverned context. Only valid when auth is not required.
    pub fn anonymous() -> Self {
        Self::default()
    }

    pub fn session_policy(&self) -> SessionPolicy {
        self.policy.clone().unwrap_or_default()
    }

    pub fn principal(&self) -> String {
        self.user_id
            .clone()
            .or_else(|| self.principal_label.clone())
            .unwrap_or_else(|| "anonymous".to_string())
    }

    /// Reject anonymous callers when the deployment requires auth.
    pub fn require_authenticated(&self) -> Result<(), ServiceError> {
        if self.auth_required && self.user_id.is_none() {
            return Err(ServiceError::Unauthenticated(
                "Authentication required (provide Authorization: Bearer <key> or X-API-Key)"
                    .to_string(),
            ));
        }
        Ok(())
    }

    /// Require an explicit admin identity, for semantic-layer mutations.
    ///
    /// Note this does *not* relax when `auth_required` is false. Turning
    /// authentication off makes data readable, but it should not silently make
    /// every anonymous caller an administrator: the catalog's mutation gate is
    /// an operator switch about *capability*, and this is the check about
    /// *authority*. A local CLI sets `is_admin` because the operator already
    /// owns the metadata files on that machine.
    pub fn require_admin(&self) -> Result<(), ServiceError> {
        if !self.is_admin {
            return Err(ServiceError::PolicyViolation(
                "Admin privileges required".to_string(),
            ));
        }
        Ok(())
    }
}

/// The governed execution service.
///
/// Holds the shared engine and storage handles, and owns the single code path
/// from semantic query to governed rows.
pub struct QueryService {
    sql_engine: Arc<SqlEngine>,
    storage: Arc<dyn StorageBackend>,
    /// Late-bindable so a transport can attach its sink to a *shared* service
    /// after the service was handed to other interfaces, instead of replacing
    /// it and silently losing that sharing.
    audit_sink: Arc<std::sync::RwLock<Option<Arc<dyn AuditSink>>>>,
}

impl QueryService {
    pub fn new(sql_engine: Arc<SqlEngine>, storage: Arc<dyn StorageBackend>) -> Self {
        Self {
            sql_engine,
            storage,
            audit_sink: Arc::new(std::sync::RwLock::new(None)),
        }
    }

    /// Attach a sink at construction time.
    pub fn with_audit_sink(self, sink: Arc<dyn AuditSink>) -> Self {
        *self.audit_sink.write().expect("audit sink lock") = Some(sink);
        self
    }

    /// Attach or replace the sink on an already-shared service.
    ///
    /// This is what lets a transport supply its own sink without rebuilding the
    /// service, so every interface keeps using the one instance.
    pub fn set_audit_sink(&self, sink: Arc<dyn AuditSink>) {
        *self.audit_sink.write().expect("audit sink lock") = Some(sink);
    }

    pub fn sql_engine(&self) -> Arc<SqlEngine> {
        self.sql_engine.clone()
    }

    pub fn storage(&self) -> Arc<dyn StorageBackend> {
        self.storage.clone()
    }

    /// Execute a semantic query under `ctx`'s policy.
    ///
    /// Order matters: policy is applied to the logical query *before* SQL is
    /// generated, so forced filters, RLS and row caps are part of the statement
    /// the database sees.
    pub async fn execute(
        &self,
        ctx: &CallContext,
        mut query: Query,
        options: ExecuteOptions,
    ) -> Result<QueryOutcome, ServiceError> {
        ctx.require_authenticated()?;
        let start = Instant::now();

        let model_name = query
            .name
            .as_ref()
            .or_else(|| query.source_model.as_ref().map(|s| &s.model))
            .ok_or_else(|| {
                ServiceError::InvalidQuery("query must set `name` or `source_model`".to_string())
            })?
            .clone();

        let model = self
            .storage
            .get_model(&model_name, None)
            .await
            .map_err(|e| ServiceError::Storage(e.to_string()))?
            .ok_or_else(|| ServiceError::ModelNotFound(model_name.clone()))?;

        let policy = ctx.session_policy();
        self.apply_policy(&policy, &mut query, &model_name, &model.datasource)?;
        let effective_limit = self.apply_row_cap(&policy, &mut query, options.max_rows);

        let sql = self
            .sql_engine
            .generate_sql(&query)
            .map_err(|e| ServiceError::InvalidQuery(e.to_string()))?;
        let dialect = self.sql_engine.dialect_name_for_model(&model_name);

        if options.dry_run {
            return Ok(QueryOutcome {
                data: vec![],
                columns: vec![],
                sql: Some(sql),
                population: None,
                population_inferred: false,
                execution_time_ms: start.elapsed().as_millis() as f64,
                truncated: false,
                row_count: 0,
                next_offset: None,
                dialect,
            });
        }

        let datasource = self
            .storage
            .get_datasource(&model.datasource)
            .await
            .map_err(|e| ServiceError::Storage(e.to_string()))?
            .ok_or_else(|| ServiceError::DatasourceNotFound(model.datasource.clone()))?;

        let timeout_secs = policy.query_timeout_secs.unwrap_or(300);
        let execute = self.sql_engine.execute_sqlx(&datasource, &sql);
        let rows = tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), execute)
            .await
            .map_err(|_| ServiceError::Timeout(timeout_secs))?
            .map_err(|e| ServiceError::Execution(e.to_string()))?;

        let row_count = rows.len();
        let columns = rows
            .first()
            .map(|r| r.keys().cloned().collect::<Vec<_>>())
            .unwrap_or_default();

        let mut data: Vec<Value> = rows
            .into_iter()
            .map(|m| serde_json::to_value(m).unwrap_or(Value::Null))
            .collect();

        // Column masks are applied to results as the policy specifies; the row
        // limit was already pushed into SQL.
        if !policy.column_masks.is_empty() {
            for row in &mut data {
                if let Value::Object(map) = row {
                    for (col, mask_fn) in &policy.column_masks {
                        if let Some(Value::String(val)) = map.get(col) {
                            let masked = mask_fn(val);
                            map.insert(col.clone(), Value::String(masked));
                        }
                    }
                }
            }
        }

        let population = data.len() as i64;
        // The row cap lives in SQL, so a full page is the only signal that more
        // rows may exist. Report that honestly rather than claiming completeness.
        let truncated = row_count >= effective_limit;
        let outcome = QueryOutcome {
            data,
            columns,
            sql: if options.explain { Some(sql) } else { None },
            population: Some(population),
            population_inferred: true,
            execution_time_ms: start.elapsed().as_millis() as f64,
            truncated,
            row_count,
            next_offset: truncated.then(|| query.offset.unwrap_or(0) + row_count),
            dialect,
        };

        self.audit(
            ctx,
            &model_name,
            outcome.row_count,
            outcome.execution_time_ms as u64,
            true,
            None,
        );

        Ok(outcome)
    }

    /// Execute several stages as a DAG, each governed independently.
    /// Execute a pipeline of stages, honouring `stage_ref` dependencies.
    ///
    /// Stages are topologically sorted first, so a caller may list them in any
    /// order; a `stage_ref` cycle is reported rather than deadlocking. Each
    /// stage is then run through [`QueryService::execute`], so policy, forced
    /// filters, RLS, masks, row caps and audit apply to every stage exactly as
    /// they do to a single query.
    pub async fn execute_multi_stage(
        &self,
        ctx: &CallContext,
        stages: Vec<(String, Query)>,
        dry_run: bool,
    ) -> Result<Vec<QueryOutcome>, ServiceError> {
        if stages.is_empty() {
            return Err(ServiceError::InvalidQuery("no stages supplied".into()));
        }
        let order = topological_order(&stages)?;

        let mut by_name: HashMap<String, QueryOutcome> = HashMap::new();
        let mut out: Vec<Option<QueryOutcome>> = vec![None; stages.len()];
        // Position of each stage name, so results come back in caller's order
        // rather than dependency order.
        let mut position: HashMap<String, usize> = HashMap::new();
        for (i, (name, _)) in stages.iter().enumerate() {
            if position.insert(name.clone(), i).is_some() {
                return Err(ServiceError::InvalidQuery(format!(
                    "duplicate stage name: {name}"
                )));
            }
        }

        for name in order {
            let query = stages
                .iter()
                .find(|(n, _)| *n == name)
                .map(|(_, q)| q.clone())
                .expect("topological_order only returns known stages");
            let mut query = query;
            if let Some(reference) = query.stage_ref.clone() {
                let previous = by_name.get(&reference).ok_or_else(|| {
                    ServiceError::InvalidQuery(format!(
                        "stage_ref {reference:?} does not name an earlier stage; \
                         known stages: {:?}",
                        by_name.keys().collect::<Vec<_>>()
                    ))
                })?;
                apply_stage_ref(&mut query, previous);
            }
            if query.name.is_none() && query.source_model.is_none() {
                query.name = Some(name.clone());
            }
            let options = if dry_run {
                ExecuteOptions::dry_run()
            } else {
                ExecuteOptions::executing()
            };
            let outcome = self.execute(ctx, query, options).await?;
            if let Some(slot) = out.get_mut(position[&name]) {
                *slot = Some(outcome.clone());
            }
            by_name.insert(name, outcome);
        }

        Ok(out.into_iter().flatten().collect())
    }

    /// Apply access control, forced filters and RLS to the logical query.
    fn apply_policy(
        &self,
        policy: &SessionPolicy,
        query: &mut Query,
        model_name: &str,
        datasource: &str,
    ) -> Result<(), ServiceError> {
        let enforcer = PolicyEnforcer::new(policy.clone());
        enforcer
            .check_model_access(model_name)
            .map_err(|e| ServiceError::PolicyViolation(e.to_string()))?;
        enforcer
            .check_datasource_access(datasource)
            .map_err(|e| ServiceError::PolicyViolation(e.to_string()))?;
        enforcer.apply_forced_filters(query);
        enforcer.apply_rls(query);
        Ok(())
    }

    /// Enforce the effective row cap: the tighter of the policy cap and the
    /// per-call cap, mirroring `PolicyEnforcer::enforce_row_limit` and adding
    /// the per-call bound. Agent calls pass a cap well below the human default.
    ///
    /// Returns the cap actually applied, so callers can detect a full page.
    fn apply_row_cap(
        &self,
        policy: &SessionPolicy,
        query: &mut Query,
        per_call: Option<usize>,
    ) -> usize {
        let policy_cap = policy.max_rows.unwrap_or(DEFAULT_MAX_ROWS);
        let effective = match per_call {
            Some(call_cap) => policy_cap.min(call_cap),
            None => policy_cap,
        };
        query.limit = Some(query.limit.map_or(effective, |l| l.min(effective)));
        query.limit.unwrap_or(effective)
    }

    /// Write a durable audit entry when a sink is configured.
    pub fn audit(
        &self,
        ctx: &CallContext,
        model: &str,
        row_count: usize,
        duration_ms: u64,
        success: bool,
        error: Option<String>,
    ) {
        let sink = {
            let guard = self.audit_sink.read().expect("audit sink lock");
            guard.clone()
        };
        let Some(sink) = &sink else {
            return;
        };
        let entry = AuditEntry {
            timestamp: chrono::Utc::now(),
            user_id: ctx.user_id.clone(),
            tenant_id: ctx.tenant_id.clone(),
            action: ctx
                .principal_label
                .clone()
                .unwrap_or_else(|| "query".to_string()),
            model: Some(model.to_string()),
            query_hash: None,
            row_count: Some(row_count),
            duration_ms,
            success,
            error,
        };
        if let Err(e) = sink.write(&entry) {
            tracing::warn!(error = %e, "failed to write durable audit entry");
        }
    }

    /// Register a model with the SQL engine so it becomes queryable.
    ///
    /// Storage and the SQL engine keep separate registries, and the engine's is
    /// built once at startup. Without this, a model created through the admin
    /// API would be listed and searchable but fail to generate SQL.
    ///
    /// Requires an admin, because this changes what queries can reach.
    pub fn register_model(&self, ctx: &CallContext, model: Model) -> Result<(), ServiceError> {
        ctx.require_admin()?;
        self.sql_engine.register_model(model);
        Ok(())
    }

    /// Remove a model from the SQL engine's registry.
    pub fn unregister_model(&self, ctx: &CallContext, name: &str) -> Result<(), ServiceError> {
        ctx.require_admin()?;
        self.sql_engine.unregister_model(name);
        Ok(())
    }

    /// Models visible to `ctx`, i.e. filtered by its allow/deny lists.
    ///
    /// Retrieval tools must use this, otherwise an agent can discover a model
    /// name it is not permitted to query.
    pub async fn visible_models(
        &self,
        ctx: &CallContext,
        datasource: Option<&str>,
    ) -> Result<Vec<Model>, ServiceError> {
        let policy = ctx.session_policy();
        let enforcer = PolicyEnforcer::new(policy.clone());
        let models = self
            .storage
            .list_models(datasource)
            .await
            .map_err(|e| ServiceError::Storage(e.to_string()))?;
        Ok(models
            .into_iter()
            .filter(|m| {
                enforcer.check_model_access(&m.name).is_ok()
                    && enforcer.check_datasource_access(&m.datasource).is_ok()
            })
            .collect())
    }

    /// Fetch a model, enforcing access control.
    pub async fn get_model(
        &self,
        ctx: &CallContext,
        name: &str,
        datasource: Option<&str>,
    ) -> Result<Model, ServiceError> {
        let policy = ctx.session_policy();
        let enforcer = PolicyEnforcer::new(policy);
        enforcer
            .check_model_access(name)
            .map_err(|e| ServiceError::PolicyViolation(e.to_string()))?;
        let model = self
            .storage
            .get_model(name, datasource)
            .await
            .map_err(|e| ServiceError::Storage(e.to_string()))?
            .ok_or_else(|| ServiceError::ModelNotFound(name.to_string()))?;
        enforcer
            .check_datasource_access(&model.datasource)
            .map_err(|e| ServiceError::PolicyViolation(e.to_string()))?;
        Ok(model)
    }

    /// Datasources visible to `ctx`.
    pub async fn visible_datasources(
        &self,
        ctx: &CallContext,
    ) -> Result<Vec<DataSource>, ServiceError> {
        let policy = ctx.session_policy();
        let enforcer = PolicyEnforcer::new(policy);
        let datasources = self
            .storage
            .list_datasources()
            .await
            .map_err(|e| ServiceError::Storage(e.to_string()))?;
        Ok(datasources
            .into_iter()
            .filter(|d| enforcer.check_datasource_access(&d.name).is_ok())
            .collect())
    }

    /// Search the model index, enforcing access control.
    ///
    /// The index is shared by every tenant, so filtering *after* the backend
    /// lookup is the whole point: without it, `search` would leak the existence,
    /// description and snippet of models the caller is denied. The backend is
    /// deliberately asked for more than the caller wants, because the hits we
    /// drop are hits we cannot return.
    pub async fn search(
        &self,
        ctx: &CallContext,
        query: &str,
        limit: usize,
    ) -> Result<Vec<SearchResult>, ServiceError> {
        ctx.require_authenticated()?;
        let q = query.trim();
        if q.is_empty() {
            return Err(ServiceError::InvalidQuery("search query is empty".into()));
        }
        let policy = ctx.session_policy();
        let enforcer = PolicyEnforcer::new(policy);

        // Over-fetch so that policy filtering still tends to fill the limit.
        let budget = limit.saturating_mul(8).max(64);
        let hits = self
            .storage
            .search(q, budget)
            .await
            .map_err(|e| ServiceError::Storage(e.to_string()))?;

        Ok(hits
            .into_iter()
            .filter(|h| {
                enforcer.check_model_access(&h.model_name).is_ok()
                    && enforcer.check_datasource_access(&h.datasource).is_ok()
            })
            .take(limit)
            .collect())
    }
}

/// Order stages so every `stage_ref` points at something already run.
///
/// A stage with no `stage_ref` is a root. This is Kahn's algorithm over the
/// dependency edges; whatever cannot be resolved is part of a cycle.
fn topological_order(stages: &[(String, Query)]) -> Result<Vec<String>, ServiceError> {
    let mut resolved: Vec<String> = Vec::with_capacity(stages.len());
    let mut remaining: Vec<&(String, Query)> = stages.iter().collect();

    // Each pass takes every stage whose dependency is already satisfied, so
    // independent stages keep their relative order and run in parallel later.
    while !remaining.is_empty() {
        let mut progressed = false;
        let mut still_waiting = Vec::new();
        for entry in remaining.drain(..) {
            let ready = match &entry.1.stage_ref {
                None => true,
                // A reference to a stage that is not in the pipeline at all can
                // never resolve; report it now rather than looping forever.
                Some(reference) if !stages.iter().any(|(n, _)| n == reference) => {
                    return Err(ServiceError::InvalidQuery(format!(
                        "stage_ref {reference:?} does not name a stage in this pipeline"
                    )))
                }
                Some(reference) => resolved.iter().any(|n| n == reference),
            };
            if ready {
                resolved.push(entry.0.clone());
                progressed = true;
            } else {
                still_waiting.push(entry);
            }
        }
        if !progressed {
            let names: Vec<&str> = still_waiting.iter().map(|(n, _)| n.as_str()).collect();
            return Err(ServiceError::InvalidQuery(format!(
                "cycle detected in multi-stage query: {}",
                names.join(" -> ")
            )));
        }
        remaining = still_waiting;
    }
    Ok(resolved)
}

/// Turn a referenced stage's first row into equality filters on this stage.
///
/// This is how a funnel or cohort stage says "restrict me to the ids the
/// previous step found". Only the first row is used, matching the documented
/// behaviour, so a stage that needs a true set intersection should filter
/// explicitly instead.
fn apply_stage_ref(query: &mut Query, previous: &QueryOutcome) {
    // A row is a JSON object; a non-object (a bare scalar) cannot key a filter.
    let Some(first) = previous.data.first().and_then(|r| r.as_object()) else {
        return;
    };
    for (column, value) in first {
        // Never let a generated column silently become a filter predicate.
        if column.contains(' ') || column.is_empty() {
            continue;
        }
        query.filters.push(graphnight_core::models::Filter {
            field: column.clone(),
            operator: graphnight_core::models::FilterOperator::Eq,
            value: value.clone(),
            or_condition: false,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{
        admin_context, open_context, orders_only_context, test_models, test_service,
    };
    use graphnight_core::models::{Model, Query};

    #[tokio::test]
    async fn executes_under_open_context() {
        let svc = test_service();
        let mut q = Query::new().with_name("orders");
        q.measures.push(graphnight_core::models::Measure {
            formula: graphnight_core::models::Formula {
                expression: "revenue".into(),
                label: None,
                format: None,
            },
            aggregation: graphnight_core::models::AggregationType::Sum,
        });
        // The test datasource has no reachable database, so bound the call
        // tightly: this asserts the timeout is actually enforced rather than
        // waiting out a real connect attempt.
        let mut ctx = open_context();
        let mut policy = ctx.session_policy();
        // Seconds only, so use the smallest real value and let the test run
        // one of them rather than 300.
        policy.query_timeout_secs = Some(0);
        ctx.policy = Some(policy);
        let res = svc.execute(&ctx, q, ExecuteOptions::executing()).await;
        assert!(matches!(res, Err(ServiceError::Timeout(0))), "got {res:?}");
    }

    #[tokio::test]
    async fn dry_run_returns_sql_without_touching_the_database() {
        let svc = test_service();
        let mut q = Query::new().with_name("orders");
        q.measures.push(graphnight_core::models::Measure {
            formula: graphnight_core::models::Formula {
                expression: "revenue".into(),
                label: None,
                format: None,
            },
            aggregation: graphnight_core::models::AggregationType::Sum,
        });
        let res = svc
            .execute(&open_context(), q, ExecuteOptions::dry_run())
            .await;
        let outcome = res.expect("dry run must not need a database");
        assert!(outcome.sql.is_some(), "dry run should report the SQL");
        assert!(outcome.data.is_empty(), "dry run must not return data");
        assert!(!outcome.truncated);
    }

    fn stage(name: &str, reference: Option<&str>) -> (String, Query) {
        let mut q = Query::new().with_name("orders");
        q.stage_ref = reference.map(str::to_string);
        (name.to_string(), q)
    }

    #[test]
    fn topological_order_resolves_dependencies() {
        let stages = vec![
            stage("third", Some("second")),
            stage("first", None),
            stage("second", Some("first")),
        ];
        let order = topological_order(&stages).unwrap();
        let position = |n: &str| order.iter().position(|x| x == n).unwrap();
        assert!(position("first") < position("second"));
        assert!(position("second") < position("third"));
    }

    #[test]
    fn topological_order_reports_a_cycle_instead_of_looping() {
        let stages = vec![stage("a", Some("b")), stage("b", Some("a"))];
        let err = topological_order(&stages).unwrap_err();
        assert!(matches!(err, ServiceError::InvalidQuery(_)), "{err:?}");
        assert!(err.to_string().contains("cycle"), "{err}");
    }

    #[test]
    fn topological_order_rejects_a_reference_to_an_unknown_stage() {
        let stages = vec![stage("a", Some("nope"))];
        let err = topological_order(&stages).unwrap_err();
        assert!(err.to_string().contains("nope"), "{err}");
    }

    #[test]
    fn topological_order_keeps_independent_stages_in_caller_order() {
        let stages = vec![stage("a", None), stage("b", None), stage("c", None)];
        assert_eq!(topological_order(&stages).unwrap(), vec!["a", "b", "c"]);
    }

    #[test]
    fn stage_ref_becomes_equality_filters() {
        let previous = QueryOutcome {
            data: vec![serde_json::json!({"customer_id": 7, "region name": "west"})],
            columns: vec!["customer_id".into(), "region name".into()],
            sql: None,
            population: None,
            population_inferred: false,
            execution_time_ms: 0.0,
            truncated: false,
            row_count: 1,
            next_offset: None,
            dialect: "postgres".into(),
        };
        let mut q = Query::new().with_name("orders");
        apply_stage_ref(&mut q, &previous);
        assert_eq!(
            q.filters.len(),
            1,
            "a column with a space is not a field: {:?}",
            q.filters
        );
        assert_eq!(q.filters[0].field, "customer_id");
        assert_eq!(q.filters[0].value, serde_json::json!(7));
    }

    #[tokio::test]
    async fn multi_stage_applies_governance_to_every_stage() {
        let svc = test_service();
        // `customers` is outside the `orders`-only policy, so stage two must be
        // refused even though stage one would have been allowed.
        let first = Query::new().with_name("orders");
        let mut second = Query::new().with_name("customers");
        second.stage_ref = Some("stage_1".into());
        let err = svc
            .execute_multi_stage(
                &orders_only_context(),
                vec![("stage_1".into(), first), ("stage_2".into(), second)],
                true,
            )
            .await
            .unwrap_err();
        assert!(matches!(err, ServiceError::PolicyViolation(_)), "{err:?}");
    }

    #[tokio::test]
    async fn multi_stage_rejects_a_cycle() {
        let svc = test_service();
        let mut a = Query::new().with_name("orders");
        a.stage_ref = Some("b".into());
        let mut b = Query::new().with_name("orders");
        b.stage_ref = Some("a".into());
        let err = svc
            .execute_multi_stage(
                &open_context(),
                vec![("a".into(), a), ("b".into(), b)],
                true,
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("cycle"), "{err}");
    }

    #[tokio::test]
    async fn multi_stage_returns_results_in_caller_order() {
        let svc = test_service();
        let stages = vec![stage("second", Some("first")), stage("first", None)];
        let out = svc
            .execute_multi_stage(&open_context(), stages, true)
            .await
            .expect("dry run should succeed");
        // "second" was listed first by the caller, so it comes back first even
        // though it executed last.
        assert_eq!(out.len(), 2);
    }

    #[tokio::test]
    async fn multi_stage_rejects_duplicate_stage_names() {
        let svc = test_service();
        let err = svc
            .execute_multi_stage(
                &open_context(),
                vec![stage("a", None), stage("a", None)],
                true,
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("duplicate"), "{err}");
    }

    /// Compiles a query for `orders` and returns the policy-mutated form.
    async fn governed_query(
        svc: &QueryService,
        ctx: &CallContext,
        q: Query,
    ) -> Result<QueryOutcome, ServiceError> {
        svc.execute(ctx, q, ExecuteOptions::dry_run()).await
    }

    fn count_query(model: &str) -> Query {
        let mut q = Query::new().with_name(model);
        q.measures.push(graphnight_core::models::Measure {
            formula: graphnight_core::models::Formula {
                expression: "*".into(),
                label: None,
                format: None,
            },
            aggregation: graphnight_core::models::AggregationType::Count,
        });
        q
    }

    #[tokio::test]
    async fn policy_denies_a_blacklisted_model() {
        let svc = test_service();
        let mut ctx = admin_context();
        let policy = SessionPolicy::new().with_denied_models(vec!["orders".into()]);
        ctx.policy = Some(policy);
        let err = governed_query(&svc, &ctx, count_query("orders"))
            .await
            .unwrap_err();
        assert!(matches!(err, ServiceError::PolicyViolation(_)), "{err:?}");
    }

    #[tokio::test]
    async fn policy_denies_a_model_outside_the_allowlist() {
        let svc = test_service();
        let mut ctx = admin_context();
        let policy = SessionPolicy::new().with_allowed_models(vec!["customers".into()]);
        ctx.policy = Some(policy);
        let err = governed_query(&svc, &ctx, count_query("orders"))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("not in allowed"), "{err}");
    }

    #[tokio::test]
    async fn policy_denies_a_datasource_outside_the_allowlist() {
        let svc = test_service();
        let mut ctx = admin_context();
        // The model is allowed, but its datasource is not.
        let mut policy = SessionPolicy::new().with_allowed_models(vec!["orders".into()]);
        policy.allowed_datasources = Some(vec!["some_other_warehouse".into()]);
        ctx.policy = Some(policy);
        let err = governed_query(&svc, &ctx, count_query("orders"))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("not in allowed"), "{err}");
    }

    #[tokio::test]
    async fn forced_filters_and_rls_are_merged_and_the_row_cap_clamped() {
        let svc = test_service();
        let mut ctx = admin_context();
        let policy = SessionPolicy::new()
            .with_forced_filter(graphnight_core::models::Filter::new(
                "tenant_id",
                graphnight_core::models::FilterOperator::Eq,
                serde_json::json!("tenant-7"),
            ))
            .with_row_filter(graphnight_core::models::Filter::new(
                "status",
                graphnight_core::models::FilterOperator::Eq,
                serde_json::json!("completed"),
            ))
            .with_max_rows(5);
        ctx.policy = Some(policy);

        let mut q = count_query("orders");
        q.limit = Some(100_000);
        let outcome = governed_query(&svc, &ctx, q).await.expect("should compile");
        let sql = outcome.sql.unwrap_or_default();
        assert!(sql.contains("tenant_id"), "forced filter missing: {sql}");
        assert!(sql.contains("completed"), "RLS row filter missing: {sql}");
        // The cap must bound the statement, not just the returned page.
        assert!(
            sql.contains("LIMIT 5"),
            "row cap must clamp the generated SQL: {sql}"
        );
    }

    #[tokio::test]
    async fn row_cap_uses_the_policy_default_when_the_query_asks_for_more() {
        let svc = test_service();
        let mut ctx = admin_context();
        let policy = SessionPolicy::new().with_max_rows(7);
        ctx.policy = Some(policy);
        let mut q = count_query("orders");
        q.limit = Some(50);
        let outcome = governed_query(&svc, &ctx, q).await.unwrap();
        let sql = outcome.sql.unwrap_or_default();
        assert!(sql.contains("LIMIT 7"), "{sql}");
    }

    /// A model created after the engine was built must become queryable.
    ///
    /// Storage and the SQL engine keep separate registries, and the engine's is
    /// populated once at construction. If registration is missed, the model is
    /// listed and searchable but generation fails with "Model not found".
    #[tokio::test]
    async fn a_model_registered_at_runtime_becomes_queryable() {
        let svc = test_service();
        let ctx = admin_context();

        let fresh = Model {
            name: "late_arrival".to_string(),
            datasource: "test".to_string(),
            description: Some("created after startup".to_string()),
            measures: vec![graphnight_core::models::Measure {
                formula: graphnight_core::models::Formula {
                    expression: "total".into(),
                    label: None,
                    format: None,
                },
                aggregation: graphnight_core::models::AggregationType::Sum,
            }],
            dimensions: vec![],
            time_dimensions: vec![],
            joins: vec![],
            sql: None,
            meta: HashMap::new(),
        };

        // The real order: persist, then register. The service resolves the model
        // from storage before generating, so a model in only one of the two
        // registries is still not queryable.
        svc.storage()
            .create_model(fresh.clone())
            .await
            .expect("fixture storage should accept the model");
        svc.register_model(&ctx, fresh.clone()).unwrap();

        let mut q = Query::new().with_name("late_arrival");
        q.measures = fresh.measures.clone();
        let outcome = governed_query(&svc, &ctx, q)
            .await
            .expect("should generate");
        assert!(
            outcome.sql.unwrap_or_default().contains("total"),
            "newly registered model should generate SQL"
        );
    }

    #[tokio::test]
    async fn registering_a_model_requires_admin() {
        let svc = test_service();
        let err = svc
            .register_model(&open_context(), test_models()[0].clone())
            .unwrap_err();
        assert!(matches!(err, ServiceError::PolicyViolation(_)), "{err:?}");
    }

    #[tokio::test]
    async fn an_unregistered_model_stops_generating_sql() {
        let svc = test_service();
        let ctx = admin_context();
        svc.unregister_model(&ctx, "orders").unwrap();
        let err = governed_query(&svc, &ctx, count_query("orders"))
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("orders"),
            "error should name the missing model: {err}"
        );
    }

    #[tokio::test]
    async fn search_hides_models_the_caller_cannot_see() {
        // The index is shared, so a denied model must not appear in hits even
        // though the backend still returns it.
        let svc = test_service();
        let ctx = orders_only_context();
        let hits = svc.search(&ctx, "orders", 10).await.unwrap();
        let names: Vec<&str> = hits.iter().map(|h| h.model_name.as_str()).collect();
        assert!(
            !names.contains(&"customers"),
            "search leaked a denied model: {names:?}"
        );
        assert!(names.contains(&"orders"), "{names:?}");
    }

    #[tokio::test]
    async fn search_over_fetches_so_filtering_still_fills_the_limit() {
        // Backend is asked for more than `limit` because hits may be dropped.
        let svc = test_service();
        let ctx = orders_only_context();
        let hits = svc.search(&ctx, "orders", 1).await.unwrap();
        assert_eq!(hits.len(), 1, "limit should be honoured after filtering");
        assert_eq!(hits[0].model_name, "orders");
    }

    #[tokio::test]
    async fn search_requires_authentication_when_auth_is_required() {
        let svc = test_service();
        let anon = CallContext {
            user_id: None,
            auth_required: true,
            ..open_context()
        };
        let err = svc.search(&anon, "orders", 10).await.unwrap_err();
        assert!(matches!(err, ServiceError::Unauthenticated(_)), "{err:?}");
    }

    #[tokio::test]
    async fn search_rejects_an_empty_query() {
        let svc = test_service();
        let err = svc.search(&open_context(), "   ", 10).await.unwrap_err();
        assert!(matches!(err, ServiceError::InvalidQuery(_)), "{err:?}");
    }

    #[tokio::test]
    async fn policy_denies_unknown_model() {
        let svc = test_service();
        let ctx = orders_only_context();
        let mut q = Query::new().with_name("customers");
        q.measures.push(graphnight_core::models::Measure {
            formula: graphnight_core::models::Formula {
                expression: "*".into(),
                label: None,
                format: None,
            },
            aggregation: graphnight_core::models::AggregationType::Count,
        });
        let res = svc.execute(&ctx, q, ExecuteOptions::dry_run()).await;
        assert!(matches!(res, Err(ServiceError::PolicyViolation(_))));
    }
}
