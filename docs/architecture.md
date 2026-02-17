# Beeno Runtime Architecture (M1)

## Goal

Build a standalone runtime that executes JavaScript directly and compiles non-JS input through an LLM pipeline without depending on Node.js runtime execution.

## Implemented M1 Components

- `beeno-cli`: command surface (`run`, `eval`, `repl`)
- `beeno-core`: orchestration (`read -> compile -> execute`)
- `beeno-engine`: `JsEngine` trait + `BoaEngine` implementation
- `beeno-compiler`: source-kind routing + compile cache
- `beeno-llm`: provider contracts, output normalization, provider router
- `beeno-llm-ollama`: dedicated Ollama client with reachability preflight
- `beeno-llm-openai`: OpenAI-compatible HTTP client

## Runtime Flow (`beeno run`)

1. Read file source.
2. Resolve source kind from `--lang` or file extension.
3. If JavaScript and not `--force-llm`, passthrough compile.
4. Otherwise compile via LLM provider routing.
5. Provider mode `auto`:
   - use Ollama first if reachable,
   - fallback to OpenAI-compatible provider on failure.
6. Cache compiled JS under provider/model-aware key.
7. Execute generated JS through `JsEngine`.

## Public Internal Interfaces

- `JsEngine` (`beeno-engine`)
- `Compiler` and `CompileCache` (`beeno-compiler`)
- `LlmClient` and `TranslationService` (`beeno-llm`)
- `ProviderRouter` (`beeno-llm`)

## Compile Cache

- Location: `$HOME/.beeno/cache/compile`
- Key fields:
  - source content hash
  - source id
  - language hint
  - provider
  - model
  - prompt version (`m1-v1`)

## Testing Strategy

Required CI tier (offline, deterministic):

- Unit tests for engine behavior, compiler routing, provider routing, normalization, cache behavior.
- Integration-style tests for run flow and failure behavior.
- CLI tests for JS execution, eval output, and missing API key failure path.

Optional/scheduled live tier:

- Ignored tests in provider crates gated by `BEENO_RUN_LIVE_TESTS=1`.
- Ollama and OpenAI-compatible live translation checks.

## Deferred Work

- Node compatibility shims (`node:*`).
- Permission flags and host API capability model.
- Module graph/import execution.
- Advanced diagnostics/source maps.
