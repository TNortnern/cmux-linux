#!/usr/bin/env bash
# Shared helpers for the packaging scripts. Source it; do not execute it.
#
# Inputs (environment, all optional):
#   CMUX_APP_BIN       cmux-app binary      (default: target/release/cmux-app)
#   CMUX_CLI_BIN       cmux CLI binary      (default: target/release/cmux)
#   CMUXD_REMOTE_BIN   remote daemon        (default: daemon/remote/cmuxd-remote, optional)
#   AGENT_BROWSER_BIN  agent-browser        (optional; not part of this repo, never required)
#   GHOSTTY_SHARE      pre-staged dir with ghostty/ and terminfo/ (default: staged from ghostty/)
#   OUTPUT_DIR         where packages are written (default: dist/)

PACKAGING_SCRIPTS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$PACKAGING_SCRIPTS_DIR/../.." && pwd)"

: "${CMUX_APP_BIN:=$REPO_ROOT/target/release/cmux-app}"
: "${CMUX_CLI_BIN:=$REPO_ROOT/target/release/cmux}"
: "${CMUXD_REMOTE_BIN:=$REPO_ROOT/daemon/remote/cmuxd-remote}"
: "${AGENT_BROWSER_BIN:=}"
: "${GHOSTTY_SHARE:=}"
: "${OUTPUT_DIR:=$REPO_ROOT/dist}"

die() {
    echo "error: $*" >&2
    exit 1
}

require_cmd() {
    command -v "$1" >/dev/null 2>&1 || die "$1 is required but not installed"
}

require_file() {
    [[ -f "$1" ]] || die "file not found: $1"
}

# Version from the [package] table of Cargo.toml (first `version = "..."` in that table).
cargo_version() {
    awk '
        /^\[package\]/ { in_pkg = 1; next }
        /^\[/          { in_pkg = 0 }
        in_pkg && /^version[[:space:]]*=/ {
            gsub(/"/, "", $3); print $3; exit
        }
    ' "$REPO_ROOT/Cargo.toml"
}
