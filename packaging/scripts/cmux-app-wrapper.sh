#!/bin/sh
# cmux-app launcher, installed as /usr/bin/cmux-app (cmux-app.bin is next to it).
#
# - Picks the GDK backend: GTK4 may choose Wayland/EGL even in X11 sessions when
#   Wayland libraries are present, which fails on NVIDIA proprietary drivers.
# - Points GHOSTTY_RESOURCES_DIR at the packaged shell integration when it is not
#   already set. Works from /usr/bin and from inside an AppImage.

here=$(dirname "$(readlink -f "$0")")

if [ -z "$GDK_BACKEND" ]; then
    case "${XDG_SESSION_TYPE}" in
        x11)
            export GDK_BACKEND=x11
            ;;
        wayland)
            # Check for NVIDIA proprietary driver: EGL often fails there
            if command -v nvidia-smi >/dev/null 2>&1; then
                export GDK_BACKEND=x11
            fi
            ;;
    esac
fi

if [ -z "$GHOSTTY_RESOURCES_DIR" ] && [ -d "$here/../share/cmux/ghostty" ]; then
    GHOSTTY_RESOURCES_DIR=$(cd "$here/../share/cmux/ghostty" && pwd)
    export GHOSTTY_RESOURCES_DIR
fi

exec "$here/cmux-app.bin" "$@"
