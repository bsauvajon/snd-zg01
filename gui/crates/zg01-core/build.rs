//! Builds the `.zgpreset` parser of `tools/`.
//!
//! The parser is shared with `zgctl` and its tests, so the GUI compiles the
//! same C file instead of carrying a second implementation. The header lives
//! next to it and is included as `"zgpreset.h"`.

use std::path::PathBuf;

fn main() {
    let crate_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let tools = crate_dir.join("../../../tools");
    let source = tools.join("zgpreset.c");
    let header = tools.join("zgpreset.h");

    let out = std::env::var("OUT_DIR").expect("OUT_DIR");
    cc::Build::new()
        .file(&source)
        .include(&tools)
        .warnings(true)
        .flag_if_supported("-std=c11")
        // `makepkg` on Arch enables LTO and appends it after these flags.
        // The PKGBUILD also appends `-fno-lto`; keep the C archive plain.
        .flag_if_supported("-fno-lto")
        // The archive is linked by the final binary and by this crate's
        // tests; asking Cargo to bundle it into the rlib leaves an object
        // without an archive index under some linkers, so the symbols are
        // undefined at the final link. Keep the archive external.
        .cargo_metadata(false)
        .compile("zgpreset");

    println!("cargo:rerun-if-changed={}", source.display());
    println!("cargo:rerun-if-changed={}", header.display());
    println!("cargo:libdir={out}");
    println!("cargo:rustc-link-arg-tests={out}/libzgpreset.a");
    println!("cargo:rustc-link-arg-doctests={out}/libzgpreset.a");
}
