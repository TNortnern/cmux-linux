# Changelog

All notable changes to cmux for Linux are documented here.

This project is a Linux port of [cmux](https://github.com/manaflow-ai/cmux) by manaflow-ai, which was originally written for macOS.

## 0.2.0 (unreleased)

### Build and source

- The build is reproducible from public sources. libghostty is built from the GTK 4 platform fork published at [TNortnern/ghostty](https://github.com/TNortnern/ghostty) (branch `cmux-linux`), with Zig 0.15.2 (SHA-256 verified). `scripts/setup-linux.sh` installs the dependencies and builds it.
- Removed committed binaries, `node_modules`, and the macOS app, Xcode project, Sparkle updater, and release scripts.

### Packaging and CI

- AppImage, `.deb`, and `.rpm` packages built from the same file layout. The launcher `/usr/bin/cmux-app` sets the GDK backend and the Ghostty resources path. Packages list their runtime dependencies explicitly and do not require `agent-browser`.
- Ghostty shell integration and the `xterm-ghostty` terminfo are packaged under `/usr/share/cmux`, so new tabs and splits open in the current folder.
- The AppImage is built with pinned linuxdeploy and GTK plugin versions, verified by SHA-256.
- CI builds on Ubuntu 24.04 (clippy, tests, release build, Go remote daemon, all packages, and package validation). Tag pushes (`v*`) publish a GitHub release with `SHA256SUMS`. Actions are pinned to commit SHAs.

### Security

- Security fixes for this release are listed here before the release is tagged.
