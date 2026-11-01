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

    cc::Build::new()
        .file(&source)
        .include(&tools)
        .warnings(true)
        .flag_if_supported("-std=c11")
        .compile("zgpreset");

    println!("cargo:rerun-if-changed={}", source.display());
    println!("cargo:rerun-if-changed={}", header.display());
}
