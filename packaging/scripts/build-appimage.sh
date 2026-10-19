#!/usr/bin/env bash
# Build dist/cmux-<version>-x86_64.AppImage.
#
# linuxdeploy collects the shared libraries and the GTK 4 runtime into an AppDir;
# appimagetool packs that AppDir into an AppImage.
#
# The tools are pinned to exact releases or commits and verified by SHA-256 before
# use. To bump one: download the new asset, `sha256sum` it, and update the pin here.
# Downloads are cached in $CMUX_PACKAGING_CACHE (default: ~/.cache/cmux-packaging).
set -euo pipefail
# shellcheck source=common.sh
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

# linuxdeploy 1-alpha-20251107-1 (GitHub release asset digest)
LINUXDEPLOY_URL="https://github.com/linuxdeploy/linuxdeploy/releases/download/1-alpha-20251107-1/linuxdeploy-x86_64.AppImage"
LINUXDEPLOY_SHA256="c20cd71e3a4e3b80c3483cef793cda3f4e990aca14014d23c544ca3ce1270b4d"

# linuxdeploy-plugin-gtk at commit 7a3fbc31a9e5075073ff8790f26effbac5f84453
# (the project publishes no releases, so the script is pinned by commit, not branch)
GTK_PLUGIN_URL="https://raw.githubusercontent.com/linuxdeploy/linuxdeploy-plugin-gtk/7a3fbc31a9e5075073ff8790f26effbac5f84453/linuxdeploy-plugin-gtk.sh"
GTK_PLUGIN_SHA256="b0f4cbc684a0103a9651f0955b635eaea0096b3a66c0f5a2c2aa337960375171"

# appimagetool 1.9.1 (GitHub release asset digest)
APPIMAGETOOL_URL="https://github.com/AppImage/appimagetool/releases/download/1.9.1/appimagetool-x86_64.AppImage"
APPIMAGETOOL_SHA256="ed4ce84f0d9caff66f50bcca6ff6f35aae54ce8135408b3fa33abfc3cb384eb0"

APP_ID="com.cmux_lx.terminal"
ARCH="x86_64"

# CI and containers have no FUSE: run AppImage tools by extraction.
export APPIMAGE_EXTRACT_AND_RUN=1

require_cmd curl
require_cmd sha256sum
VERSION="$(cargo_version)"
[[ -n "$VERSION" ]] || die "could not read version from Cargo.toml"

CACHE="${CMUX_PACKAGING_CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}/cmux-packaging}"
mkdir -p "$CACHE" "$OUTPUT_DIR"

# fetch <url> <sha256> <dest>: download once, verify on every use
fetch() {
    local url="$1" sha="$2" dest="$3"
    if [[ ! -f "$dest" ]] || ! echo "$sha  $dest" | sha256sum -c --status -; then
        echo "==> Downloading $(basename "$dest")"
        curl -fSL --retry 3 -o "$dest.tmp" "$url"
        mv "$dest.tmp" "$dest"
    fi
    echo "$sha  $dest" | sha256sum -c - >/dev/null || die "checksum mismatch for $dest (expected $sha)"
}

LINUXDEPLOY="$CACHE/linuxdeploy-$ARCH.AppImage"
GTK_PLUGIN="$CACHE/linuxdeploy-plugin-gtk.sh"
APPIMAGETOOL="$CACHE/appimagetool-$ARCH.AppImage"
fetch "$LINUXDEPLOY_URL" "$LINUXDEPLOY_SHA256" "$LINUXDEPLOY"
fetch "$GTK_PLUGIN_URL" "$GTK_PLUGIN_SHA256" "$GTK_PLUGIN"
fetch "$APPIMAGETOOL_URL" "$APPIMAGETOOL_SHA256" "$APPIMAGETOOL"
chmod 0755 "$LINUXDEPLOY" "$GTK_PLUGIN" "$APPIMAGETOOL"

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
APPDIR="$WORK/cmux.AppDir"
mkdir -p "$APPDIR"

"$PACKAGING_SCRIPTS_DIR/stage-payload.sh" "$APPDIR"

# AppRun sources the GTK plugin's hooks (apprun-hooks/) before starting the launcher,
# so GTK finds the bundled modules and schemas.
cat > "$APPDIR/AppRun" <<'EOF'
#!/bin/sh
HERE="$(dirname "$(readlink -f "$0")")"
export APPDIR="$HERE"
for hook in "$HERE"/apprun-hooks/*.sh; do
    [ -r "$hook" ] && . "$hook"
done
exec "$HERE/usr/bin/cmux-app" "$@"
EOF
chmod 0755 "$APPDIR/AppRun"

echo "==> Collecting libraries with linuxdeploy"
env PATH="$CACHE:$PATH" DEPLOY_GTK_VERSION=4 \
    "$LINUXDEPLOY" \
    --appdir "$APPDIR" \
    --desktop-file "$APPDIR/usr/share/applications/$APP_ID.desktop" \
    --icon-file "$APPDIR/usr/share/icons/hicolor/256x256/apps/$APP_ID.png" \
    --executable "$APPDIR/usr/bin/cmux-app.bin" \
    --executable "$APPDIR/usr/bin/cmux" \
    --plugin gtk

echo "==> Packing AppImage with appimagetool"
OUT="$OUTPUT_DIR/cmux-${VERSION}-${ARCH}.AppImage"
ARCH="$ARCH" "$APPIMAGETOOL" "$APPDIR" "$OUT"
chmod 0755 "$OUT"
echo "Built: $OUT"
