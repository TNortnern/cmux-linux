#!/usr/bin/env bash
# validate-deb.sh -- check the file list and metadata of a built cmux .deb
# Usage: validate-deb.sh [path-to-deb]
set -euo pipefail
# shellcheck source=common.sh
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

require_cmd dpkg-deb

if [[ -n "${1:-}" ]]; then
    DEB="$1"
else
    # shellcheck disable=SC2012
    DEB="$(ls -t "$OUTPUT_DIR"/cmux_*_amd64.deb 2>/dev/null | head -n 1 || true)"
    [[ -n "$DEB" ]] || die "no .deb found in $OUTPUT_DIR; build first or pass a path"
fi
require_file "$DEB"

echo "Validating: $DEB"
echo ""

PASS=0
FAIL=0

check() {
    local desc="$1"; shift
    if "$@" >/dev/null 2>&1; then
        echo "  PASS: $desc"
        PASS=$((PASS + 1))
    else
        echo "  FAIL: $desc"
        FAIL=$((FAIL + 1))
    fi
}

FILE_LIST="$(dpkg-deb -c "$DEB")"
CONTROL="$(dpkg-deb -f "$DEB")"

# has_path <path>: the archive contains exactly this path (dpkg-deb -c prints ./-prefixed names)
has_path() {
    grep -q -E " \./${1}\$" <<<"$FILE_LIST"
}

# lacks <text> <haystack>: haystack does not contain text
lacks() {
    ! grep -q -F "$1" <<<"$2"
}

# has_dep <package>: Depends lists the package (version constraints ignored)
has_dep() {
    grep "^Depends:" <<<"$CONTROL" | tr ',' '\n' \
        | sed -e 's/^Depends: *//' -e 's/^ *//' -e 's/ *(.*)$//' | grep -qxF "$1"
}

echo "File listing:"
check "usr/bin/cmux-app (launcher)" has_path usr/bin/cmux-app
check "usr/bin/cmux-app.bin (binary)" has_path usr/bin/cmux-app.bin
check "usr/bin/cmux (CLI)" has_path usr/bin/cmux
check "desktop entry" has_path usr/share/applications/com.cmux_lx.terminal.desktop
check "metainfo" has_path usr/share/metainfo/com.cmux_lx.terminal.metainfo.xml
check "48x48 icon" has_path usr/share/icons/hicolor/48x48/apps/com.cmux_lx.terminal.png
check "128x128 icon" has_path usr/share/icons/hicolor/128x128/apps/com.cmux_lx.terminal.png
check "256x256 icon" has_path usr/share/icons/hicolor/256x256/apps/com.cmux_lx.terminal.png
check "scalable icon" has_path usr/share/icons/hicolor/scalable/apps/com.cmux_lx.terminal.svg
check "bash completion" has_path usr/share/bash-completion/completions/cmux
check "zsh completion" has_path usr/share/zsh/vendor-completions/_cmux
check "fish completion" has_path usr/share/fish/vendor_completions.d/cmux.fish
check "man page" has_path usr/share/man/man1/cmux.1.gz
check "license" has_path usr/share/licenses/cmux/LICENSE

echo ""
echo "Ghostty resources:"
check "shell integration (bash)" has_path usr/share/cmux/ghostty/shell-integration/bash/ghostty.bash
check "shell integration (zsh)" has_path usr/share/cmux/ghostty/shell-integration/zsh/ghostty-integration
check "shell integration (fish)" has_path usr/share/cmux/ghostty/shell-integration/fish/vendor_conf.d/ghostty-shell-integration.fish
check "terminfo xterm-ghostty" has_path usr/share/cmux/terminfo/x/xterm-ghostty

echo ""
echo "Skills & CLAUDE.md:"
check "cmux skill SKILL.md" has_path usr/share/cmux/skills/cmux/SKILL.md
check "cmux-browser skill SKILL.md" has_path usr/share/cmux/skills/cmux-browser/SKILL.md
check "cmux-browser commands.md" has_path usr/share/cmux/skills/cmux-browser/references/commands.md
check "CLAUDE.md" has_path usr/share/cmux/CLAUDE.md
check "no cmux-debug-windows skill" lacks "cmux-debug-windows" "$FILE_LIST"
check "no release skill" lacks "skills/release" "$FILE_LIST"

echo ""
echo "Optional (must not be required):"
check "no agent-browser is required by Depends" lacks "agent-browser" "$(grep '^Depends:' <<<"$CONTROL")"

echo ""
echo "Metadata:"
check "Package: cmux" grep -q -x "Package: cmux" <<<"$CONTROL"
check "Architecture: amd64" grep -q -x "Architecture: amd64" <<<"$CONTROL"
check "Version is set" grep -q -E "^Version: [0-9]" <<<"$CONTROL"
check "copyright declares AGPL-3.0-or-later" has_path usr/share/doc/cmux/copyright

echo ""
echo "Dependencies:"
for dep in libgtk-4-1 libfontconfig1 libfreetype6 libgl1 libbz2-1.0 libstdc++6 libgcc-s1; do
    check "Depends: $dep" has_dep "$dep"
done
check "GTK 4.6 minimum" grep -q -E "libgtk-4-1 \(>= 4\.6\)" <<<"$CONTROL"

echo ""
echo "Results: $PASS passed, $FAIL failed"
[[ "$FAIL" -eq 0 ]]
