#!/usr/bin/env bash
# Prepare a Linux dev environment and build libghostty.
#
# Installs system packages (apt, dnf or pacman), Zig 0.15.2 into ~/.local/zig
# (SHA-256 verified), initializes the ghostty submodule and builds libghostty.
# Afterwards:  cargo build --release --bin cmux-app --bin cmux
#
# Usage: scripts/setup-linux.sh [--no-deps]   (--no-deps skips system packages)
set -euo pipefail

ZIG_VERSION="0.15.2"
ZIG_SHA256="02aa270f183da276e5b5920b1dac44a63f1a49e55050ebde3aecc9eb82f93239"
ZIG_HOME="${HOME}/.local/zig"

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

INSTALL_DEPS=true
for arg in "$@"; do
    case "$arg" in
        --no-deps) INSTALL_DEPS=false ;;
        -h|--help) sed -n '2,8p' "$0"; exit 0 ;;
        *) echo "error: unknown option: $arg" >&2; exit 2 ;;
    esac
done

SUDO=""
if [[ "$(id -u)" -ne 0 ]]; then
    SUDO="sudo"
fi

install_system_deps() {
    if pkg-config --exists gtk4 libadwaita-1 fontconfig freetype2 && command -v clang >/dev/null; then
        echo "==> System dependencies already present"
        return
    fi

    echo "==> Installing system dependencies"
    if command -v apt-get >/dev/null; then
        $SUDO apt-get update
        $SUDO env DEBIAN_FRONTEND=noninteractive apt-get install -y \
            build-essential pkg-config curl xz-utils git \
            libgtk-4-dev libadwaita-1-dev libclang-dev clang \
            libfontconfig-dev libfreetype-dev libgl-dev libegl-dev \
            libonig-dev libbz2-dev
    elif command -v dnf >/dev/null; then
        $SUDO dnf install -y \
            gcc gcc-c++ make pkgconf-pkg-config curl xz git \
            gtk4-devel libadwaita-devel clang-devel clang \
            fontconfig-devel freetype-devel mesa-libGL-devel mesa-libEGL-devel \
            oniguruma-devel bzip2-devel
    elif command -v pacman >/dev/null; then
        $SUDO pacman -S --needed --noconfirm \
            base-devel pkgconf curl xz git \
            gtk4 libadwaita clang fontconfig freetype2 mesa oniguruma bzip2
    else
        echo "error: no supported package manager (apt, dnf or pacman)." >&2
        echo "Install GTK 4, libadwaita, clang, fontconfig, freetype, GL/EGL, oniguruma and bzip2 development files." >&2
        exit 1
    fi
}

ensure_zig() {
    export PATH="${ZIG_HOME}:${PATH}"
    if [[ "$(zig version 2>/dev/null || true)" == "$ZIG_VERSION" ]]; then
        echo "==> zig $ZIG_VERSION found: $(command -v zig)"
        return
    fi

    local arch
    arch="$(uname -m)"
    if [[ "$arch" != "x86_64" ]]; then
        echo "error: Zig $ZIG_VERSION is only auto-installed for x86_64 (this is $arch)." >&2
        echo "Install Zig $ZIG_VERSION manually and put it on PATH." >&2
        exit 1
    fi

    echo "==> Installing zig $ZIG_VERSION to $ZIG_HOME"
    local tmp url
    tmp="$(mktemp -d)"
    url="https://ziglang.org/download/${ZIG_VERSION}/zig-x86_64-linux-${ZIG_VERSION}.tar.xz"
    curl -fSL --retry 3 -o "$tmp/zig.tar.xz" "$url"
    echo "${ZIG_SHA256}  $tmp/zig.tar.xz" | sha256sum -c -
    tar -xJf "$tmp/zig.tar.xz" -C "$tmp"
    rm -rf "$ZIG_HOME"
    mkdir -p "$(dirname "$ZIG_HOME")"
    mv "$tmp/zig-x86_64-linux-${ZIG_VERSION}" "$ZIG_HOME"
    rm -rf "$tmp"
    echo "==> zig $("$ZIG_HOME/zig" version)"
}

init_ghostty() {
    if [[ ! -f ghostty/build.zig ]]; then
        echo "==> Initializing ghostty submodule"
        git submodule update --init --depth 1 ghostty
    fi
}

build_libghostty() {
    echo "==> Building libghostty (this takes a few minutes the first time)"
    (
        cd ghostty
        # Stale dependency archives from an earlier configuration must not be linked.
        rm -f zig-out/lib/libghostty-dep-*.a
        # -fsys=freetype: one FreeType per process. fontconfig is always the system library and
        # receives ghostty's FT_Face objects; a second, vendored FreeType corrupts the heap.
        zig build \
            -Dapp-runtime=none \
            -Doptimize=ReleaseFast \
            -Dcpu=baseline \
            -fsys=freetype
    )

    local lib_dir="ghostty/zig-out/lib"
    if [[ ! -f "$lib_dir/libghostty.a" ]]; then
        echo "error: $lib_dir/libghostty.a was not produced" >&2
        exit 1
    fi
    if ! compgen -G "$lib_dir/libghostty-dep-*.a" >/dev/null; then
        echo "error: no libghostty-dep-*.a in $lib_dir (the ghostty submodule must be the TNortnern/ghostty cmux-linux branch)" >&2
        exit 1
    fi
    ls -lh "$lib_dir/libghostty.a"
}

stage_resources() {
    echo "==> Staging ghostty shell integration and terminfo"
    "$REPO_ROOT/packaging/scripts/stage-ghostty-resources.sh" \
        "$REPO_ROOT/ghostty" "$REPO_ROOT/ghostty/zig-out/share"
}

if [[ "$INSTALL_DEPS" == true ]]; then
    install_system_deps
fi
ensure_zig
init_ghostty
build_libghostty
stage_resources

cat <<EOF

==> Done. Build the app and CLI with:

    cargo build --release --bin cmux-app --bin cmux

Optional: the remote daemon (SSH workspaces) is built with
    (cd daemon/remote && CGO_ENABLED=0 go build -trimpath -ldflags "-s -w" -o cmuxd-remote ./cmd/cmuxd-remote)
EOF
