#!/usr/bin/env bash
set -euo pipefail

PROJECT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

rm -rf "$PROJECT_DIR/src"
mkdir -p "$PROJECT_DIR/src"
cp -R "$PROJECT_DIR/buggy-baseline/src/." "$PROJECT_DIR/src/"
cp "$PROJECT_DIR/buggy-baseline/package.json" "$PROJECT_DIR/package.json"
cp "$PROJECT_DIR/buggy-baseline/beeno.json" "$PROJECT_DIR/beeno.json"

find "$PROJECT_DIR/src" -name "*.beeno.bak" -delete

echo "[react-buggy] restored buggy baseline"
