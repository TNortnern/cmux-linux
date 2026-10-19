#!/usr/bin/env bash
# Build dist/cmux-<version>-1.x86_64.rpm from release binaries.
#
# Needs rpmbuild (Debian/Ubuntu: `sudo apt install rpm`; Fedora: `dnf install rpm-build`).
# See common.sh for the binary overrides.
set -euo pipefail
# shellcheck source=common.sh
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

require_cmd rpmbuild
VERSION="$(cargo_version)"
[[ -n "$VERSION" ]] || die "could not read version from Cargo.toml"

mkdir -p "$OUTPUT_DIR"
BUILD_DIR="$(mktemp -d)"
trap 'rm -rf "$BUILD_DIR"' EXIT

PAYLOAD="$BUILD_DIR/payload"
mkdir -p "$PAYLOAD" "$BUILD_DIR/rpmbuild"/{BUILD,BUILDROOT,RPMS,SOURCES,SPECS,SRPMS}
"$PACKAGING_SCRIPTS_DIR/stage-payload.sh" "$PAYLOAD"

rpmbuild -bb \
    --define "_topdir $BUILD_DIR/rpmbuild" \
    --define "_cmux_version $VERSION" \
    --define "_cmux_payload $PAYLOAD" \
    "$REPO_ROOT/packaging/rpm/cmux.spec"

RPM_FILE="$(find "$BUILD_DIR/rpmbuild/RPMS" -name '*.rpm' | head -n 1)"
[[ -n "$RPM_FILE" ]] || die "rpmbuild produced no .rpm"
cp "$RPM_FILE" "$OUTPUT_DIR/"
echo "Built: $OUTPUT_DIR/$(basename "$RPM_FILE")"
