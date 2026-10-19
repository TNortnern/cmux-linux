#!/usr/bin/env bash
# Lay out the installed cmux file tree (FHS paths) under <dest>.
#
# Shared by build-deb.sh, build-rpm.sh and build-appimage.sh so all three ship the
# same files. Usage: stage-payload.sh <dest>
set -euo pipefail
# shellcheck source=common.sh
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

DEST="${1:?usage: stage-payload.sh <dest>}"
ASSETS="$REPO_ROOT/packaging"
APP_ID="com.cmux_lx.terminal"

require_file "$CMUX_APP_BIN"
require_file "$CMUX_CLI_BIN"
require_file "$ASSETS/scripts/cmux-app-wrapper.sh"

# Binaries. /usr/bin/cmux-app is a launcher that picks the GDK backend and then
# execs cmux-app.bin next to it, so it also works from an AppImage's usr/bin.
install -Dm0755 "$CMUX_APP_BIN" "$DEST/usr/bin/cmux-app.bin"
install -Dm0755 "$ASSETS/scripts/cmux-app-wrapper.sh" "$DEST/usr/bin/cmux-app"
install -Dm0755 "$CMUX_CLI_BIN" "$DEST/usr/bin/cmux"

# Optional helpers. Packages never require them; cmux finds them at runtime.
if [[ -f "$CMUXD_REMOTE_BIN" ]]; then
    install -Dm0755 "$CMUXD_REMOTE_BIN" "$DEST/usr/lib/cmux/cmuxd-remote"
else
    echo "note: cmuxd-remote not found at $CMUXD_REMOTE_BIN; SSH workspaces will be unavailable" >&2
fi
if [[ -n "$AGENT_BROWSER_BIN" ]]; then
    require_file "$AGENT_BROWSER_BIN"
    install -Dm0755 "$AGENT_BROWSER_BIN" "$DEST/usr/lib/cmux/agent-browser"
fi

# Desktop integration
install -Dm0644 "$ASSETS/desktop/$APP_ID.desktop" "$DEST/usr/share/applications/$APP_ID.desktop"
install -Dm0644 "$ASSETS/desktop/$APP_ID.metainfo.xml" "$DEST/usr/share/metainfo/$APP_ID.metainfo.xml"
for size in 48x48 128x128 256x256; do
    install -Dm0644 "$ASSETS/icons/hicolor/$size/apps/$APP_ID.png" \
        "$DEST/usr/share/icons/hicolor/$size/apps/$APP_ID.png"
done
install -Dm0644 "$REPO_ROOT/resources/cmux.svg" \
    "$DEST/usr/share/icons/hicolor/scalable/apps/$APP_ID.svg"

# Shell completions and man page
install -Dm0644 "$ASSETS/completions/cmux.bash" "$DEST/usr/share/bash-completion/completions/cmux"
install -Dm0644 "$ASSETS/completions/_cmux" "$DEST/usr/share/zsh/vendor-completions/_cmux"
install -Dm0644 "$ASSETS/completions/cmux.fish" "$DEST/usr/share/fish/vendor_completions.d/cmux.fish"
install -dm0755 "$DEST/usr/share/man/man1"
gzip -9n < "$ASSETS/man/cmux.1" > "$DEST/usr/share/man/man1/cmux.1.gz"
chmod 0644 "$DEST/usr/share/man/man1/cmux.1.gz"

# Agent skills (only the two that are meant for end users) and the CLAUDE.md that points at them
for skill in cmux cmux-browser; do
    install -dm0755 "$DEST/usr/share/cmux/skills/$skill"
    cp -R "$REPO_ROOT/skills/$skill/." "$DEST/usr/share/cmux/skills/$skill/"
done
find "$DEST/usr/share/cmux/skills" -type f -exec chmod 0644 {} +
install -Dm0644 "$ASSETS/CLAUDE.md" "$DEST/usr/share/cmux/CLAUDE.md"

# Ghostty runtime resources: shell integration (cwd reporting) and terminfo.
# GHOSTTY_RESOURCES_DIR is /usr/share/cmux/ghostty, TERMINFO is /usr/share/cmux/terminfo.
if [[ -n "$GHOSTTY_SHARE" ]]; then
    cp -R "$GHOSTTY_SHARE/." "$DEST/usr/share/cmux/"
else
    require_cmd zig
    "$PACKAGING_SCRIPTS_DIR/stage-ghostty-resources.sh" "$REPO_ROOT/ghostty" "$DEST/usr/share/cmux"
fi
find "$DEST/usr/share/cmux/ghostty" "$DEST/usr/share/cmux/terminfo" -type d -exec chmod 0755 {} +
find "$DEST/usr/share/cmux/ghostty" "$DEST/usr/share/cmux/terminfo" -type f -exec chmod 0644 {} +

# License
install -Dm0644 "$REPO_ROOT/LICENSE" "$DEST/usr/share/licenses/cmux/LICENSE"
