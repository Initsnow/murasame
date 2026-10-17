use std::path::PathBuf;

fn main() {
    let target = std::env::var("TARGET").unwrap_or_default();
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let lib_dir = manifest_dir.join("lib").join(&target);

    let fallback_dir = manifest_dir.join("lib").join("x86_64-pc-windows-msvc");

    if target.contains("x86_64") || target.contains("windows") {
        if lib_dir.exists() {
            println!("cargo:rustc-link-search=native={}", lib_dir.display());
            println!("cargo:rustc-link-lib=static=beatrice");
        } else if fallback_dir.exists() {
            println!("cargo:rustc-link-search=native={}", fallback_dir.display());
            println!("cargo:rustc-link-lib=static=beatrice");
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
        } else if lib_dir.join("Live2DCubismCore_MD.lib").exists() {
            println!("cargo:rustc-link-search=native={}", lib_dir.display());
            println!("cargo:rustc-link-lib=static=Live2DCubismCore_MD");
        } else if fallback_dir.join("Live2DCubismCore_MD.lib").exists() {
            println!("cargo:rustc-link-search=native={}", fallback_dir.display());
            println!("cargo:rustc-link-lib=static=Live2DCubismCore_MD");
        }

        // Copy Live2DCubismCore.dll to target directories if dll exists
        let dll_candidate = if cubism_sdk_dll_dir.join("Live2DCubismCore.dll").exists() {
            Some(cubism_sdk_dll_dir.join("Live2DCubismCore.dll"))
        } else if lib_dir.join("Live2DCubismCore.dll").exists() {
            Some(lib_dir.join("Live2DCubismCore.dll"))
        } else if fallback_dir.join("Live2DCubismCore.dll").exists() {
            Some(fallback_dir.join("Live2DCubismCore.dll"))
        } else {
            None
        };

        if let (Some(dll_path), Ok(out_dir)) = (dll_candidate, std::env::var("OUT_DIR")) {
            let target_dir = PathBuf::from(out_dir)
                .ancestors()
                .nth(3)
                .map(|p| p.to_path_buf());
            if let Some(target_dir) = target_dir {
                let _ = std::fs::copy(&dll_path, target_dir.join("Live2DCubismCore.dll"));
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
