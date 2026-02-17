#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT_DIR"

echo "[offline-repl] start and exit repl"
OUT="$(printf '.exit\n' | cargo run -p beeno --)"
if [[ "$OUT" != *"Beeno REPL"* ]]; then
  echo "[offline-repl] expected REPL banner, got: $OUT" >&2
  exit 1
fi

echo "[offline-repl] ok"
