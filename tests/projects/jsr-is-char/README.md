# JSR is-char Smoke Project

This project validates Klumo runtime support for JSR imports using `@arvid/is-char`.

## Run

From repository root:

```bash
cd tests/projects/jsr-is-char
cargo run -p klumo -- run start
```

Expected output includes:

- `isChar(B)=true`
- `isChar(be)=false`
- `JSR_IS_CHAR_OK`
