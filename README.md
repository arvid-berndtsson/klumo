# Beeno

**Slogan:** "Beeno, run node smarter, but with LLM benefits"

Beeno is a standalone runtime in Rust with LLM-assisted compilation for non-JS input.

## Current UX (M2)

- `beeno run <file>` is the primary command.
- Project defaults can live in `beeno.json`.
- Common runs no longer need long flag lists.
- Non-JS runs show minimal progress lines by default.

## Workspace Crates

- `crates/beeno-cli`
- `crates/beeno-config`
- `crates/beeno-core`
- `crates/beeno-engine`
- `crates/beeno-compiler`
- `crates/beeno-llm`
- `crates/beeno-llm-ollama`
- `crates/beeno-llm-openai`

## Quickstart

```bash
cargo run -p beeno -- run examples/hello.js
cargo run -p beeno -- eval "1 + 2 + 3"
cargo run -p beeno -- repl
```

If no input file is provided (`beeno` or `beeno run`), Beeno starts REPL automatically.
REPL input is treated as pseudocode and sent through the LLM compile path before execution.

## Short Dev Commands

Cargo aliases are configured in `.cargo/config.toml`:

```bash
cargo beeno run examples/hello.pseudocode
cargo btest
```

## `beeno run` Flags

- `--config <path>`
- `--lang <hint>`
- `--provider <auto|ollama|openai>`
- `--ollama-url <url>`
- `--model <name>`
- `--force-llm`
- `--print-js`
- `--no-cache`
- `--verbose`
- `--no-progress`

## `beeno.json` (project defaults)

Example:

```json
{
  "provider": "auto",
  "ollama_url": "http://127.0.0.1:11434",
  "ollama_model": "qwen2.5-coder:7b",
  "openai_base_url": "https://api.openai.com/v1",
  "openai_api_key": "sk-...",
  "openai_model": "gpt-4.1-mini",
  "lang": "pseudocode",
  "force_llm": true,
  "print_js": false,
  "no_cache": false,
  "verbose": false,
  "progress": "auto"
}
```

Lookup order:
1. `--config <path>`
2. `./beeno.json`
3. no file

Precedence:
`CLI flags > env vars > beeno.json > defaults`

Note: prefer environment variables for secrets in shared repos.

## Environment Variables

- `BEENO_ENGINE` (`boa` default, `v8` experimental scaffold)
- `BEENO_PROVIDER`
- `BEENO_OLLAMA_URL`
- `BEENO_OLLAMA_MODEL`
- `OPENAI_API_KEY`
- `BEENO_OPENAI_API_KEY`
- `OPENAI_BASE_URL`
- `BEENO_MODEL`
- `BEENO_LANG`
- `BEENO_FORCE_LLM`
- `BEENO_PRINT_JS`
- `BEENO_NO_CACHE`
- `BEENO_VERBOSE`
- `BEENO_PROGRESS`

## Progress Output

Default behavior:
- JS passthrough runs: no status lines.
- LLM compile path: minimal status lines (compile + execute).

Controls:
- `--verbose` for detailed trace.
- `--no-progress` to suppress status lines.

When `--verbose` is used and the run goes through LLM compilation, Beeno prints the generated JavaScript before execution.

## Tests

```bash
cargo test --workspace
```

Live provider tests remain ignored by default and are gated by:

- `BEENO_RUN_LIVE_TESTS=1`

## Deferred

- V8 migration (`deno_core` / `rusty_v8`) is scaffolded via `crates/beeno-engine-v8` and planned for full implementation next.
- Node compatibility shims are not implemented yet.
- Module graph/import loader and permission flags are deferred.
