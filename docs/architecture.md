# Beeno Runtime Architecture

## Goal

Build a standalone runtime that can execute JavaScript and translated source inputs without Node.js.

## High-Level Pipeline

1. `Input Loader`: reads file/stdin/URL sources.
2. `Planner`: determines source type and compile strategy.
3. `LLM Compiler`: translates non-JS inputs into JavaScript modules.
4. `Module Graph Builder`: resolves imports and caches compile artifacts.
5. `Runtime Core`: executes module graph in embedded engine.
6. `Host APIs`: Beeno-owned implementations (`fs`, `net`, `crypto`, timers).

## Runtime Composition

- `beeno-cli` (Rust binary)
  - Commands: `run`, `repl`, `cache`, `fmt`, `check`
- `beeno-core`
  - Module loader
  - Event loop
  - Permission gates
  - Snapshot/bootstrap logic
- `beeno-llm`
  - Provider adapters
  - Prompt contracts
  - Deterministic compile cache
- `beeno-compat-node`
  - Subset `node:` modules mapped to Beeno APIs

## Engine Choice

Primary option: `deno_core` (embedded V8)

- Pros: mature ops model, high compatibility, proven runtime shape.
- Cons: larger footprint.

Alternative option: QuickJS

- Pros: lightweight, easier embedding.
- Cons: lower compatibility/perf for some workloads.

## Security Model

- Default deny for `fs`, `net`, env access.
- Explicit flags: `--allow-read`, `--allow-write`, `--allow-net`, `--allow-env`.
- LLM compiler sandbox:
  - No direct host execution.
  - Strict output contract: JS only.
  - Optional policy scan before runtime execution.

## Compatibility Strategy

1. First-class Beeno APIs.
2. Web-standard APIs where possible.
3. Node compatibility shims for migration (`node:fs`, `node:path`, `node:events` first).

## Milestones

1. `M0`: CLI skeleton + embedded engine hello world.
2. `M1`: JS module execution + file loader + permissions.
3. `M2`: LLM compile step for non-JS inputs.
4. `M3`: cache, diagnostics, source maps.
5. `M4`: Node compatibility subset + npm interop plan.
