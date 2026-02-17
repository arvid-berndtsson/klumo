#!/usr/bin/env bash
set -euo pipefail

PROJECT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$PROJECT_DIR"

mkdir -p dist
exec cargo beeno bundle src/main.js --config beeno.runtime.json --output dist/main.bundle.js
