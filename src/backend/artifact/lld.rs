use std::{
    fs,
    path::{Path, PathBuf},
    process,
    sync::atomic::{AtomicU64, Ordering},
};

use crate::backend::{
    llvm::{NativeError, NativeMultiversionObjectBundle, NativeObject, NativeStage, ffi},
    native_runtime::{
        embedded_profile_runtime_object, embedded_runtime_objects, embedded_windows_import_library,
    },
};

/// Validated outputs produced by the in-process, allowlisted LLD driver.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeDynamicLibrary {
    bytes: Vec<u8>,
    import_library: Option<Vec<u8>>,
}

/// Validated standalone executable bytes produced by embedded LLD.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeExecutable {
    bytes: Vec<u8>,
}

impl NativeExecutable {
    /// Returns the validated platform executable bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }
}

impl NativeDynamicLibrary {
    /// Returns the validated shared-library bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Returns the Windows import library, if this is a COFF host.
    #[must_use]
    pub fn import_library(&self) -> Option<&[u8]> {
        self.import_library.as_deref()
    }
}

/// Links one compiler-produced object using the embedded host LLD driver.
///
/// No path, object, library, response file, script, or raw linker flag is
/// accepted from the caller. The only variable linker data is the checked CK
/// export-name list.
pub fn link_native_dynamic_library(
    object: &NativeObject,
    export_names: &[String],
) -> Result<NativeDynamicLibrary, NativeError> {
    link_dynamic_library(object, export_names, false)
}

/// Links one closed multiversion object bundle without merging or reoptimizing
/// any independently audited module.
pub fn link_native_multiversion_dynamic_library(
    bundle: &NativeMultiversionObjectBundle,
    export_names: &[String],
) -> Result<NativeDynamicLibrary, NativeError> {
    bundle.validate()?;
    let inputs = bundle
        .objects()
        .iter()
        .map(|object| (object.name(), object.object().as_bytes()))
        .collect::<Vec<_>>();
    link_dynamic_library_inputs(&inputs, export_names, true)
}

/// Links a temporary generation library with the private collection runtime.
pub fn link_native_profile_generation_dynamic_library(
    object: &NativeObject,
    export_names: &[String],
) -> Result<NativeDynamicLibrary, NativeError> {
    link_dynamic_library(object, export_names, true)
}

fn link_dynamic_library(
    object: &NativeObject,
    export_names: &[String],
    profile_generation: bool,
) -> Result<NativeDynamicLibrary, NativeError> {
    let platform = super::NativePlatform::host();
    let module_name = match platform {
        super::NativePlatform::Windows => "module.obj",
        _ => "module.o",
    };
    let mut inputs = vec![(module_name, object.as_bytes())];
    if profile_generation {
        inputs.push(("profile-runtime.o", embedded_profile_runtime_object()));
    }
    link_dynamic_library_inputs(&inputs, export_names, profile_generation)
}

fn link_dynamic_library_inputs(
    inputs: &[(&str, &[u8])],
    export_names: &[String],
    needs_platform_input: bool,
) -> Result<NativeDynamicLibrary, NativeError> {
    validate_exports(export_names)?;
    let staging = LinkStaging::create()?;
    let platform = super::NativePlatform::host();
    let output_path = staging.path.join(match platform {
        super::NativePlatform::Linux => "module.so",
        super::NativePlatform::Darwin => "module.dylib",
        super::NativePlatform::Windows => "module.dll",
    });
    let import_path = staging.path.join("module.lib");
    let mut object_paths = Vec::with_capacity(inputs.len());
    for (name, bytes) in inputs {
        validate_staging_name(name)?;
        let path = staging.path.join(name);
        fs::write(&path, bytes).map_err(link_io_error)?;
        object_paths.push(path_text(&path)?.to_string());
    }
    let platform_input_path = stage_platform_input(&staging, platform, needs_platform_input)?;
    ffi::lld_link_shared(
        &object_paths,
        path_text(&output_path)?,
        if platform == super::NativePlatform::Windows {
            path_text(&import_path)?
        } else {
            "unused"
        },
        path_text(&platform_input_path)?,
        export_names,
    )?;
    let bytes = fs::read(&output_path).map_err(link_io_error)?;
    let import_library = if platform == super::NativePlatform::Windows {
        Some(fs::read(&import_path).map_err(link_io_error)?)
    } else {
        None
    };
    Ok(NativeDynamicLibrary {
        bytes,
        import_library,
    })
}

/// Links one verified entry-bearing program with the five embedded runtime
/// objects and the single allowlisted platform import description.
pub fn link_native_executable(object: &NativeObject) -> Result<NativeExecutable, NativeError> {
    link_executable(object, false)
}

/// Links one closed multiversion executable bundle plus the ordinary fixed CK
/// runtime object set.
pub fn link_native_multiversion_executable(
    bundle: &NativeMultiversionObjectBundle,
) -> Result<NativeExecutable, NativeError> {
    bundle.validate()?;
    let inputs = bundle
        .objects()
        .iter()
        .map(|object| (object.name(), object.object().as_bytes()))
        .collect::<Vec<_>>();
    link_executable_inputs(&inputs, false)
}

/// Links a temporary generation executable with the private collection runtime.
pub fn link_native_profile_generation_executable(
    object: &NativeObject,
) -> Result<NativeExecutable, NativeError> {
    link_executable(object, true)
}

fn link_executable(
    object: &NativeObject,
    profile_generation: bool,
) -> Result<NativeExecutable, NativeError> {
    let platform = super::NativePlatform::host();
    let object_extension = if platform == super::NativePlatform::Windows {
        "obj"
    } else {
        "o"
    };
    let module_name = format!("program.{object_extension}");
    let inputs = vec![(module_name.as_str(), object.as_bytes())];
    link_executable_inputs(&inputs, profile_generation)
}

fn link_executable_inputs(
    inputs: &[(&str, &[u8])],
    profile_generation: bool,
) -> Result<NativeExecutable, NativeError> {
    let staging = LinkStaging::create()?;
    let platform = super::NativePlatform::host();
    let object_extension = if platform == super::NativePlatform::Windows {
        "obj"
    } else {
        "o"
    };
    let mut object_paths = Vec::with_capacity(inputs.len() + 6);
    for (name, bytes) in inputs {
        validate_staging_name(name)?;
        let path = staging.path.join(name);
        fs::write(&path, bytes).map_err(link_io_error)?;
        object_paths.push(path_text(&path)?.to_string());
    }
    for (index, bytes) in embedded_runtime_objects().iter().enumerate() {
        let path = staging
            .path
            .join(format!("runtime-{index}.{object_extension}"));
        fs::write(&path, bytes).map_err(link_io_error)?;
        object_paths.push(path_text(&path)?.to_string());
    }
    if profile_generation {
        let path = staging
            .path
            .join(format!("profile-runtime.{object_extension}"));
        fs::write(&path, embedded_profile_runtime_object()).map_err(link_io_error)?;
        object_paths.push(path_text(&path)?.to_string());
    }
    let output_path = staging
        .path
        .join(if platform == super::NativePlatform::Windows {
            "program.exe"
        } else {
            "program"
        });
    let platform_input_path = stage_platform_input(&staging, platform, true)?;
    ffi::lld_link_executable(
        &object_paths,
        path_text(&output_path)?,
        path_text(&platform_input_path)?,
    )?;
    Ok(NativeExecutable {
        bytes: fs::read(output_path).map_err(link_io_error)?,
    })
}

fn validate_staging_name(name: &str) -> Result<(), NativeError> {
    if name.is_empty()
        || name.len() > 255
        || name.starts_with('.')
        || name.contains("..")
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(NativeError::new(
            NativeStage::Link,
            1,
            format!("invalid native named object `{name}`"),
        ));
    }
    Ok(())
}

fn stage_platform_input(
    staging: &LinkStaging,
    platform: super::NativePlatform,
    required: bool,
) -> Result<PathBuf, NativeError> {
    if !required {
        return Ok(staging.path.join("unused"));
    }
    match platform {
        super::NativePlatform::Darwin => {
            let path = staging.path.join("libSystem.tbd");
            fs::write(
                &path,
                include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/native/runtime/platform/libSystem.tbd"
                )),
            )
            .map_err(link_io_error)?;
            Ok(path)
        }
        super::NativePlatform::Windows => {
            let path = staging.path.join("kernel32.lib");
            fs::write(&path, embedded_windows_import_library()).map_err(link_io_error)?;
            Ok(path)
        }
        super::NativePlatform::Linux => Ok(staging.path.join("unused")),
    }
}

fn validate_exports(exports: &[String]) -> Result<(), NativeError> {
    if let Some(name) = exports.iter().find(|name| {
        name.is_empty()
            || !name.bytes().enumerate().all(|(index, byte)| {
                byte == b'_'
                    || byte.is_ascii_alphanumeric() && (index != 0 || !byte.is_ascii_digit())
            })
    }) {
        return Err(NativeError::new(
            NativeStage::Link,
            1,
            format!("invalid native export symbol `{name}`"),
        ));
    }
    Ok(())
}

fn path_text(path: &Path) -> Result<&str, NativeError> {
    path.to_str().ok_or_else(|| {
        NativeError::new(
            NativeStage::Link,
            1,
            format!(
                "native staging path is not valid Unicode: {}",
                path.display()
            ),
        )
    })
}

fn link_io_error(error: std::io::Error) -> NativeError {
    NativeError::new(NativeStage::Link, 3, error.to_string())
}

struct LinkStaging {
    path: PathBuf,
}

impl LinkStaging {
    fn create() -> Result<Self, NativeError> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir();
        for _ in 0..128 {
            let serial = NEXT.fetch_add(1, Ordering::Relaxed);
            let path = root.join(format!("ckc-lld-{}-{serial}", process::id()));
            match create_private_dir(&path) {
                Ok(()) => return Ok(Self { path }),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(link_io_error(error)),
            }
        }
        Err(NativeError::new(
            NativeStage::Link,
            3,
            "could not allocate a unique LLD staging directory".to_string(),
        ))
    }
}

impl Drop for LinkStaging {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[cfg(unix)]
fn create_private_dir(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    let mut builder = fs::DirBuilder::new();
    builder.mode(0o700).create(path)
}

#[cfg(not(unix))]
fn create_private_dir(path: &Path) -> std::io::Result<()> {
    fs::create_dir(path)
}
