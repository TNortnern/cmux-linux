#!/usr/bin/env bash
# validate-all.sh -- static checks on the desktop metadata, icons and completions
# (no package needed). Usage: validate-all.sh
set -euo pipefail
# shellcheck source=common.sh
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

APP_ID="com.cmux_lx.terminal"
DESKTOP="$REPO_ROOT/packaging/desktop/$APP_ID.desktop"
METAINFO="$REPO_ROOT/packaging/desktop/$APP_ID.metainfo.xml"

PASS=0
FAIL=0

check() {
    local desc="$1"; shift
    if "$@" >/dev/null 2>&1; then
        echo "PASS: $desc"
        PASS=$((PASS + 1))
    else
        echo "FAIL: $desc"
        FAIL=$((FAIL + 1))
    fi
}

check "desktop entry id matches app id" grep -q "^Icon=$APP_ID\$" "$DESKTOP"
check "metainfo id matches app id" grep -q "<id>$APP_ID</id>" "$METAINFO"
check "metainfo license is AGPL-3.0-or-later" grep -q "<project_license>AGPL-3.0-or-later</project_license>" "$METAINFO"

if command -v desktop-file-validate >/dev/null; then
    check "desktop-file-validate" desktop-file-validate "$DESKTOP"
else
    echo "SKIP: desktop-file-validate (install desktop-file-utils)"
fi
if command -v appstreamcli >/dev/null; then
    check "appstreamcli validate" appstreamcli validate --no-net "$METAINFO"
else
    echo "SKIP: appstreamcli (install appstream)"
fi

for size in 48 128 256; do
    check "${size}px icon exists" test -f "$REPO_ROOT/packaging/icons/hicolor/${size}x${size}/apps/$APP_ID.png"
done
check "scalable icon exists" test -f "$REPO_ROOT/resources/cmux.svg"

check "bash completion exists" test -f "$REPO_ROOT/packaging/completions/cmux.bash"
check "zsh completion exists" test -f "$REPO_ROOT/packaging/completions/_cmux"
check "fish completion exists" test -f "$REPO_ROOT/packaging/completions/cmux.fish"
check "man page exists" test -f "$REPO_ROOT/packaging/man/cmux.1"
if command -v man >/dev/null; then
    check "man page renders" man -l "$REPO_ROOT/packaging/man/cmux.1"
fi
check "detect-deps.sh is executable" test -x "$PACKAGING_SCRIPTS_DIR/detect-deps.sh"

echo ""
echo "Results: $PASS passed, $FAIL failed"
[[ "$FAIL" -eq 0 ]]
