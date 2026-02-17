# Beeno

**Slogan:** "Beeno, run node smarter, but with LLM benefits"

Beeno is a standalone Rust runtime direction with LLM-assisted source compilation.

## Status

M1 foundation is implemented with engine abstraction, compiler abstraction, provider routing, and tests.

Current workspace crates:

- `/Users/arvid/Private/beeno/crates/beeno-cli` (binary `beeno`)
- `/Users/arvid/Private/beeno/crates/beeno-core`
- `/Users/arvid/Private/beeno/crates/beeno-engine`
- `/Users/arvid/Private/beeno/crates/beeno-compiler`
- `/Users/arvid/Private/beeno/crates/beeno-llm`
- `/Users/arvid/Private/beeno/crates/beeno-llm-ollama`
- `/Users/arvid/Private/beeno/crates/beeno-llm-openai`

Legacy Node prototype (transitional only):

- `/Users/arvid/Private/beeno/src`

## Quickstart

Run JavaScript directly:

```bash
cargo run -p beeno -- run /Users/arvid/Private/beeno/examples/hello.js
```

Eval JavaScript expression:

```bash
cargo run -p beeno -- eval "1 + 2 + 3"
```

REPL:

```bash
cargo run -p beeno -- repl
```

## `beeno run` flags

- `--lang <hint>`: source hint (e.g. `pseudocode`, `python`, `js`)
- `--print-js`: print compiled JS before execution
- `--no-cache`: disable compile cache
- `--force-llm`: force translation even for `.js`
- `--provider <auto|ollama|openai>`: provider routing mode (default: `auto`)
- `--ollama-url <url>`: override local Ollama base URL
- `--model <name>`: override model for selected provider

## Provider behavior

Default provider mode is local-first auto routing:

1. Try Ollama if reachable (`http://127.0.0.1:11434` by default)
2. Fall back to OpenAI-compatible HTTP provider

Environment variables:

- `BEENO_OLLAMA_URL` (default: `http://127.0.0.1:11434`)
- `BEENO_OLLAMA_MODEL` (default: `qwen2.5-coder:7b`)
- `OPENAI_API_KEY` (required when OpenAI-compatible provider is used)
- `OPENAI_BASE_URL` (default: `https://api.openai.com/v1`)
- `BEENO_MODEL` (default: `gpt-4.1-mini`)

## Cache

Compile cache is file-based at:

- `$HOME/.beeno/cache/compile`

Cache key includes source hash, source id, language hint, provider id, model id, and prompt version.

## Tests

Run required offline tests:

```bash
cargo test --workspace
```

Live-provider tests are present but ignored by default and gated by:

- `BEENO_RUN_LIVE_TESTS=1`

## Current non-goals

- Node compatibility shims (`node:*`) are not implemented yet.
- Module graph/import loader is deferred.
- Permission model flags are deferred.
