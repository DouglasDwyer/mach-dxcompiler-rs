//! Downloads and statically links the `mach-dxcompiler` C library.

use std::env;
use std::process::Command;
use std::{
    fs,
    io::ErrorKind::NotFound,
    path::{Path, PathBuf},
};

/// The GitHub repository that hosts the prebuilt `mach-dxcompiler` releases.
const RELEASE_REPO_OWNER: &str = "DouglasDwyer";
/// The name of the repository that hosts the prebuilt `mach-dxcompiler` releases.
const RELEASE_REPO_NAME: &str = "mach-dxcompiler";
/// The release tag from which the prebuilt binaries are downloaded.
const RELEASE_TAG: &str = "2024.11.22+284d956.1";

/// Lowercase hex SHA-256 digests of the prebuilt release archives for [`RELEASE_TAG`].
///
/// Each key is a release asset name without the `.tar.gz` suffix (see
/// [`get_target_artifact`]); each value is the expected SHA-256 of that archive. The
/// downloaded archive is checked against this table before it is linked, so a build
/// fails loudly if a release asset is ever replaced with different bytes.
///
/// These MUST be refreshed whenever [`RELEASE_TAG`] changes. For an immutable release
/// GitHub publishes the digests itself, so they can be copied straight from the API:
///
/// ```text
/// gh api "repos/DouglasDwyer/mach-dxcompiler/releases/tags/<TAG>" \
///     --jq '.assets[] | select(.name | endswith(".tar.gz")) | [ (.name | rtrimstr(".tar.gz")), .digest ] | @tsv'
/// ```
///
/// (the API reports each digest as `sha256:<hex>`). Otherwise download each asset and
/// hash it locally with `sha256sum` / `shasum -a 256`.
#[cfg(feature = "verify_checksum")]
const RELEASE_CHECKSUMS: &[(&str, &str)] = &[
    (
        "x86_64-linux-gnu_ReleaseFast_lib",
        "a1f3afc81b4806a248fa66639820d2f0834493fd487ffa621a2fee2ec7029fdf",
    ),
    (
        "x86_64-linux-musl_ReleaseFast_lib",
        "a7e1e6e0c7834a62345c089bcc8884842688a669ffb5b7c932cf4702e56c54e0",
    ),
    (
        "aarch64-linux-gnu_ReleaseFast_lib",
        "0f2a60cb362e6e274471c854d997872256d82d3e3cc416403499c875c6936b04",
    ),
    (
        "aarch64-linux-musl_ReleaseFast_lib",
        "b040850fcab3d886d9cb46fddfa584659fda28b425cab63cd854ce3597f25ebd",
    ),
    (
        "x86_64-windows-gnu_ReleaseFast_lib",
        "839a30779cfbd69fea6a65f76f2cc0d10e27bac1b4d54a2a18ec8c4c4f106101",
    ),
    (
        "aarch64-windows-gnu_ReleaseFast_lib",
        "63d9940b6f839cf80ab6196bb75af531fdacc24169df874a7c20e1fce0a46fb9",
    ),
    (
        "x86_64-windows-msvc_ReleaseFast_lib",
        "cc3ae4ede81cc0d9c212420c97d6c98a580acf116acbf1d62caed6f3fd1b1b16",
    ),
    (
        "x86_64-windows-msvc_ReleaseFast_Dynamic_lib",
        "b5e1a7dd3c2d57e1ac27a357003b39d1c58ecdac3d548aca0ef4127f2833d2e1",
    ),
    (
        "x86_64-macos-none_ReleaseFast_lib",
        "724a75552589e72d08a4dd752b930127b216b5451af5d86cad99a95e4df37240",
    ),
    (
        "aarch64-macos-none_ReleaseFast_lib",
        "9fc8cc0b0bc855d0a67a782145145e90a5ce70a130a2ae87d05eafa151896f91",
    ),
];

/// A prebuilt release archive to download and link.
struct ReleaseArtifact {
    /// The URL the `.tar.gz` archive is downloaded from.
    url: String,
    /// The release asset name (without the `.tar.gz` suffix), used for diagnostics
    /// and to look the archive up in [`RELEASE_CHECKSUMS`].
    #[cfg_attr(not(feature = "verify_checksum"), allow(dead_code))]
    asset_name: String,
}

/// Downloads and links the static DXC binary.
fn main() {
    println!("cargo:rerun-if-changed=msvc_version.c");
    println!("cargo:rerun-if-changed=mach_dxc.h");
    println!("cargo:rerun-if-env-changed=GITHUB_TOKEN");
    println!("cargo:rerun-if-env-changed=GH_TOKEN");
    #[cfg(all(feature = "msvc_version_validation", target_env = "msvc"))]
    validate_msvc_version();
    #[cfg(feature = "verify_immutable_release")]
    verify_release_is_immutable(RELEASE_REPO_OWNER, RELEASE_REPO_NAME, RELEASE_TAG);
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("Failed to get OUT_DIR environment"));
    let artifact = get_target_artifact(static_crt());
    let file_path = out_dir.join("machdxcompiler.tar.gz");
    download_released_library(&artifact.url, &file_path);
    #[cfg(feature = "verify_checksum")]
    verify_checksum(&file_path, &artifact.asset_name);
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

/// Verifies that the GitHub release being downloaded is an [immutable release].
///
/// Immutable releases guarantee that the release assets and tag cannot be altered
/// after publication. Without this guarantee the prebuilt native binaries could be
/// swapped out at any time, letting downstream builds silently incorporate
/// attacker-controlled native code without any change to the Rust source or
/// `Cargo.lock`.
///
/// Panics if the release is not immutable, or if its immutability status cannot be
/// determined by querying the GitHub REST API.
///
/// [immutable release]: https://docs.github.com/en/code-security/concepts/supply-chain-security/immutable-releases
#[cfg(feature = "verify_immutable_release")]
fn verify_release_is_immutable(repo_owner: &str, repo_name: &str, tag: &str) {
    let api_url =
        format!("https://api.github.com/repos/{repo_owner}/{repo_name}/releases/tags/{tag}");

    let mut command = Command::new("curl");
    command
        .arg("--location")
        .arg("--fail")
        .arg("--silent")
        .arg("--show-error")
        .arg("--header")
        .arg("Accept: application/vnd.github+json")
        .arg("--header")
        .arg("X-GitHub-Api-Version: 2022-11-28")
        .arg("--user-agent")
        .arg("mach-dxcompiler-rs-build-script");

    // Authenticate when a token is available so CI builds are not blocked by the
    // low unauthenticated GitHub API rate limit.
    if let Some(token) = env::var("GITHUB_TOKEN")
        .or_else(|_| env::var("GH_TOKEN"))
        .ok()
        .filter(|token| !token.is_empty())
    {
        command
            .arg("--header")
            .arg(format!("Authorization: Bearer {token}"));
    }

    let output = command
        .arg(&api_url)
        .output()
        .expect("Failed to start Curl to query the GitHub releases API");
    if !output.status.success() {
        panic!(
            "Failed to query the GitHub releases API at {api_url} to verify that the \
             release is immutable:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let body = String::from_utf8(output.stdout)
        .expect("GitHub releases API returned a non-UTF-8 response");

    if !release_json_is_immutable(&body) {
        panic!(
            "Refusing to download a mutable GitHub release.\n\
             \n\
             The release `{tag}` of `{repo_owner}/{repo_name}` is not an immutable release, so its \
             assets could be replaced after publication without any change to this crate's source \
             (https://docs.github.com/en/code-security/concepts/supply-chain-security/immutable-releases).\n\
             \n\
             A new immutable release must be published and referenced by this crate before it can \
             be built. To bypass this check at your own risk, disable the default \
             `verify_immutable_release` feature of `mach-dxcompiler-rs`."
        );
    }
}

/// Returns `true` if the GitHub release API JSON body reports `"immutable": true`.
///
/// The release payload is a flat JSON object, so this scans for the field directly
/// rather than pulling a JSON parser into the build dependencies.
#[cfg(feature = "verify_immutable_release")]
fn release_json_is_immutable(body: &str) -> bool {
    let Some((_, after_key)) = body.split_once("\"immutable\"") else {
        return false;
    };
    let Some(after_colon) = after_key.trim_start().strip_prefix(':') else {
        return false;
    };
    after_colon.trim_start().starts_with("true")
}

/// Verifies that the downloaded archive matches the SHA-256 digest pinned in
/// [`RELEASE_CHECKSUMS`].
///
/// This pins the exact bytes of the prebuilt native library, so downstream builds
/// cannot silently pick up a modified binary without a corresponding change to this
/// crate's source.
///
/// Panics if no checksum is pinned for `asset_name`, if the archive cannot be read,
/// or if its digest does not match.
#[cfg(feature = "verify_checksum")]
fn verify_checksum(file_path: &Path, asset_name: &str) {
    use sha2::{Digest, Sha256};
    use std::fmt::Write;

    let expected = RELEASE_CHECKSUMS
        .iter()
        .find_map(|(name, sha)| (*name == asset_name).then_some(*sha))
        .unwrap_or_else(|| {
            panic!(
                "No SHA-256 checksum is pinned in build.rs for release asset \
                 `{asset_name}.tar.gz`.\nAdd its digest to `RELEASE_CHECKSUMS`."
            )
        });

    let bytes = fs::read(file_path).unwrap_or_else(|e| {
        panic!(
            "Failed to read downloaded archive {} for checksum verification: {e}",
            file_path.display()
        )
    });

    let mut actual = String::with_capacity(64);
    for byte in Sha256::digest(&bytes) {
        let _ = write!(actual, "{byte:02x}");
    }

    if !actual.eq_ignore_ascii_case(expected) {
        // Don't leave the untrusted archive lying around for the extract step.
        let _ = fs::remove_file(file_path);
        panic!(
            "Checksum mismatch for release asset `{asset_name}.tar.gz`.\n\
             \n\
             expected SHA-256: {expected}\n\
             actual SHA-256:   {actual}\n\
             \n\
             The downloaded archive does not match the checksum pinned in build.rs. The \
             release asset may have been tampered with, or `RELEASE_CHECKSUMS` may be stale \
             for release `{RELEASE_TAG}`. Refusing to link an unverified binary."
        );
    }
}

/// Gets the release archive from which the DXC binary should be downloaded.
fn get_target_artifact(static_crt: bool) -> ReleaseArtifact {
    let base_url = format!("https://github.com/{RELEASE_REPO_OWNER}/{RELEASE_REPO_NAME}/releases");
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
        panic!("Unsupported target: {target}\nCheck supported targets on {base_url}");
    }
    let crt = if abi == "msvc" && !static_crt {
        "Dynamic_lib"
    } else {
        "lib"
    };
    let asset_name = format!("{target}_ReleaseFast_{crt}");
    ReleaseArtifact {
        url: format!("{base_url}/download/{RELEASE_TAG}/{asset_name}.tar.gz"),
        asset_name,
    }
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
