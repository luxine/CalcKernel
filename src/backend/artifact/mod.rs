mod archive;
mod lld;
mod platform;

pub use archive::{NativeArchive, create_native_static_archive};
pub use lld::{
    NativeDynamicLibrary, NativeExecutable, link_native_dynamic_library, link_native_executable,
};
pub use platform::{NativeArtifactKind, NativeArtifactPaths, NativePlatform};
