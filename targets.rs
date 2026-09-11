//! The list of prebuilt `mach-dxcompiler` archives that [`build.rs`](../build.rs) can
//! download, along with their checksums.

/// A prebuilt archive this crate can download for one target triple and CRT linkage.
pub struct Target {
    pub name: &'static str,
    /// Whether this archive links the CRT statically. Only targets that publish more
    /// than one archive (currently just MSVC) select between builds using this;
    /// every other target's sole entry is used regardless of its value.
    pub static_crt: bool,
    /// SHA-256 of the archive.
    pub sha256: &'static str,
}

/// Every archive this crate can download, for the release named by `RELEASE_TAG` in
/// `build.rs`. Refresh these whenever that tag changes; for an immutable release the
/// digests can be copied from the `digest` field of the GitHub releases API.
pub const AVAILABLE_TARGETS: &[Target] = &[
    Target {
        name: "x86_64-linux-gnu",
        static_crt: false,
        sha256: "a1f3afc81b4806a248fa66639820d2f0834493fd487ffa621a2fee2ec7029fdf",
    },
    Target {
        name: "x86_64-linux-musl",
        static_crt: true,
        sha256: "a7e1e6e0c7834a62345c089bcc8884842688a669ffb5b7c932cf4702e56c54e0",
    },
    Target {
        name: "aarch64-linux-gnu",
        static_crt: false,
        sha256: "0f2a60cb362e6e274471c854d997872256d82d3e3cc416403499c875c6936b04",
    },
    Target {
        name: "aarch64-linux-musl",
        static_crt: true,
        sha256: "b040850fcab3d886d9cb46fddfa584659fda28b425cab63cd854ce3597f25ebd",
    },
    Target {
        name: "x86_64-windows-gnu",
        static_crt: false,
        sha256: "839a30779cfbd69fea6a65f76f2cc0d10e27bac1b4d54a2a18ec8c4c4f106101",
    },
    Target {
        name: "aarch64-windows-gnu",
        static_crt: false,
        sha256: "63d9940b6f839cf80ab6196bb75af531fdacc24169df874a7c20e1fce0a46fb9",
    },
    Target {
        name: "x86_64-windows-msvc",
        static_crt: true,
        sha256: "cc3ae4ede81cc0d9c212420c97d6c98a580acf116acbf1d62caed6f3fd1b1b16",
    },
    Target {
        name: "x86_64-windows-msvc",
        static_crt: false,
        sha256: "b5e1a7dd3c2d57e1ac27a357003b39d1c58ecdac3d548aca0ef4127f2833d2e1",
    },
    Target {
        name: "x86_64-macos-none",
        static_crt: false,
        sha256: "724a75552589e72d08a4dd752b930127b216b5451af5d86cad99a95e4df37240",
    },
    Target {
        name: "aarch64-macos-none",
        static_crt: false,
        sha256: "9fc8cc0b0bc855d0a67a782145145e90a5ce70a130a2ae87d05eafa151896f91",
    },
];
