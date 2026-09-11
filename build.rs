//! Downloads and statically links the `mach-dxcompiler` C library.

use std::env;
use std::process::Command;
use std::{
    fs,
    io::ErrorKind::NotFound,
    path::{Path, PathBuf},
};

/// Owner of the GitHub repository hosting the prebuilt `mach-dxcompiler` releases.
const RELEASE_REPO_OWNER: &str = "DouglasDwyer";
/// Name of the GitHub repository hosting the prebuilt `mach-dxcompiler` releases.
const RELEASE_REPO_NAME: &str = "mach-dxcompiler";
/// Release tag the prebuilt binaries are downloaded from.
const RELEASE_TAG: &str = "2024.11.22+284d956.1";

/// A target triple this crate can download a prebuilt archive for, along with the
/// expected SHA-256 of each of its release archives. Refresh these whenever
/// [`RELEASE_TAG`] changes; for an immutable release they can be copied from the
/// `digest` field of the GitHub releases API.
struct Target {
    name: &'static str,
    /// SHA-256 of the statically-linked `..._ReleaseFast_lib.tar.gz` archive.
    lib_sha256: &'static str,
    /// SHA-256 of the `..._ReleaseFast_Dynamic_lib.tar.gz` archive, for MSVC targets
    /// that dynamically link the CRT.
    dynamic_lib_sha256: Option<&'static str>,
}

const AVAILABLE_TARGETS: &[Target] = &[
    Target {
        name: "x86_64-linux-gnu",
        lib_sha256: "a1f3afc81b4806a248fa66639820d2f0834493fd487ffa621a2fee2ec7029fdf",
        dynamic_lib_sha256: None,
    },
    Target {
        name: "x86_64-linux-musl",
        lib_sha256: "a7e1e6e0c7834a62345c089bcc8884842688a669ffb5b7c932cf4702e56c54e0",
        dynamic_lib_sha256: None,
    },
    Target {
        name: "aarch64-linux-gnu",
        lib_sha256: "0f2a60cb362e6e274471c854d997872256d82d3e3cc416403499c875c6936b04",
        dynamic_lib_sha256: None,
    },
    Target {
        name: "aarch64-linux-musl",
        lib_sha256: "b040850fcab3d886d9cb46fddfa584659fda28b425cab63cd854ce3597f25ebd",
        dynamic_lib_sha256: None,
    },
    Target {
        name: "x86_64-windows-gnu",
        lib_sha256: "839a30779cfbd69fea6a65f76f2cc0d10e27bac1b4d54a2a18ec8c4c4f106101",
        dynamic_lib_sha256: None,
    },
    Target {
        name: "aarch64-windows-gnu",
        lib_sha256: "63d9940b6f839cf80ab6196bb75af531fdacc24169df874a7c20e1fce0a46fb9",
        dynamic_lib_sha256: None,
    },
    Target {
        name: "x86_64-windows-msvc",
        lib_sha256: "cc3ae4ede81cc0d9c212420c97d6c98a580acf116acbf1d62caed6f3fd1b1b16",
        dynamic_lib_sha256: Some(
            "b5e1a7dd3c2d57e1ac27a357003b39d1c58ecdac3d548aca0ef4127f2833d2e1",
        ),
    },
    Target {
        name: "x86_64-macos-none",
        lib_sha256: "724a75552589e72d08a4dd752b930127b216b5451af5d86cad99a95e4df37240",
        dynamic_lib_sha256: None,
    },
    Target {
        name: "aarch64-macos-none",
        lib_sha256: "9fc8cc0b0bc855d0a67a782145145e90a5ce70a130a2ae87d05eafa151896f91",
        dynamic_lib_sha256: None,
    },
];

/// A prebuilt release archive to download and link.
struct ReleaseArtifact {
    /// URL of the `.tar.gz` archive.
    url: String,
    /// Expected SHA-256 of the archive.
    sha256: &'static str,
}

/// Downloads and links the static DXC binary.
fn main() {
    println!("cargo:rerun-if-changed=msvc_version.c");
    println!("cargo:rerun-if-changed=mach_dxc.h");
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

/// Verifies that [`RELEASE_TAG`] is an [immutable release], so its assets cannot be
/// swapped out after publication. Panics if the release is not immutable or its status
/// cannot be determined via the GitHub REST API.
///
/// [immutable release]: https://docs.github.com/en/code-security/concepts/supply-chain-security/immutable-releases
fn verify_release_is_immutable() {
    let api_url = format!(
        "https://api.github.com/repos/{RELEASE_REPO_OWNER}/{RELEASE_REPO_NAME}/releases/tags/{RELEASE_TAG}"
    );

    let output = Command::new("curl")
        .arg("--location")
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
            "Refusing to download a mutable GitHub release.\n\n\
             `{RELEASE_REPO_OWNER}/{RELEASE_REPO_NAME}` release `{RELEASE_TAG}` is not an immutable \
             release, so its assets could be replaced after publication without any change to this \
             crate's source. Publish an immutable release and bump `RELEASE_TAG` before building.\n\
             https://docs.github.com/en/code-security/concepts/supply-chain-security/immutable-releases"
        );
    }
}

/// Returns `true` if the release API JSON reports `"immutable": true`, scanning the
/// flat payload directly to avoid a JSON-parser build dependency.
fn release_json_is_immutable(body: &str) -> bool {
    let Some((_, after_key)) = body.split_once("\"immutable\"") else {
        return false;
    };
    let Some(after_colon) = after_key.trim_start().strip_prefix(':') else {
        return false;
    };
    after_colon.trim_start().starts_with("true")
}

/// Verifies that the downloaded archive's SHA-256 matches `expected`, pinning the
/// exact native-binary bytes that get linked. Panics if the archive cannot be read
/// or its digest does not match.
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
fn sha256_hex(data: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];

    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];

    let mut message = data.to_vec();
    let bit_len = (data.len() as u64) * 8;
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&bit_len.to_be_bytes());

    for chunk in message.chunks_exact(64) {
        let mut w = [0u32; 64];
        for (i, word) in w.iter_mut().take(16).enumerate() {
            *word = u32::from_be_bytes(chunk[4 * i..4 * i + 4].try_into().unwrap());
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }

        let (mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh) =
            (h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7]);

        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);

            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }

        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }

    let mut hex = String::with_capacity(64);
    for word in h {
        use std::fmt::Write;
        let _ = write!(hex, "{word:08x}");
    }
    hex
}

/// Gets the release archive from which the DXC binary should be downloaded.
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

    let target = AVAILABLE_TARGETS
        .iter()
        .find(|t| t.name == target_name)
        .unwrap_or_else(|| {
            panic!("Unsupported target: {target_name}\nCheck supported targets on {base_url}")
        });

    let (crt, sha256) = if abi == "msvc" && !static_crt {
        (
            "Dynamic_lib",
            target.dynamic_lib_sha256.unwrap_or_else(|| {
                panic!("No dynamically-linked CRT build is available for target {target_name}")
            }),
        )
    } else {
        ("lib", target.lib_sha256)
    };

    ReleaseArtifact {
        url: format!("{base_url}/download/{RELEASE_TAG}/{target_name}_ReleaseFast_{crt}.tar.gz"),
        sha256,
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
