use std::collections::BTreeSet;

use sha2::{Digest, Sha256};

/// Native artifact kinds accepted by schema-1 offline tuning.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum TuneArtifactKind {
    Executable = 1,
    Dynamic = 2,
}

/// Closed output roles whose actual bytes are frozen into trial identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum TuneArtifactRole {
    Primary = 1,
    Header = 2,
    ImportLibrary = 3,
}

/// Digest and exact size of one role-tagged artifact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TuneArtifactRoleIdentity {
    pub role: TuneArtifactRole,
    pub size: u64,
    pub digest: [u8; 32],
}

/// Destination-independent identity of one compiler/linker result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactIdentity {
    pub kind: TuneArtifactKind,
    pub roles: Vec<TuneArtifactRoleIdentity>,
    pub object_graph_digest: [u8; 32],
    pub link_recipe_digest: [u8; 32],
    pub chosen_code_digest: [u8; 32],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TuneArtifactBytes {
    pub primary: Vec<u8>,
    pub header: Option<Vec<u8>>,
    pub import_library: Option<Vec<u8>>,
}

pub(crate) fn derive_artifact_identity(
    kind: TuneArtifactKind,
    bytes: &TuneArtifactBytes,
    object_graph: &[(String, Vec<u8>)],
    link_recipe: &[String],
) -> Result<ArtifactIdentity, String> {
    if bytes.primary.is_empty() || bytes.primary.len() > 512 * 1024 * 1024 {
        return Err("tuning primary artifact size is outside schema-1 bounds".to_string());
    }
    if object_graph.is_empty() || object_graph.len() > 64 || link_recipe.len() > 128 {
        return Err("tuning object graph or link recipe exceeds schema-1 bounds".to_string());
    }
    let mut names = BTreeSet::new();
    for (name, object) in object_graph {
        if name.is_empty()
            || name.len() > 255
            || object.is_empty()
            || object.len() > 256 * 1024 * 1024
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
            || !names.insert(name)
        {
            return Err("invalid tuning object graph".to_string());
        }
    }
    if link_recipe
        .iter()
        .any(|item| item.is_empty() || item.len() > 4_096 || item.contains('\0'))
    {
        return Err("invalid tuning link recipe".to_string());
    }

    let mut roles = vec![role_identity(TuneArtifactRole::Primary, &bytes.primary)?];
    if let Some(header) = &bytes.header {
        roles.push(role_identity(TuneArtifactRole::Header, header)?);
    }
    if let Some(import) = &bytes.import_library {
        roles.push(role_identity(TuneArtifactRole::ImportLibrary, import)?);
    }

    let mut object_hasher = Sha256::new();
    object_hasher.update(b"CK-TUNE-OBJECT-GRAPH\0");
    object_hasher.update(
        u32::try_from(object_graph.len())
            .map_err(|_| "object count")?
            .to_be_bytes(),
    );
    for (name, object) in object_graph {
        hash_text(&mut object_hasher, name)?;
        object_hasher.update(
            u64::try_from(object.len())
                .map_err(|_| "object size")?
                .to_be_bytes(),
        );
        object_hasher.update(Sha256::digest(object));
    }
    let object_graph_digest = object_hasher.finalize().into();

    let mut recipe_hasher = Sha256::new();
    recipe_hasher.update(b"CK-TUNE-LINK-RECIPE\0");
    recipe_hasher.update(
        u32::try_from(link_recipe.len())
            .map_err(|_| "recipe count")?
            .to_be_bytes(),
    );
    for item in link_recipe {
        hash_text(&mut recipe_hasher, item)?;
    }
    let link_recipe_digest = recipe_hasher.finalize().into();

    let mut chosen_hasher = Sha256::new();
    chosen_hasher.update(b"CK-TUNE-CHOSEN-CODE\0");
    chosen_hasher.update([kind as u8]);
    chosen_hasher.update(object_graph_digest);
    chosen_hasher.update(link_recipe_digest);
    for role in &roles {
        chosen_hasher.update([role.role as u8]);
        chosen_hasher.update(role.size.to_be_bytes());
        chosen_hasher.update(role.digest);
    }
    Ok(ArtifactIdentity {
        kind,
        roles,
        object_graph_digest,
        link_recipe_digest,
        chosen_code_digest: chosen_hasher.finalize().into(),
    })
}

fn role_identity(role: TuneArtifactRole, bytes: &[u8]) -> Result<TuneArtifactRoleIdentity, String> {
    Ok(TuneArtifactRoleIdentity {
        role,
        size: u64::try_from(bytes.len()).map_err(|_| "artifact role size overflow")?,
        digest: Sha256::digest(bytes).into(),
    })
}

fn hash_text(hasher: &mut Sha256, value: &str) -> Result<(), String> {
    hasher.update(
        u32::try_from(value.len())
            .map_err(|_| "text length overflow")?
            .to_be_bytes(),
    );
    hasher.update(value.as_bytes());
    Ok(())
}
