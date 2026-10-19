//! Links libghostty (built by scripts/setup-linux.sh into ghostty/zig-out) and generates
//! Rust bindings from the submodule's own header, so the header always matches the library.
//!
//! Everything links statically from source: libghostty.a, its vendored C/C++ dependencies
//! (installed by our ghostty fork as zig-out/lib/libghostty-dep-*.a), and the GLAD loader.
//! `--exclude-libs,ALL` keeps those static symbols out of the executable's dynamic symbol
//! table, so GTK/Pango keep using the system freetype/harfbuzz instead of ours.
//!
//! glslang, SPIRV-Cross and Dear ImGui are C++ libraries that Zig compiles against LLVM's
//! libc++, which would mean shipping a second C++ runtime. They only power custom GLSL
//! shaders and the terminal inspector, so we skip them and link no-op stubs (stubs.c).

use std::env;
use std::path::{Path, PathBuf};

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let ghostty = manifest_dir.join("ghostty");
    let lib_dir = ghostty.join("zig-out/lib");
    let header = ghostty.join("include/ghostty.h");

    let libghostty = lib_dir.join("libghostty.a");
    if !libghostty.exists() {
        panic!(
            "{} not found: run scripts/setup-linux.sh first (builds libghostty with Zig)",
            libghostty.display()
        );
    }

    // Vendored deps, in a stable order (libghostty-dep-0.a, -1, ... -10).
    let mut deps: Vec<PathBuf> = std::fs::read_dir(&lib_dir)
        .expect("read ghostty/zig-out/lib")
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("libghostty-dep-") && n.ends_with(".a"))
        })
        .collect();
    deps.sort_by_key(|p| dep_index(p));
    // Identify the libc++-dependent libraries by content, not position, so a ghostty update
    // that reorders its dependency list can't silently change what we link.
    deps.retain(|p| !archive_members(p).iter().any(|m| {
        ["glslang", "spirv_cross", "imgui", "CodeGen.o"].iter().any(|skip| m.contains(skip))
    }));
    if deps.is_empty() {
        panic!("no libghostty-dep-*.a in {}: the ghostty submodule must be our fork (TNortnern/ghostty)", lib_dir.display());
    }

    // GLAD (OpenGL loader) from ghostty's vendored copy; libghostty expects the host to provide it.
    cc::Build::new()
        .file(ghostty.join("vendor/glad/src/gl.c"))
        .include(ghostty.join("vendor/glad/include"))
        .warnings(false)
        .compile("cmux_glad");

    // No-op glslang/SPIRV-Cross/ImGui symbols referenced by libghostty (see module docs).
    cc::Build::new()
        .file(manifest_dir.join("stubs.c"))
        .warnings(false)
        .compile("cmux_stubs");

    // A group lets the deps reference each other regardless of order.
    println!("cargo:rustc-link-arg=-Wl,--exclude-libs,ALL");
    println!("cargo:rustc-link-arg=-Wl,--start-group");
    println!("cargo:rustc-link-arg={}", libghostty.display());
    for dep in &deps {
        println!("cargo:rustc-link-arg={}", dep.display());
    }
    println!("cargo:rustc-link-arg=-Wl,--end-group");

    // System libraries libghostty needs dynamically.
    for lib in ["fontconfig", "GL", "stdc++"] {
        println!("cargo:rustc-link-lib=dylib={lib}");
    }

    println!("cargo:rerun-if-changed={}", libghostty.display());
    println!("cargo:rerun-if-changed={}", header.display());
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=stubs.c");

    let bindings = bindgen::Builder::default()
        .header(header.to_string_lossy())
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
        .allowlist_item("ghostty_.*")
        .allowlist_item("GHOSTTY_.*")
        .generate()
        .expect("Unable to generate ghostty bindings");

    let out_path = PathBuf::from(env::var("OUT_DIR").unwrap());
    bindings
        .write_to_file(out_path.join("ghostty_sys.rs"))
        .expect("Couldn't write ghostty_sys.rs");
}

fn archive_members(path: &Path) -> Vec<String> {
    let out = std::process::Command::new("ar")
        .arg("t")
        .arg(path)
        .output()
        .unwrap_or_else(|e| panic!("running `ar t {}`: {e}", path.display()));
    String::from_utf8_lossy(&out.stdout).lines().map(str::to_owned).collect()
}

fn dep_index(path: &Path) -> u32 {
    path.file_stem()
        .and_then(|s| s.to_str())
        .and_then(|s| s.strip_prefix("libghostty-dep-"))
        .and_then(|n| n.parse().ok())
        .unwrap_or(u32::MAX)
}
