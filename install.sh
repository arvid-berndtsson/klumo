#!/usr/bin/env bash
set -euo pipefail

REPO_DEFAULT="arvid-berndtsson/beeno"
REPO="${BEENO_GITHUB_REPO:-$REPO_DEFAULT}"
VERSION="${BEENO_VERSION:-latest}"
INSTALL_DIR="${BEENO_INSTALL_DIR:-$HOME/.local/bin}"
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT
MODE="install"

usage() {
  cat <<'EOF'
beeno install script

Usage:
  install.sh [--install|--update|--uninstall] [--help]

Modes:
  --install     Install Beeno (default)
  --update      Update Beeno (same as install, defaults to latest)
  --uninstall   Remove Beeno binary from install dir

Environment variables:
  BEENO_GITHUB_REPO   GitHub repo, default: arvid-berndtsson/beeno
  BEENO_VERSION       Release tag (vX.Y.Z) or latest, default: latest
  BEENO_INSTALL_DIR   Install directory, default: ~/.local/bin
EOF
}

need_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "error: required command '$1' not found" >&2
    exit 1
  fi
}

parse_args() {
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --install)
        MODE="install"
        ;;
      --update)
        MODE="update"
        ;;
      --uninstall)
        MODE="uninstall"
        ;;
      --help|-h)
        usage
        exit 0
        ;;
      *)
        echo "error: unknown argument: $1" >&2
        usage >&2
        exit 1
        ;;
    esac
    shift
  done
}

detect_target() {
  local os arch
  os="$(uname -s | tr '[:upper:]' '[:lower:]')"
  arch="$(uname -m)"

  case "$os" in
    linux)
      case "$arch" in
        x86_64|amd64) echo "x86_64-unknown-linux-gnu" ;;
        *) echo "unsupported Linux arch: $arch" >&2; exit 1 ;;
      esac
      ;;
    darwin)
      case "$arch" in
        x86_64) echo "x86_64-apple-darwin" ;;
        arm64|aarch64) echo "aarch64-apple-darwin" ;;
        *) echo "unsupported macOS arch: $arch" >&2; exit 1 ;;
      esac
      ;;
    msys*|mingw*|cygwin*)
      case "$arch" in
        x86_64|amd64) echo "x86_64-pc-windows-msvc" ;;
        *) echo "unsupported Windows arch: $arch" >&2; exit 1 ;;
      esac
      ;;
    *)
      echo "unsupported OS: $os" >&2
      exit 1
      ;;
  esac
}

download() {
  local url out
  url="$1"
  out="$2"
  curl --fail --location --silent --show-error "$url" --output "$out"
}

verify_checksum_if_available() {
  local checksums_file archive_name archive_path expected actual
  checksums_file="$1"
  archive_name="$2"
  archive_path="$3"

  if [[ ! -s "$checksums_file" ]]; then
    echo "[beeno-install] checksums.txt not found; skipping checksum verification"
    return 0
  fi

  expected="$(grep "  ${archive_name}$" "$checksums_file" | awk '{print $1}' || true)"
  if [[ -z "$expected" ]]; then
    echo "[beeno-install] no checksum entry for ${archive_name}; skipping verification"
    return 0
  fi

  if command -v sha256sum >/dev/null 2>&1; then
    actual="$(sha256sum "$archive_path" | awk '{print $1}')"
  elif command -v shasum >/dev/null 2>&1; then
    actual="$(shasum -a 256 "$archive_path" | awk '{print $1}')"
  else
    echo "[beeno-install] no sha256 tool found; skipping checksum verification"
    return 0
  fi

  if [[ "$expected" != "$actual" ]]; then
    echo "error: checksum mismatch for ${archive_name}" >&2
    echo "expected: $expected" >&2
    echo "actual:   $actual" >&2
    exit 1
  fi
}

extract_binary() {
  local archive target out_bin
  archive="$1"
  target="$2"
  out_bin="$3"

  case "$target" in
    *windows*)
      need_cmd unzip
      unzip -q "$archive" -d "$TMP_DIR/extract"
      cp "$TMP_DIR/extract/beeno.exe" "$out_bin"
      ;;
    *)
      tar -xzf "$archive" -C "$TMP_DIR"
      cp "$TMP_DIR/beeno" "$out_bin"
      ;;
  esac
}

uninstall_binary() {
  local bin_name path
  bin_name="$1"
  path="$INSTALL_DIR/$bin_name"

  if [[ -f "$path" ]]; then
    rm -f "$path"
    echo "[beeno-install] uninstalled: $path"
  else
    echo "[beeno-install] no binary found at: $path"
  fi
}

parse_args "$@"

need_cmd curl
need_cmd tar

TARGET="$(detect_target)"
if [[ "$TARGET" == *windows* ]]; then
  ARCHIVE_EXT="zip"
  BIN_NAME="beeno.exe"
else
  ARCHIVE_EXT="tar.gz"
  BIN_NAME="beeno"
fi

if [[ "$VERSION" == "latest" ]]; then
  BASE_URL="https://github.com/${REPO}/releases/latest/download"
else
  BASE_URL="https://github.com/${REPO}/releases/download/${VERSION}"
fi

ARCHIVE_NAME="beeno-${TARGET}.${ARCHIVE_EXT}"
ARCHIVE_PATH="$TMP_DIR/$ARCHIVE_NAME"
CHECKSUMS_PATH="$TMP_DIR/checksums.txt"

echo "[beeno-install] repo: ${REPO}"
echo "[beeno-install] version: ${VERSION}"
echo "[beeno-install] target: ${TARGET}"

if [[ "$MODE" == "uninstall" ]]; then
  uninstall_binary "$BIN_NAME"
  exit 0
fi

download "${BASE_URL}/${ARCHIVE_NAME}" "$ARCHIVE_PATH"
download "${BASE_URL}/checksums.txt" "$CHECKSUMS_PATH" || true
verify_checksum_if_available "$CHECKSUMS_PATH" "$ARCHIVE_NAME" "$ARCHIVE_PATH"

mkdir -p "$INSTALL_DIR"
extract_binary "$ARCHIVE_PATH" "$TARGET" "$TMP_DIR/$BIN_NAME"
install -m 0755 "$TMP_DIR/$BIN_NAME" "$INSTALL_DIR/$BIN_NAME"

if [[ "$MODE" == "update" ]]; then
  echo "[beeno-install] updated: $INSTALL_DIR/$BIN_NAME"
else
  echo "[beeno-install] installed: $INSTALL_DIR/$BIN_NAME"
fi
echo "[beeno-install] run: beeno --version"
