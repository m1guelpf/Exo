#![deny(
    clippy::correctness,
    clippy::suspicious,
    clippy::complexity,
    clippy::perf
)]
//! Compiles Exo's icons (`assets/icons`) into the crate: writes `$OUT_DIR/icons.rs`, a table of
//! every SVG under its key (its path without `.svg`).

use std::{
    fmt::Write as _,
    path::{Path, PathBuf},
};

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/icons");
    println!("cargo:rerun-if-changed={}", root.display());
    let mut files = Vec::new();
    collect(&root, &mut files);
    files.sort();
    let mut table = String::from("&[\n");
    for file in &files {
        let key = file
            .strip_prefix(&root)
            .expect("under the icons folder")
            .with_extension("");
        let key = key
            .to_str()
            .expect("icon paths are UTF-8")
            .replace('\\', "/");
        writeln!(
            table,
            "    ({key:?}, include_bytes!({:?}).as_slice()),",
            file.display().to_string()
        )
        .expect("writing to a String");
    }
    table.push(']');
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"));
    std::fs::write(out.join("icons.rs"), table).expect("OUT_DIR is writable");
}

fn collect(dir: &Path, files: &mut Vec<PathBuf>) {
    let entries =
        std::fs::read_dir(dir).unwrap_or_else(|error| panic!("{}: {error}", dir.display()));
    for entry in entries {
        let path = entry.expect("a readable icons folder").path();
        if path.is_dir() {
            collect(&path, files);
        } else if path.extension().is_some_and(|extension| extension == "svg") {
            files.push(path);
        }
    }
}
