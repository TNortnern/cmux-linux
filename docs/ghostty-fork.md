# Ghostty Fork (Linux)

cmux-linux builds against a fork of Ghostty: github.com/TNortnern/ghostty, branch `cmux-linux`.
The `ghostty` submodule (path `ghostty`) is pinned to a commit on that branch.

## Fork changes

Two commits sit on top of the upstream base:

### 1) GTK4 embedded platform (`c8e38ef8bc`)

- Adds `GHOSTTY_PLATFORM_GTK4` to the embedded platform API. The host passes a
  `GtkGLArea` handle (`gl_area`). No `GdkGLContext` is passed.
- The Rust side makes the GL context current in the GtkGLArea realize handler.
- Includes the GL realize exports and the Linux render and cursor fixes.

### 2) Vendored deps and libbz2 (`8fda349153`)

- A non-Darwin static libghostty does not include its vendored C/C++ dependencies.
  The fork installs them next to `libghostty.a` in `zig-out/lib` as
  `libghostty-dep-0.a` through `libghostty-dep-N.a`, so `build.rs` can link them.
- With `-fsys=freetype`, libbz2 is linked by its real name, `bz2`
  (Ubuntu ships no bzip2.pc).

## Build

In the submodule:

```sh
zig build -Dapp-runtime=none -Doptimize=ReleaseFast -Dcpu=baseline
```

This produces `zig-out/lib/libghostty.a` plus the `libghostty-dep-*.a` archives.
`build.rs` links them. `scripts/setup-linux.sh` runs this step and checks for the archives.

## Bindings

Bindings are generated at build time by bindgen from `ghostty/include/ghostty.h`
(see `build.rs`). The header always matches the library that is linked.

## Updating the fork

1. Commit the change to the `cmux-linux` branch of TNortnern/ghostty and push it.
2. In this repo, rebuild the submodule with the command above.
3. Confirm `build.rs` still finds every `libghostty-dep-*.a` archive. It panics
   if none are present, which usually means the submodule is not the fork.
4. `git add ghostty` and commit the new submodule SHA.
