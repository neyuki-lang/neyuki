use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    let manifest_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let lib_dir = manifest_dir.join("lib");
    println!("cargo:rerun-if-changed={}", lib_dir.display());

    let mut libraries = fs::read_dir(&lib_dir)
        .unwrap_or_else(|err| panic!("failed to read {}: {}", lib_dir.display(), err))
        .map(|entry| entry.expect("failed to read library entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "nyk"))
        .collect::<Vec<_>>();
    libraries.sort();

    let mut generated = String::from("&[\n");
    for path in libraries {
        let name = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or_else(|| panic!("invalid library filename: {}", path.display()));
        generated.push_str(&format!(
            "    (\"@neyuki/{name}\", include_str!({:?})),\n",
            path.to_string_lossy()
        ));
    }
    generated.push_str("]\n");

    let output = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("bundled_libraries.rs");
    fs::write(&output, generated)
        .unwrap_or_else(|err| panic!("failed to write {}: {}", output.display(), err));
}
