#!/usr/bin/env bash
set -euo pipefail

if [[ "${KLUMO_RUN_LIVE_TESTS:-0}" != "1" ]]; then
  echo "[live-ollama-react] skipped (set KLUMO_RUN_LIVE_TESTS=1 to enable)"
  exit 0
fi

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT_DIR"

bash tests/projects/react-buggy/scripts/run_self_heal_ollama.sh

echo "[live-ollama-react] ok"
