use calckernel::{TuneCache, TuneCacheDomain};
use sha2::{Digest, Sha256};
use std::fs;

#[path = "../support/temp.rs"]
mod temp;

#[test]
fn cache_domains_and_installation_salt_are_part_of_keys() {
    let first_root = temp::temp_dir("tune-cache-domain-a");
    let second_root = temp::temp_dir("tune-cache-domain-b");
    let first = TuneCache::open_at(&first_root).expect("first cache");
    let reopened = TuneCache::open_at(&first_root).expect("reopen first cache");
    let second = TuneCache::open_at(&second_root).expect("second cache");
    let identity = [b"same-identity".as_slice()];

    assert_ne!(
        first.derive_key(TuneCacheDomain::Compile, &identity),
        first.derive_key(TuneCacheDomain::Measurement, &identity)
    );
    assert_ne!(
        first.derive_key(TuneCacheDomain::Decision, &identity),
        first.derive_key(TuneCacheDomain::Compile, &identity)
    );
    assert_eq!(first.salt_digest(), reopened.salt_digest());
    assert_ne!(first.salt_digest(), second.salt_digest());
    assert_eq!(
        first.derive_key(TuneCacheDomain::Measurement, &identity),
        reopened.derive_key(TuneCacheDomain::Measurement, &identity)
    );
    assert_ne!(
        first.derive_key(TuneCacheDomain::Measurement, &identity),
        second.derive_key(TuneCacheDomain::Measurement, &identity)
    );
    let raw_salt = fs::read(first.root().join("salt")).expect("salt bytes");
    assert_eq!(raw_salt.len(), 32);
    assert_ne!(raw_salt.as_slice(), first.salt_digest());

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(first.root().join("salt"))
                .expect("salt metadata")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(first.root())
                .expect("root metadata")
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
    }
}

#[test]
fn cache_round_trip_is_checked_and_corruption_is_a_miss() {
    let base = temp::temp_dir("tune-cache-round-trip");
    let cache = TuneCache::open_at(&base).expect("cache");
    let key = cache.derive_key(
        TuneCacheDomain::Compile,
        &[b"compiler".as_slice(), b"plan".as_slice()],
    );
    let write = cache
        .write(TuneCacheDomain::Compile, key, b"verified trial")
        .expect("write");
    let hit = cache
        .read(TuneCacheDomain::Compile, key)
        .expect("read")
        .expect("hit");
    assert_eq!(hit.payload(), b"verified trial");
    assert_eq!(hit.receipt().key_digest(), key.as_bytes());
    assert_eq!(hit.receipt().entry_digest(), write.entry_digest());

    let path = cache.entry_path(TuneCacheDomain::Compile, key);
    let mut corrupt = fs::read(&path).expect("entry bytes");
    let index = corrupt.len() / 2;
    corrupt[index] ^= 0x80;
    fs::write(&path, corrupt).expect("corrupt entry");
    assert_eq!(
        cache.read(TuneCacheDomain::Compile, key).expect("miss"),
        None
    );
    assert!(
        !path.exists(),
        "corrupt entry must be quarantined by removal"
    );
}

#[test]
fn cache_rejects_cross_domain_entries_and_symlink_roots() {
    let base = temp::temp_dir("tune-cache-cross-domain");
    let cache = TuneCache::open_at(&base).expect("cache");
    let key = cache.derive_key(TuneCacheDomain::Compile, &[b"identity"]);
    cache
        .write(TuneCacheDomain::Compile, key, b"compile")
        .expect("write");
    assert_eq!(
        cache.read(TuneCacheDomain::Measurement, key).expect("miss"),
        None
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        let target = temp::temp_dir("tune-cache-symlink-target");
        let link_parent = temp::temp_dir("tune-cache-symlink-parent");
        fs::create_dir_all(&target).expect("target");
        fs::create_dir_all(&link_parent).expect("link parent");
        let link = link_parent.join("linked");
        symlink(&target, &link).expect("symlink");
        assert!(TuneCache::open_at(&link).is_err());
    }
}

#[test]
fn cache_hard_limit_uses_deterministic_lru_across_domains() {
    let base = temp::temp_dir("tune-cache-lru");
    let cache = TuneCache::open_at_with_limit(&base, 280).expect("cache");
    let old = cache.derive_key(TuneCacheDomain::Compile, &[b"old"]);
    let new = cache.derive_key(TuneCacheDomain::Decision, &[b"new"]);
    cache
        .write(TuneCacheDomain::Compile, old, &[1; 80])
        .expect("old");
    std::thread::sleep(std::time::Duration::from_millis(10));
    cache
        .write(TuneCacheDomain::Decision, new, &[2; 80])
        .expect("new");

    assert_eq!(
        cache.read(TuneCacheDomain::Compile, old).expect("old miss"),
        None
    );
    assert_eq!(
        cache
            .read(TuneCacheDomain::Decision, new)
            .expect("new read")
            .expect("new hit")
            .payload(),
        &[2; 80]
    );
    let digest = <[u8; 32]>::from(Sha256::digest(b"never a path"));
    assert!(
        cache
            .entry_path(TuneCacheDomain::Compile, digest.into())
            .starts_with(cache.root())
    );
}
