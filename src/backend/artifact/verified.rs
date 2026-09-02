use super::{
    NativeArtifactKind, NativeDynamicLibrary, NativeExecutable, create_native_static_archive,
    link_native_dynamic_library, link_native_executable,
};
use crate::backend::llvm::{NativeError, NativeObject};

/// Role-tagged bytes produced by the one shared Native object/link pipeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedNativeBuild {
    kind: NativeArtifactKind,
    primary: Vec<u8>,
    header: Option<Vec<u8>>,
    import_library: Option<Vec<u8>>,
    object_graph: Vec<(String, Vec<u8>)>,
    link_recipe: Vec<String>,
}

impl VerifiedNativeBuild {
    #[must_use]
    pub const fn kind(&self) -> NativeArtifactKind {
        self.kind
    }

    #[must_use]
    pub fn primary(&self) -> &[u8] {
        &self.primary
    }

    #[must_use]
    pub fn header(&self) -> Option<&[u8]> {
        self.header.as_deref()
    }

    #[must_use]
    pub fn import_library(&self) -> Option<&[u8]> {
        self.import_library.as_deref()
    }

    #[must_use]
    pub fn object_graph(&self) -> &[(String, Vec<u8>)] {
        &self.object_graph
    }

    #[must_use]
    pub fn link_recipe(&self) -> &[String] {
        &self.link_recipe
    }
}

/// Packages a verified object using the same embedded linker/archive path for
/// ordinary output and offline tuning trials.
pub fn build_verified_native_artifact(
    kind: NativeArtifactKind,
    object: &NativeObject,
    export_names: &[String],
    header: Option<Vec<u8>>,
) -> Result<VerifiedNativeBuild, NativeError> {
    let object_name = if cfg!(target_os = "windows") {
        "module.obj"
    } else {
        "module.o"
    };
    let object_graph = vec![(object_name.to_string(), object.as_bytes().to_vec())];
    let (primary, import_library, recipe) = match kind {
        NativeArtifactKind::Executable => {
            let executable: NativeExecutable = link_native_executable(object)?;
            (
                executable.as_bytes().to_vec(),
                None,
                "embedded-lld-executable-v1",
            )
        }
        NativeArtifactKind::Dynamic => {
            let library: NativeDynamicLibrary = link_native_dynamic_library(object, export_names)?;
            (
                library.as_bytes().to_vec(),
                library.import_library().map(<[u8]>::to_vec),
                "embedded-lld-dynamic-v1",
            )
        }
        NativeArtifactKind::Static => (
            create_native_static_archive(object)?.as_bytes().to_vec(),
            None,
            "embedded-archive-static-v1",
        ),
        NativeArtifactKind::Object => (object.as_bytes().to_vec(), None, "native-object-v1"),
    };
    Ok(VerifiedNativeBuild {
        kind,
        primary,
        header,
        import_library,
        object_graph,
        link_recipe: vec![recipe.to_string()],
    })
}
