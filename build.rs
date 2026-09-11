//! Downloads and statically links the `mach-dxcompiler` C library.

/// The list of prebuilt targets this crate can download and their checksums.
#[cfg(not(feature = "update_targets"))]
mod targets;

use std::fs;
use std::process::Command;
#[cfg(not(feature = "update_targets"))]
use std::{
    env,
    io::ErrorKind::NotFound,
    path::{Path, PathBuf},
};
#[cfg(not(feature = "update_targets"))]
use targets::{AVAILABLE_TARGETS, RELEASE_TAG};

/// Owner of the GitHub repository hosting the prebuilt `mach-dxcompiler` releases.
const RELEASE_REPO_OWNER: &str = "DouglasDwyer";
/// Name of the GitHub repository hosting the prebuilt `mach-dxcompiler` releases.
const RELEASE_REPO_NAME: &str = "mach-dxcompiler";

/// A prebuilt release archive to download and link.
#[cfg(not(feature = "update_targets"))]
struct ReleaseArtifact {
    /// URL of the `.tar.gz` archive.
    pub url: String,
    /// Expected SHA-256 of the archive.
    pub sha256: &'static str,
}

/// Downloads and links the static DXC binary.
#[cfg(not(feature = "update_targets"))]
fn main() {
    println!("cargo:rerun-if-changed=msvc_version.c");
    println!("cargo:rerun-if-changed=mach_dxc.h");
    println!("cargo:rerun-if-changed=targets.rs");
    #[cfg(all(feature = "msvc_version_validation", target_env = "msvc"))]
    validate_msvc_version();
    verify_release_is_immutable();
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("Failed to get OUT_DIR environment"));
    let artifact = get_target_artifact(static_crt());
    let file_path = out_dir.join("machdxcompiler.tar.gz");
    download_released_library(&artifact.url, &file_path);
    verify_checksum(&file_path, artifact.sha256);
    extract_tar_gz(&file_path, &out_dir);
    #[cfg(feature = "cbindings")]
    generate_bindings();
    println!("cargo:rustc-link-lib=static=machdxcompiler");
    println!("cargo:rustc-link-search=native={}", out_dir.display());
}

/// Regenerates `targets.rs` instead of building, since the two are mutually exclusive:
/// there's no library to link until `targets.rs` reflects a real, immutable release.
#[cfg(feature = "update_targets")]
fn main() {
    update_targets();
}

/// Generates C API bindings.
#[cfg(all(feature = "cbindings", not(feature = "update_targets")))]
fn generate_bindings() {
    let bindings = bindgen::Builder::default()
        .rust_target(bindgen::RustTarget::Stable_1_73)
        .header("mach_dxc.h")
        .generate()
        .expect("Unable to generate bindings");

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
#[cfg(all(
    feature = "msvc_version_validation",
    target_env = "msvc",
    not(feature = "update_targets")
))]
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

/// Fetches the JSON body of `GET /repos/{owner}/{repo}/releases/{path}` from the
/// GitHub REST API. Panics if the request fails or the response isn't valid UTF-8.
fn fetch_release_json(path: &str) -> String {
    let api_url = format!(
        "https://api.github.com/repos/{RELEASE_REPO_OWNER}/{RELEASE_REPO_NAME}/releases/{path}"
    );

    let output = Command::new("curl")
        .arg("--location")
        .arg(&api_url)
        .output()
        .expect("Failed to start Curl to query the GitHub releases API");
    if !output.status.success() {
        panic!(
            "Failed to query the GitHub releases API at {api_url}:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    String::from_utf8(output.stdout).expect("GitHub releases API returned a non-UTF-8 response")
}

/// Verifies that [`RELEASE_TAG`] is an [immutable release], so its assets cannot be
/// swapped out after publication. Panics if the release is not immutable or its status
/// cannot be determined via the GitHub REST API.
///
/// [immutable release]: https://docs.github.com/en/code-security/concepts/supply-chain-security/immutable-releases
#[cfg(not(feature = "update_targets"))]
fn verify_release_is_immutable() {
    let body = fetch_release_json(&format!("tags/{RELEASE_TAG}"));

    if !release_json_is_immutable(&body) {
        panic!(
            "Refusing to download a mutable GitHub release.\n\n\
             `{RELEASE_REPO_OWNER}/{RELEASE_REPO_NAME}` release `{RELEASE_TAG}` is not an immutable \
             release, so its assets could be replaced after publication without any change to this \
             crate's source. Publish an immutable release and bump `RELEASE_TAG` before building.\n\
             https://docs.github.com/en/code-security/concepts/supply-chain-security/immutable-releases"
        );
    }
}

/// Returns `true` if the release API JSON reports `"immutable": true`.
fn release_json_is_immutable(body: &str) -> bool {
    json_field(body, "immutable").is_some_and(|value| value.starts_with("true"))
}

/// Returns the JSON text immediately following a top-level `"key":`, with leading
/// whitespace trimmed, scanning the payload directly to avoid a JSON-parser build
/// dependency.
fn json_field<'a>(json: &'a str, key: &str) -> Option<&'a str> {
    let (_, after_key) = json.split_once(&format!("\"{key}\""))?;
    Some(after_key.trim_start().strip_prefix(':')?.trim_start())
}

/// Regenerates `targets.rs` from the latest GitHub release and stops the build.
///
/// Fetches the repository's latest release, verifies it is immutable, reads each
/// `*_ReleaseFast_{lib,Dynamic_lib}.tar.gz` asset's SHA-256 straight from the
/// GitHub-computed `digest` field, and rewrites `targets.rs` to match. Never returns:
/// it panics either way, since there's nothing valid left to link.
#[cfg(feature = "update_targets")]
fn update_targets() -> ! {
    let body = fetch_release_json("latest");

    if !release_json_is_immutable(&body) {
        panic!(
            "Refusing to pin a mutable GitHub release.\n\n\
             The latest release of `{RELEASE_REPO_OWNER}/{RELEASE_REPO_NAME}` is not an \
             immutable release, so its assets could be replaced after publication. Wait for an \
             immutable release before regenerating targets.rs.\n\
             https://docs.github.com/en/code-security/concepts/supply-chain-security/immutable-releases"
        );
    }

    let tag = json_string_field(&body, "tag_name").expect("Release JSON had no `tag_name` field");
    let assets_body =
        json_array_field(&body, "assets").expect("Release JSON had no `assets` array");

    let mut targets: Vec<(&str, bool, &str)> = json_objects(assets_body)
        .into_iter()
        .filter_map(|asset| {
            let name = json_string_field(asset, "name")?;
            let (target_name, crt) = name.strip_suffix(".tar.gz")?.split_once("_ReleaseFast_")?;
            let static_crt = match crt {
                "lib" => true,
                "Dynamic_lib" => false,
                _ => return None,
            };
            let digest = json_string_field(asset, "digest")
                .unwrap_or_else(|| panic!("Asset `{name}` has no digest"));
            let sha256 = digest
                .strip_prefix("sha256:")
                .unwrap_or_else(|| panic!("Asset `{name}` has a non-SHA-256 digest: {digest}"));
            Some((target_name, static_crt, sha256))
        })
        .collect();
    targets.sort_by_key(|&(name, static_crt, _)| (name, std::cmp::Reverse(static_crt)));

    fs::write("targets.rs", render_targets_file(tag, &targets))
        .expect("Failed to write targets.rs");
    let _ = Command::new("rustfmt").arg("targets.rs").status();

    panic!(
        "Updated targets.rs for release `{tag}` with {} target(s). This always \"fails\" the \
         build; re-run `cargo build` without `--features update_targets` to build normally.",
        targets.len()
    );
}

/// Renders the contents of `targets.rs` for the given release tag and target list.
/// Formatting doesn't matter here: [`update_targets`] runs `rustfmt` on the result.
#[cfg(feature = "update_targets")]
fn render_targets_file(tag: &str, targets: &[(&str, bool, &str)]) -> String {
    use std::fmt::Write;

    let mut out = String::from(
        "//! The list of prebuilt `mach-dxcompiler` archives that [`build.rs`](../build.rs) can\n\
         //! download, along with their checksums.\n\
         //!\n\
         //! Regenerate this file with `cargo build --features update_targets` rather than\n\
         //! editing it by hand.\n\n\
         /// A prebuilt archive this crate can download for one target triple and CRT linkage.\n\
         pub struct Target {\n\
         /// Target triple, e.g. `\"x86_64-linux-gnu\"`.\n\
         pub name: &'static str,\n\
         /// Whether this archive links the CRT statically. Only targets that publish more\n\
         /// than one archive (currently just MSVC) select between builds using this;\n\
         /// every other target's sole entry is used regardless of its value.\n\
         pub static_crt: bool,\n\
         /// SHA-256 of the archive.\n\
         pub sha256: &'static str,\n\
         }\n\n\
         /// Release tag the prebuilt binaries are downloaded from.\n",
    );
    let _ = writeln!(out, "pub const RELEASE_TAG: &str = \"{tag}\";\n");
    out.push_str(
        "/// Every archive this crate can download, for [`RELEASE_TAG`].\n\
         pub const AVAILABLE_TARGETS: &[Target] = &[\n",
    );
    for (name, static_crt, sha256) in targets {
        let _ = writeln!(
            out,
            "Target {{ name: \"{name}\", static_crt: {static_crt}, sha256: \"{sha256}\" }},"
        );
    }
    out.push_str("];\n");
    out
}

/// Reads a top-level `"key": "<string>"` field out of a JSON object.
#[cfg(feature = "update_targets")]
fn json_string_field<'a>(json: &'a str, key: &str) -> Option<&'a str> {
    let rest = json_field(json, key)?.strip_prefix('"')?;
    Some(&rest[..rest.find('"')?])
}

/// Reads a top-level `"key": [...]` field out of a JSON object, returning the raw text
/// between the array's brackets.
#[cfg(feature = "update_targets")]
fn json_array_field<'a>(json: &'a str, key: &str) -> Option<&'a str> {
    let body = json_field(json, key)?.strip_prefix('[')?;
    Some(&body[..find_closing_bracket(body)?])
}

/// Splits the body of a JSON array into the raw text of each top-level `{...}` object.
#[cfg(feature = "update_targets")]
fn json_objects(array_body: &str) -> Vec<&str> {
    let mut objects = Vec::new();
    let mut rest = array_body;
    while let Some(start) = rest.find('{') {
        let body = &rest[start + 1..];
        let Some(end) = find_closing_bracket(body) else {
            break;
        };
        objects.push(&body[..end]);
        rest = &body[end + 1..];
    }
    objects
}

/// Given the text just after an opening `{` or `[`, returns the index of the matching
/// closing bracket, correctly skipping over nested brackets and quoted strings.
#[cfg(feature = "update_targets")]
fn find_closing_bracket(s: &str) -> Option<usize> {
    let mut depth = 0u32;
    let mut in_string = false;
    let mut escaped = false;
    for (i, c) in s.char_indices() {
        if in_string {
            match c {
                '\\' if !escaped => escaped = true,
                '"' if !escaped => in_string = false,
                _ => escaped = false,
            }
            continue;
        }
        match c {
            '"' => in_string = true,
            '[' | '{' => depth += 1,
            ']' | '}' if depth == 0 => return Some(i),
            ']' | '}' => depth -= 1,
            _ => {}
        }
    }
    None
}

/// Verifies that the downloaded archive's SHA-256 matches `expected`, pinning the
/// exact native-binary bytes that get linked. Panics if the archive cannot be read
/// or its digest does not match.
#[cfg(not(feature = "update_targets"))]
fn verify_checksum(file_path: &Path, expected: &str) {
    let bytes = fs::read(file_path).unwrap_or_else(|e| {
        panic!(
            "Failed to read downloaded archive {} for checksum verification: {e}",
            file_path.display()
        )
    });
    let actual = sha256_hex(&bytes);

    if !actual.eq_ignore_ascii_case(expected) {
        let _ = fs::remove_file(file_path);
        panic!(
            "Checksum mismatch for downloaded archive {}.\n\
             \n\
             expected SHA-256: {expected}\n\
             actual SHA-256:   {actual}\n\
             \n\
             The downloaded archive does not match the checksum pinned in build.rs. The \
             release asset may have been tampered with, or `AVAILABLE_TARGETS` may be stale \
             for release `{RELEASE_TAG}`. Refusing to link an unverified binary.",
            file_path.display()
        );
    }
}

/// Computes the SHA-256 digest of `data`, returned as lowercase hex.
#[cfg(not(feature = "update_targets"))]
fn sha256_hex(data: &[u8]) -> String {
    use std::fmt::Write;

    let mut hex = String::with_capacity(64);
    for byte in lhash::sha256(data) {
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}

/// Gets the release archive from which the DXC binary should be downloaded. Prefers a
/// target whose CRT linkage matches `static_crt`, falling back to whatever's published
/// for that name; only targets with more than one published archive (currently just
/// MSVC) name a dynamically-linked build differently.
#[cfg(not(feature = "update_targets"))]
fn get_target_artifact(static_crt: bool) -> ReleaseArtifact {
    let base_url = format!("https://github.com/{RELEASE_REPO_OWNER}/{RELEASE_REPO_NAME}/releases");
    let arch = env::var("CARGO_CFG_TARGET_ARCH").expect("Failed to get architecture");
    let mut os = env::var("CARGO_CFG_TARGET_OS").expect("Failed to get os");
    if env::var("CARGO_CFG_TARGET_VENDOR").unwrap_or_default() == "apple" && os == "darwin" {
        os = "macos".to_owned();
    }
    let mut abi = env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    if abi.is_empty() {
        abi = "none".to_owned();
    }
    let target_name = format!("{arch}-{os}-{abi}");

    let candidates: Vec<_> = AVAILABLE_TARGETS
        .iter()
        .filter(|t| t.name == target_name)
        .collect();
    let target = candidates
        .iter()
        .copied()
        .max_by_key(|t| t.static_crt == static_crt)
        .unwrap_or_else(|| {
            panic!("Unsupported target: {target_name}\nCheck supported targets on {base_url}")
        });

    let crt = if candidates.len() > 1 && !target.static_crt {
        "Dynamic_lib"
    } else {
        "lib"
    };

    ReleaseArtifact {
        url: format!("{base_url}/download/{RELEASE_TAG}/{target_name}_ReleaseFast_{crt}.tar.gz"),
        sha256: target.sha256,
    }
}

/// Downloads the provided URL to a file.
#[cfg(not(feature = "update_targets"))]
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
#[cfg(not(feature = "update_targets"))]
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
#[cfg(not(feature = "update_targets"))]
fn static_crt() -> bool {
    env::var("CARGO_ENCODED_RUSTFLAGS")
        .map(|flags| flags.contains("target-feature=+crt-static"))
        .unwrap_or(false)
}
