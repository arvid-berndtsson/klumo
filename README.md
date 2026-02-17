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
cargo run -p beeno -- bundle examples/hello.pseudocode --provider ollama --force-llm -o dist/hello.js
cargo run -p beeno -- eval "1 + 2 + 3"
cargo run -p beeno -- repl
```

## Install (curl)

Install latest release:

```bash
curl -fsSL https://raw.githubusercontent.com/arvid-berndtsson/beeno/main/install.sh | sh
```

Install specific version:

```bash
curl -fsSL https://raw.githubusercontent.com/arvid-berndtsson/beeno/main/install.sh | BEENO_VERSION=v0.1.0 sh
```

Update to latest:

```bash
curl -fsSL https://raw.githubusercontent.com/arvid-berndtsson/beeno/main/install.sh | sh -s -- --update
```

Uninstall:

```bash
curl -fsSL https://raw.githubusercontent.com/arvid-berndtsson/beeno/main/install.sh | sh -s -- --uninstall
```

Optional environment variables:
- `BEENO_GITHUB_REPO` (default `arvid-berndtsson/beeno`)
- `BEENO_VERSION` (default `latest`)
- `BEENO_INSTALL_DIR` (default `~/.local/bin`)

If no input file is provided (`beeno` or `beeno run`), Beeno starts REPL automatically.
REPL input is treated as pseudocode and sent through the LLM compile path before execution.

REPL daemon controls:
- `.web start [--dir <path>] [--port <n>] [--host <ip>] [--open|--no-open|--no-open-prompt]`
- `.web status`, `.web open`, `.web stop`, `.web restart`
- The web daemon serves files directly from disk, so UI hot fixes are live after refresh.
- By default, `.web start` asks whether to open the hosted page in your default browser.

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
- `--self-heal`
- `--max-heal-attempts <n>`
- `--print-js`
- `--no-cache`
- `--verbose`
- `--no-progress`

Self-heal mode:
- When enabled for JS files (`.js/.mjs/.cjs/.jsx`), runtime crashes trigger an LLM patch attempt.
- Beeno rewrites the source file with the generated fix and retries execution.
- A backup is written once to `<file>.beeno.bak` before the first patch.

## `beeno bundle`

Compile a source file to JavaScript without executing it.

Examples:

```bash
beeno bundle examples/hello.js
beeno bundle examples/hello.pseudocode --provider ollama --force-llm -o dist/hello.js
```

Behavior:
- Default output path is `<input>.bundle.js` when `--output` is not provided.
- Uses the same config/env/provider resolution as `beeno run`.
- Produces JS artifacts you can run later with `beeno run <bundle.js>` without LLM translation.

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

Root smoke checks:

```bash
bash tests/smoke/offline.sh
bash tests/smoke/offline_repl.sh
bash tests/smoke/offline_self_heal.sh
BEENO_RUN_LIVE_TESTS=1 bash tests/smoke/live_ollama.sh
BEENO_RUN_LIVE_TESTS=1 bash tests/smoke/live_ollama_react_project.sh
```

Live provider tests remain ignored by default and are gated by:

- `BEENO_RUN_LIVE_TESTS=1`

## Deferred

- V8 migration (`deno_core` / `rusty_v8`) is scaffolded via `crates/beeno-engine-v8` and planned for full implementation next.
- Node compatibility shims are not implemented yet.
- Module graph/import loader and permission flags are deferred.
