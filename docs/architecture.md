# Beeno Runtime Architecture (M2 UX)

## Goal

Deliver a Deno-like daily workflow while keeping Beeno standalone and LLM-capable.

## Implemented Layers

- `beeno-cli`: command entrypoint and UX flags.
- `beeno-config`: `beeno.json` loading + env + CLI merge.
- `beeno-core`: run orchestration (`load -> compile -> execute`) and progress modes.
- `beeno-engine`: `JsEngine` trait + `BoaEngine` backend.
- `beeno-compiler`: source routing + provider/model-aware compile cache.
- `beeno-llm`: provider contracts + routing + normalization.
- `beeno-llm-ollama`: local Ollama adapter.
- `beeno-llm-openai`: OpenAI-compatible adapter.

## Config Resolution

Run defaults are resolved with strict precedence:

1. CLI flags
2. Environment variables
3. `beeno.json`
4. Hardcoded defaults

`beeno.json` currently supports:
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

Routing errors are now rendered as readable multi-line attempt summaries.

## Dev Ergonomics

Cargo aliases in `.cargo/config.toml`:
- `cargo beeno ...`
- `cargo btest`

## Next Major Milestone

Engine migration from Boa to V8 (`deno_core` / `rusty_v8`) while preserving current trait boundaries and UX surface.
