mod archive;
mod lld;
mod platform;

pub use archive::{
    NativeArchive, create_native_multiversion_static_archive,
    create_native_profile_generation_static_archive, create_native_static_archive,
};
pub use lld::{
    NativeDynamicLibrary, NativeExecutable, link_native_dynamic_library, link_native_executable,
    link_native_multiversion_dynamic_library, link_native_multiversion_executable,
    link_native_profile_generation_dynamic_library, link_native_profile_generation_executable,
};
pub use platform::{NativeArtifactKind, NativeArtifactPaths, NativePlatform};
