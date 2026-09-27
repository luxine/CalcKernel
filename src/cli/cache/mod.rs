mod entry;
mod evict;
mod key;
mod path;
mod store;

use std::fs;

use calckernel::{
    KirMultiversionBundle, NativeMultiversionObjectBundle, NativeObject, NativeTarget,
};
use sha2::{Digest, Sha256};

pub(super) use entry::CacheManifest;
pub(super) use key::{CacheKeyInput, cache_key_hex};
use path::resolve_cache_root;
use store::CacheStore;

pub(super) fn load_object(
    target: &NativeTarget,
    manifest: &CacheManifest,
    bypass: bool,
) -> Option<NativeObject> {
    if bypass {
        return None;
    }
    let store = CacheStore::open_at(resolve_cache_root()?).ok()?;
    let (actual, bytes) = store.read_entry(&manifest.key)?;
    if actual != *manifest {
        return None;
    }
    target.parse_cached_object(&bytes).ok()
}

pub(super) fn store_object(manifest: &CacheManifest, object: &NativeObject, bypass: bool) {
    if bypass {
        return;
    }
    let Some(root) = resolve_cache_root() else {
        return;
    };
    let Ok(store) = CacheStore::open_at(root) else {
        return;
    };
    let _ = store.write(manifest, object.as_bytes());
}

pub(super) fn load_multiversion_bundle(
    target: &NativeTarget,
    manifest: &CacheManifest,
    planned: &KirMultiversionBundle,
    bypass: bool,
) -> Option<NativeMultiversionObjectBundle> {
    if bypass {
        return None;
    }
    let store = CacheStore::open_at(resolve_cache_root()?).ok()?;
    let (actual_manifest, bytes) = store.read_entry(&manifest.key)?;
    if actual_manifest != *manifest {
        return None;
    }
    let index = entry::decode_bundle_index(&bytes).ok()?;
    if index.target_set_digest != planned.target_set.digest
        || index.dispatch_runtime_digest
            != <[u8; 32]>::from(Sha256::digest(
                calckernel::embedded_dispatch_runtime_object(),
            ))
    {
        return None;
    }
    let expected_layout = NativeMultiversionObjectBundle::expected_layout(planned);
    if index.objects.len() != expected_layout.len()
        || index
            .objects
            .iter()
            .zip(&expected_layout)
            .any(|(actual, expected)| actual.name != expected.0 || actual.role != expected.1)
    {
        return None;
    }
    let mut objects = Vec::with_capacity(index.objects.len());
    for reference in index.objects {
        let object_key = bundle_object_key(&manifest.key, &reference.digest);
        let (object_manifest, object) = store.read_entry(&object_key)?;
        let mut expected_manifest = manifest.clone();
        expected_manifest.key = object_key;
        if object_manifest != expected_manifest
            || <[u8; 32]>::from(Sha256::digest(&object)) != reference.digest
        {
            return None;
        }
        objects.push((reference.name, reference.role, object));
    }
    NativeMultiversionObjectBundle::from_cached_objects(
        target,
        index.target_set_digest,
        index.dispatch_runtime_digest,
        objects,
    )
    .ok()
}

pub(super) fn store_multiversion_bundle(
    manifest: &CacheManifest,
    bundle: &NativeMultiversionObjectBundle,
    bypass: bool,
) {
    if bypass {
        return;
    }
    let Some(root) = resolve_cache_root() else {
        return;
    };
    let Ok(store) = CacheStore::open_at(root) else {
        return;
    };
    for object in bundle.objects() {
        let mut object_manifest = manifest.clone();
        object_manifest.key = bundle_object_key(&manifest.key, object.digest());
        if store
            .write(&object_manifest, object.object().as_bytes())
            .is_err()
        {
            return;
        }
    }
    let index = entry::bundle_index(bundle);
    let Ok(bytes) = entry::encode_bundle_index(&index) else {
        return;
    };
    let _ = store.write(manifest, &bytes);
}

fn bundle_object_key(bundle_key: &str, digest: &[u8; 32]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"CKC-CACHE-BUNDLE-OBJECT\0");
    hasher.update(bundle_key.as_bytes());
    hasher.update(digest);
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub(super) fn clean_default() -> Result<(), String> {
    let Some(root) = resolve_cache_root() else {
        return Ok(());
    };
    match fs::symlink_metadata(&root) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("inspect cache root {}: {error}", root.display())),
        Ok(_) => {}
    }
    CacheStore::open_at(root)?.clean()
}
