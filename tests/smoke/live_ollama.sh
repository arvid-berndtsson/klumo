#!/usr/bin/env bash
set -euo pipefail

if [[ "${KLUMO_RUN_LIVE_TESTS:-0}" != "1" ]]; then
  echo "[live-ollama] skipped (set KLUMO_RUN_LIVE_TESTS=1 to enable)"
  exit 0
fi

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT_DIR"

if ! curl -fsS "http://127.0.0.1:11434/api/tags" >/dev/null; then
  echo "[live-ollama] Ollama is not reachable at http://127.0.0.1:11434" >&2
  exit 1
fi

mkdir -p "$ROOT_DIR/tests/.tmp"
TMP_DIR="$(mktemp -d "$ROOT_DIR/tests/.tmp/live-ollama.XXXXXX")"
trap 'rm -rf "$TMP_DIR"' EXIT
cp "$ROOT_DIR/tests/fixtures/broken_syntax.js" "$TMP_DIR/broken_syntax.js"

echo "[live-ollama] run with self-heal"
set +e
RUN_OUT="$(cargo run -p klumo -- run "$TMP_DIR/broken_syntax.js" --self-heal --max-heal-attempts 2 --provider ollama 2>&1)"
STATUS=$?
set -e

if [[ $STATUS -ne 0 ]]; then
  echo "[live-ollama] command failed:" >&2
  echo "$RUN_OUT" >&2
  exit $STATUS
fi

if [[ "$RUN_OUT" != *"SELF_HEAL_OK"* ]]; then
  echo "[live-ollama] expected healed run output to contain SELF_HEAL_OK, got:" >&2
  echo "$RUN_OUT" >&2
  exit 1
fi

if [[ ! -f "$TMP_DIR/broken_syntax.js.klumo.bak" ]]; then
  echo "[live-ollama] expected backup file was not created" >&2
  exit 1
fi

echo "[live-ollama] ok"
