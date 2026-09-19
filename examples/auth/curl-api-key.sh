#!/usr/bin/env bash
# Call GraphQL with an API key (Authorization: Bearer or X-API-Key).
#
# Prerequisites:
#   export GRAPHNIGHT_API_KEYS='alice:secret1'
#   export GRAPHNIGHT_ADMIN_KEYS='admin:adminsecret'   # optional, for mutations
#   cargo run -p graphnight-server -- --config examples/graphnight.toml \
#     --storage-path examples/data --host 127.0.0.1 --port 8080
#
# Usage:
#   ./examples/auth/curl-api-key.sh
#   API_KEY=secret1 BASE_URL=http://127.0.0.1:8080 ./examples/auth/curl-api-key.sh

set -euo pipefail

BASE_URL="${BASE_URL:-http://127.0.0.1:8080}"
API_KEY="${API_KEY:-secret1}"
TENANT_ID="${TENANT_ID:-}"

HEADERS=(-H "content-type: application/json" -H "Authorization: Bearer ${API_KEY}")
# Equivalent: -H "X-API-Key: ${API_KEY}"
if [[ -n "${TENANT_ID}" ]]; then
  HEADERS+=(-H "X-Tenant-Id: ${TENANT_ID}")
fi

echo "== health =="
curl -fsS "${BASE_URL}/health"
echo
echo

echo "== dry-run query (authenticated) =="
curl -fsS "${BASE_URL}/graphql" \
  "${HEADERS[@]}" \
  -d @- <<'EOF'
{
  "query": "query { query(input: { name: \"orders\", measures: [{ formula: \"amount_usd\", aggregation: SUM, label: \"Revenue\" }], dimensions: [{ name: \"status\" }], limit: 10 }, dryRun: true) { sql columns executionTimeMs } }"
}
EOF
echo
echo

echo "== list models =="
curl -fsS "${BASE_URL}/graphql" \
  "${HEADERS[@]}" \
  -d '{"query":"query { models { name datasource description } }"}'
echo
