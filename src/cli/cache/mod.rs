mod entry;
mod evict;
mod key;
mod path;
mod store;

use std::fs;

use calckernel::{NativeObject, NativeTarget};

pub(super) use entry::CacheManifest;
pub(super) use key::{CacheKeyInput, cache_key_hex};
use path::resolve_cache_root;
use store::CacheStore;

pub(super) fn load_object(target: &NativeTarget, key: &str, bypass: bool) -> Option<NativeObject> {
    if bypass {
        return None;
    }
    let store = CacheStore::open_at(resolve_cache_root()?).ok()?;
    let bytes = store.read(key)?;
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
