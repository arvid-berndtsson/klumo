#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT_DIR"

mkdir -p "$ROOT_DIR/tests/.tmp"
TMP_DIR="$(mktemp -d "$ROOT_DIR/tests/.tmp/offline-self-heal.XXXXXX")"
trap 'rm -rf "$TMP_DIR"' EXIT

cp "$ROOT_DIR/tests/fixtures/broken_runtime.js" "$TMP_DIR/broken_runtime.js"

echo "[offline-self-heal] run with self-heal and missing remote key"
set +e
OUT="$(env -u OPENAI_API_KEY -u KLUMO_OPENAI_API_KEY cargo run -p klumo -- run "$TMP_DIR/broken_runtime.js" --self-heal --max-heal-attempts 1 --provider openai 2>&1)"
STATUS=$?
set -e

if [[ $STATUS -eq 0 ]]; then
  echo "[offline-self-heal] expected failure due to missing OPENAI_API_KEY" >&2
  exit 1
fi

if [[ "$OUT" != *"self-heal attempt 1"* ]]; then
  echo "[offline-self-heal] expected self-heal attempt log, got:" >&2
  echo "$OUT" >&2
  exit 1
fi

if [[ "$OUT" != *"OPENAI_API_KEY is required"* ]]; then
  echo "[offline-self-heal] expected missing key error, got:" >&2
  echo "$OUT" >&2
  exit 1
fi

if [[ ! -f "$TMP_DIR/broken_runtime.js.klumo.bak" ]]; then
  echo "[offline-self-heal] expected backup file to be created" >&2
  exit 1
fi

echo "[offline-self-heal] ok"
