# Klumo

**Slogan:** "Klumo, run node smarter, but with LLM benefits"

Klumo is a standalone runtime in Rust with LLM-assisted compilation for non-JS input.

## Current UX (M2)

- `klumo run <file>` is the primary command.
- Project defaults can live in `klumo.json`.
- Common runs no longer need long flag lists.
- Non-JS runs show minimal progress lines by default.

## Workspace Crates

- `crates/klumo-cli`
- `crates/klumo-config`
- `crates/klumo-core`
- `crates/klumo-engine`
- `crates/klumo-compiler`
- `crates/klumo-llm`
- `crates/klumo-llm-ollama`
- `crates/klumo-llm-openai`

## Quickstart

```bash
cargo run -p klumo -- run examples/hello.js
cargo run -p klumo -- bundle examples/hello.pseudocode --provider ollama --force-llm -o dist/hello.js
cargo run -p klumo -- eval "1 + 2 + 3"
cargo run -p klumo -- repl
```

## Install (curl)

Install latest release:

```bash
curl -fsSL https://raw.githubusercontent.com/arvid-berndtsson/klumo/main/install.sh | sh
```

Install specific version:

```bash
curl -fsSL https://raw.githubusercontent.com/arvid-berndtsson/klumo/main/install.sh | KLUMO_VERSION=v0.1.0 sh
```

Update to latest:

```bash
curl -fsSL https://raw.githubusercontent.com/arvid-berndtsson/klumo/main/install.sh | sh -s -- --update
```

Uninstall:

```bash
curl -fsSL https://raw.githubusercontent.com/arvid-berndtsson/klumo/main/install.sh | sh -s -- --uninstall
```

Optional environment variables:
- `KLUMO_GITHUB_REPO` (default `arvid-berndtsson/klumo`)
- `KLUMO_VERSION` (default `latest`)
- `KLUMO_INSTALL_DIR` (default `~/.local/bin`)

If no input file is provided (`klumo` or `klumo run`), Klumo starts REPL automatically.
REPL input is treated as pseudocode and sent through the LLM compile path before execution.

## Short Dev Commands

Cargo aliases are configured in `.cargo/config.toml`:

```bash
cargo klumo run examples/hello.pseudocode
cargo btest
```

## `klumo run` Flags

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

JSR support:
- JavaScript input containing `jsr:` specifiers is automatically routed through the LLM compile path (equivalent to force-LLM behavior for that source).

Self-heal mode:
- When enabled for JS files (`.js/.mjs/.cjs/.jsx`), runtime crashes trigger an LLM patch attempt.
- Klumo rewrites the source file with the generated fix and retries execution.
- A backup is written once to `<file>.klumo.bak` before the first patch.

## `klumo bundle`

Compile a source file to JavaScript without executing it.

Examples:

```bash
klumo bundle examples/hello.js
klumo bundle examples/hello.pseudocode --provider ollama --force-llm -o dist/hello.js
```

Behavior:
- Default output path is `<input>.bundle.js` when `--output` is not provided.
- Uses the same config/env/provider resolution as `klumo run`.
- Produces JS artifacts you can run later with `klumo run <bundle.js>` without LLM translation.

## `klumo.json` (project defaults)

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
2. `./klumo.json`
3. no file

Precedence:
`CLI flags > env vars > klumo.json > defaults`

Note: prefer environment variables for secrets in shared repos.

## Environment Variables

- `KLUMO_ENGINE` (`boa` default, `v8` experimental scaffold)
- `KLUMO_PROVIDER`
- `KLUMO_OLLAMA_URL`
- `KLUMO_OLLAMA_MODEL`
- `OPENAI_API_KEY`
- `KLUMO_OPENAI_API_KEY`
- `OPENAI_BASE_URL`
- `KLUMO_MODEL`
- `KLUMO_LANG`
- `KLUMO_FORCE_LLM`
- `KLUMO_PRINT_JS`
- `KLUMO_NO_CACHE`
- `KLUMO_VERBOSE`
- `KLUMO_PROGRESS`

## Progress Output

Default behavior:
- JS passthrough runs: no status lines.
- LLM compile path: minimal status lines (compile + execute).

Controls:
- `--verbose` for detailed trace.
- `--no-progress` to suppress status lines.

When `--verbose` is used and the run goes through LLM compilation, Klumo prints the generated JavaScript before execution.

## REPL Web APIs

Inside REPL, Klumo now exposes a web daemon and route controls both as dot-commands and JavaScript APIs.

Dot-commands:
- `.web start [--dir <path>] [--port <n>] [--host <ip>] [--open|--no-open|--no-open-prompt]`
- `.web status`, `.web open`, `.web stop`, `.web restart`

JavaScript APIs:
- `klumo.web.start({ dir, port, host, open, noOpenPrompt })`
- `klumo.web.stop()`
- `klumo.web.restart({ dir, port, host, open })`
- `klumo.web.open()`
- `klumo.web.status()`
- `klumo.web.routeJson(path, payload, { status })`
- `klumo.web.routeText(path, text, { status, contentType })`
- `klumo.web.unroute(path)`

Notes:
- Static files are served directly from disk, so hot fixes are visible after browser refresh.
- Registered API routes are available on the same daemon, enabling browser UI + local API prototyping in the same REPL session.
- By default, start asks whether to open the page in the default browser unless `open`/`noOpenPrompt` override it.
- REPL prompt failures trigger automatic self-heal retries (translation + runtime) until success, except clearly non-recoverable provider/config errors.
- Optional cap: set `KLUMO_REPL_SELF_HEAL_MAX_ATTEMPTS=<n>` (`0` or unset means unlimited retries).

## Tests

```bash
cargo test --workspace
```

Root smoke checks:

```bash
bash tests/smoke/offline.sh
bash tests/smoke/offline_repl.sh
bash tests/smoke/offline_self_heal.sh
KLUMO_RUN_LIVE_TESTS=1 bash tests/smoke/live_ollama.sh
KLUMO_RUN_LIVE_TESTS=1 bash tests/smoke/live_ollama_react_project.sh
```

Live provider tests remain ignored by default and are gated by:

- `KLUMO_RUN_LIVE_TESTS=1`

## Deferred

- V8 migration (`deno_core` / `rusty_v8`) is scaffolded via `crates/klumo-engine-v8` and planned for full implementation next.
- Node compatibility shims are not implemented yet.
- Module graph/import loader and permission flags are deferred.
