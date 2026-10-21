# cmux for Linux

A GPU-accelerated terminal multiplexer for Linux: workspaces, tabs, and splits, built with GTK 4 and [libghostty](https://github.com/ghostty-org/ghostty).

Built for running several coding agents (Claude Code, Codex, and others) side by side. Each workspace shows what it is doing, notifications tell you when an agent needs attention, and a socket CLI lets scripts drive the same workspaces.

## Features

- **Terminal:** GPU-rendered terminals with libghostty. It reads your Ghostty config for fonts, colors, and themes.
- **Workspaces, tabs, and splits:** Organize sessions per project. New tabs and splits open in the folder of the current pane (bash, zsh, and fish, through shell integration).
- **Session restore:** On launch, workspaces, tabs, splits, and each pane's folder come back. Shells are not kept running. You can turn this off in settings.
- **Notifications:** Per-pane bell and notification tracking, with sidebar indicators.
- **Socket CLI:** The `cmux` command controls workspaces, panes, and surfaces from scripts and agents over a local socket.
- **Optional browser automation:** `cmux browser` drives a browser through [agent-browser](https://github.com/vercel-labs/agent-browser). Nothing runs until you use it.
- **Optional SSH workspaces:** Remote workspaces through `cmuxd-remote`. Nothing runs until you use it.

## Install

Download the package for your distribution from the [Releases](https://github.com/TNortnern/cmux-linux/releases) page.

### AppImage (any distribution)

```bash
chmod +x cmux-<version>-x86_64.AppImage
./cmux-<version>-x86_64.AppImage
```

### Debian and Ubuntu (.deb)

```bash
sudo apt install ./cmux_<version>_amd64.deb
```

### Fedora (.rpm)

```bash
sudo dnf install ./cmux-<version>.x86_64.rpm
```

Packages install `cmux-app` (the GUI), `cmux` (the CLI), a desktop entry, shell completions, a man page, and the agent skills under `/usr/share/cmux/skills/`.

## Requirements

- Ubuntu 22.04 or newer, Debian 12 or newer, Fedora 38 or newer, or a current Arch Linux
- GTK 4.6 or newer (fractional scaling needs GTK 4.12+; it falls back to integer scaling)
- OpenGL 4.3, from Mesa or your GPU vendor's driver

## Keyboard shortcuts

The keymap follows Windows Terminal. Plain Ctrl+letter chords (Ctrl+D, Ctrl+W, Ctrl+C, Ctrl+B, ...) always go to the shell. Shortcuts can be rebound in `[shortcuts]` in `~/.config/cmux/config.toml`.

| Action | Shortcut |
|---|---|
| New tab (in the focused pane's folder) | `Ctrl+Shift+T` |
| Close pane (closes the tab when it is the last pane) | `Ctrl+Shift+W` |
| Next tab / previous tab | `Ctrl+Tab` / `Ctrl+Shift+Tab` |
| Select tab 1 to 8 / last tab | `Ctrl+1` to `Ctrl+8` / `Ctrl+9` |
| Rename tab | `Ctrl+Shift+R` |
| Split right / split down | `Alt+Shift+=` / `Alt+Shift+-` |
| Split along the longer side | `Alt+Shift+D` |
| Focus pane left / right / up / down | `Alt+Left` / `Alt+Right` / `Alt+Up` / `Alt+Down` |
| Resize pane (40 px) | `Alt+Shift+Arrows` |
| Zoom pane | `Ctrl+Shift+Z` |
| Equalize panes | `Ctrl+Shift+E` |
| New workspace | `Ctrl+Shift+N` |
| Select workspace 1 to 9 | `Ctrl+Alt+1` to `Ctrl+Alt+9` |
| Toggle sidebar | `Ctrl+Shift+B` |
| New SSH workspace | `Ctrl+Shift+S` |
| Open / close browser pane | `Ctrl+Alt+B` / `Ctrl+Alt+Shift+B` |
| Find (not implemented yet) | `Ctrl+Shift+F` |
| Copy / paste | `Ctrl+Shift+C` / `Ctrl+Shift+V` |
| Preferences | `Ctrl+,` |
| Quit | `Ctrl+Shift+Q` |

## Socket and security

- The control socket is at `$XDG_RUNTIME_DIR/cmux/cmux.sock`, owned by your user with mode 0600. Each connection is checked with `SO_PEERCRED`, so only processes running as your user can talk to it.
- Terminal programs cannot read your clipboard through OSC 52 by default. Enabling clipboard reads is an explicit opt-in.

## Build from source

Requirements: a Rust toolchain, Git, and `curl`. The steps below install the rest.

```bash
git clone --recurse-submodules https://github.com/TNortnern/cmux-linux.git
cd cmux-linux
scripts/setup-linux.sh          # system packages, Zig 0.15.2, and libghostty (Ubuntu, Fedora, or Arch)
cargo build --release --bin cmux-app --bin cmux
```

The binaries are in `target/release/`. `scripts/setup-linux.sh` builds `ghostty/` (the TNortnern/ghostty `cmux-linux` branch) with Zig; see [docs/ghostty-fork.md](docs/ghostty-fork.md).

Optional components:

```bash
# Remote daemon for SSH workspaces (Go 1.22+)
(cd daemon/remote && CGO_ENABLED=0 go build -trimpath -ldflags "-s -w" -o cmuxd-remote ./cmd/cmuxd-remote)

# Packages (the .deb and .rpm builders need dpkg-deb or rpmbuild; the AppImage builder downloads linuxdeploy)
packaging/scripts/build-deb.sh
packaging/scripts/build-rpm.sh
packaging/scripts/build-appimage.sh
packaging/scripts/validate-deb.sh   # and validate-rpm.sh, validate-all.sh
```

Packages are written to `dist/`.

## Browser automation

Browser commands need the optional `agent-browser` tool on your `PATH`, or installed at `/usr/lib/cmux/agent-browser`. cmux runs without it.

```bash
cmux browser open example.com                 # returns a surface handle, for example surface:1
cmux browser surface:1 snapshot --interactive # accessibility tree with element refs
cmux browser surface:1 click e3               # click an element by ref
cmux browser surface:1 fill e5 "search term"  # fill an input
cmux browser surface:1 eval 'document.title'  # run JavaScript
```

Agent skills that document the browser commands are in `/usr/share/cmux/skills/cmux-browser/`.

## Contributing

Issues and pull requests are welcome. Please run `cargo test --bins` and `cargo clippy --bins` before opening a pull request, and keep changes focused.

## Credits

cmux-linux is a Linux port of [cmux](https://github.com/manaflow-ai/cmux) by manaflow-ai, which was originally written for macOS. It builds on [Ghostty](https://github.com/ghostty-org/ghostty) and on earlier work in [bradwilson331/cmux-linux](https://github.com/bradwilson331/cmux-linux).

## License

Licensed under the GNU Affero General Public License v3.0 or later (`AGPL-3.0-or-later`). See [LICENSE](LICENSE).
