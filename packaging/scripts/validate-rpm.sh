#!/usr/bin/env bash
# validate-rpm.sh -- check the file list and metadata of a built cmux .rpm
# Usage: validate-rpm.sh [path-to-rpm]
set -euo pipefail
# shellcheck source=common.sh
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

require_cmd rpm

if [[ -n "${1:-}" ]]; then
    RPM="$1"
else
    shopt -s nullglob
    RPMS=("$OUTPUT_DIR"/cmux-*.rpm)
    shopt -u nullglob
    [[ ${#RPMS[@]} -gt 0 ]] || die "no .rpm found in $OUTPUT_DIR; build first or pass a path"
    [[ ${#RPMS[@]} -eq 1 ]] || die "multiple .rpm files in $OUTPUT_DIR; pass an explicit path"
    RPM="${RPMS[0]}"
fi
require_file "$RPM"

echo "Validating: $RPM"
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

FILE_LIST="$(rpm -qpl "$RPM")"
INFO="$(rpm -qpi "$RPM")"
REQUIRES="$(rpm -qpR "$RPM")"

has_file() {
    grep -q -x -F "$1" <<<"$FILE_LIST"
}

has_requires() {
    awk -v name="$1" '$1 == name { found = 1 } END { exit !found }' <<<"$REQUIRES"
}

# lacks <text> <haystack>: haystack does not contain text
lacks() {
    ! grep -q -F "$1" <<<"$2"
}

echo "File listing:"
check "/usr/bin/cmux-app (launcher)" has_file /usr/bin/cmux-app
check "/usr/bin/cmux-app.bin (binary)" has_file /usr/bin/cmux-app.bin
check "/usr/bin/cmux (CLI)" has_file /usr/bin/cmux
check "desktop entry" has_file /usr/share/applications/com.cmux_lx.terminal.desktop
check "metainfo" has_file /usr/share/metainfo/com.cmux_lx.terminal.metainfo.xml
check "icon 48x48" has_file /usr/share/icons/hicolor/48x48/apps/com.cmux_lx.terminal.png
check "icon 128x128" has_file /usr/share/icons/hicolor/128x128/apps/com.cmux_lx.terminal.png
check "icon 256x256" has_file /usr/share/icons/hicolor/256x256/apps/com.cmux_lx.terminal.png
check "scalable icon" has_file /usr/share/icons/hicolor/scalable/apps/com.cmux_lx.terminal.svg
check "bash completion" has_file /usr/share/bash-completion/completions/cmux
check "zsh completion" has_file /usr/share/zsh/vendor-completions/_cmux
check "fish completion" has_file /usr/share/fish/vendor_completions.d/cmux.fish
check "man page" has_file /usr/share/man/man1/cmux.1.gz
check "license" has_file /usr/share/licenses/cmux/LICENSE

echo ""
echo "Ghostty resources:"
check "shell integration (bash)" has_file /usr/share/cmux/ghostty/shell-integration/bash/ghostty.bash
check "terminfo xterm-ghostty" has_file /usr/share/cmux/terminfo/x/xterm-ghostty

echo ""
echo "Skills & CLAUDE.md:"
check "cmux skill SKILL.md" has_file /usr/share/cmux/skills/cmux/SKILL.md
check "cmux-browser skill SKILL.md" has_file /usr/share/cmux/skills/cmux-browser/SKILL.md
check "cmux-browser commands.md" has_file /usr/share/cmux/skills/cmux-browser/references/commands.md
check "CLAUDE.md" has_file /usr/share/cmux/CLAUDE.md
check "no cmux-debug-windows skill" lacks "cmux-debug-windows" "$FILE_LIST"
check "no release skill" lacks "skills/release" "$FILE_LIST"

echo ""
echo "Metadata:"
check "Name is cmux" grep -q -E "^Name +: cmux$" <<<"$INFO"
check "Architecture is x86_64" grep -q -E "^Architecture: x86_64$" <<<"$INFO"
check "License is AGPL-3.0-or-later" grep -q -E "^License +: AGPL-3.0-or-later$" <<<"$INFO"

echo ""
echo "Dependencies:"
for dep in "gtk4 >= 4.14" fontconfig freetype mesa-libGL bzip2-libs libstdc++ libgcc; do
    name="${dep%% *}"
    check "Requires $name" has_requires "$name"
done
check "agent-browser is not required" lacks "agent-browser" "$REQUIRES"
check "libgtk 4.14 minimum" grep -q -F "gtk4 >= 4.14" <<<"$REQUIRES"

echo ""
echo "Results: $PASS passed, $FAIL failed"
[[ "$FAIL" -eq 0 ]]
