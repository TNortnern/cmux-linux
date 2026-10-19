#!/usr/bin/env bash
# detect-deps.sh -- map a binary's shared-library dependencies to distro packages
# Usage: detect-deps.sh [--json] <binary>
# Output: TSV (soname, debian package, fedora package), or JSON with --json.
# Use it to re-check the Depends/Requires lists in build-deb.sh and rpm/cmux.spec.
set -euo pipefail

JSON_MODE=false
if [[ "${1:-}" == "--json" ]]; then
    JSON_MODE=true
    shift
fi

BINARY="${1:?Usage: detect-deps.sh [--json] <binary>}"
if [[ ! -f "$BINARY" ]]; then
    echo "ERROR: Binary not found: $BINARY" >&2
    exit 1
fi

# Fallbacks for hosts without dpkg/rpm, or when the lookup finds nothing.
declare -A FEDORA_FALLBACK=(
    ["libgtk-4.so.1"]="gtk4"
    ["libadwaita-1.so.0"]="libadwaita"
    ["libfontconfig.so.1"]="fontconfig"
    ["libfreetype.so.6"]="freetype"
    ["libonig.so.5"]="oniguruma"
    ["libGL.so.1"]="mesa-libGL"
    ["libEGL.so.1"]="mesa-libEGL"
    ["libbz2.so.1.0"]="bzip2-libs"
    ["libstdc++.so.6"]="libstdc++"
    ["libgcc_s.so.1"]="libgcc"
)
declare -A DEBIAN_FALLBACK=(
    ["libgtk-4.so.1"]="libgtk-4-1"
    ["libadwaita-1.so.0"]="libadwaita-1-0"
    ["libfontconfig.so.1"]="libfontconfig1"
    ["libfreetype.so.6"]="libfreetype6"
    ["libonig.so.5"]="libonig5"
    ["libGL.so.1"]="libgl1"
    ["libEGL.so.1"]="libegl1"
    ["libbz2.so.1.0"]="libbz2-1.0"
    ["libstdc++.so.6"]="libstdc++6"
    ["libgcc_s.so.1"]="libgcc-s1"
)

# Runtime libraries that every distro ships and that are not package dependencies.
SKIP_PATTERN='^(libc\.so|libm\.so|libpthread|libdl|librt\.so|linux-vdso|ld-linux)'

HAS_DPKG=false
HAS_RPM=false
command -v dpkg &>/dev/null && HAS_DPKG=true
command -v rpm &>/dev/null && HAS_RPM=true

# soname -> absolute path, from the dynamic loader cache
lib_path() {
    ldconfig -p 2>/dev/null | awk -v lib="$1" '$1 == lib { print $NF; exit }'
}

mapfile -t LIBS < <(ldd "$BINARY" 2>/dev/null | awk '/=>/ {print $1}' | grep -v -E "$SKIP_PATTERN" | sort -u)

deb_pkg_for() {
    local path="$1" pkg=""
    if $HAS_DPKG && [[ -n "$path" ]]; then
        # dpkg -S prints "<pkg>:<arch>: <path>"; keep the package name
        pkg="$(dpkg -S "$path" 2>/dev/null | head -n 1 | cut -d: -f1 || true)"
    fi
    echo "${pkg:-${DEBIAN_FALLBACK[$2]:-UNKNOWN}}"
}

fed_pkg_for() {
    local path="$1" pkg=""
    if $HAS_RPM && [[ -n "$path" ]]; then
        pkg="$(rpm -qf --qf '%{NAME}\n' "$path" 2>/dev/null | head -n 1 || true)"
    fi
    echo "${pkg:-${FEDORA_FALLBACK[$2]:-UNKNOWN}}"
}

if $JSON_MODE; then
    echo "["
fi

FIRST=true
for lib in "${LIBS[@]}"; do
    path="$(lib_path "$lib")"
    deb_pkg="$(deb_pkg_for "$path" "$lib")"
    fed_pkg="$(fed_pkg_for "$path" "$lib")"

    if $JSON_MODE; then
        $FIRST || echo ","
        FIRST=false
        printf '  {"library": "%s", "debian": "%s", "fedora": "%s"}' "$lib" "$deb_pkg" "$fed_pkg"
    else
        printf '%s\t%s\t%s\n' "$lib" "$deb_pkg" "$fed_pkg"
    fi
done

if $JSON_MODE; then
    echo ""
    echo "]"
else
    echo ""
    echo "--- Debian packages (unique) ---"
    for lib in "${LIBS[@]}"; do deb_pkg_for "$(lib_path "$lib")" "$lib"; done | sort -u
    echo ""
    echo "--- Fedora packages (unique) ---"
    for lib in "${LIBS[@]}"; do fed_pkg_for "$(lib_path "$lib")" "$lib"; done | sort -u
fi
