# React Buggy Sandbox

This is a small intentionally broken React-style project for validating Beeno self-heal behavior.

## Files

- `src/`: active project files (starts in buggy state).
- `scripts/run_self_heal_ollama.sh`: runs Beeno self-heal against the buggy entry file.

## Quick flow

```bash
# 1) Run this project with Beeno runtime
bash tests/projects/react-buggy/scripts/run_beeno.sh

# 2) Bundle with Beeno runtime
bash tests/projects/react-buggy/scripts/bundle_beeno.sh

# 1) Run self-heal (live Ollama)
BEENO_RUN_LIVE_TESTS=1 bash tests/projects/react-buggy/scripts/run_self_heal_ollama.sh
```

## Notes

- This sandbox is for runtime self-heal behavior, not for full React bundler/dev-server execution.
- The entrypoint is intentionally malformed and should trigger repair attempts.
- `beeno.json` in this folder is used as package-style project metadata (`name`, `version`, `dependencies`, `scripts`).
