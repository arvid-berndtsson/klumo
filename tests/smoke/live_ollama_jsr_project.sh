#!/usr/bin/env bash
set -euo pipefail

if [[ "${KLUMO_RUN_LIVE_TESTS:-0}" != "1" ]]; then
  echo "[live-ollama-jsr] skipped (set KLUMO_RUN_LIVE_TESTS=1 to enable)"
  exit 0
fi

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PROJECT_DIR="$ROOT_DIR/tests/projects/jsr-is-char"
cd "$PROJECT_DIR"

if ! curl -fsS "http://127.0.0.1:11434/api/tags" >/dev/null; then
  echo "[live-ollama-jsr] Ollama is not reachable at http://127.0.0.1:11434" >&2
  exit 1
fi

echo "[live-ollama-jsr] run project script via klumo run start"
set +e
RUN_OUT="$(cargo run -p klumo -- run start 2>&1)"
STATUS=$?
set -e

if [[ $STATUS -ne 0 ]]; then
  echo "[live-ollama-jsr] command failed:" >&2
  echo "$RUN_OUT" >&2
  exit $STATUS
fi

if [[ "$RUN_OUT" != *"isChar(B)=true"* ]]; then
  echo "[live-ollama-jsr] expected output to contain isChar(B)=true, got:" >&2
  echo "$RUN_OUT" >&2
  exit 1
fi

if [[ "$RUN_OUT" != *"isChar(be)=false"* ]]; then
  echo "[live-ollama-jsr] expected output to contain isChar(be)=false, got:" >&2
  echo "$RUN_OUT" >&2
  exit 1
fi

if [[ "$RUN_OUT" != *"JSR_IS_CHAR_OK"* ]]; then
  echo "[live-ollama-jsr] expected output to contain JSR_IS_CHAR_OK, got:" >&2
  echo "$RUN_OUT" >&2
  exit 1
fi

echo "[live-ollama-jsr] ok"
