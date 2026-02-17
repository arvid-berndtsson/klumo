# Klumo Runtime Architecture (M2 UX)

## Goal

Deliver a Deno-like daily workflow while keeping Klumo standalone and LLM-capable.

## Implemented Layers

- `klumo-cli`: command entrypoint and UX flags.
- `klumo-config`: `klumo.json` loading + env + CLI merge.
- `klumo-core`: run orchestration (`load -> compile -> execute`) and progress modes.
  - also exposes compile-only orchestration for bundling (`load -> compile`).
- `klumo-engine`: `JsEngine` trait + `BoaEngine` backend.
- `klumo-engine-v8`: V8 backend scaffold behind `JsEngine`.
- `klumo-compiler`: source routing + provider/model-aware compile cache.
- `klumo-llm`: provider contracts + routing + normalization.
- `klumo-llm-ollama`: local Ollama adapter.
- `klumo-llm-openai`: OpenAI-compatible adapter.

## Config Resolution

Run defaults are resolved with strict precedence:

1. CLI flags
2. Environment variables
3. `klumo.json`
4. Hardcoded defaults

`klumo.json` currently supports:
- provider, model/base URLs
- lang
- force_llm / print_js / no_cache
- verbose / progress

Unknown fields are rejected.

## Progress Modes

`RunOptions` now carries:
- `ProgressMode::Silent`
- `ProgressMode::Minimal`
- `ProgressMode::Verbose`

Behavior:
- `Silent`: no runtime status logs.
- `Minimal`: shows compile/execute status for LLM path.
- `Verbose`: detailed phase-by-phase diagnostics.

## Provider Routing

Auto mode remains local-first:
1. Try Ollama if reachable.
2. Fallback to OpenAI-compatible provider.

REPL behavior:
- `klumo` and `klumo run` (without file) enter REPL.
- REPL lines are compiled via LLM (pseudocode hint) before execution.

Routing errors are now rendered as readable multi-line attempt summaries.

## Dev Ergonomics

Cargo aliases in `.cargo/config.toml`:
- `cargo klumo ...`
- `cargo btest`

## Bundle Flow

`klumo bundle <file>` uses the same compile path/options as `klumo run` but skips execution and writes the generated JavaScript to disk.

## Self-Heal Flow (Run)

`klumo run <file> --self-heal` adds an error-recovery loop for JavaScript files:
- execute
- on runtime failure, request an LLM-generated full-file patch
- rewrite file and retry (bounded by `--max-heal-attempts`)

## Next Major Milestone

Engine migration from Boa to full V8 implementation (`deno_core` / `rusty_v8`) while preserving current trait boundaries and UX surface.
