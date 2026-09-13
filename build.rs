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
    println!("cargo:rerun-if-changed=msvc_version.c");
    println!("cargo:rerun-if-changed=mach_dxc.h");
    #[cfg(all(feature = "msvc_version_validation", target_env = "msvc"))]
    validate_msvc_version();
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("Failed to get OUT_DIR environment"));
    let target_url = get_target_url(static_crt());
    let file_path = out_dir.join("machdxcompiler.tar.gz");
    download_released_library(&target_url, &file_path);
    extract_tar_gz(&file_path, &out_dir);
    #[cfg(feature = "cbindings")]
    generate_bindings();
    println!("cargo:rustc-link-lib=static=machdxcompiler");
    println!("cargo:rustc-link-search=native={}", out_dir.display());
    link_gnu_abi_runtime();
}

/// On linux-gnu and windows-gnu, `machdxcompiler` is compiled against the target's own, real
/// libstdc++ (not bundled into the archive -- see the mach-dxcompiler-rs#12 follow-up report and
/// mach-dxcompiler's `GnuLibstdcxx` in build.zig), so the consumer supplies it, same as any other
/// C++ static library. windows-msvc needs none of this: its .lib carries `/DEFAULTLIB`
/// directives that auto-request the matching CRT from the consumer's own Visual Studio install.
fn link_gnu_abi_runtime() {
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_env = env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    match (target_os.as_str(), target_env.as_str()) {
        ("linux", "gnu") => {
            println!("cargo:rustc-link-lib=stdc++");
        }
        ("windows", "gnu") => {
            // mingw-w64 implements the "secure" CRT functions DXC calls (e.g. `_snprintf_s`)
            // exclusively via UCRT's common entry points, even under a classic msvcrt.dll build
            // (this is mingw-w64's own hybrid-CRT design, not specific to this crate) -- link
            // UCRT alongside the default msvcrt for those. The two runtimes both define a
            // handful of legacy functions identically (e.g. `mbsrtowcs`); tell the linker to
            // keep the first definition instead of erroring on the duplicate.
            println!("cargo:rustc-link-lib=ucrt");
            println!("cargo:rustc-link-lib=ole32");
            println!("cargo:rustc-link-lib=oleaut32");
            println!("cargo:rustc-link-lib=version");
            println!("cargo:rustc-link-arg=-Wl,--allow-multiple-definition");
        }
        _ => {}
    }
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

/// This checks if the installed version of the Microsoft Visual C++ compiler (MSVC)
/// is compatible with the features introduced in Visual Studio 2022 version 17.11.
/// It retrieves the compiler version using the `cl.exe` executable and compares it against
/// a predefined minimum version. Panics if the current version is lower than the required minimum version.
#[cfg(all(feature = "msvc_version_validation", target_env = "msvc"))]
fn validate_msvc_version() {
    let target = std::env::var("TARGET").expect("Faild to get TARGET environment");

    // https://learn.microsoft.com/en-us/cpp/build/reference/ep-preprocess-to-stdout-without-hash-line-directives?view=msvc-170
    // Preprocess the _MSC_VER macro to get current version
    const CL_ARGS: &[&str] = &["/nologo", "/EP", "msvc_version.c"];

    // https://learn.microsoft.com/en-us/cpp/overview/compiler-versions?view=msvc-170#version-macros
    // MSVC version of Visual Studio 2022 version 17.11
    const MINIMUM_MSVC_VERSION: usize = 1941;

    let status = cc::windows_registry::find(&target, "cl.exe")
        .expect("Failed to locate cl.exe.\nPlease ensure it is installed and included in your system's PATH environment variable.")
        .args(CL_ARGS)
        .output()
        .expect(&format!("Failed to run cl.exe with args: {CL_ARGS:?}"));
    if status.stdout.is_empty() {
        panic!("Output from cl.exe is empty");
    }
    let output_str = match std::str::from_utf8(&status.stdout) {
        Ok(s) => s,
        Err(e) => panic!(
            "Failed to parse output of cl.exe as utf-8:\nsrc{:?}\nerror:\n{e:?}",
            status.stdout
        ),
    };

    let version_str = output_str.trim();
    let current_version = version_str.parse::<usize>().expect(&format!(
        "Failed to parse version from output of cl.exe: {version_str}"
    ));
    if current_version < MINIMUM_MSVC_VERSION {
        panic!("Please upgrade the Visual Studio, the minimum supported version of MSVC is {MINIMUM_MSVC_VERSION}, which is correspond to [Visual Studio 2022 version 17.11], but the current version of MSVC is {current_version}\nCheck versions on https://learn.microsoft.com/en-us/cpp/overview/compiler-versions?view=msvc-170#version-macros");
    }
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
