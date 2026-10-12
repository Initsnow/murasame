use std::path::PathBuf;

fn main() {
    let target = std::env::var("TARGET").unwrap_or_default();
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let lib_dir = manifest_dir.join("lib").join(&target);

    if target.contains("x86_64") || target.contains("windows") {
        if lib_dir.exists() {
            println!("cargo:rustc-link-search=native={}", lib_dir.display());
            println!("cargo:rustc-link-lib=static=beatrice");
        } else {
            let fallback_dir = manifest_dir.join("lib").join("x86_64-pc-windows-msvc");
            if fallback_dir.exists() {
                println!("cargo:rustc-link-search=native={}", fallback_dir.display());
                println!("cargo:rustc-link-lib=static=beatrice");
            }
        }
    }

    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=lib");
    println!("cargo:rerun-if-changed=ui");

    let ui_file = manifest_dir.join("ui").join("main.slint");
    if ui_file.exists() {
        slint_build::compile(&ui_file).expect("Failed to compile Slint UI file");
    }
}
