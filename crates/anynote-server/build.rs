//! Embed the built web UI (`app/dist`). Without it the server still builds
//! and serves a page explaining how to build the UI.

use std::path::PathBuf;

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let dist = manifest.join("../../app/dist");
    println!("cargo:rerun-if-changed={}", dist.display());
    let dir = if dist.join("index.html").is_file() {
        dist
    } else {
        let empty = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("no-ui");
        std::fs::create_dir_all(&empty).unwrap();
        empty
    };
    println!("cargo:rustc-env=ANYNOTE_UI_DIR={}", dir.display());
    // index.html names the content-hashed bundles, so it fingerprints the UI.
    // A changed value makes cargo recompile, re-embedding the new files.
    let index = std::fs::read(dir.join("index.html")).unwrap_or_default();
    let stamp = index.iter().fold(0xcbf29ce484222325u64, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x100000001b3));
    println!("cargo:rustc-env=ANYNOTE_UI_STAMP={stamp:016x}");
}
