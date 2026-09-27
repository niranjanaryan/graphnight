# Agent, MCP and Tools

GraphNight exposes one governed query engine through three agent-facing surfaces:

| Surface | Use it when |
|---|---|
| **MCP server** | An external agent (Claude Desktop, an MCP client) should reach your semantic layer |
| **Built-in agent** | You want to ask a question in natural language from a terminal |
| **`graphnight tools`** | You want to script or debug a single tool call |

All three sit on the same `QueryService` as GraphQL and REST, so they share one
policy, row cap, column mask set, timeout budget and audit trail. An agent
cannot query a model that an HTTP client could not, and it cannot query one you
have not also granted it.

---

## Quick start

```bash
graphnight init --datasource warehouse
graphnight tools list                 # what can I call?
graphnight agent "monthly revenue by store, last 6 months"
```

---

## Built-in agent

```bash
graphnight agent "which stores had a revenue drop in March?" \
  --model claude-sonnet-4-5 \
  --max-steps 12 \
  --trace
```

The agent is a bounded tool-calling loop. It cannot answer from anything except
the tools, so the semantic layer is the only thing it can see.

| Option | Default | Meaning |
|---|---|---|
| `--model` | — | Anthropic model id, or a model name when using `--base-url` |
| `--base-url` | — | Any OpenAI-compatible endpoint (Ollama, vLLM, OpenRouter, …) |
| `--api-key` | — | Provider key. Prefer `GRAPHNIGHT_LLM_API_KEY` so it stays out of your shell history |
| `--max-steps` | `12` | Maximum model round-trips |
| `--max-tokens` | `200000` | Token budget for the whole run |
| `--timeout` | `120` | Wall-clock budget in seconds |
| `--trace` | off | Print each tool call as it happens |

Every flag also reads an environment variable (`GRAPHNIGHT_LLM_MODEL`,
`GRAPHNIGHT_LLM_BASE_URL`, `GRAPHNIGHT_LLM_API_KEY`).

The run stops on the first of: the model answering, the step budget, the token
budget, or the timeout. A failed run exits non-zero, so scripts can branch on it.

### Typical loop

1. `get_capabilities` — what this server supports
2. `search_models` — turn intent into a model name
3. `get_model` — real field names and worked examples
4. `validate_query` — compile without touching the database
5. `run_query` — the answer

`--trace` makes this visible, which is the fastest way to see why an answer went
wrong.

---

## MCP server

### Local (stdio)

For a desktop agent that launches the server itself:

```bash
graphnight mcp --stdio --user alice --tenant acme
```

Configuration equivalent (Claude Desktop `claude_desktop_config.json`):

```json
{
  "mcpServers": {
    "graphnight": {
      "command": "graphnight",
      "args": ["mcp", "--stdio", "--user", "alice", "--tenant", "acme"],
      "env": { "GRAPHNIGHT_STORAGE_PATH": "/srv/graphnight_data" }
    }
  }
}
```

`--user` and `--tenant` also read `GRAPHNIGHT_MCP_USER` and
`GRAPHNIGHT_MCP_TENANT`.

A stdio server has no credential channel, so it cannot demand a token: whatever
identity you pass is trusted out of band. **That is why you should not expose a
stdio server on a network** — run the HTTP server behind real auth instead.

### HTTP

```
POST /mcp
```

Send JSON-RPC 2.0 with `Content-Type: application/json`. Authenticate with the
same headers as the REST API:

```bash
curl -sS localhost:8080/mcp \
  -H "Authorization: Bearer $GRAPHNIGHT_API_KEY" \
  -H "X-Tenant-Id: acme" \
  -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/list"}'
```

Responses are `200` with a JSON-RPC body. A JSON-RPC *notification* (no `id`)
returns `204 No Content`, per the MCP spec. A missing or invalid credential
returns `401`; a policy denial returns `403`.

### Mutations are off by default

The six model/datasource tools are hidden unless you opt in, and they still
require an admin identity:

```bash
graphnight mcp --stdio --allow-mutations --user admin
```

The HTTP server takes the same flag via `GRAPHNIGHT_MCP_MUTATIONS=1`.

Two independent gates apply, on purpose:

- **is the tool exposed?** — opt-in flag or env var
- **may this caller use it?** — the caller's own admin authority

Turning on the first does not grant the second. A non-admin caller sees a
policy error, not a silent success.

---

## Tools CLI

```bash
graphnight tools list              # read-only tools
graphnight tools list --all        # include mutating tools
graphnight tools show run_query    # input schema
graphnight tools call get_model '{"name": "orders"}'
```

`tools call` prints JSON to stdout and exits non-zero on failure, with the error
as JSON on stderr, so it composes with `jq` and shell conditionals.

### The read-only tools

| Tool | Purpose |
|---|---|
| `get_capabilities` | Dialects, aggregations, granularities, formula functions, row limits |
| `search_models` | Rank models by intent; policy-filtered |
| `list_models` | Models this caller may query |
| `get_model` | Full schema plus worked example queries |
| `validate_query` | Compile without executing; precise errors and suggestions |
| `run_query` | Execute a governed query |
| `multi_stage_query` | Ordered DAG where a stage filters on a previous one |
| `recall_memories` | Relevant stored learnings, ranked |
| `remember` | Store a durable learning (requires an identity) |
| `tools_list` | What this caller may call |

### The mutating tools

`create_model`, `update_model`, `delete_model`, `create_datasource`,
`update_datasource`, `delete_datasource`.

All require admin. Datasource connection strings are redacted in every response,
and a datasource that still has models attached cannot be deleted.

---

## Hybrid retrieval

`search_models` and `list_models` are lexical by default (BM25 over the model
index), which is fast and needs nothing configured. Point GraphNight at an
OpenAI-compatible `/embeddings` endpoint to add a dense leg:

| Variable | Example | Purpose |
|---|---|---|
| `GRAPHNIGHT_EMBEDDING_BASE_URL` | `https://api.openai.com/v1` | Endpoint root; `/embeddings` is appended |
| `GRAPHNIGHT_EMBEDDING_MODEL` | `text-embedding-3-small` | Embedding model |
| `GRAPHNIGHT_EMBEDDING_DIMS` | `1536` | Vector width; a mismatch is reported, not ignored |
| `GRAPHNIGHT_EMBEDDING_API_KEY` | `sk-…` | Optional; omit for a local server that needs no auth |

Works with OpenAI and anything speaking the same shape (Ollama, vLLM, TEI).

When both legs are active, results are fused with reciprocal rank fusion, and
each result reports how it was found: `hybrid`, `lexical` or `semantic`. That
matters because the dense leg is what recovers a model whose wording shares
nothing with your question but whose meaning matches.

Behaviour worth knowing:

- **Lexical is never lost.** A dense-only hit can be promoted, but the two score
  scales are combined by rank, so neither can swamp the other.
- **The embedder cannot break search.** If the provider errors, times out or is
  misconfigured, search logs a warning and returns lexical results.
- **Model embeddings are cached** in memory and recomputed only when a model's
  description or fields change.
- **Policy still applies after fusion.** A denied model cannot be surfaced by
  the dense leg either.
- A misconfigured `GRAPHNIGHT_EMBEDDING_*` value is logged and ignored at
  startup rather than preventing the server from starting.

Verify which mode is active:

```bash
graphnight tools call search_models '{"q": "monthly revenue by store"}'
# "retrieval": "hybrid"   (or "lexical" if no provider is configured)
```

---

## Governance

Everything an agent does is subject to the same rules as an HTTP request:

- **Model and datasource allow/deny lists** — an agent cannot reach a model you
  have not granted it
- **Forced filters and RLS** — merged into the query before generation
- **Row cap** — clamped regardless of the limit the agent asks for
- **Column masks** — applied after the database returns
- **Timeouts** — a runaway query is cut off
- **Audit** — every execution records the caller's identity

`search_models` and `list_models` are filtered too, so an agent cannot discover
the existence of a model it may not query.

`remember` is not a semantic-layer mutation: it stores caller-scoped knowledge
and is always available to an identified caller. What it cannot do is read
another user's memories.

---

## See also

- [Authentication](auth.md) — API keys, tenants, admin identity
- [CLI Usage](usage-cli.md) — models, datasources, memory
- [GraphQL Usage](usage-graphql.md) — the same service over GraphQL
