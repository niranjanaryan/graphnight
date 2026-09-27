//! The bounded agent loop.
//!
//! An agent is a loop that calls a model, executes whatever tools the model
//! asked for, and feeds the results back. Left unbounded, that loop will spend
//! money, hit rate limits and hammer a warehouse until someone notices. So the
//! bounds here are not advisory — they are enforced in the loop and they are
//! what stop a run:
//!
//! - **steps**, so it cannot iterate forever;
//! - **tokens**, so cost is capped per run rather than discovered afterwards;
//! - **wall-clock**, so a slow query cannot outlive its request;
//! - **repeated calls**, so a model stuck in a retry loop is cut off rather
//!   than billed for it.
//!
//! A run that stops because of a bound returns
//! [`AgentStopReason::BudgetExhausted`], never a fabricated answer. The
//! distinction matters more than it looks: a truncated run that looks complete
//! is how wrong numbers reach a dashboard.

use crate::governance::{CallContext, QueryService};
use crate::llm::{
    default_system_prompt, ChatMessage, CompletionRequest, LlmError, LlmProvider, StopReason,
    ToolDefinition, Usage,
};
use crate::tools::{ToolCatalog, ToolError};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Hard limits on one agent run.
#[derive(Debug, Clone)]
pub struct AgentBudget {
    /// Maximum model round-trips.
    pub max_steps: u32,
    /// Maximum total tokens across the run.
    pub max_tokens: u64,
    /// Maximum wall-clock time.
    pub max_duration: Duration,
    /// Cap on `max_tokens` for any single request.
    pub max_output_tokens: u32,
    /// Identical tool calls (same name + arguments) allowed before the run is
    /// cut off. Two is enough for a legitimate retry and low enough to catch a
    /// loop on the third.
    pub max_repeat_calls: u32,
    /// Consecutive tool failures tolerated before stopping. A model that keeps
    /// failing is not going to succeed on the next turn.
    pub max_consecutive_errors: u32,
}

impl Default for AgentBudget {
    fn default() -> Self {
        Self {
            max_steps: 12,
            max_tokens: 200_000,
            max_duration: Duration::from_secs(120),
            max_output_tokens: 4096,
            max_repeat_calls: 2,
            max_consecutive_errors: 3,
        }
    }
}

impl AgentBudget {
    /// A deliberately tight budget for interactive use.
    pub fn interactive() -> Self {
        Self {
            max_steps: 8,
            max_tokens: 100_000,
            max_duration: Duration::from_secs(60),
            ..Default::default()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentStopReason {
    /// The model produced a final answer.
    Completed,
    MaxSteps,
    TokenBudget,
    Timeout,
    RepeatedCalls,
    ConsecutiveErrors,
    /// The model stopped mid-thought (e.g. hit `max_tokens`).
    Truncated,
    /// The provider failed.
    ProviderError(String),
}

impl AgentStopReason {
    /// Whether the run produced a usable answer.
    pub fn is_success(&self) -> bool {
        matches!(self, AgentStopReason::Completed)
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            AgentStopReason::Completed => "completed",
            AgentStopReason::MaxSteps => "max_steps",
            AgentStopReason::TokenBudget => "token_budget",
            AgentStopReason::Timeout => "timeout",
            AgentStopReason::RepeatedCalls => "repeated_calls",
            AgentStopReason::ConsecutiveErrors => "consecutive_errors",
            AgentStopReason::Truncated => "truncated",
            AgentStopReason::ProviderError(_) => "provider_error",
        }
    }

    /// A plain-language explanation, for appending to a partial answer so a
    /// bounded run is never mistaken for a complete one.
    pub fn notice(&self, steps: usize, tokens: u64, elapsed: Duration) -> String {
        match self {
            AgentStopReason::Completed => "Completed.".to_string(),
            AgentStopReason::MaxSteps => {
                format!("Stopped after {steps} steps without a final answer.")
            }
            AgentStopReason::TokenBudget => format!("Stopped after using {tokens} tokens."),
            AgentStopReason::Timeout => format!("Stopped after {:.0}s.", elapsed.as_secs_f64()),
            AgentStopReason::RepeatedCalls => {
                "Stopped because the agent repeated the same call.".to_string()
            }
            AgentStopReason::ConsecutiveErrors => {
                "Stopped after repeated tool failures.".to_string()
            }
            AgentStopReason::Truncated => {
                "The response was cut off before it finished.".to_string()
            }
            AgentStopReason::ProviderError(e) => format!("Model call failed: {e}"),
        }
    }
}

/// One executed tool call, kept for the audit trail and for debugging.
#[derive(Debug, Clone)]
pub struct StepRecord {
    pub step: u32,
    pub tool: String,
    pub arguments: Value,
    pub result: Result<Value, ToolError>,
    pub duration_ms: u64,
}

impl StepRecord {
    pub fn to_json(&self) -> Value {
        json!({
            "step": self.step,
            "tool": self.tool,
            "arguments": self.arguments,
            "ok": self.result.is_ok(),
            "result": match &self.result {
                Ok(v) => v.clone(),
                Err(e) => e.to_json(),
            },
            "duration_ms": self.duration_ms,
        })
    }
}

/// The outcome of a run.
#[derive(Debug, Clone)]
pub struct AgentRun {
    pub answer: Option<String>,
    pub stop_reason: AgentStopReason,
    pub steps: Vec<StepRecord>,
    pub usage: Usage,
    pub elapsed: Duration,
    /// Tool names called, in order, deduplicated.
    pub tools_used: Vec<String>,
}

impl AgentRun {
    pub fn to_json(&self) -> Value {
        json!({
            "answer": self.answer,
            "stop_reason": self.stop_reason.as_str(),
            "completed": self.stop_reason.is_success(),
            "usage": {
                "input_tokens": self.usage.input_tokens,
                "output_tokens": self.usage.output_tokens,
                "total": self.usage.total(),
            },
            "elapsed_ms": self.elapsed.as_millis() as u64,
            "tools_used": self.tools_used,
            "steps": self.steps.iter().map(|s| s.to_json()).collect::<Vec<_>>(),
        })
    }

    /// A one-line explanation of why a run ended, for appending to a partial answer.
    pub fn stop_notice(&self) -> Option<String> {
        if self.stop_reason.is_success() {
            return None;
        }
        Some(
            self.stop_reason
                .notice(self.steps.len(), self.usage.total(), self.elapsed),
        )
    }
}

/// A configured agent.
pub struct Agent {
    name: String,
    llm: Arc<dyn LlmProvider>,
    catalog: Arc<ToolCatalog>,
    budget: AgentBudget,
    system_prompt: String,
}

impl Agent {
    pub fn new(
        name: impl Into<String>,
        llm: Arc<dyn LlmProvider>,
        catalog: Arc<ToolCatalog>,
    ) -> Self {
        Self {
            name: name.into(),
            llm,
            catalog,
            budget: AgentBudget::default(),
            system_prompt: default_system_prompt(crate::VERSION),
        }
    }

    pub fn with_budget(mut self, budget: AgentBudget) -> Self {
        self.budget = budget;
        self
    }

    pub fn with_system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.system_prompt = prompt.into();
        self
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn catalog(&self) -> &ToolCatalog {
        &self.catalog
    }

    fn tool_definitions(&self) -> Vec<ToolDefinition> {
        self.catalog
            .specs()
            .into_iter()
            .map(|spec| ToolDefinition::new(spec.name, spec.description, spec.input_schema()))
            .collect()
    }

    /// Run the agent against a question.
    ///
    /// `ctx` is resolved from the caller's own credentials by the transport, so
    /// an agent's authority is exactly the caller's authority — the loop itself
    /// has no privileged path.
    pub async fn run(&self, ctx: &CallContext, question: &str) -> AgentRun {
        let start = Instant::now();
        let mut messages = vec![ChatMessage::user(question)];
        let mut usage = Usage::default();
        let mut steps: Vec<StepRecord> = Vec::new();
        let mut tools_used: Vec<String> = Vec::new();
        // Fingerprint -> times seen, to catch retry loops.
        let mut call_counts: HashMap<String, u32> = HashMap::new();
        let mut consecutive_errors = 0u32;

        let mut step_index = 0u32;
        loop {
            // --- bounds, checked before spending anything ---
            if step_index >= self.budget.max_steps {
                return self.finish(
                    None,
                    AgentStopReason::MaxSteps,
                    steps,
                    usage,
                    start,
                    tools_used,
                );
            }
            if usage.total() >= self.budget.max_tokens {
                return self.finish(
                    None,
                    AgentStopReason::TokenBudget,
                    steps,
                    usage,
                    start,
                    tools_used,
                );
            }
            if start.elapsed() >= self.budget.max_duration {
                return self.finish(
                    None,
                    AgentStopReason::Timeout,
                    steps,
                    usage,
                    start,
                    tools_used,
                );
            }

            let request = CompletionRequest {
                model: self.llm.default_model().to_string(),
                messages: messages.clone(),
                tools: self.tool_definitions(),
                max_tokens: self.budget.max_output_tokens,
                temperature: None,
                system: Some(self.system_prompt.clone()),
            };

            let response = match tokio::time::timeout(
                self.budget.max_duration.saturating_sub(start.elapsed()),
                self.llm.complete(&request),
            )
            .await
            {
                // A model call that overruns the whole budget is a timeout, not
                // a provider error: the distinction tells the user whether to retry.
                Err(_) => {
                    return self.finish(
                        None,
                        AgentStopReason::Timeout,
                        steps,
                        usage,
                        start,
                        tools_used,
                    )
                }
                Ok(Err(LlmError::Timeout(_))) => {
                    return self.finish(
                        None,
                        AgentStopReason::Timeout,
                        steps,
                        usage,
                        start,
                        tools_used,
                    )
                }
                Ok(Err(e)) => {
                    return self.finish(
                        None,
                        AgentStopReason::ProviderError(e.to_string()),
                        steps,
                        usage,
                        start,
                        tools_used,
                    )
                }
                Ok(Ok(r)) => r,
            };

            usage.merge(response.usage);

            // A truncated turn is not an answer. Say so rather than returning
            // half a sentence as if it were complete.
            if response.stop_reason == StopReason::MaxTokens {
                return self.finish(
                    response.content,
                    AgentStopReason::Truncated,
                    steps,
                    usage,
                    start,
                    tools_used,
                );
            }

            if response.tool_calls.is_empty() {
                return self.finish(
                    response.content,
                    AgentStopReason::Completed,
                    steps,
                    usage,
                    start,
                    tools_used,
                );
            }

            // Record the assistant turn so the provider sees its own tool calls.
            messages.push(ChatMessage::Assistant {
                content: response.content.clone(),
                tool_calls: response.tool_calls.clone(),
            });

            for call in &response.tool_calls {
                if start.elapsed() >= self.budget.max_duration {
                    return self.finish(
                        None,
                        AgentStopReason::Timeout,
                        steps,
                        usage,
                        start,
                        tools_used,
                    );
                }

                // --- repeat detection, before executing ---
                let fingerprint = format!("{}:{}", call.name, call.arguments);
                let seen = call_counts.entry(fingerprint).or_insert(0);
                *seen += 1;
                if *seen > self.budget.max_repeat_calls {
                    return self.finish(
                        None,
                        AgentStopReason::RepeatedCalls,
                        steps,
                        usage,
                        start,
                        tools_used,
                    );
                }

                let tool_start = Instant::now();
                let result = self
                    .catalog
                    .call(ctx, &call.name, call.arguments.clone())
                    .await;
                let duration_ms = tool_start.elapsed().as_millis() as u64;

                consecutive_errors = if result.is_ok() {
                    0
                } else {
                    consecutive_errors + 1
                };
                if consecutive_errors > self.budget.max_consecutive_errors {
                    steps.push(StepRecord {
                        step: step_index,
                        tool: call.name.clone(),
                        arguments: call.arguments.clone(),
                        result,
                        duration_ms,
                    });
                    if !tools_used.contains(&call.name) {
                        tools_used.push(call.name.clone());
                    }
                    return self.finish(
                        None,
                        AgentStopReason::ConsecutiveErrors,
                        steps,
                        usage,
                        start,
                        tools_used,
                    );
                }

                if !tools_used.contains(&call.name) {
                    tools_used.push(call.name.clone());
                }

                // Feed the model the structured error, not a stringified one, so
                // it can read `code` and act on `hint`.
                let content = match &result {
                    Ok(v) => Ok(serde_json::to_string(v).unwrap_or_else(|_| "{}".to_string())),
                    Err(e) => {
                        Err(serde_json::to_string(&e.to_json()).unwrap_or_else(|_| e.to_string()))
                    }
                };
                messages.push(ChatMessage::tool_result(call, content));

                steps.push(StepRecord {
                    step: step_index,
                    tool: call.name.clone(),
                    arguments: call.arguments.clone(),
                    result,
                    duration_ms,
                });
            }

            step_index += 1;
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn finish(
        &self,
        answer: Option<String>,
        stop_reason: AgentStopReason,
        steps: Vec<StepRecord>,
        usage: Usage,
        start: Instant,
        tools_used: Vec<String>,
    ) -> AgentRun {
        let elapsed = start.elapsed();
        // A partial answer that hit a bound is worse than no answer: it reads as
        // complete. Append an explicit notice, and never claim completion.
        let answer = match (answer, stop_reason.is_success()) {
            (Some(mut text), false) => {
                let notice = stop_reason.notice(steps.len(), usage.total(), elapsed);
                if !text.trim().is_empty() {
                    text.push_str("\n\n");
                }
                text.push_str(&notice);
                Some(text)
            }
            (other, _) => other,
        };

        AgentRun {
            answer,
            stop_reason,
            steps,
            usage,
            elapsed,
            tools_used,
        }
    }
}

impl std::fmt::Debug for Agent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Agent")
            .field("name", &self.name)
            .field("provider", &self.llm.provider_name())
            .field("budget", &self.budget)
            .finish()
    }
}

/// Audit every governed step of a run.
///
/// The agent is just another principal, so its queries are audited the same way
/// a human's are — which is what makes "the agent read this table" answerable
/// after the fact.
pub fn audit_run(service: &QueryService, ctx: &CallContext, agent: &str, run: &AgentRun) {
    for step in &run.steps {
        service.audit(
            ctx,
            &step.tool,
            0,
            step.duration_ms,
            step.result.is_ok(),
            step.result.as_ref().err().map(|e| e.code.clone()),
        );
    }
    tracing::info!(
        agent = agent,
        steps = run.steps.len(),
        tokens = run.usage.total(),
        stop = run.stop_reason.as_str(),
        "agent run finished"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::{tool_use_response, LlmResponse, ScriptedProvider, ToolCall};

    fn tool(name: &str, args: Value) -> LlmResponse {
        tool_use_response(name, args)
    }

    fn agent_with(
        responses: Vec<LlmResponse>,
        budget: AgentBudget,
    ) -> (Agent, Arc<ScriptedProvider>) {
        let llm = Arc::new(ScriptedProvider::new(responses));
        let service = Arc::new(QueryService::new(
            Arc::new(crate::testing::test_sql_engine()),
            crate::testing::test_storage(),
        ));
        let catalog = Arc::new(ToolCatalog::new(service.clone()));
        let agent = Agent::new("test", llm.clone(), catalog).with_budget(budget);
        (agent, llm)
    }

    fn budget() -> AgentBudget {
        AgentBudget {
            max_steps: 5,
            max_tokens: 10_000,
            max_duration: Duration::from_secs(10),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn returns_a_final_answer_without_tools() {
        let (agent, _) = agent_with(vec![LlmResponse::text("Revenue was 42.")], budget());
        let run = agent
            .run(&CallContext::anonymous(), "how much revenue?")
            .await;
        assert_eq!(run.stop_reason, AgentStopReason::Completed);
        assert_eq!(run.answer.as_deref(), Some("Revenue was 42."));
        assert!(run.steps.is_empty());
    }

    #[tokio::test]
    async fn stops_at_max_steps() {
        // Ten *distinct, succeeding* tool calls. Distinct so repeat-detection
        // cannot fire, and succeeding so the consecutive-error bound cannot fire
        // either — otherwise neither test would reach the step budget.
        let responses: Vec<LlmResponse> = (1..=10)
            .map(|i| {
                tool(
                    "validate_query",
                    json!({
                        "query": {
                            "name": "orders",
                            "measures": ["revenue:sum"],
                            "limit": i
                        }
                    }),
                )
            })
            .collect();
        let (agent, llm) = agent_with(responses, budget());
        let run = agent.run(&CallContext::anonymous(), "hi").await;
        assert_eq!(run.stop_reason, AgentStopReason::MaxSteps);
        // Every recorded step actually succeeded, confirming the bound that
        // fired was the step budget and not an error bound.
        assert!(run.steps.iter().all(|s| s.result.is_ok()));
        // Exactly the budget, no more: the sixth model call is never made.
        assert_eq!(llm.call_count(), 5);
        assert_eq!(run.steps.len(), 5);
    }

    #[tokio::test]
    async fn token_budget_stops_the_run() {
        let mut b = budget();
        b.max_tokens = 1;
        let (agent, _) = agent_with(
            vec![LlmResponse {
                content: None,
                tool_calls: vec![ToolCall::new("get_capabilities", json!({}))],
                usage: Usage {
                    input_tokens: 100,
                    output_tokens: 100,
                },
                stop_reason: StopReason::ToolUse,
                model: "scripted".into(),
            }],
            b,
        );
        let run = agent.run(&CallContext::anonymous(), "hi").await;
        assert_eq!(run.stop_reason, AgentStopReason::TokenBudget);
        assert!(run.answer.is_none());
    }

    #[tokio::test]
    async fn repeated_identical_calls_are_cut_off() {
        let (agent, _) = agent_with(
            vec![
                tool("get_capabilities", json!({})),
                tool("get_capabilities", json!({})),
                tool("get_capabilities", json!({})),
            ],
            budget(),
        );
        let run = agent.run(&CallContext::anonymous(), "hi").await;
        assert_eq!(run.stop_reason, AgentStopReason::RepeatedCalls);
    }

    #[tokio::test]
    async fn distinct_calls_are_not_treated_as_repeats() {
        let (agent, _) = agent_with(
            vec![
                tool("get_capabilities", json!({})),
                tool("list_models", json!({})),
                LlmResponse::text("done"),
            ],
            budget(),
        );
        let run = agent.run(&CallContext::anonymous(), "hi").await;
        assert_eq!(run.stop_reason, AgentStopReason::Completed);
        assert_eq!(run.tools_used, vec!["get_capabilities", "list_models"]);
    }

    #[tokio::test]
    async fn truncated_response_is_reported_not_passed_off_as_complete() {
        let (agent, _) = agent_with(
            vec![LlmResponse {
                content: Some("Revenue was 4".to_string()),
                tool_calls: vec![],
                usage: Usage::default(),
                stop_reason: StopReason::MaxTokens,
                model: "scripted".into(),
            }],
            budget(),
        );
        let run = agent.run(&CallContext::anonymous(), "hi").await;
        assert_eq!(run.stop_reason, AgentStopReason::Truncated);
        let answer = run.answer.unwrap();
        assert!(answer.contains("cut off"), "{answer}");
    }

    #[tokio::test]
    async fn provider_error_is_surfaced() {
        let llm = Arc::new(ScriptedProvider::failing(LlmError::Unauthorized));
        let service = Arc::new(QueryService::new(
            Arc::new(crate::testing::test_sql_engine()),
            crate::testing::test_storage(),
        ));
        let catalog = Arc::new(ToolCatalog::new(service.clone()));
        let agent = Agent::new("t", llm, catalog).with_budget(budget());
        let run = agent.run(&CallContext::anonymous(), "hi").await;
        assert!(matches!(run.stop_reason, AgentStopReason::ProviderError(_)));
        assert!(run.answer.is_none());
    }

    #[tokio::test]
    async fn unknown_tool_errors_do_not_stop_the_run() {
        let (agent, _) = agent_with(
            vec![
                tool("no_such_tool", json!({})),
                LlmResponse::text("I could not do that."),
            ],
            budget(),
        );
        let run = agent.run(&CallContext::anonymous(), "hi").await;
        assert_eq!(run.stop_reason, AgentStopReason::Completed);
        assert!(matches!(run.steps[0].result, Err(ref e) if e.code == "UNKNOWN_TOOL"));
    }

    #[tokio::test]
    async fn consecutive_failures_stop_the_run() {
        let mut b = budget();
        b.max_repeat_calls = 10;
        b.max_consecutive_errors = 2;
        let (agent, _) = agent_with(
            vec![
                tool("get_model", json!({"name": "nope1"})),
                tool("get_model", json!({"name": "nope2"})),
                tool("get_model", json!({"name": "nope3"})),
            ],
            b,
        );
        let run = agent.run(&CallContext::anonymous(), "hi").await;
        assert_eq!(run.stop_reason, AgentStopReason::ConsecutiveErrors);
    }

    #[tokio::test]
    async fn usage_is_accumulated_across_steps() {
        let (agent, _) = agent_with(
            vec![
                tool("get_capabilities", json!({})),
                LlmResponse {
                    content: Some("ok".into()),
                    tool_calls: vec![],
                    usage: Usage {
                        input_tokens: 10,
                        output_tokens: 5,
                    },
                    stop_reason: StopReason::EndTurn,
                    model: "scripted".into(),
                },
            ],
            budget(),
        );
        let run = agent.run(&CallContext::anonymous(), "hi").await;
        // 100+50 from the tool-use turn, 10+5 from the final turn.
        assert_eq!(run.usage.total(), 165);
    }

    #[test]
    fn successful_runs_have_no_stop_notice() {
        let run = AgentRun {
            answer: Some("x".into()),
            stop_reason: AgentStopReason::Completed,
            steps: vec![],
            usage: Usage::default(),
            elapsed: Duration::from_secs(1),
            tools_used: vec![],
        };
        assert!(run.stop_notice().is_none());
    }
}
