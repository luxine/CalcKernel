/// Version of the compiler-to-native-artifact ABI contract.
pub const NATIVE_ABI_VERSION: u32 = 1;

/// Version of the no-heap executable runtime ABI contract.
pub const RUNTIME_ABI_VERSION: u32 = 2;

/// Current private Native cache entry identity reported by every compiler build.
pub const NATIVE_CACHE_ENTRY_MAGIC: &str = "CKCOBJ04";
/// Current private Native cache-key schema.
pub const NATIVE_CACHE_KEY_SCHEMA: u32 = 5;
/// Current private Native cache-manifest schema.
pub const NATIVE_CACHE_MANIFEST_SCHEMA: u32 = 5;

/// One notice embedded into every `ckc` binary.
#[derive(Debug, Clone, Copy)]
pub struct EmbeddedNotice {
    /// Human-readable component name.
    pub name: &'static str,
    /// Exact upstream notice bytes.
    pub bytes: &'static [u8],
}

/// Returns every license/notice needed by the current embedded compiler.
#[must_use]
pub const fn embedded_notices() -> &'static [EmbeddedNotice] {
    &[
        EmbeddedNotice {
            name: "CalcKernel",
            bytes: include_bytes!("../../../LICENSE"),
        },
        EmbeddedNotice {
            name: "CalcKernel third-party component index",
            bytes: include_bytes!("../../../THIRD_PARTY_NOTICES.md"),
        },
        EmbeddedNotice {
            name: "CalcKernel Cargo provenance manifest",
            bytes: include_bytes!("../../../third_party/cargo/provenance.toml"),
        },
        EmbeddedNotice {
            name: "Rust Project copyright",
            bytes: include_bytes!("../../../third_party/licenses/RUST-COPYRIGHT"),
        },
        EmbeddedNotice {
            name: "Rust Project MIT license",
            bytes: include_bytes!("../../../third_party/licenses/RUST-LICENSE-MIT"),
        },
        EmbeddedNotice {
            name: "generic-array MIT license",
            bytes: include_bytes!("../../../third_party/licenses/generic-array-MIT.txt"),
        },
        EmbeddedNotice {
            name: "memchr MIT license",
            bytes: include_bytes!("../../../third_party/licenses/memchr-MIT.txt"),
        },
        EmbeddedNotice {
            name: "Unicode License V3",
            bytes: include_bytes!("../../../third_party/licenses/LICENSE-UNICODE"),
        },
        EmbeddedNotice {
            name: "LLVM Project 22.1.8 (LLVM and LLD)",
            bytes: include_bytes!("../../../native/llvm/LICENSE.TXT"),
        },
        EmbeddedNotice {
            name: "LLD 22.1.8",
            bytes: include_bytes!("../../../native/llvm/LLD-LICENSE.TXT"),
        },
        EmbeddedNotice {
            name: "LLVM Support BLAKE3",
            bytes: include_bytes!("../../../native/llvm/third-party/BLAKE3-LICENSE"),
        },
        EmbeddedNotice {
            name: "LLVM Support regex implementation",
            bytes: include_bytes!("../../../native/llvm/third-party/COPYRIGHT.regex"),
        },
        EmbeddedNotice {
            name: "Ryu floating-point conversion (Apache-2.0 option)",
            bytes: include_bytes!("../../../native/runtime/vendor/ryu/LICENSE-Apache2"),
        },
        EmbeddedNotice {
            name: "Ryu floating-point conversion (Boost-1.0 option)",
            bytes: include_bytes!("../../../native/runtime/vendor/ryu/LICENSE-Boost"),
        },
        EmbeddedNotice {
            name: "CK profile runtime provenance",
            bytes: include_bytes!("../../../native/profile_runtime/provenance.toml"),
        },
        EmbeddedNotice {
            name: "CK dispatch runtime provenance",
            bytes: include_bytes!("../../../native/dispatch_runtime/provenance.toml"),
        },
    ]
}
