# Root Smoke Tests

This folder contains end-to-end smoke checks for the Beeno runtime from the repository root.

## Structure

- `tests/fixtures/`: input files used by smoke tests.
- `tests/smoke/offline.sh`: deterministic local checks (no provider required).
- `tests/smoke/offline_repl.sh`: deterministic REPL boot/exit check.
- `tests/smoke/offline_self_heal.sh`: deterministic self-heal failure-path check.
- `tests/smoke/live_ollama.sh`: optional live Ollama checks, including self-heal.
- `tests/smoke/live_ollama_react_project.sh`: optional live React buggy-sandbox self-heal check.
- `tests/smoke/live_ollama_jsr_project.sh`: optional live JSR import check for `@arvid/is-char`.
- `tests/projects/react-buggy/`: intentionally buggy React-style project for self-heal behavior.
- `tests/projects/jsr-is-char/`: JSR-focused runtime smoke project using `@arvid/is-char`.

## Run

Offline:

```bash
bash tests/smoke/offline.sh
bash tests/smoke/offline_repl.sh
bash tests/smoke/offline_self_heal.sh
```

Live Ollama (opt-in):

```bash
BEENO_RUN_LIVE_TESTS=1 bash tests/smoke/live_ollama.sh
BEENO_RUN_LIVE_TESTS=1 bash tests/smoke/live_ollama_react_project.sh
BEENO_RUN_LIVE_TESTS=1 bash tests/smoke/live_ollama_jsr_project.sh
```

Notes:

- Live test expects Ollama running at `http://127.0.0.1:11434`.
- Live test uses `BEENO_OLLAMA_MODEL` if set, otherwise runtime defaults.
