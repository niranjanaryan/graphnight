//! MCP (Model Context Protocol) server: JSON-RPC 2.0 over stdio and HTTP.
//!
//! Exposes the same [`ToolCatalog`] an in-process agent uses, so an external
//! agent — Claude Desktop, an IDE, another service — gets identical governance
//! with no separate code path to keep in sync.
//!
//! Three things this deliberately does not do:
//!
//! - **No ambient authority.** [`resolve_context`] builds the caller's
//!   [`CallContext`] from the transport's own credentials. A stdio server
//!   inherits the operator's session; an HTTP server requires a real token. The
//!   MCP layer cannot grant access the caller did not already have.
//! - **No raw SQL.** There is no `execute_sql` tool. A client that wants SQL
//!   gets the governed generated statement, and only if it asked for it.
//! - **No silent coercion.** A JSON-RPC error keeps its code, so a client can
//!   distinguish "unknown tool" from "policy denied" without parsing prose.

use crate::governance::{CallContext, QueryService};
use crate::tools::ToolCatalog;
use serde_json::{json, Value};

/// JSON-RPC protocol version this server implements.
pub const PROTOCOL_VERSION: &str = "2024-11-05";

/// A JSON-RPC request, as received on the wire.
#[derive(Debug, Clone, PartialEq)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    /// Absent for notifications, which expect no reply.
    pub id: Option<Value>,
    pub method: String,
    pub params: Value,
}

/// A JSON-RPC response.
#[derive(Debug, Clone, PartialEq)]
pub struct JsonRpcResponse {
    pub jsonrpc: &'static str,
    pub id: Value,
    /// `None` is omitted from the wire form, per JSON-RPC 2.0.
    pub result: Option<Value>,
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    pub data: Option<Value>,
}

impl JsonRpcError {
    pub fn new(code: i32, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }

    pub fn with_data(mut self, data: Value) -> Self {
        self.data = Some(data);
        self
    }

    pub fn to_json(&self) -> Value {
        json!({
            "code": self.code,
            "message": self.message,
            "data": self.data,
        })
    }
}

impl JsonRpcResponse {
    pub fn ok(id: Value, result: Value) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: Some(result),
            error: None,
        }
    }

    /// A null id, for a request whose id could not be parsed.
    pub fn null_id(error: JsonRpcError) -> Self {
        Self {
            jsonrpc: "2.0",
            id: Value::Null,
            result: None,
            error: Some(error),
        }
    }

    pub fn to_json(&self) -> Value {
        let mut out = json!({"jsonrpc": self.jsonrpc, "id": self.id});
        if let Some(r) = &self.result {
            out["result"] = r.clone();
        }
        if let Some(e) = &self.error {
            out["error"] = e.to_json();
        }
        out
    }
}

/// JSON-RPC error codes. The first block is spec-defined; the second is
/// GraphNight's, so a client can branch on a tool failure without string
/// matching.
pub mod error_codes {
    pub const PARSE_ERROR: i32 = -32700;
    pub const INVALID_REQUEST: i32 = -32600;
    pub const METHOD_NOT_FOUND: i32 = -32601;
    pub const INVALID_PARAMS: i32 = -32602;
    pub const INTERNAL_ERROR: i32 = -32603;

    /// A governed tool refused or failed. `data.code` carries the tool error code.
    pub const TOOL_ERROR: i32 = -32000;
}

/// Parse one line of JSON-RPC input.
///
/// Returns `None` for a notification (a request with no `id`), which the
/// transport must not reply to.
pub fn parse_request(line: &str) -> Result<Option<JsonRpcRequest>, JsonRpcError> {
    let value: Value = serde_json::from_str(line.trim())
        .map_err(|e| JsonRpcError::new(error_codes::PARSE_ERROR, format!("invalid JSON: {e}")))?;

    if !value.is_object() {
        return Err(JsonRpcError::new(
            error_codes::INVALID_REQUEST,
            "request must be a JSON object",
        ));
    }
    if value["jsonrpc"] != "2.0" {
        return Err(JsonRpcError::new(
            error_codes::INVALID_REQUEST,
            "jsonrpc must be exactly \"2.0\"",
        ));
    }
    let Some(method) = value["method"].as_str() else {
        return Err(JsonRpcError::new(
            error_codes::INVALID_REQUEST,
            "request is missing `method`",
        ));
    };
    let params = value.get("params").cloned().unwrap_or(Value::Null);
    let id = value.get("id").cloned().filter(|v| !v.is_null());

    Ok(Some(JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        id,
        method: method.to_string(),
        params,
    }))
}

/// The MCP server.
pub struct McpServer {
    catalog: ToolCatalog,
    service: std::sync::Arc<QueryService>,
    server_name: String,
    server_version: String,
}

impl McpServer {
    pub fn new(catalog: ToolCatalog, service: std::sync::Arc<QueryService>) -> Self {
        Self {
            catalog,
            service,
            server_name: "graphnight".to_string(),
            server_version: crate::VERSION.to_string(),
        }
    }

    /// Open the semantic-layer mutation gate. Off by default, so an agent
    /// cannot author models or datasources unless an operator says so.
    pub fn with_mutations(self, allow: bool) -> Self {
        let service = self.service.clone();
        Self {
            catalog: self.catalog.with_mutations(allow),
            service,
            server_name: self.server_name,
            server_version: self.server_version,
        }
    }

    pub fn with_identity(mut self, name: impl Into<String>, version: impl Into<String>) -> Self {
        self.server_name = name.into();
        self.server_version = version.into();
        self
    }

    /// Handle one request line, returning the response line to write, if any.
    pub async fn handle_line(&self, ctx: &CallContext, line: &str) -> Option<String> {
        match parse_request(line) {
            Ok(Some(request)) => {
                // A notification (no id) gets no reply.
                request.id.as_ref()?;
                let response = self.handle(ctx, request).await;
                Some(response.to_json().to_string())
            }
            Ok(None) => None,
            Err(e) => Some(JsonRpcResponse::null_id(e).to_json().to_string()),
        }
    }

    /// Handle a parsed request.
    pub async fn handle(&self, ctx: &CallContext, request: JsonRpcRequest) -> JsonRpcResponse {
        let id = request.id.clone().unwrap_or(Value::Null);
        match request.method.as_str() {
            "initialize" => JsonRpcResponse::ok(
                id,
                json!({
                    "protocolVersion": PROTOCOL_VERSION,
                    "capabilities": {
                        // Tools are the only capability; GraphNight exposes no
                        // prompts or resources, and claiming otherwise would make
                        // clients wait on features that do not exist.
                        "tools": {"listChanged": false}
                    },
                    "serverInfo": {
                        "name": self.server_name,
                        "version": self.server_version
                    },
                    "instructions": "Query the semantic layer through tools. \
                        Call search_models or list_models to find a model, get_model to read \
                        its real fields, validate_query before run_query, and run_query for \
                        rows. All queries are subject to the caller's access policy."
                }),
            ),
            "ping" => JsonRpcResponse::ok(id, json!({})),
            "tools/list" => {
                JsonRpcResponse::ok(id, json!({"tools": self.catalog.mcp_descriptors()}))
            }
            "tools/call" => {
                let name = request.params["name"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string();
                if name.is_empty() {
                    return JsonRpcResponse {
                        jsonrpc: "2.0",
                        id,
                        result: None,
                        error: Some(JsonRpcError::new(
                            error_codes::INVALID_PARAMS,
                            "tools/call requires `name`",
                        )),
                    };
                }
                let arguments = request
                    .params
                    .get("arguments")
                    .cloned()
                    .unwrap_or_else(|| json!({}));
                self.call_tool(ctx, id, &name, arguments).await
            }
            // `notifications/initialized` is a notification; a client that sends
            // it with an id still deserves an acknowledgement rather than a
            // "method not found".
            "notifications/initialized" | "initialized" => JsonRpcResponse::ok(id, json!({})),
            other => JsonRpcResponse {
                jsonrpc: "2.0",
                id,
                result: None,
                error: Some(
                    JsonRpcError::new(
                        error_codes::METHOD_NOT_FOUND,
                        format!("unknown method {other:?}"),
                    )
                    .with_data(json!({
                        "supported": ["initialize", "ping", "tools/list", "tools/call"]
                    })),
                ),
            },
        }
    }

    async fn call_tool(
        &self,
        ctx: &CallContext,
        id: Value,
        name: &str,
        arguments: Value,
    ) -> JsonRpcResponse {
        match self.catalog.call(ctx, name, arguments).await {
            Ok(result) => JsonRpcResponse::ok(
                id,
                json!({
                    "content": [{
                        "type": "text",
                        "text": serde_json::to_string_pretty(&result)
                            .unwrap_or_else(|_| result.to_string()),
                    }],
                    // Echoed so a client can correlate without parsing the text.
                    "structuredContent": result,
                    "isError": false,
                }),
            ),
            Err(e) => {
                // A tool failure is reported as a successful JSON-RPC call with
                // `isError: true`, per the MCP convention: the *call* succeeded,
                // the *tool* did not. Returning a JSON-RPC error instead would
                // make clients treat a correctable field typo as a transport
                // failure and stop.
                self.service
                    .audit(ctx, name, 0, 0, false, Some(e.code.clone()));
                JsonRpcResponse::ok(
                    id,
                    json!({
                        "content": [{
                            "type": "text",
                            "text": serde_json::to_string_pretty(&e.to_json())
                                .unwrap_or_else(|_| e.to_string()),
                        }],
                        "structuredContent": e.to_json(),
                        "isError": true,
                    }),
                )
            }
        }
    }
}

/// Resolve the caller's context for a stdio connection.
///
/// A stdio MCP server is a child process of its client, so it has no
/// per-request credentials of its own. The honest options are: inherit an
/// explicitly configured identity, or require the client to pass one. Silently
/// granting an unauthenticated context here would mean any local process could
/// start the server and query everything.
pub fn resolve_context(
    user_id: Option<String>,
    tenant_id: Option<String>,
    is_admin: bool,
    auth_required: bool,
) -> CallContext {
    CallContext {
        user_id,
        tenant_id,
        is_admin,
        auth_required,
        policy: None,
        principal_label: Some("mcp:stdio".to_string()),
    }
}

/// Extract a bearer token from an HTTP `Authorization` header.
///
/// Returns `None` for anything that is not exactly `Bearer <token>`, so a
/// malformed header is treated as absent rather than as a valid identity.
pub fn bearer_token(header: Option<&str>) -> Option<String> {
    let value = header?.trim();
    let token = value
        .strip_prefix("Bearer ")
        .or_else(|| value.strip_prefix("bearer "))?;
    let token = token.trim();
    if token.is_empty() {
        None
    } else {
        Some(token.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::open_context;

    fn server() -> McpServer {
        let service = crate::testing::test_service();
        McpServer::new(ToolCatalog::new(service.clone()), service)
    }

    #[test]
    fn parses_a_request() {
        let r = parse_request(r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#)
            .unwrap()
            .unwrap();
        assert_eq!(r.method, "tools/list");
        assert_eq!(r.id, Some(json!(1)));
    }

    #[test]
    fn notification_has_no_id() {
        let r = parse_request(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#)
            .unwrap()
            .unwrap();
        assert_eq!(r.id, None);
    }

    #[test]
    fn rejects_wrong_jsonrpc_version() {
        let e = parse_request(r#"{"jsonrpc":"1.0","id":1,"method":"x"}"#).unwrap_err();
        assert_eq!(e.code, error_codes::INVALID_REQUEST);
    }

    #[test]
    fn rejects_malformed_json() {
        let e = parse_request("{not json").unwrap_err();
        assert_eq!(e.code, error_codes::PARSE_ERROR);
    }

    #[test]
    fn rejects_non_object() {
        assert!(parse_request("[1,2,3]").is_err());
    }

    #[tokio::test]
    async fn initialize_advertises_tools_only() {
        let s = server();
        let r = s
            .handle(
                &open_context(),
                JsonRpcRequest {
                    jsonrpc: "2.0".into(),
                    id: Some(json!(1)),
                    method: "initialize".into(),
                    params: json!({}),
                },
            )
            .await;
        let result = r.result.unwrap();
        assert_eq!(result["protocolVersion"], PROTOCOL_VERSION);
        let caps = result["capabilities"].clone();
        assert!(caps.get("tools").is_some());
        assert!(caps.get("prompts").is_none());
        assert!(caps.get("resources").is_none());
    }

    #[tokio::test]
    async fn tools_list_includes_schemas() {
        let s = server();
        let r = s
            .handle(
                &open_context(),
                JsonRpcRequest {
                    jsonrpc: "2.0".into(),
                    id: Some(json!(1)),
                    method: "tools/list".into(),
                    params: json!({}),
                },
            )
            .await;
        let tools = r.result.unwrap()["tools"].as_array().unwrap().clone();
        assert!(tools.iter().any(|t| t["name"] == "run_query"));
        for t in &tools {
            assert!(
                t["inputSchema"].is_object(),
                "tool {} has no schema",
                t["name"]
            );
            assert!(t["description"].is_string());
        }
    }

    #[tokio::test]
    async fn mutations_are_absent_by_default() {
        let s = server();
        let r = s
            .handle(
                &open_context(),
                JsonRpcRequest {
                    jsonrpc: "2.0".into(),
                    id: Some(json!(1)),
                    method: "tools/list".into(),
                    params: json!({}),
                },
            )
            .await;
        let tools = r.result.unwrap();
        let names: Vec<&str> = tools["tools"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|t| t["name"].as_str())
            .collect();
        for gated in [
            "create_model",
            "update_model",
            "delete_model",
            "create_datasource",
            "update_datasource",
            "delete_datasource",
        ] {
            assert!(
                !names.contains(&gated),
                "{gated} authors the semantic layer and must be hidden by default: {names:?}"
            );
        }
        // Reads and memory recall stay available; only writes to the semantic
        // layer are withheld.
        assert!(names.contains(&"run_query"), "{names:?}");
        assert!(names.contains(&"recall_memories"), "{names:?}");
    }

    #[tokio::test]
    async fn mutation_tools_appear_only_when_the_gate_is_open() {
        let s = server().with_mutations(true);
        let r = s
            .handle(
                &open_context(),
                JsonRpcRequest {
                    jsonrpc: "2.0".into(),
                    id: Some(json!(1)),
                    method: "tools/list".into(),
                    params: json!({}),
                },
            )
            .await;
        let tools = r.result.unwrap();
        let names: Vec<&str> = tools["tools"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|t| t["name"].as_str())
            .collect();
        assert!(names.contains(&"create_model"), "{names:?}");
    }

    #[tokio::test]
    async fn gated_tool_call_is_refused_with_a_hint() {
        let s = server();
        let r = s
            .handle(
                &open_context(),
                JsonRpcRequest {
                    jsonrpc: "2.0".into(),
                    id: Some(json!(2)),
                    method: "tools/call".into(),
                    params: json!({"name": "delete_model", "arguments": {"name": "orders"}}),
                },
            )
            .await;
        // Refused as a tool error, not a protocol error, so the model can read
        // the hint and stop trying.
        assert!(r.error.is_none());
        let result = r.result.unwrap();
        assert_eq!(result["isError"], true);
        assert_eq!(result["structuredContent"]["code"], "UNKNOWN_TOOL");
    }

    #[tokio::test]
    async fn unknown_method_returns_method_not_found() {
        let s = server();
        let r = s
            .handle(
                &open_context(),
                JsonRpcRequest {
                    jsonrpc: "2.0".into(),
                    id: Some(json!(7)),
                    method: "resources/list".into(),
                    params: json!({}),
                },
            )
            .await;
        let e = r.error.unwrap();
        assert_eq!(e.code, error_codes::METHOD_NOT_FOUND);
        assert!(e.data.unwrap()["supported"].is_array());
    }

    #[tokio::test]
    async fn tool_error_is_a_result_with_is_error_not_a_transport_error() {
        let s = server();
        let r = s
            .handle(
                &open_context(),
                JsonRpcRequest {
                    jsonrpc: "2.0".into(),
                    id: Some(json!(2)),
                    method: "tools/call".into(),
                    params: json!({"name": "no_such_tool", "arguments": {}}),
                },
            )
            .await;
        assert!(
            r.error.is_none(),
            "tool failure must not be a JSON-RPC error"
        );
        let result = r.result.unwrap();
        assert_eq!(result["isError"], true);
        assert_eq!(result["structuredContent"]["code"], "UNKNOWN_TOOL");
        assert!(result["structuredContent"]["hint"].is_string());
    }

    #[tokio::test]
    async fn tools_call_without_name_is_invalid_params() {
        let s = server();
        let r = s
            .handle(
                &open_context(),
                JsonRpcRequest {
                    jsonrpc: "2.0".into(),
                    id: Some(json!(3)),
                    method: "tools/call".into(),
                    params: json!({}),
                },
            )
            .await;
        assert_eq!(r.error.unwrap().code, error_codes::INVALID_PARAMS);
    }

    #[tokio::test]
    async fn capabilities_tool_works_over_mcp() {
        let s = server();
        let r = s
            .handle(
                &open_context(),
                JsonRpcRequest {
                    jsonrpc: "2.0".into(),
                    id: Some(json!(4)),
                    method: "tools/call".into(),
                    params: json!({"name": "get_capabilities", "arguments": {}}),
                },
            )
            .await;
        let result = r.result.unwrap();
        assert_eq!(result["isError"], false);
        assert!(result["structuredContent"]["sql_dialects"].is_array());
    }

    #[tokio::test]
    async fn notifications_get_no_reply() {
        let s = server();
        assert_eq!(
            s.handle_line(
                &open_context(),
                r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#
            )
            .await,
            None
        );
    }

    #[tokio::test]
    async fn handle_line_returns_serialised_response() {
        let s = server();
        let line = s
            .handle_line(
                &open_context(),
                r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#,
            )
            .await
            .unwrap();
        let parsed: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(parsed["id"], 1);
        assert_eq!(parsed["result"], json!({}));
    }

    #[test]
    fn bearer_token_parsing() {
        assert_eq!(bearer_token(Some("Bearer abc")), Some("abc".into()));
        assert_eq!(bearer_token(Some("bearer abc")), Some("abc".into()));
        assert_eq!(bearer_token(Some("Bearer  abc ")), Some("abc".into()));
        assert_eq!(bearer_token(Some("Basic abc")), None);
        assert_eq!(bearer_token(Some("Bearer   ")), None);
        assert_eq!(bearer_token(None), None);
    }

    #[test]
    fn stdio_context_is_not_silently_admin() {
        let ctx = resolve_context(None, None, false, true);
        assert!(ctx.user_id.is_none());
        assert!(!ctx.is_admin);
        assert!(ctx.auth_required);
        // And an unauthenticated caller must be refused.
        assert!(ctx.require_authenticated().is_err());
    }
}
