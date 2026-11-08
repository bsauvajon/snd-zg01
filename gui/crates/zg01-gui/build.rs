//! Links the `.zgpreset` parser built by `zg01-core`.
//!
//! Cargo normally carries a dependency's `rustc-link-lib` through its rlib,
//! but `makepkg` on Arch has been observed to reach the final binary link
//! without the native archive. `zg01-core` declares `links = "zgpreset"`
//! and exports its `OUT_DIR`; this script repeats the link directives on the
//! binary so the final link is independent of that propagation.

fn main() {
    let libdir = std::env::var("DEP_ZGPRESET_LIBDIR")
        .expect("zg01-core must export the zgpreset library directory");
    println!("cargo:rustc-link-search=native={libdir}");
    println!("cargo:rustc-link-lib=static=zgpreset");
    println!("cargo:rerun-if-changed=build.rs");
}
