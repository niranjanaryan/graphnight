#!/usr/bin/env bash
# ============================================================================
#  Double-blind artifact mirror for CIDR / VLDB Systems & Industry submission.
#
#  Produces an anonymous copy of the repository (no author metadata, no
#  identifiable URLs in the paper body) plus a compiled PDF, so the submission
#  artifact can be shared with reviewers before the decision is public.
#
#  Usage:  ./scripts/make-anon-mirror.sh [output-dir]
#  Default output-dir: ./anon-mirror
# ============================================================================

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="${1:-$ROOT/anon-mirror}"
PAPER_DIR="$ROOT/docs/paper"

echo "==> Building anonymous mirror at $OUT"

# 1. Copy the repository, excluding VCS, build artifacts, and local config.
rm -rf "$OUT"
mkdir -p "$OUT"

rsync -a \
  --exclude='.git' \
  --exclude='target' \
  --exclude='**/*.pdf' \
  --exclude='anon-mirror' \
  --exclude='.kilo' \
  --exclude='AGENTS.md' \
  --exclude='.agents' \
  "$ROOT/" "$OUT/"

# 2. Build an anonymous paper source from the canonical one.
mkdir -p "$OUT/docs/paper"

python3 - "$PAPER_DIR/paper.tex" "$OUT/docs/paper/paper-anon.tex" <<'PY'
import re
import sys

src, dst = sys.argv[1], sys.argv[2]
text = open(src).read()

# Replace the \author{...} block (balanced braces) with an anonymous placeholder.
def replace_author(s):
    start = s.find('\\author{')
    if start == -1:
        return s
    depth = 0
    i = start + len('\\author{')
    while i < len(s):
        if s[i] == '{':
            depth += 1
        elif s[i] == '}':
            if depth == 0:
                return s[:start] + '\\author{Anonymous Authors}' + s[i + 1:]
            depth -= 1
        i += 1
    return s

text = replace_author(text)

# Replace the identifiable repo URL in the Availability section.
text = text.replace(
    '\\url{https://github.com/niranjanaryan/graphnight}',
    '\\url{https://github.com/anonymous/graphnight}',
)

open(dst, 'w').write(text)
PY

# 3. Compile the anonymous paper (twice, for references).
cd "$OUT/docs/paper"
pdflatex -interaction=nonstopmode paper-anon.tex >/dev/null 2>&1 || true
pdflatex -interaction=nonstopmode paper-anon.tex >/dev/null 2>&1 || true

if [ -f paper-anon.pdf ]; then
  echo "==> Anonymous PDF: $OUT/docs/paper/paper-anon.pdf"
else
  echo "==> WARNING: anonymous PDF did not compile; check TeX Live basic deps." >&2
fi

echo "==> Mirror complete."