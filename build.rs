//! Downloads and statically links the `mach-dxcompiler` C library.

use std::env;
use std::process::Command;
use std::{
    fs,
    io::ErrorKind::NotFound,
    path::{Path, PathBuf},
};

/// Downloads and links the static DXC binary.
fn main() {
    println!("cargo:rerun-if-changed=mach_dxc.h");
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("Failed to get OUT_DIR environment"));
    let target_url = get_target_url(static_crt());
    let file_path = out_dir.join("machdxcompiler.tar.gz");
    download_released_library(&target_url, &file_path);
    extract_tar_gz(&file_path, &out_dir);
    #[cfg(feature = "cbindings")]
    generate_bindings();
    println!("cargo:rustc-link-lib=static=machdxcompiler");
    println!("cargo:rustc-link-search=native={}", out_dir.display());
}

/// Generates C API bindings.
#[cfg(feature = "cbindings")]
fn generate_bindings() {
    let bindings = bindgen::Builder::default()
        .rust_target(bindgen::RustTarget::Stable_1_73)
        // The input header we would like to generate
        // bindings for.
        .header("mach_dxc.h")
        // Finish the builder and generate the bindings.
        .generate()
        // Unwrap the Result and panic on failure.
        .expect("Unable to generate bindings");

    // Write the bindings to the src/bindings.rs file.
    let out_path =
        PathBuf::from(env::var("OUT_DIR").expect("Failed to get OUT_DIR environment variable"));
    bindings
        .write_to_file(out_path.join("cbindings.rs"))
        .expect("Couldn't write bindings!");
}

/// Gets the URL from which the DXC binary should be downloaded.
fn get_target_url(static_crt: bool) -> String {
    const BASE_URL: &str = "https://github.com/DouglasDwyer/mach-dxcompiler/releases";
    const LATEST_RELEASE: &str = "2024.11.22+284d956.1";
    const AVAILABLE_TARGETS: &[&str] = &[
        "x86_64-linux-gnu",
        "x86_64-linux-musl",
        "aarch64-linux-gnu",
        "aarch64-linux-musl",
        "x86_64-windows-gnu",
        "x86_64-windows-msvc",
        "aarch64-windows-gnu",
        "x86_64-macos-none",
        "aarch64-macos-none",
    ];
    let arch = env::var("CARGO_CFG_TARGET_ARCH").expect("Failed to get architecture");
    let mut os = env::var("CARGO_CFG_TARGET_OS").expect("Failed to get os");
    // apple-darwin => macos
    if env::var("CARGO_CFG_TARGET_VENDOR").unwrap_or_default() == "apple" && os == "darwin" {
        os = "macos".to_owned();
    }
    // CARGO_CFG_TARGET_ENV may be empty
    let mut abi = env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    if abi.is_empty() {
        abi = "none".to_owned();
    }
    let target = format!("{arch}-{os}-{abi}");

    if !AVAILABLE_TARGETS.contains(&target.as_str()) {
        panic!("Unsupported target: {target}\nCheck supported targets on {BASE_URL}");
    }
    let crt = if abi == "msvc" && !static_crt {
        "Dynamic_lib"
    } else {
        "lib"
    };
    format!("{BASE_URL}/download/{LATEST_RELEASE}/{target}_ReleaseFast_{crt}.tar.gz")
}

/// Downloads the provided URL to a file.
fn download_released_library(url: &str, file_path: &Path) {
    match Command::new("curl")
        .arg("--location")
        .arg("-o")
        .arg(file_path)
        .arg(url)
        .spawn()
        .expect("Failed to start Curl to download DXC binary")
        .wait()
    {
        Ok(result) => {
            if !result.success() {
                if let Err(e) = fs::remove_file(file_path) {
                    if e.kind() != NotFound {
                        panic!("Failed to remove incomplete file");
                    }
                }
                panic!("{result}");
            }
        }
        Err(_) => {
            if let Err(e) = fs::remove_file(file_path) {
                if e.kind() != NotFound {
                    panic!("Failed to remove incomplete file");
                }
            }
            panic!("Failed to download DXC binary");
        }
    }
}

/// Extracts the file at the provided path as a `.tar.gz` file.
/// The contents are extracted to the current directory.
fn extract_tar_gz(path: &Path, output_dir: &Path) {
    let result = Command::new("tar")
        .current_dir(output_dir)
        .arg("-xzf")
        .arg(path)
        .spawn()
        .expect("Failed to start Tar to extract DXC binary")
        .wait()
        .expect("Failed to extract DXC binary");
    if !result.success() {
        panic!("{result}");
    }
}

/// Determines whether the CRT is being statically or dynamically linked.
fn static_crt() -> bool {
    env::var("CARGO_ENCODED_RUSTFLAGS")
        .map(|flags| flags.contains("target-feature=+crt-static"))
        .unwrap_or(false)
}
