//! Downloads and statically links the `mach-dxcompiler` C library.

use std::env;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::process::Command;
use std::{
    fs,
    io::ErrorKind::NotFound,
    path::{Path, PathBuf},
};

/// Downloads and links the static DXC binary.
fn main() {
    println!("cargo:rerun-if-changed=mach_dxc.h");
    let cache_dir = get_cached_path();
    if !cache_dir.exists() {
        fs::create_dir_all(&cache_dir).expect("Failed to create cache directory");
    }
    let target_url = get_target_url(static_crt());
    let file_name = get_file_name(&target_url);
    let file_path = cache_dir.join(format!("{file_name}.tar.gz"));
    download_if_not_existed(&target_url, &file_path);
    let out_dir = cache_dir.join(file_name);
    extract_tar_gz(&file_path, &out_dir);
    #[cfg(feature = "cbindings")]
    generate_bindings();
    println!("cargo:rustc-link-lib=static=machdxcompiler");
    println!("cargo:rustc-link-search=native={}", out_dir.display());
}

/// Get unique hashed file name for cache.
fn get_file_name(url: &str) -> String {
    let mut hasher = DefaultHasher::new();
    url.hash(&mut hasher);
    let hash = hasher.finish();
    format!("{hash}_machdxcompiler")
}

/// Generates C API bindings.
#[cfg(feature = "cbindings")]
fn generate_bindings() {
    let bindings = bindgen::Builder::default()
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
    let os = env::var("CARGO_CFG_TARGET_OS").expect("Failed to get os");
    // CARGO_CFG_TARGET_ENV may be empty
    let mut abi = env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    if abi.is_empty() {
        abi = "none".to_owned();
    }
    let target = format!("{arch}-{os}-{abi}");

    if !AVAILABLE_TARGETS.contains(&target.as_str()) {
        panic!("Unsupported target: {target}\nCheck supported targets on {BASE_URL}");
    }
    let crt = if abi == "msvc" && static_crt {
        "Dynamic_lib"
    } else {
        "lib"
    };
    format!("{BASE_URL}/download/{LATEST_RELEASE}/{target}_ReleaseFast_{crt}.tar.gz")
}

/// Checks if the file is existed and its size is not zero.
fn is_file_exists(file_path: &Path) -> bool {
    if let Ok(true) = file_path.try_exists() {
        if fs::metadata(file_path)
            .expect("unable to get metadata")
            .len()
            > 0
        {
            return true;
        }
    }
    false
}

/// Downloads the provided URL to a file if there is no existed one.
fn download_if_not_existed(url: &str, file_path: &Path) {
    if is_file_exists(file_path) {
        return;
    }
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
    if output_dir.exists() {
        if let Ok(entrys) = fs::read_dir(output_dir) {
            if entrys.count() > 0 {
                return;
            }
        }
    } else {
        fs::create_dir(&output_dir).expect("Failed to create output directory");
    }
    let result = Command::new("tar")
        .current_dir(output_dir)
        .arg("-xf")
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

/// Get global cache path for downloaded file.
fn get_cached_path() -> PathBuf {
    const CACHE_FOLDER_NAME: &str = "mach_dxcompiler_rs";
    if let Ok(cache_root) = env::var("CARGO_TARGET_DIR") {
        PathBuf::from(cache_root)
    } else {
        env::current_dir()
            .expect("Failed to get current directory")
            .join("target")
    }
    .join(CACHE_FOLDER_NAME)
}
