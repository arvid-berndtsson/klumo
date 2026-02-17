# Beeno

**Slogan:** "Beeno, run node smarter, but with LLM benefits"

Beeno is a standalone JavaScript runtime with an LLM-native compile pipeline.

## Core Direction

Beeno must not depend on Node.js to run user programs.

Runtime stack target:

- Embedded JS engine (V8/QuickJS) owned by Beeno.
- Beeno-native standard library and runtime APIs.
- Node compatibility layer implemented in Beeno (not delegated to Node).
- LLM translation pipeline from arbitrary input to executable JavaScript.

## What Exists Today

This repository currently contains an early Node-based prototype used to validate the LLM translation flow.

- Prototype path: `/Users/arvid/Private/beeno/src`
- Limitation: executes through Node.js

This prototype is transitional and will be replaced by the standalone runtime.

## Immediate Build Plan

1. Bootstrap `beeno-runtime` in Rust with embedded V8 (`deno_core`) or QuickJS.
2. Implement module loader, permissions model, and event loop in Beeno.
3. Move LLM translator into a compile stage: `source(any) -> js module graph`.
4. Add Beeno-owned APIs (`fs`, `net`, `timers`, `process`) with explicit permissions.
5. Add Node compatibility shims for high-value modules.
6. Ship `beeno run` and `beeno repl` without requiring Node installation.

## Product Principle

Beeno should feel as practical as Node, but safer and more capable due to:

- LLM-assisted source ingestion and transformation.
- Reproducible execution controls.
- First-class security model.

## Next Artifact

See `/Users/arvid/Private/beeno/docs/architecture.md` for the concrete architecture skeleton and runtime boundaries.
