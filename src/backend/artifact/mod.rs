mod archive;
mod lld;
mod platform;

pub use archive::{NativeArchive, create_native_static_archive};
pub use lld::{NativeDynamicLibrary, link_native_dynamic_library};
pub use platform::{NativeArtifactKind, NativeArtifactPaths, NativePlatform};
