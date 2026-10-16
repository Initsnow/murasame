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

        // Live2D Cubism SDK for Native Core linking
        let sdk_dir = manifest_dir.join("CubismSdkForNative-5-r.5");
        let zip_file = manifest_dir.join("CubismSdkForNative-5-r.5.zip");
        if !sdk_dir.exists() && zip_file.exists() {
            println!("cargo:warning=Extracting CubismSdkForNative-5-r.5.zip...");
            let _ = std::process::Command::new("tar")
                .args(["-xf", zip_file.to_str().unwrap()])
                .current_dir(&manifest_dir)
                .status();
        }

        let cubism_sdk_core_dir = sdk_dir
            .join("Core")
            .join("lib")
            .join("windows")
            .join("x86_64")
            .join("143");
        let cubism_sdk_dll_dir = sdk_dir
            .join("Core")
            .join("dll")
            .join("windows")
            .join("x86_64");

        if cubism_sdk_core_dir.exists() {
            println!(
                "cargo:rustc-link-search=native={}",
                cubism_sdk_core_dir.display()
            );
            println!("cargo:rustc-link-lib=static=Live2DCubismCore_MD");
        } else if cubism_sdk_dll_dir.exists() {
            println!(
                "cargo:rustc-link-search=native={}",
                cubism_sdk_dll_dir.display()
            );
            println!("cargo:rustc-link-lib=dylib=Live2DCubismCore");
        }

        // Copy Live2DCubismCore.dll to target directories if dll exists
        let dll_path = cubism_sdk_dll_dir.join("Live2DCubismCore.dll");
        if dll_path.exists() {
            if let Ok(out_dir) = std::env::var("OUT_DIR") {
                let target_dir = PathBuf::from(out_dir)
                    .ancestors()
                    .nth(3)
                    .map(|p| p.to_path_buf());
                if let Some(target_dir) = target_dir {
                    let _ = std::fs::copy(&dll_path, target_dir.join("Live2DCubismCore.dll"));
                }
            }
        }
    }

    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=lib");
    println!("cargo:rerun-if-changed=CubismSdkForNative-5-r.5");
    println!("cargo:rerun-if-changed=ui");
    println!("cargo:rerun-if-changed=assets/icons/app_icon.ico");

    #[cfg(target_os = "windows")]
    {
        let mut res = winres::WindowsResource::new();
        res.set_icon("assets/icons/app_icon.ico");
        if let Err(e) = res.compile() {
            eprintln!("Warning: Failed to compile windows resource icon: {e}");
        }
    }

    let ui_file = manifest_dir.join("ui").join("main.slint");
    if ui_file.exists() {
        slint_build::compile(&ui_file).expect("Failed to compile Slint UI file");
    }
}
