#!/usr/bin/env bash
# Stage ghostty's runtime resources so shell integration (cwd reporting) and the
# xterm-ghostty terminfo entry work in cmux.
#
# Usage: stage-ghostty-resources.sh <ghostty-src-dir> <share-dir>
#
# Writes:
#   <share-dir>/ghostty/shell-integration/**   (bash, zsh, fish, ... without *.md)
#   <share-dir>/terminfo/x/xterm-ghostty       (+ g/ghostty alias)
#
# Layout follows ghostty's own: TERMINFO is resolved as the sibling of the
# resources dir, so with GHOSTTY_RESOURCES_DIR=<share>/ghostty the database lives in
# <share>/terminfo. Packages install <share> as /usr/share/cmux.
set -euo pipefail

if [[ $# -ne 2 ]]; then
    echo "usage: $0 <ghostty-src-dir> <share-dir>" >&2
    exit 2
fi

GHOSTTY="$(cd "$1" && pwd)"
SHARE="$2"
ZIG="${ZIG:-zig}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
GENERATOR="$SCRIPT_DIR/../ghostty/gen-terminfo.zig"

SI_SRC="$GHOSTTY/src/shell-integration"
TI_SRC="$GHOSTTY/src/terminfo/main.zig"
for path in "$SI_SRC" "$TI_SRC" "$GENERATOR"; do
    if [[ ! -e "$path" ]]; then
        echo "error: missing $path" >&2
        exit 1
    fi
done

# Shell integration scripts (the .md files are documentation only).
mkdir -p "$SHARE/ghostty/shell-integration"
cp -R "$SI_SRC/." "$SHARE/ghostty/shell-integration/"
find "$SHARE/ghostty/shell-integration" -name '*.md' -delete

# Terminfo: build the generator, print the source, compile it with tic.
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
(
    cd "$work"
    "$ZIG" build-exe -OReleaseSafe \
        --dep terminfo \
        -Mroot="$GENERATOR" \
        -Mterminfo="$TI_SRC" \
        --name ghostty-terminfo-gen \
        -femit-bin="$work/ghostty-terminfo-gen"
)
"$work/ghostty-terminfo-gen" > "$work/ghostty.terminfo"

rm -rf "$SHARE/terminfo"
mkdir -p "$SHARE/terminfo"
tic -x -o "$SHARE/terminfo" "$work/ghostty.terminfo"

# tic files the entry under its first name's initial; make sure the alias resolves.
if [[ ! -e "$SHARE/terminfo/x/xterm-ghostty" ]]; then
    echo "error: tic did not produce $SHARE/terminfo/x/xterm-ghostty" >&2
    exit 1
fi

echo "Staged ghostty resources in $SHARE"
