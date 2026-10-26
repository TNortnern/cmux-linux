# Changelog

All notable changes to cmux for Linux are documented here.

This project is a Linux port of [cmux](https://github.com/manaflow-ai/cmux) by manaflow-ai, which was originally written for macOS.

## 0.2.4

- **Fix: scrolling direction.** The mouse wheel and touchpad scrolled the opposite way (wheel up
  moved toward the bottom). cmux now inverts GTK's deltas for Ghostty the same way Ghostty's own
  Linux app does. Touchpad scrolling is also smoother, no longer rounded to whole steps.

## 0.2.3

- **Dark chrome under any theme.** Header bar buttons, tab close and new-tab buttons and the
  sidebar's add button no longer show as white boxes under Adwaita light or Yaru, and the
  AppImage no longer forces the light theme. Menus, dialogs and window controls use the
  theme's dark variant.
- **Fix: renaming tabs and workspaces.** Double-click, Ctrl+Shift+R and the sidebar's Rename
  now focus the editor with the old name selected. Before, typing went to the terminal.
  Clicking away commits the new name.
- **The bell dot is a small circle** on GTK 4.6. It used to stretch into a tall oval.

## 0.2.2

- **Fix: `cmux send-text` commands now run.** Line breaks in the text are sent as a real Enter
  key press. Before, the shell received them inside a bracketed paste and only inserted them, so
  scripted commands were typed but never executed in packaged installs. `cmux send-key enter`
  presses Enter too.

## 0.2.1

### Runs on Ubuntu 22.04

- **GTK 4.6 is now the minimum** (was 4.14), so cmux runs on Ubuntu 22.04, Debian 12 and newer.
  Fractional scaling is used when GTK 4.12+ provides it, with a fall back to integer scaling.
  The close-workspace dialog and the shortcuts window use APIs every GTK 4 has.
- **Release packages are built on Ubuntu 22.04** (glibc 2.35 baseline), so the AppImage, `.deb`
  and `.rpm` run on 22.04 and everything newer.
- **Older OpenGL fallback.** When the GPU driver offers less than the OpenGL 4.3 that Ghostty
  needs (older GPUs, WSL), cmux restarts itself once on Mesa's software rasterizer instead of
  exiting silently.
- Verified on a clean Ubuntu 22.04.5 (GTK 4.6.9, glibc 2.35): built from a fresh clone with
  `scripts/setup-linux.sh`, 111 unit tests and 19/19 end-to-end checks.

## 0.2.0

### Highlights

- **Real tabs.** The model is now workspace → tabs → splits, with a tab bar (close buttons, "+", drag to reorder, double-click to rename) and per-tab attention dots.
- **New tabs and splits open in the focused pane's current folder.** Ghostty shell integration is shipped and enabled. A `/proc` fallback covers shells without it.
- **Session restore.** Workspaces, tabs, splits and each pane's folder come back on the next launch. Shells are ended on quit. Saves are debounced, flushed on quit, SIGTERM and SIGHUP, and survive `kill -9`. Opt out with `[session] restore = false`.
- **Windows Terminal-style keymap**, matching cmux for Windows. Plain Ctrl+letter chords go to the shell, and AltGr characters are never swallowed.
- **Less memory.** Measured under WSL: 234 → 168 MB RSS idle (−28%), 512 → 450 MB with 12 busy panes, and 109 → 49 threads. Changes:
  - 2 tokio workers instead of one per core
  - capped malloc arenas
  - no 10 Hz idle timer
  - 2 MB scrollback cap per pane instead of 10 MB
  - hidden tabs stop rendering, and restored tabs start their shells lazily
  - cairo GTK renderer and 2 llvmpipe threads on machines without a GPU render node

### Fixes

- Fresh clones could not build: the ghostty commit the app depended on was never published, and a workspace member was missing.
- Heap corruption (crashes in `FT_Done_Face`) from two FreeType copies in one process. The build now uses the system FreeType only: 15 crashes in a 40-cycle stress run before, none after.
- Use-after-free when closing a workspace; shells and GL state leaked for every closed workspace.
- Paste could land in the wrong pane. Clipboard requests now complete on the requesting pane, without a nested main loop.
- `exit` in a pane now closes it. It used to leave a dead pane.
- Re-entrant `RefCell` borrows from rename and focus signals (abort).
- Pane ids collided across restored workspaces.
- Splitter and close bugs in nested layouts.

### Security

- OSC 52 clipboard **reads** are denied by default. Writes are honoured only for the focused pane.
- SSH targets are validated and passed after `--`, so a host like `-oProxyCommand=…` can't run commands. Deploys use `BatchMode` and a timeout.
- Socket server: 1 MiB request cap, 64 connections, 30 s timeouts, and an accept loop that survives errors.
- The CLI only connects to sockets owned by you (no `/tmp` fallbacks).
- Browser automation is limited to http, https and about:blank. Its daemon directory is private (0700). Release builds no longer have `debug.*` socket methods.
- `session.json` is written with mode 0600.
- Removed a GitHub workflow that exposed an OAuth secret to any commenter.


### Build and source

- The build is reproducible from public sources. libghostty is built from the GTK 4 platform fork published at [TNortnern/ghostty](https://github.com/TNortnern/ghostty) (branch `cmux-linux`), with Zig 0.15.2 (SHA-256 verified). `scripts/setup-linux.sh` installs the dependencies and builds it.
- Removed committed binaries, `node_modules`, and the macOS app, Xcode project, Sparkle updater, and release scripts.

### Packaging and CI

- AppImage, `.deb`, and `.rpm` packages built from the same file layout. The launcher `/usr/bin/cmux-app` sets the GDK backend and the Ghostty resources path. Packages list their runtime dependencies explicitly and do not require `agent-browser`.
- Ghostty shell integration and the `xterm-ghostty` terminfo are packaged under `/usr/share/cmux`, so new tabs and splits open in the current folder.
- The AppImage is built with pinned linuxdeploy and GTK plugin versions, verified by SHA-256.
- CI builds on Ubuntu 24.04 (clippy, tests, release build, Go remote daemon, all packages, and package validation). Tag pushes (`v*`) publish a GitHub release with `SHA256SUMS`. Actions are pinned to commit SHAs.

