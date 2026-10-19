#!/usr/bin/env bash
# Build dist/cmux_<version>_amd64.deb from release binaries.
#
# Expects `cargo build --release --bin cmux-app --bin cmux` to have run (see
# scripts/setup-linux.sh for the libghostty prerequisites). See common.sh for overrides.
set -euo pipefail
# shellcheck source=common.sh
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

require_cmd dpkg-deb
VERSION="$(cargo_version)"
[[ -n "$VERSION" ]] || die "could not read version from Cargo.toml"
ARCH="amd64"

mkdir -p "$OUTPUT_DIR"
PKG_ROOT="$(mktemp -d)"
trap 'rm -rf "$PKG_ROOT"' EXIT

"$PACKAGING_SCRIPTS_DIR/stage-payload.sh" "$PKG_ROOT"

# Debian copyright file (the license text itself lives in /usr/share/licenses/cmux)
install -dm0755 "$PKG_ROOT/usr/share/doc/cmux"
cat > "$PKG_ROOT/usr/share/doc/cmux/copyright" <<'EOF'
Format: https://www.debian.org/doc/packaging-manuals/copyright-format/1.0/
Upstream-Name: cmux-linux
Upstream-Contact: https://github.com/TNortnern/cmux-linux/issues
Source: https://github.com/TNortnern/cmux-linux

Files: *
Copyright: cmux-linux contributors and the upstream cmux and Ghostty authors
License: AGPL-3.0-or-later
 This program is free software: you can redistribute it and/or modify it under
 the terms of the GNU Affero General Public License as published by the Free
 Software Foundation, either version 3 of the License, or (at your option) any
 later version. The full text is in /usr/share/common-licenses/AGPL-3.
EOF
chmod 0644 "$PKG_ROOT/usr/share/doc/cmux/copyright"

# Package metadata
INSTALLED_SIZE="$(du -sk "$PKG_ROOT/usr" | cut -f1)"
mkdir -p "$PKG_ROOT/DEBIAN"
cat > "$PKG_ROOT/DEBIAN/control" <<CTRL
Package: cmux
Version: ${VERSION}
Architecture: ${ARCH}
Maintainer: cmux-linux contributors <noreply@github.com>
Installed-Size: ${INSTALLED_SIZE}
Section: utils
Priority: optional
Depends: libgtk-4-1 (>= 4.14), libfontconfig1, libfreetype6, libgl1, libbz2-1.0, libstdc++6, libgcc-s1
Homepage: https://github.com/TNortnern/cmux-linux
Description: GPU-accelerated terminal multiplexer
 cmux provides workspaces, tabs, splits, session restore, notifications and
 socket CLI control, powered by Ghostty's GPU-accelerated terminal rendering.
 Browser automation (agent-browser) and SSH workspaces are optional.
CTRL

# dpkg requires directories to be 0755 and root-owned (--root-owner-group)
find "$PKG_ROOT" -type d -exec chmod 0755 {} +

DEB_FILE="$OUTPUT_DIR/cmux_${VERSION}_${ARCH}.deb"
dpkg-deb --build --root-owner-group "$PKG_ROOT" "$DEB_FILE"
echo "Built: $DEB_FILE"
