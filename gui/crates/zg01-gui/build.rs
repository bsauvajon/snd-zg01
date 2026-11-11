//! Builds and links the `.zgpreset` parser into the GUI binary.
//!
//! `zg01-core` also builds the archive for its own tests, but Cargo's
//! propagation of that static library through the rlib is unreliable under
//! `makepkg`/lld: the final binary either misses the symbols or links the
//! object twice. Building the small C parser again in this package keeps
//! the archive and the link arguments in the binary's own `OUT_DIR`.

use std::path::PathBuf;

fn main() {
    let crate_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let tools = crate_dir.join("../../../tools");
    let source = tools.join("zgpreset.c");
    let header = tools.join("zgpreset.h");
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));

    cc::Build::new()
        .file(&source)
        .include(&tools)
        .warnings(true)
        .flag_if_supported("-std=c11")
        // `makepkg` on Arch enables LTO; lld linked through Rust has no GCC
        // LTO plugin, so the C member must stay a plain object file.
        .flag_if_supported("-fno-lto")
        .cargo_metadata(false)
        .compile("zgpreset");

    // Link the archive by path and as a whole archive. The path avoids the
    // `-Wl,-Bdynamic` state that makes `-lzgpreset` search for a shared
    // library; whole-archive makes the result independent of whether rustc
    // places the native argument before or after the Rust rlibs.
    let archive = out.join("libzgpreset.a");
    println!("cargo:rustc-link-arg=-Wl,--whole-archive");
    println!("cargo:rustc-link-arg={}", archive.display());
    println!("cargo:rustc-link-arg=-Wl,--no-whole-archive");
    println!("cargo:rerun-if-changed={}", source.display());
    println!("cargo:rerun-if-changed={}", header.display());
    println!("cargo:rerun-if-changed=build.rs");
}
