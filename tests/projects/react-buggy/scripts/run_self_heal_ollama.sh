#!/usr/bin/env bash
set -euo pipefail

if [[ "${KLUMO_RUN_LIVE_TESTS:-0}" != "1" ]]; then
  echo "[react-buggy] skipped (set KLUMO_RUN_LIVE_TESTS=1 to enable)"
  exit 0
fi

PROJECT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$PROJECT_DIR"

if ! curl -fsS "http://127.0.0.1:11434/api/tags" >/dev/null; then
  echo "[react-buggy] Ollama is not reachable at http://127.0.0.1:11434" >&2
  exit 1
fi

echo "[react-buggy] running self-heal on $PROJECT_DIR/src/main.jsx"
set +e
OUT="$(cargo klumo run src/main.jsx --self-heal --max-heal-attempts 2 2>&1)"
STATUS=$?
set -e

echo "$OUT"
if [[ $STATUS -ne 0 ]]; then
  echo "[react-buggy] self-heal run failed" >&2
  exit $STATUS
fi

if [[ ! -f "$PROJECT_DIR/src/main.jsx.klumo.bak" ]]; then
  echo "[react-buggy] expected backup file was not created" >&2
  exit 1
fi

if cmp -s "$PROJECT_DIR/src/main.jsx" "$PROJECT_DIR/src/main.jsx.klumo.bak"; then
  echo "[react-buggy] expected healed file to differ from backup, but no change detected" >&2
  exit 1
fi

echo "[react-buggy] self-heal changed file"
echo "[react-buggy] restore manually from git if needed"
