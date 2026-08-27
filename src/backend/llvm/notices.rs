/// Version of the compiler-to-native-artifact ABI contract.
pub const NATIVE_ABI_VERSION: u32 = 1;

/// Version of the no-heap executable runtime ABI contract.
pub const RUNTIME_ABI_VERSION: u32 = 1;

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
    ]
}
