//! The pluggable LLM provider layer.
//!
//! One trait, two real implementations (Anthropic and OpenAI-compatible), and a
//! deterministic provider for tests. Three decisions shape this module:
//!
//! - **Providers own no policy.** A provider translates messages to a vendor
//!   API and back. It never sees credentials, never decides what a tool may do,
//!   and never retries a governed query. Policy lives in
//!   [`crate::governance`], so swapping vendors cannot change what a caller is
//!   allowed to see.
//! - **Secrets are write-only.** An API key is taken by value at construction
//!   and is never stored on a struct that derives `Debug`, so it cannot leak
//!   into a log line or an error message.
//! - **Usage is mandatory.** [`Usage`] is returned on every response, not
//!   optionally, because the agent's step and token budgets depend on it.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::VecDeque;
use std::sync::Arc;

/// One message in a conversation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "role", rename_all = "lowercase")]
pub enum ChatMessage {
    System {
        content: String,
    },
    User {
        content: String,
    },
    /// Assistant turn that requested tool calls.
    Assistant {
        content: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        tool_calls: Vec<ToolCall>,
    },
    /// The result of a tool call, fed back to the model.
    Tool {
        tool_call_id: String,
        name: String,
        /// `Err` carries the structured tool error so the model can read the
        /// code and the hint, not just a stringified failure.
        content: Result<String, String>,
    },
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self::System {
            content: content.into(),
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self::User {
            content: content.into(),
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self::Assistant {
            content: Some(content.into()),
            tool_calls: vec![],
        }
    }

    pub fn tool_result(call: &ToolCall, content: Result<String, String>) -> Self {
        Self::Tool {
            tool_call_id: call.id.clone(),
            name: call.name.clone(),
            content,
        }
    }

    /// Whether this message carries a tool result, which providers must send
    /// back immediately and without modification.
    pub fn is_tool_result(&self) -> bool {
        matches!(self, Self::Tool { .. })
    }
}

/// A tool invocation requested by the model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    /// Arguments as the model produced them. Kept as raw JSON because it is
    /// untrusted input that has not been validated yet.
    pub arguments: Value,
}

impl ToolCall {
    pub fn new(name: impl Into<String>, arguments: Value) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.into(),
            arguments,
        }
    }
}

/// A tool advertised to the model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    /// JSON Schema object describing the input.
    pub input_schema: Value,
}

impl ToolDefinition {
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        input_schema: Value,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            input_schema,
        }
    }
}

/// Token accounting for one response.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

impl Usage {
    pub fn total(&self) -> u64 {
        self.input_tokens + self.output_tokens
    }

    /// Accumulate across an agent run.
    pub fn merge(&mut self, other: Usage) {
        self.input_tokens += other.input_tokens;
        self.output_tokens += other.output_tokens;
    }
}

/// The model's reply.
#[derive(Debug, Clone, PartialEq)]
pub struct LlmResponse {
    pub content: Option<String>,
    pub tool_calls: Vec<ToolCall>,
    pub usage: Usage,
    /// Set when the provider stopped because a limit was hit (`max_tokens`,
    /// or a provider-side budget). The agent treats this as "not done" rather
    /// than as a successful final answer.
    pub stop_reason: StopReason,
    /// Which model actually served the request, for audit trails.
    pub model: String,
}

impl LlmResponse {
    pub fn text(content: impl Into<String>) -> Self {
        Self {
            content: Some(content.into()),
            tool_calls: vec![],
            usage: Usage::default(),
            stop_reason: StopReason::EndTurn,
            model: String::new(),
        }
    }

    pub fn is_final(&self) -> bool {
        self.tool_calls.is_empty() && self.stop_reason == StopReason::EndTurn
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    #[default]
    EndTurn,
    MaxTokens,
    ToolUse,
    StopSequence,
    /// The provider or transport failed; not a valid final answer.
    Error,
}

/// Sampling configuration.
#[derive(Debug, Clone, PartialEq)]
pub struct CompletionRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    pub tools: Vec<ToolDefinition>,
    pub max_tokens: u32,
    pub temperature: Option<f32>,
    pub system: Option<String>,
}

impl CompletionRequest {
    pub fn new(model: impl Into<String>, messages: Vec<ChatMessage>) -> Self {
        Self {
            model: model.into(),
            messages,
            tools: vec![],
            max_tokens: 4096,
            temperature: None,
            system: None,
        }
    }

    pub fn with_tools(mut self, tools: Vec<ToolDefinition>) -> Self {
        self.tools = tools;
        self
    }

    pub fn with_system(mut self, system: impl Into<String>) -> Self {
        self.system = Some(system.into());
        self
    }
}

#[derive(Debug, thiserror::Error)]
pub enum LlmError {
    #[error("no LLM provider is configured")]
    NotConfigured,
    #[error("LLM request failed: {0}")]
    Provider(String),
    #[error("LLM request timed out after {0}s")]
    Timeout(u64),
    #[error("rate limited; retry after {0}s")]
    RateLimited(u64),
    #[error("authentication failed: check the configured API key")]
    Unauthorized,
    #[error("context length exceeded ({requested} tokens, limit {limit})")]
    ContextTooLong { requested: u32, limit: u32 },
    #[error("LLM response could not be parsed: {0}")]
    MalformedResponse(String),
    #[error("{0}")]
    Invalid(String),
}

/// A chat-completion provider.
#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// Stable provider name, for logs and audit entries.
    fn provider_name(&self) -> &str;

    /// Model used when a request does not name one.
    fn default_model(&self) -> &str;

    async fn complete(&self, request: &CompletionRequest) -> Result<LlmResponse, LlmError>;

    /// Upper bound on the context window, used to fail fast instead of letting
    /// the provider reject an oversized request.
    fn context_window(&self) -> u32 {
        128_000
    }
}

pub type SharedLlm = Arc<dyn LlmProvider>;

// ---------------------------------------------------------------------------
// Anthropic
// ---------------------------------------------------------------------------

/// Anthropic Messages API.
///
/// The key is moved into the struct and never read back out, and `Debug` is
/// implemented manually so the key cannot reach a log.
pub struct AnthropicProvider {
    client: reqwest::Client,
    api_key: String,
    base_url: String,
    model: String,
    max_retries: u32,
    timeout_secs: u64,
}

impl std::fmt::Debug for AnthropicProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AnthropicProvider")
            .field("base_url", &self.base_url)
            .field("model", &self.model)
            .field("api_key", &"[redacted]")
            .finish()
    }
}

impl AnthropicProvider {
    pub fn new(api_key: String, model: impl Into<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_key,
            base_url: "https://api.anthropic.com/v1".to_string(),
            model: model.into(),
            max_retries: 2,
            timeout_secs: 120,
        }
    }

    /// Point at a proxy or a test double.
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into().trim_end_matches('/').to_string();
        self
    }

    pub fn with_timeout(mut self, secs: u64) -> Self {
        self.timeout_secs = secs;
        self
    }

    pub fn with_max_retries(mut self, retries: u32) -> Self {
        self.max_retries = retries;
        self
    }

    fn to_body(&self, req: &CompletionRequest) -> Value {
        let mut messages = Vec::new();
        for m in &req.messages {
            match m {
                // Anthropic carries the system prompt out of band.
                ChatMessage::System { .. } => {}
                ChatMessage::User { content } => {
                    messages.push(serde_json::json!({
                        "role": "user",
                        "content": [{"type": "text", "text": content}]
                    }));
                }
                ChatMessage::Assistant {
                    content,
                    tool_calls,
                } => {
                    let mut blocks = Vec::new();
                    if let Some(text) = content {
                        if !text.is_empty() {
                            blocks.push(serde_json::json!({"type": "text", "text": text}));
                        }
                    }
                    for tc in tool_calls {
                        blocks.push(serde_json::json!({
                            "type": "tool_use",
                            "id": tc.id,
                            "name": tc.name,
                            "input": tc.arguments,
                        }));
                    }
                    messages.push(serde_json::json!({"role": "assistant", "content": blocks}));
                }
                ChatMessage::Tool {
                    tool_call_id,
                    content,
                    ..
                } => {
                    let body = match content {
                        Ok(v) => v.clone(),
                        // A failed tool is still a result the model must see.
                        Err(e) => e.clone(),
                    };
                    messages.push(serde_json::json!({
                        "role": "user",
                        "content": [{
                            "type": "tool_result",
                            "tool_use_id": tool_call_id,
                            "content": body,
                            "is_error": content.is_err(),
                        }]
                    }));
                }
            }
        }

        let mut body = serde_json::json!({
            "model": req.model,
            "max_tokens": req.max_tokens,
            "messages": messages,
        });
        if let Some(t) = req.temperature {
            body["temperature"] = serde_json::json!(t);
        }
        if let Some(system) = &req.system {
            body["system"] = serde_json::json!(system);
        }
        if !req.tools.is_empty() {
            body["tools"] = serde_json::json!(req
                .tools
                .iter()
                .map(|t| serde_json::json!({
                    "name": t.name,
                    "description": t.description,
                    "input_schema": t.input_schema,
                }))
                .collect::<Vec<_>>());
        }
        body
    }
}

#[async_trait]
impl LlmProvider for AnthropicProvider {
    fn provider_name(&self) -> &str {
        "anthropic"
    }

    fn default_model(&self) -> &str {
        &self.model
    }

    async fn complete(&self, request: &CompletionRequest) -> Result<LlmResponse, LlmError> {
        let body = self.to_body(request);
        let mut attempt = 0;
        loop {
            let result = self
                .client
                .post(format!("{}/messages", self.base_url))
                .header("x-api-key", &self.api_key)
                .header("anthropic-version", "2023-06-01")
                .header("content-type", "application/json")
                .json(&body)
                .timeout(std::time::Duration::from_secs(self.timeout_secs))
                .send()
                .await;

            let response = match result {
                Ok(r) => r,
                Err(e) if e.is_timeout() => {
                    return Err(LlmError::Timeout(self.timeout_secs));
                }
                Err(e) => return Err(LlmError::Provider(e.to_string())),
            };

            let status = response.status();
            if status.is_success() {
                let json: Value = response
                    .json()
                    .await
                    .map_err(|e| LlmError::MalformedResponse(e.to_string()))?;
                return parse_anthropic_response(&json);
            }

            let status_code = status.as_u16();
            let text = response.text().await.unwrap_or_default();
            match status_code {
                401 | 403 => return Err(LlmError::Unauthorized),
                429 => {
                    if attempt >= self.max_retries {
                        return Err(LlmError::RateLimited(0));
                    }
                    attempt += 1;
                    tokio::time::sleep(std::time::Duration::from_millis(500 * u64::from(attempt)))
                        .await;
                    continue;
                }
                400 if text.contains("max_tokens") || text.contains("context") => {
                    return Err(LlmError::ContextTooLong {
                        requested: request.max_tokens,
                        limit: self.context_window(),
                    });
                }
                _ if status.is_server_error() && attempt < self.max_retries => {
                    attempt += 1;
                    tokio::time::sleep(std::time::Duration::from_millis(500 * u64::from(attempt)))
                        .await;
                    continue;
                }
                _ => {
                    return Err(LlmError::Provider(format!(
                        "HTTP {status_code}: {}",
                        truncate(&text, 400)
                    )))
                }
            }
        }
    }
}

fn parse_anthropic_response(json: &Value) -> Result<LlmResponse, LlmError> {
    let mut content = String::new();
    let mut tool_calls = Vec::new();
    for block in json["content"].as_array().cloned().unwrap_or_default() {
        match block["type"].as_str() {
            Some("text") => {
                if let Some(t) = block["text"].as_str() {
                    content.push_str(t);
                }
            }
            Some("tool_use") => {
                tool_calls.push(ToolCall {
                    id: block["id"].as_str().unwrap_or_default().to_string(),
                    name: block["name"].as_str().unwrap_or_default().to_string(),
                    arguments: block["input"].clone(),
                });
            }
            _ => {}
        }
    }

    let stop_reason = match json["stop_reason"].as_str() {
        Some("tool_use") => StopReason::ToolUse,
        Some("max_tokens") => StopReason::MaxTokens,
        Some("stop_sequence") => StopReason::StopSequence,
        _ => StopReason::EndTurn,
    };

    Ok(LlmResponse {
        content: if content.is_empty() {
            None
        } else {
            Some(content)
        },
        tool_calls,
        usage: Usage {
            input_tokens: json["usage"]["input_tokens"].as_u64().unwrap_or(0),
            output_tokens: json["usage"]["output_tokens"].as_u64().unwrap_or(0),
        },
        stop_reason,
        model: json["model"].as_str().unwrap_or_default().to_string(),
    })
}

// ---------------------------------------------------------------------------
// OpenAI-compatible
// ---------------------------------------------------------------------------

/// Any OpenAI-compatible `/chat/completions` endpoint: OpenAI, Ollama, vLLM,
/// Together, Groq, OpenRouter, LM Studio, and so on.
pub struct OpenAiCompatibleProvider {
    client: reqwest::Client,
    api_key: Option<String>,
    base_url: String,
    model: String,
    timeout_secs: u64,
    max_retries: u32,
    /// Some local servers do not implement `tool_choice`; 0.8-era Ollama and
    /// several proxies reject the field outright.
    supports_tool_choice: bool,
}

impl std::fmt::Debug for OpenAiCompatibleProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenAiCompatibleProvider")
            .field("base_url", &self.base_url)
            .field("model", &self.model)
            .field(
                "api_key",
                &if self.api_key.is_some() {
                    "[redacted]"
                } else {
                    "[none]"
                },
            )
            .finish()
    }
}

impl OpenAiCompatibleProvider {
    /// `api_key` may be empty for a local server that requires no auth.
    pub fn new(
        base_url: impl Into<String>,
        api_key: Option<String>,
        model: impl Into<String>,
    ) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_key,
            base_url: base_url.into().trim_end_matches('/').to_string(),
            model: model.into(),
            timeout_secs: 120,
            max_retries: 2,
            supports_tool_choice: true,
        }
    }

    /// `http://localhost:11434/v1` for Ollama.
    pub fn ollama(model: impl Into<String>) -> Self {
        Self::new("http://localhost:11434/v1", None, model).with_tool_choice_support(false)
    }

    pub fn with_tool_choice_support(mut self, supported: bool) -> Self {
        self.supports_tool_choice = supported;
        self
    }

    pub fn with_timeout(mut self, secs: u64) -> Self {
        self.timeout_secs = secs;
        self
    }

    fn to_body(&self, req: &CompletionRequest) -> Value {
        let mut messages: Vec<Value> = Vec::new();
        if let Some(system) = &req.system {
            messages.push(serde_json::json!({"role": "system", "content": system}));
        }
        for m in &req.messages {
            match m {
                ChatMessage::System { content } => {
                    messages.push(serde_json::json!({"role": "system", "content": content}));
                }
                ChatMessage::User { content } => {
                    messages.push(serde_json::json!({"role": "user", "content": content}));
                }
                ChatMessage::Assistant {
                    content,
                    tool_calls,
                } => {
                    let mut entry = serde_json::json!({
                        "role": "assistant",
                        "content": content.clone().unwrap_or_default(),
                    });
                    if !tool_calls.is_empty() {
                        entry["tool_calls"] = serde_json::json!(tool_calls
                            .iter()
                            .map(|tc| serde_json::json!({
                                "id": tc.id,
                                "type": "function",
                                "function": {
                                    "name": tc.name,
                                    "arguments": serde_json::to_string(&tc.arguments)
                                        .unwrap_or_else(|_| "{}".to_string()),
                                }
                            }))
                            .collect::<Vec<_>>());
                    }
                    messages.push(entry);
                }
                ChatMessage::Tool {
                    tool_call_id,
                    content,
                    ..
                } => {
                    let body = match content {
                        Ok(v) => v.clone(),
                        Err(e) => e.clone(),
                    };
                    messages.push(serde_json::json!({
                        "role": "tool",
                        "tool_call_id": tool_call_id,
                        "content": body,
                    }));
                }
            }
        }

        let mut body = serde_json::json!({
            "model": req.model,
            "messages": messages,
            "max_tokens": req.max_tokens,
        });
        if let Some(t) = req.temperature {
            body["temperature"] = serde_json::json!(t);
        }
        if !req.tools.is_empty() {
            body["tools"] = serde_json::json!(req
                .tools
                .iter()
                .map(|t| serde_json::json!({
                    "type": "function",
                    "function": {
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.input_schema,
                    }
                }))
                .collect::<Vec<_>>());
            if self.supports_tool_choice {
                body["tool_choice"] = serde_json::json!("auto");
            }
        }
        body
    }
}

#[async_trait]
impl LlmProvider for OpenAiCompatibleProvider {
    fn provider_name(&self) -> &str {
        "openai-compatible"
    }

    fn default_model(&self) -> &str {
        &self.model
    }

    async fn complete(&self, request: &CompletionRequest) -> Result<LlmResponse, LlmError> {
        let body = self.to_body(request);
        let mut attempt = 0;
        loop {
            let mut req = self
                .client
                .post(format!("{}/chat/completions", self.base_url))
                .header("content-type", "application/json")
                .timeout(std::time::Duration::from_secs(self.timeout_secs));
            if let Some(key) = &self.api_key {
                req = req.bearer_auth(key);
            }
            let result = req.json(&body).send().await;

            let response = match result {
                Ok(r) => r,
                Err(e) if e.is_timeout() => {
                    return Err(LlmError::Timeout(self.timeout_secs));
                }
                Err(e) => return Err(LlmError::Provider(e.to_string())),
            };

            let status = response.status();
            if status.is_success() {
                let json: Value = response
                    .json()
                    .await
                    .map_err(|e| LlmError::MalformedResponse(e.to_string()))?;
                return parse_openai_response(&json);
            }

            let status_code = status.as_u16();
            let text = response.text().await.unwrap_or_default();
            match status_code {
                401 | 403 => return Err(LlmError::Unauthorized),
                429 => {
                    if attempt >= self.max_retries {
                        return Err(LlmError::RateLimited(0));
                    }
                    attempt += 1;
                    tokio::time::sleep(std::time::Duration::from_millis(500 * u64::from(attempt)))
                        .await;
                    continue;
                }
                _ if status.is_server_error() && attempt < self.max_retries => {
                    attempt += 1;
                    tokio::time::sleep(std::time::Duration::from_millis(500 * u64::from(attempt)))
                        .await;
                    continue;
                }
                _ => {
                    return Err(LlmError::Provider(format!(
                        "HTTP {status_code}: {}",
                        truncate(&text, 400)
                    )))
                }
            }
        }
    }
}

fn parse_openai_response(json: &Value) -> Result<LlmResponse, LlmError> {
    let choice = json["choices"]
        .get(0)
        .ok_or_else(|| LlmError::MalformedResponse("response has no choices".to_string()))?;
    let message = &choice["message"];

    let mut tool_calls = Vec::new();
    for tc in message["tool_calls"]
        .as_array()
        .cloned()
        .unwrap_or_default()
    {
        let function = &tc["function"];
        let arguments = match function["arguments"].as_str() {
            // A model may emit an empty string for "no arguments"; treat that
            // as `{}` rather than failing the whole turn.
            Some("") | None => serde_json::json!({}),
            Some(s) => serde_json::from_str(s).unwrap_or_else(|_| serde_json::json!({})),
        };
        tool_calls.push(ToolCall {
            id: tc["id"].as_str().unwrap_or_default().to_string(),
            name: function["name"].as_str().unwrap_or_default().to_string(),
            arguments,
        });
    }

    let stop_reason = match choice["finish_reason"].as_str() {
        Some("tool_calls") | Some("function_call") => StopReason::ToolUse,
        Some("length") => StopReason::MaxTokens,
        Some("stop") => StopReason::EndTurn,
        _ => StopReason::EndTurn,
    };

    Ok(LlmResponse {
        content: message["content"].as_str().map(|s| s.to_string()),
        tool_calls,
        usage: Usage {
            input_tokens: json["usage"]["prompt_tokens"].as_u64().unwrap_or(0),
            output_tokens: json["usage"]["completion_tokens"].as_u64().unwrap_or(0),
        },
        stop_reason,
        model: json["model"].as_str().unwrap_or_default().to_string(),
    })
}

// ---------------------------------------------------------------------------
// Deterministic provider (tests and offline development)
// ---------------------------------------------------------------------------

/// A provider that replays a scripted sequence of responses.
///
/// This is what makes the agent loop testable without a network or an API key:
/// a test asserts the loop's *behaviour* — did it stop, did it report the right
/// error, did it stay inside budget — independently of any model's mood.
pub struct ScriptedProvider {
    responses: std::sync::Mutex<VecDeque<Result<LlmResponse, LlmError>>>,
    requests: std::sync::Mutex<Vec<CompletionRequest>>,
    model: String,
}

impl ScriptedProvider {
    pub fn new(responses: Vec<LlmResponse>) -> Self {
        Self {
            responses: std::sync::Mutex::new(responses.into_iter().map(Ok).collect()),
            requests: std::sync::Mutex::new(vec![]),
            model: "scripted".to_string(),
        }
    }

    /// Convenience: fail every call with `err`.
    pub fn failing(err: LlmError) -> Self {
        Self::new(vec![]).with_error(err)
    }

    fn with_error(self, err: LlmError) -> Self {
        self.responses
            .lock()
            .expect("scripted provider poisoned")
            .push_back(Err(err));
        self
    }

    /// Every request the loop made, for assertions.
    pub fn captured_requests(&self) -> Vec<CompletionRequest> {
        self.requests
            .lock()
            .expect("scripted provider poisoned")
            .clone()
    }

    /// How many calls the loop made.
    pub fn call_count(&self) -> usize {
        self.requests
            .lock()
            .expect("scripted provider poisoned")
            .len()
    }
}

#[async_trait]
impl LlmProvider for ScriptedProvider {
    fn provider_name(&self) -> &str {
        "scripted"
    }

    fn default_model(&self) -> &str {
        &self.model
    }

    async fn complete(&self, request: &CompletionRequest) -> Result<LlmResponse, LlmError> {
        self.requests
            .lock()
            .expect("scripted provider poisoned")
            .push(request.clone());
        self.responses
            .lock()
            .expect("scripted provider poisoned")
            .pop_front()
            .unwrap_or_else(|| {
                Ok(LlmResponse {
                    content: Some("script exhausted".to_string()),
                    tool_calls: vec![],
                    usage: Usage::default(),
                    stop_reason: StopReason::EndTurn,
                    model: self.model.clone(),
                })
            })
    }
}

/// Build the response that requests a tool call.
pub fn tool_use_response(name: &str, arguments: Value) -> LlmResponse {
    LlmResponse {
        content: None,
        tool_calls: vec![ToolCall::new(name, arguments)],
        usage: Usage {
            input_tokens: 100,
            output_tokens: 50,
        },
        stop_reason: StopReason::ToolUse,
        model: "scripted".to_string(),
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        let mut end = max;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}…", &s[..end])
    }
}

/// Build the default system prompt for an agent run.
///
/// The ordering is not cosmetic: the model is told the shape of the workflow
/// before it is told it has tools, and is told that a failed tool carries a
/// hint *before* it is told to retry. Instructions that match the shape of the
/// actual error path measurably reduce wasted turns.
pub fn default_system_prompt(version: &str) -> String {
    format!(
        "You are GraphNight's data analyst agent (server {version}).\n\
         \n\
         You answer questions about a semantic layer by calling tools. You cannot \
         see the database directly; every fact you report must come from a tool result.\n\
         \n\
         Follow this flow:\n\
         1. get_capabilities, once, if you are unsure what is supported.\n\
         2. search_models or list_models to find the right model.\n\
         3. get_model to read its real fields. Never guess a field name.\n\
         4. validate_query before run_query while you are still learning a model.\n\
         5. run_query for rows.\n\
         \n\
         Rules:\n\
         - Use the exact field names from get_model. Field errors carry a `hint` \
           with the closest correct name; use it.\n\
         - A tool error is information, not failure. Read `code` and `hint`, then \
           correct the call once. Do not repeat an identical failing call.\n\
         - If a call is denied by policy, stop and report it. Do not try to work \
           around a denial.\n\
         - Prefer recall_memories before answering from memory, and remember any \
           durable, non-obvious fact you learn.\n\
         - State the model and filters you used. If the result was truncated, say so \
           rather than implying you saw everything.\n\
         - Never invent numbers. If a tool did not return it, you do not know it."
    )
}

/// A timestamped conversation record, for audit and replay.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationRecord {
    pub id: String,
    pub agent: String,
    pub started_at: DateTime<Utc>,
    pub messages: Vec<ChatMessage>,
    pub usage: Usage,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool_def() -> ToolDefinition {
        ToolDefinition::new(
            "run_query",
            "run a query",
            serde_json::json!({"type": "object"}),
        )
    }

    #[test]
    fn usage_accumulates() {
        let mut u = Usage {
            input_tokens: 1,
            output_tokens: 2,
        };
        u.merge(Usage {
            input_tokens: 10,
            output_tokens: 20,
        });
        assert_eq!(u.total(), 33);
    }

    #[test]
    fn anthropic_body_extracts_tool_use() {
        let p = AnthropicProvider::new("sk-test".into(), "claude-x");
        let body = p.to_body(&CompletionRequest {
            model: "claude-x".into(),
            messages: vec![ChatMessage::user("hi")],
            tools: vec![tool_def()],
            max_tokens: 100,
            temperature: None,
            system: Some("sys".into()),
        });
        assert_eq!(body["system"], "sys");
        assert_eq!(body["tools"][0]["input_schema"]["type"], "object");
        assert_eq!(body["messages"][0]["role"], "user");
    }

    #[test]
    fn anthropic_system_message_is_not_duplicated_into_messages() {
        let p = AnthropicProvider::new("sk-test".into(), "claude-x");
        let body = p.to_body(&CompletionRequest {
            model: "m".into(),
            messages: vec![ChatMessage::system("sys"), ChatMessage::user("hi")],
            tools: vec![],
            max_tokens: 10,
            temperature: None,
            system: Some("sys".into()),
        });
        assert_eq!(body["messages"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn anthropic_tool_error_is_flagged_to_the_model() {
        let p = AnthropicProvider::new("sk-test".into(), "claude-x");
        let call = ToolCall::new("run_query", serde_json::json!({}));
        let body = p.to_body(&CompletionRequest {
            model: "m".into(),
            messages: vec![ChatMessage::tool_result(
                &call,
                Err("MODEL_NOT_FOUND".into()),
            )],
            tools: vec![],
            max_tokens: 10,
            temperature: None,
            system: None,
        });
        assert_eq!(body["messages"][0]["content"][0]["is_error"], true);
    }

    #[test]
    fn openai_body_serialises_tool_arguments_as_a_string() {
        let p = OpenAiCompatibleProvider::new("http://x/v1", None, "m");
        let call = ToolCall::new("run_query", serde_json::json!({"a": 1}));
        let body = p.to_body(&CompletionRequest {
            model: "m".into(),
            messages: vec![ChatMessage::Assistant {
                content: None,
                tool_calls: vec![call],
            }],
            tools: vec![tool_def()],
            max_tokens: 10,
            temperature: None,
            system: None,
        });
        // OpenAI wire format requires arguments to be a JSON *string*.
        assert_eq!(
            body["messages"][0]["tool_calls"][0]["function"]["arguments"],
            "{\"a\":1}"
        );
    }

    #[test]
    fn ollama_omits_tool_choice_when_unsupported() {
        let p = OpenAiCompatibleProvider::ollama("llama3");
        let body = p.to_body(&CompletionRequest {
            model: "llama3".into(),
            messages: vec![],
            tools: vec![tool_def()],
            max_tokens: 10,
            temperature: None,
            system: None,
        });
        assert!(body.get("tool_choice").is_none());
        assert!(body["tools"].is_array());
    }

    #[test]
    fn parses_anthropic_tool_use() {
        let json = serde_json::json!({
            "model": "claude-x",
            "stop_reason": "tool_use",
            "content": [
                {"type": "text", "text": "let me look"},
                {"type": "tool_use", "id": "t1", "name": "get_model", "input": {"name": "orders"}}
            ],
            "usage": {"input_tokens": 10, "output_tokens": 4}
        });
        let r = parse_anthropic_response(&json).unwrap();
        assert_eq!(r.stop_reason, StopReason::ToolUse);
        assert_eq!(r.tool_calls.len(), 1);
        assert_eq!(r.tool_calls[0].name, "get_model");
        assert_eq!(r.content.as_deref(), Some("let me look"));
        assert_eq!(r.usage.total(), 14);
    }

    #[test]
    fn parses_openai_response() {
        let json = serde_json::json!({
            "model": "gpt-x",
            "choices": [{
                "finish_reason": "tool_calls",
                "message": {
                    "content": null,
                    "tool_calls": [{
                        "id": "c1",
                        "function": {"name": "run_query", "arguments": "{\"query\":{}}"}
                    }]
                }
            }],
            "usage": {"prompt_tokens": 7, "completion_tokens": 3}
        });
        let r = parse_openai_response(&json).unwrap();
        assert_eq!(r.stop_reason, StopReason::ToolUse);
        assert_eq!(r.tool_calls[0].arguments["query"], serde_json::json!({}));
        assert_eq!(r.usage.output_tokens, 3);
    }

    #[test]
    fn openai_empty_arguments_becomes_object() {
        let json = serde_json::json!({
            "choices": [{
                "finish_reason": "tool_calls",
                "message": {"tool_calls": [{"id": "c1", "function": {"name": "t", "arguments": ""}}]}
            }]
        });
        let r = parse_openai_response(&json).unwrap();
        assert_eq!(r.tool_calls[0].arguments, serde_json::json!({}));
    }

    #[test]
    fn openai_missing_choices_is_an_error_not_a_panic() {
        assert!(parse_openai_response(&serde_json::json!({})).is_err());
    }

    #[test]
    fn keys_are_not_in_debug_output() {
        let a = format!(
            "{:?}",
            AnthropicProvider::new("sk-super-secret".into(), "m")
        );
        let o = format!(
            "{:?}",
            OpenAiCompatibleProvider::new("http://x/v1", Some("sk-secret".into()), "m")
        );
        assert!(!a.contains("sk-super-secret"), "{a}");
        assert!(!o.contains("sk-secret"), "{o}");
    }

    #[test]
    fn truncate_respects_char_boundaries() {
        assert_eq!(truncate("hello", 10), "hello");
        let t = truncate("aééé", 4);
        assert!(t.chars().count() <= 5);
    }

    #[tokio::test]
    async fn scripted_provider_replays_in_order() {
        let p = ScriptedProvider::new(vec![
            LlmResponse::text("one"),
            tool_use_response("get_capabilities", serde_json::json!({})),
        ]);
        let req = CompletionRequest::new("m", vec![]);
        assert_eq!(
            p.complete(&req).await.unwrap().content.as_deref(),
            Some("one")
        );
        assert_eq!(p.complete(&req).await.unwrap().tool_calls.len(), 1);
        // Exhausted: returns a final answer rather than looping forever.
        assert!(p.complete(&req).await.unwrap().is_final());
        assert_eq!(p.call_count(), 3);
    }

    #[test]
    fn system_prompt_names_the_recovery_path() {
        let p = default_system_prompt("1.0.0");
        assert!(p.contains("hint"));
        assert!(p.contains("validate_query"));
        assert!(p.contains("Never invent numbers"));
    }
}
