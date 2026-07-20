#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
GO_CACHE="${GOCACHE:-$ROOT/target/go-cache}"
MODE="${1:---full}"
PYTHON="${PYTHON:-python3}"
MIN_PYTHON_MAJOR=3
MIN_PYTHON_MINOR=11

usage() {
  cat <<'USAGE'
Usage: ./scripts/action-manifest-conformance.sh [--full|--quick]

Runs the shared action-availability expectation manifest across parser
packages, SDKs, and framework adapters, including grouped role/action target
buckets used to scope replay plans.

Requires Python 3.11 or newer because the conformance set includes the
Browser Use adapter. Set PYTHON=/path/to/python to select an interpreter.

  --full   Run each package's normal action-plan test suite. Default.
  --quick  Run the narrow shared-manifest checks for faster CI feedback.
USAGE
}

case "$MODE" in
  --full | --quick)
    ;;
  -h | --help)
    usage
    exit 0
    ;;
  *)
    usage >&2
    exit 2
    ;;
esac

if ! command -v "$PYTHON" >/dev/null 2>&1; then
  printf 'Python interpreter not found: %s\n' "$PYTHON" >&2
  exit 2
fi

if ! "$PYTHON" -c "import sys; raise SystemExit(0 if sys.version_info >= ($MIN_PYTHON_MAJOR, $MIN_PYTHON_MINOR) else 1)"; then
  version="$($PYTHON -c 'import platform; print(platform.python_version())')"
  printf 'Python %s is unsupported; action manifest conformance requires Python %s.%s+. Set PYTHON to a supported interpreter.\n' \
    "$version" "$MIN_PYTHON_MAJOR" "$MIN_PYTHON_MINOR" >&2
  exit 2
fi

run_in() {
  local dir="$1"
  local label="$2"
  shift 2

  printf '\n==> %s\n' "$label"
  (
    cd "$ROOT/$dir"
    "$@"
  )
}

run_in "." \
  "Focused WPT module corpus contract" \
  "$PYTHON" scripts/check-wpt-module-corpus.py

if [ "$MODE" = "--quick" ]; then
  run_in "packages/som-parser-python" \
    "Python parser action manifest" \
    env PYTHONPATH=. "$PYTHON" -m pytest \
      tests/test_parser.py::TestGetActionPlan::test_matches_shared_action_availability_manifest -q

  run_in "packages/som-parser-node" \
    "Node parser action manifest" \
    npm test -- tests/parser.test.ts -t "matches the shared action availability manifest"

  run_in "sdk/go" \
    "Go SDK action manifest" \
    env GOCACHE="$GO_CACHE" go test ./... \
      -run 'Test(GetActionPlanMatchesSharedAvailabilityManifest|ActionPlanLookupHelpers|EnabledActionPlanIndexFiltersBlockedTargets)'

  run_in "sdk/python" \
    "Python SDK action manifest" \
    env PYTHONPATH=src "$PYTHON" -m pytest \
      tests/test_query.py::TestGetActionPlan::test_matches_shared_action_availability_manifest -q

  run_in "sdk/node" \
    "Node SDK action manifest" \
    sh -c 'npm run build && node --test --test-name-pattern "matches the shared action availability manifest" dist/query.test.js'
else
  run_in "packages/som-parser-python" \
    "Python parser action manifest" \
    env PYTHONPATH=. "$PYTHON" -m pytest tests/test_parser.py -q

  run_in "packages/som-parser-node" \
    "Node parser action manifest" \
    npm test

  run_in "sdk/go" \
    "Go SDK action manifest" \
    env GOCACHE="$GO_CACHE" go test ./...

  run_in "sdk/python" \
    "Python SDK action manifest" \
    env PYTHONPATH=src "$PYTHON" -m pytest tests/test_query.py -q

  run_in "sdk/node" \
    "Node SDK action manifest" \
    npm test
fi

run_in "integrations/browser-use" \
  "Browser Use adapter action manifest" \
  env PYTHONPATH="$ROOT/packages/som-parser-python:$ROOT/integrations/browser-use" \
    "$PYTHON" -m pytest tests/test_extractor.py -q

run_in "integrations/langchain" \
  "LangChain adapter action manifest" \
  env PYTHONPATH="$ROOT/packages/som-parser-python:$ROOT/sdk/python/src:$ROOT/integrations/langchain" \
    "$PYTHON" -m pytest tests/test_som_output.py -q

run_in "integrations/vercel-ai" \
  "Vercel AI adapter action manifest" \
  npm test

printf '\nAction manifest conformance passed.\n'
