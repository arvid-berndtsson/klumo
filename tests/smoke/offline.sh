#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT_DIR"

mkdir -p "$ROOT_DIR/tests/.tmp"
TMP_DIR="$(mktemp -d "$ROOT_DIR/tests/.tmp/offline.XXXXXX")"
trap 'rm -rf "$TMP_DIR"' EXIT

echo "[offline] run fixture JS"
RUN_OUT="$(cargo run -p klumo -- run "$ROOT_DIR/tests/fixtures/math.js")"
if [[ "$RUN_OUT" != *"42"* ]]; then
  echo "[offline] expected output to contain 42, got: $RUN_OUT" >&2
  exit 1
fi

echo "[offline] bundle fixture JS"
BUNDLE_OUT="$(cargo run -p klumo -- bundle "$ROOT_DIR/tests/fixtures/math.js" --output "$TMP_DIR/math.bundle.js")"
if [[ "$BUNDLE_OUT" != *"$TMP_DIR/math.bundle.js"* ]]; then
  echo "[offline] expected bundle path in stdout, got: $BUNDLE_OUT" >&2
  exit 1
fi
if [[ ! -f "$TMP_DIR/math.bundle.js" ]]; then
  echo "[offline] expected bundle file at $TMP_DIR/math.bundle.js" >&2
  exit 1
fi

echo "[offline] run bundled JS"
BUNDLED_RUN_OUT="$(cargo run -p klumo -- run "$TMP_DIR/math.bundle.js")"
if [[ "$BUNDLED_RUN_OUT" != *"42"* ]]; then
  echo "[offline] expected bundled output to contain 42, got: $BUNDLED_RUN_OUT" >&2
  exit 1
fi

echo "[offline] ok"
