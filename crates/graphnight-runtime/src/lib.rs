//! GraphNight's agentic runtime.
//!
//! This crate is the single place where governance, tools, transport and model
//! calls meet. Three properties are deliberate and worth stating up front,
//! because they are what keep an LLM from becoming a liability in front of a
//! data warehouse:
//!
//! 1. **One governed path.** Every query — from GraphQL, REST, MCP, the SDKs or
//!    an agent — goes through [`governance::QueryService`]. There is no
//!    "agent mode" that skips access control.
//! 2. **Errors are instructions.** Tool failures carry a stable `code` and a
//!    `hint`, so a model can correct itself in the next turn instead of
//!    retrying the same mistake.
//! 3. **Bounds are not optional.** The agent loop enforces step, token, time
//!    and repeat budgets. See [`agent`].
//!
//! Nothing here is enabled implicitly: mutations are off, embeddings are off,
//! and the agent is only reachable through an interface that supplies a
//! [`governance::CallContext`] resolved from real credentials.

pub mod agent;
pub mod embedding;
pub mod governance;
pub mod llm;
pub mod mcp;
pub mod memory;
pub mod suggest;
pub mod testing;
pub mod tools;

pub use embedding::{EmbeddingProvider, SharedEmbedder};
pub use governance::{CallContext, ExecuteOptions, QueryOutcome, QueryService, ServiceError};
pub use llm::{ChatMessage, LlmProvider, LlmResponse, ToolCall, Usage};
pub use memory::{MemoryKind, MemoryRecord, MemoryScope};
pub use tools::{ToolCatalog, ToolError, ToolSpec};

/// Server version reported in capabilities and the MCP handshake.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
