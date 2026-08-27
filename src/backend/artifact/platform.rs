use std::path::{Path, PathBuf};

/// Host artifact formats supported by the native toolchain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativePlatform {
    Linux,
    Darwin,
    Windows,
}

impl NativePlatform {
    /// Returns the platform selected when this compiler binary was built.
    #[must_use]
    pub const fn host() -> Self {
        if cfg!(target_os = "windows") {
            Self::Windows
        } else if cfg!(target_os = "macos") {
            Self::Darwin
        } else {
            Self::Linux
        }
    }
}

/// Native artifact selected by `ckc build --kind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeArtifactKind {
    Executable,
    Dynamic,
    Static,
    Object,
}

/// Complete platform-specific output set for one native artifact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeArtifactPaths {
    pub primary: PathBuf,
    pub header: Option<PathBuf>,
    pub import_library: Option<PathBuf>,
}

impl NativeArtifactPaths {
    /// Derives exact platform suffixes without duplicating an existing suffix.
    #[must_use]
    pub fn new(platform: NativePlatform, kind: NativeArtifactKind, base: &Path) -> Self {
        let extension = match (platform, kind) {
            (NativePlatform::Windows, NativeArtifactKind::Executable) => Some("exe"),
            (NativePlatform::Linux, NativeArtifactKind::Dynamic) => Some("so"),
            (NativePlatform::Darwin, NativeArtifactKind::Dynamic) => Some("dylib"),
            (NativePlatform::Windows, NativeArtifactKind::Dynamic) => Some("dll"),
            (NativePlatform::Linux | NativePlatform::Darwin, NativeArtifactKind::Static) => {
                Some("a")
            }
            (NativePlatform::Windows, NativeArtifactKind::Static) => Some("lib"),
            (NativePlatform::Linux | NativePlatform::Darwin, NativeArtifactKind::Object) => {
                Some("o")
            }
            (NativePlatform::Windows, NativeArtifactKind::Object) => Some("obj"),
            (_, NativeArtifactKind::Executable) => None,
        };
        let primary_base = if platform == NativePlatform::Windows
            && matches!(
                kind,
                NativeArtifactKind::Dynamic | NativeArtifactKind::Static
            ) {
            strip_windows_lib_prefix(base)
        } else {
            base.to_path_buf()
        };
        let primary = with_exact_extension(&primary_base, extension);
        let artifact_base = strip_known_artifact_extension(base);
        let header = (!matches!(kind, NativeArtifactKind::Executable))
            .then(|| artifact_base.with_extension("h"));
        let import_library = (platform == NativePlatform::Windows
            && kind == NativeArtifactKind::Dynamic)
            .then(|| artifact_base.with_extension("lib"));
        Self {
            primary,
            header,
            import_library,
        }
    }
}

fn strip_windows_lib_prefix(path: &Path) -> PathBuf {
    let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
        return path.to_path_buf();
    };
    let Some(stripped) = name.strip_prefix("lib").filter(|value| !value.is_empty()) else {
        return path.to_path_buf();
    };
    path.with_file_name(stripped)
}

fn with_exact_extension(base: &Path, extension: Option<&str>) -> PathBuf {
    let Some(extension) = extension else {
        return base.to_path_buf();
    };
    if base.extension().and_then(|value| value.to_str()) == Some(extension) {
        base.to_path_buf()
    } else {
        base.with_extension(extension)
    }
}

fn strip_known_artifact_extension(path: &Path) -> PathBuf {
    match path.extension().and_then(|value| value.to_str()) {
        Some("so" | "dylib" | "dll" | "a" | "lib" | "o" | "obj" | "exe") => path.with_extension(""),
        _ => path.to_path_buf(),
    }
}
