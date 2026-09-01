use calckernel::{
    FunctionId, KirMultiversionTierId, NativeMultiversionObjectBundle, NativeMultiversionObjectRole,
};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

pub(in crate::cli) const ENTRY_MAGIC: &[u8; 8] = b"CKCOBJ03";
const MANIFEST_MAGIC: &[u8] = b"CKC-MANIFEST\0";
pub(in crate::cli) const MANIFEST_SCHEMA: u32 = 4;
const MAX_MANIFEST_BYTES: usize = 16 * 1024;
const MAX_OBJECT_BYTES: usize = 256 * 1024 * 1024;
const BUNDLE_MAGIC: &[u8; 8] = b"CKCBND01";
const MAX_BUNDLE_OBJECTS: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::cli) struct CacheManifest {
    pub(in crate::cli) key: String,
    pub(in crate::cli) compiler_version: String,
    pub(in crate::cli) llvm_version: String,
    pub(in crate::cli) target_triple: String,
    pub(in crate::cli) cpu: String,
    pub(in crate::cli) features: String,
    pub(in crate::cli) codegen_contract: String,
    pub(in crate::cli) native_abi: u32,
    pub(in crate::cli) runtime_abi: u32,
    pub(in crate::cli) bridge_abi: u32,
    pub(in crate::cli) optimization_level: u8,
    pub(in crate::cli) overflow_mode: u8,
    pub(in crate::cli) bounds_mode: u8,
    pub(in crate::cli) kir_contract_version: u32,
    pub(in crate::cli) sanitizer_mode: u8,
    pub(in crate::cli) target_profile_digest: String,
    pub(in crate::cli) vector_cost_model_schema: u32,
    pub(in crate::cli) vector_proof_schema: u32,
    pub(in crate::cli) vector_budget_identity: String,
    pub(in crate::cli) profile_identity: String,
    pub(in crate::cli) artifact_identity: String,
    pub(in crate::cli) pgo_identity: String,
    pub(in crate::cli) multiversion_identity: String,
    pub(in crate::cli) dispatch_identity: String,
    pub(in crate::cli) budget_identity: String,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct DecodedCacheEntry<'bytes> {
    pub(super) manifest: CacheManifest,
    pub(super) object: &'bytes [u8],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CacheBundleReference {
    pub(super) name: String,
    pub(super) role: NativeMultiversionObjectRole,
    pub(super) digest: [u8; 32],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CacheBundleIndex {
    pub(super) target_set_digest: [u8; 32],
    pub(super) dispatch_runtime_digest: [u8; 32],
    pub(super) objects: Vec<CacheBundleReference>,
}

pub(super) fn bundle_index(bundle: &NativeMultiversionObjectBundle) -> CacheBundleIndex {
    CacheBundleIndex {
        target_set_digest: *bundle.target_set_digest(),
        dispatch_runtime_digest: *bundle.dispatch_runtime_digest(),
        objects: bundle
            .objects()
            .iter()
            .map(|object| CacheBundleReference {
                name: object.name().to_string(),
                role: object.role(),
                digest: *object.digest(),
            })
            .collect(),
    }
}

pub(super) fn encode_bundle_index(index: &CacheBundleIndex) -> Result<Vec<u8>, &'static str> {
    validate_bundle_index(index)?;
    let mut output = Vec::with_capacity(128 + index.objects.len() * 96);
    output.extend_from_slice(BUNDLE_MAGIC);
    output.extend_from_slice(&1_u32.to_be_bytes());
    output.extend_from_slice(&index.target_set_digest);
    output.extend_from_slice(&index.dispatch_runtime_digest);
    output.extend_from_slice(&(index.objects.len() as u32).to_be_bytes());
    for object in &index.objects {
        let name_len = u16::try_from(object.name.len()).map_err(|_| "bundle name too long")?;
        output.extend_from_slice(&name_len.to_be_bytes());
        output.extend_from_slice(object.name.as_bytes());
        match object.role {
            NativeMultiversionObjectRole::Baseline => output.push(1),
            NativeMultiversionObjectRole::Variant { root, tier } => {
                output.push(2);
                output.extend_from_slice(&root.index().to_be_bytes());
                output.push(tier_code(tier));
            }
            NativeMultiversionObjectRole::DispatchRuntime => output.push(3),
        }
        output.extend_from_slice(&object.digest);
    }
    Ok(output)
}

pub(super) fn decode_bundle_index(bytes: &[u8]) -> Result<CacheBundleIndex, &'static str> {
    if bytes.len() < 8 + 4 + 32 + 32 + 4 || &bytes[..8] != BUNDLE_MAGIC {
        return Err("cache bundle header is invalid");
    }
    let mut reader = Reader { bytes, offset: 8 };
    if reader.u32()? != 1 {
        return Err("cache bundle schema is invalid");
    }
    let target_set_digest = reader.take(32)?.try_into().map_err(|_| "target digest")?;
    let dispatch_runtime_digest = reader.take(32)?.try_into().map_err(|_| "runtime digest")?;
    let count = reader.u32()? as usize;
    if !(2..=MAX_BUNDLE_OBJECTS).contains(&count) {
        return Err("cache bundle object count is invalid");
    }
    let mut objects = Vec::with_capacity(count);
    for _ in 0..count {
        let name_len = usize::from(u16::from_be_bytes(
            reader
                .take(2)?
                .try_into()
                .map_err(|_| "bundle name length")?,
        ));
        let name = std::str::from_utf8(reader.take(name_len)?)
            .map_err(|_| "bundle name is not UTF-8")?
            .to_string();
        let role = match reader.u8()? {
            1 => NativeMultiversionObjectRole::Baseline,
            2 => NativeMultiversionObjectRole::Variant {
                root: FunctionId::from_index(reader.u32()?),
                tier: tier_from_code(reader.u8()?)?,
            },
            3 => NativeMultiversionObjectRole::DispatchRuntime,
            _ => return Err("cache bundle role is invalid"),
        };
        let digest = reader.take(32)?.try_into().map_err(|_| "object digest")?;
        objects.push(CacheBundleReference { name, role, digest });
    }
    if reader.offset != bytes.len() {
        return Err("cache bundle has trailing data");
    }
    let index = CacheBundleIndex {
        target_set_digest,
        dispatch_runtime_digest,
        objects,
    };
    validate_bundle_index(&index)?;
    Ok(index)
}

fn validate_bundle_index(index: &CacheBundleIndex) -> Result<(), &'static str> {
    if !(2..=MAX_BUNDLE_OBJECTS).contains(&index.objects.len())
        || index.objects.first().map(|object| object.role)
            != Some(NativeMultiversionObjectRole::Baseline)
        || index.objects.last().map(|object| object.role)
            != Some(NativeMultiversionObjectRole::DispatchRuntime)
    {
        return Err("cache bundle roles are not closed");
    }
    let namespace = hex_prefix(&index.target_set_digest);
    let mut names = BTreeSet::new();
    let mut prior_variant = None;
    for (position, object) in index.objects.iter().enumerate() {
        if object.name.is_empty()
            || object.name.len() > 255
            || object.name.starts_with('.')
            || !object.name.ends_with(".o")
            || !object.name.contains(&namespace)
            || !object
                .name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
            || !names.insert(object.name.as_str())
        {
            return Err("cache bundle object name is invalid");
        }
        match object.role {
            NativeMultiversionObjectRole::Baseline if position == 0 => {}
            NativeMultiversionObjectRole::DispatchRuntime
                if position + 1 == index.objects.len() =>
            {
                if object.digest != index.dispatch_runtime_digest {
                    return Err("cache bundle dispatch digest mismatch");
                }
            }
            NativeMultiversionObjectRole::Variant { root, tier }
                if position > 0 && position + 1 < index.objects.len() =>
            {
                let key = (root.index(), tier_code(tier));
                if prior_variant.is_some_and(|prior| prior >= key) {
                    return Err("cache bundle variant order is invalid");
                }
                prior_variant = Some(key);
            }
            _ => return Err("cache bundle object role position is invalid"),
        }
    }
    Ok(())
}

fn tier_code(tier: KirMultiversionTierId) -> u8 {
    match tier {
        KirMultiversionTierId::Baseline => 0,
        KirMultiversionTierId::X86_64V3 => 1,
        KirMultiversionTierId::X86_64V4 => 2,
        KirMultiversionTierId::AArch64Sve => 3,
        KirMultiversionTierId::AArch64Sve2 => 4,
    }
}

fn tier_from_code(code: u8) -> Result<KirMultiversionTierId, &'static str> {
    match code {
        0 => Ok(KirMultiversionTierId::Baseline),
        1 => Ok(KirMultiversionTierId::X86_64V3),
        2 => Ok(KirMultiversionTierId::X86_64V4),
        3 => Ok(KirMultiversionTierId::AArch64Sve),
        4 => Ok(KirMultiversionTierId::AArch64Sve2),
        _ => Err("cache bundle tier is invalid"),
    }
}

fn hex_prefix(digest: &[u8; 32]) -> String {
    digest[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub(super) fn encode_entry(
    manifest: &CacheManifest,
    object: &[u8],
) -> Result<Vec<u8>, &'static str> {
    if object.len() > MAX_OBJECT_BYTES {
        return Err("cache object exceeds size limit");
    }
    let manifest_bytes = encode_manifest(manifest)?;
    let manifest_len =
        u32::try_from(manifest_bytes.len()).map_err(|_| "cache manifest length exceeds u32")?;
    let object_len = u64::try_from(object.len()).map_err(|_| "cache object length exceeds u64")?;
    let mut output =
        Vec::with_capacity(ENTRY_MAGIC.len() + 4 + 8 + manifest_bytes.len() + object.len() + 32);
    output.extend_from_slice(ENTRY_MAGIC);
    output.extend_from_slice(&manifest_len.to_be_bytes());
    output.extend_from_slice(&object_len.to_be_bytes());
    output.extend_from_slice(&manifest_bytes);
    output.extend_from_slice(object);
    let mut digest = Sha256::new();
    digest.update(&manifest_bytes);
    digest.update(object);
    output.extend_from_slice(&digest.finalize());
    Ok(output)
}

pub(super) fn decode_entry<'bytes>(
    expected_key: &str,
    bytes: &'bytes [u8],
) -> Result<DecodedCacheEntry<'bytes>, &'static str> {
    const HEADER: usize = 8 + 4 + 8;
    const DIGEST: usize = 32;
    if bytes.len() < HEADER + DIGEST || &bytes[..8] != ENTRY_MAGIC {
        return Err("cache entry header is invalid");
    }
    let manifest_len = u32::from_be_bytes(
        bytes[8..12]
            .try_into()
            .map_err(|_| "cache manifest length is invalid")?,
    ) as usize;
    let object_len = usize::try_from(u64::from_be_bytes(
        bytes[12..20]
            .try_into()
            .map_err(|_| "cache object length is invalid")?,
    ))
    .map_err(|_| "cache object length exceeds usize")?;
    if manifest_len > MAX_MANIFEST_BYTES || object_len > MAX_OBJECT_BYTES {
        return Err("cache entry declared length exceeds limit");
    }
    let manifest_end = HEADER
        .checked_add(manifest_len)
        .ok_or("cache manifest length overflow")?;
    let object_end = manifest_end
        .checked_add(object_len)
        .ok_or("cache object length overflow")?;
    let expected_end = object_end
        .checked_add(DIGEST)
        .ok_or("cache digest length overflow")?;
    if expected_end != bytes.len() {
        return Err("cache entry length does not match header");
    }
    let manifest_bytes = &bytes[HEADER..manifest_end];
    let object = &bytes[manifest_end..object_end];
    let mut digest = Sha256::new();
    digest.update(manifest_bytes);
    digest.update(object);
    if digest.finalize().as_slice() != &bytes[object_end..] {
        return Err("cache entry digest mismatch");
    }
    let manifest = decode_manifest(manifest_bytes)?;
    if manifest.key != expected_key {
        return Err("cache manifest key mismatch");
    }
    Ok(DecodedCacheEntry { manifest, object })
}

fn encode_manifest(manifest: &CacheManifest) -> Result<Vec<u8>, &'static str> {
    if !valid_key(&manifest.key) {
        return Err("cache manifest key is invalid");
    }
    let mut output = Vec::with_capacity(512);
    output.extend_from_slice(MANIFEST_MAGIC);
    output.extend_from_slice(&MANIFEST_SCHEMA.to_be_bytes());
    for value in [
        &manifest.key,
        &manifest.compiler_version,
        &manifest.llvm_version,
        &manifest.target_triple,
        &manifest.cpu,
        &manifest.features,
        &manifest.codegen_contract,
        &manifest.target_profile_digest,
        &manifest.vector_budget_identity,
        &manifest.profile_identity,
        &manifest.artifact_identity,
        &manifest.pgo_identity,
        &manifest.multiversion_identity,
        &manifest.dispatch_identity,
        &manifest.budget_identity,
    ] {
        write_string(&mut output, value)?;
    }
    output.extend_from_slice(&manifest.native_abi.to_be_bytes());
    output.extend_from_slice(&manifest.runtime_abi.to_be_bytes());
    output.extend_from_slice(&manifest.bridge_abi.to_be_bytes());
    output.push(manifest.optimization_level);
    output.push(manifest.overflow_mode);
    output.push(manifest.bounds_mode);
    output.extend_from_slice(&manifest.kir_contract_version.to_be_bytes());
    output.push(manifest.sanitizer_mode);
    output.extend_from_slice(&manifest.vector_cost_model_schema.to_be_bytes());
    output.extend_from_slice(&manifest.vector_proof_schema.to_be_bytes());
    if output.len() > MAX_MANIFEST_BYTES {
        return Err("cache manifest exceeds size limit");
    }
    Ok(output)
}

fn decode_manifest(bytes: &[u8]) -> Result<CacheManifest, &'static str> {
    let prefix = MANIFEST_MAGIC.len() + 4;
    if bytes.len() < prefix
        || &bytes[..MANIFEST_MAGIC.len()] != MANIFEST_MAGIC
        || u32::from_be_bytes(
            bytes[MANIFEST_MAGIC.len()..prefix]
                .try_into()
                .map_err(|_| "cache manifest schema is invalid")?,
        ) != MANIFEST_SCHEMA
    {
        return Err("cache manifest header is invalid");
    }
    let mut reader = Reader {
        bytes,
        offset: prefix,
    };
    let key = reader.string()?;
    let compiler_version = reader.string()?;
    let llvm_version = reader.string()?;
    let target_triple = reader.string()?;
    let cpu = reader.string()?;
    let features = reader.string()?;
    let codegen_contract = reader.string()?;
    let target_profile_digest = reader.string()?;
    let vector_budget_identity = reader.string()?;
    let profile_identity = reader.string()?;
    let artifact_identity = reader.string()?;
    let pgo_identity = reader.string()?;
    let multiversion_identity = reader.string()?;
    let dispatch_identity = reader.string()?;
    let budget_identity = reader.string()?;
    let native_abi = reader.u32()?;
    let runtime_abi = reader.u32()?;
    let bridge_abi = reader.u32()?;
    let optimization_level = reader.u8()?;
    let overflow_mode = reader.u8()?;
    let bounds_mode = reader.u8()?;
    let kir_contract_version = reader.u32()?;
    let sanitizer_mode = reader.u8()?;
    let vector_cost_model_schema = reader.u32()?;
    let vector_proof_schema = reader.u32()?;
    if reader.offset != bytes.len() || !valid_key(&key) {
        return Err("cache manifest has trailing or invalid data");
    }
    Ok(CacheManifest {
        key,
        compiler_version,
        llvm_version,
        target_triple,
        cpu,
        features,
        codegen_contract,
        native_abi,
        runtime_abi,
        bridge_abi,
        optimization_level,
        overflow_mode,
        bounds_mode,
        kir_contract_version,
        sanitizer_mode,
        target_profile_digest,
        vector_cost_model_schema,
        vector_proof_schema,
        vector_budget_identity,
        profile_identity,
        artifact_identity,
        pgo_identity,
        multiversion_identity,
        dispatch_identity,
        budget_identity,
    })
}

fn valid_key(key: &str) -> bool {
    key.len() == 64
        && key
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn write_string(output: &mut Vec<u8>, value: &str) -> Result<(), &'static str> {
    let length = u32::try_from(value.len()).map_err(|_| "cache manifest field exceeds u32")?;
    output.extend_from_slice(&length.to_be_bytes());
    output.extend_from_slice(value.as_bytes());
    if output.len() > MAX_MANIFEST_BYTES {
        return Err("cache manifest exceeds size limit");
    }
    Ok(())
}

struct Reader<'bytes> {
    bytes: &'bytes [u8],
    offset: usize,
}

impl Reader<'_> {
    fn take(&mut self, length: usize) -> Result<&[u8], &'static str> {
        let end = self
            .offset
            .checked_add(length)
            .ok_or("cache manifest field length overflow")?;
        let value = self
            .bytes
            .get(self.offset..end)
            .ok_or("cache manifest field is truncated")?;
        self.offset = end;
        Ok(value)
    }

    fn u8(&mut self) -> Result<u8, &'static str> {
        Ok(self.take(1)?[0])
    }

    fn u32(&mut self) -> Result<u32, &'static str> {
        Ok(u32::from_be_bytes(
            self.take(4)?
                .try_into()
                .map_err(|_| "cache manifest u32 is invalid")?,
        ))
    }

    fn string(&mut self) -> Result<String, &'static str> {
        let length = self.u32()? as usize;
        let bytes = self.take(length)?;
        std::str::from_utf8(bytes)
            .map(str::to_string)
            .map_err(|_| "cache manifest field is not UTF-8")
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CacheBundleIndex, CacheBundleReference, CacheManifest, decode_bundle_index, decode_entry,
        encode_bundle_index, encode_entry,
    };
    use calckernel::{FunctionId, KirMultiversionTierId, NativeMultiversionObjectRole};

    fn manifest() -> CacheManifest {
        CacheManifest {
            key: "ab".repeat(32),
            compiler_version: "0.10.0".to_string(),
            llvm_version: "22.1.8".to_string(),
            target_triple: "aarch64-apple-darwin".to_string(),
            cpu: "apple-m1".to_string(),
            features: "+aes,+crc,+neon".to_string(),
            codegen_contract: "strict-fp;entry-v1;native-cpu".to_string(),
            native_abi: 1,
            runtime_abi: 1,
            bridge_abi: 1,
            optimization_level: 3,
            overflow_mode: 1,
            bounds_mode: 0,
            kir_contract_version: 1,
            sanitizer_mode: 0,
            target_profile_digest: "31".repeat(32),
            vector_cost_model_schema: 1,
            vector_proof_schema: 1,
            vector_budget_identity: "vector-budget-schema=1;growth=20".to_string(),
            profile_identity: "mode=use;format=1;contract=1;digest=41".to_string(),
            artifact_identity: "kind=dynamic;topology=native-library".to_string(),
            pgo_identity: "confidence=95/100;site=3;cost=3".to_string(),
            multiversion_identity: "target-set=51;variants=baseline,x86-64-v3".to_string(),
            dispatch_identity: "table=61;detector=1;thunk=1;runtime=71".to_string(),
            budget_identity: "multiversion=1;growth=100;root=25".to_string(),
        }
    }

    #[test]
    fn cache_entry_should_round_trip_bounded_manifest_object_and_digest() {
        let manifest = manifest();
        let object = b"validated native object bytes";
        let bytes = encode_entry(&manifest, object).expect("encode cache entry");
        let decoded = decode_entry(&manifest.key, &bytes).expect("decode cache entry");
        assert_eq!(decoded.manifest, manifest);
        assert_eq!(decoded.object, object);
        assert!(bytes.len() < 16 * 1024);
    }

    #[test]
    fn cache_entry_should_reject_wrong_key_lengths_corruption_and_trailing_data() {
        let manifest = manifest();
        let bytes = encode_entry(&manifest, b"object").expect("encode cache entry");
        let mut cases = Vec::new();
        cases.push(("short", bytes[..20].to_vec()));
        let mut magic = bytes.clone();
        magic[0] ^= 1;
        cases.push(("magic", magic));
        let mut length = bytes.clone();
        length[11] = 0xff;
        cases.push(("length", length));
        let mut digest = bytes.clone();
        let last = digest.len() - 1;
        digest[last] ^= 1;
        cases.push(("digest", digest));
        let mut trailing = bytes.clone();
        trailing.push(0);
        cases.push(("trailing", trailing));
        for (name, candidate) in cases {
            assert!(
                decode_entry(&manifest.key, &candidate).is_err(),
                "accepted {name}"
            );
        }
        assert!(decode_entry(&"cd".repeat(32), &bytes).is_err());
    }

    #[test]
    fn cache_manifest_should_reject_unbounded_fields() {
        let mut manifest = manifest();
        manifest.features = "x".repeat(20 * 1024);
        assert!(encode_entry(&manifest, b"object").is_err());
    }

    fn bundle() -> CacheBundleIndex {
        let target_set_digest = [1; 32];
        let namespace = "0101010101010101";
        let dispatch_runtime_digest = [4; 32];
        CacheBundleIndex {
            target_set_digest,
            dispatch_runtime_digest,
            objects: vec![
                CacheBundleReference {
                    name: format!("baseline-{namespace}.o"),
                    role: NativeMultiversionObjectRole::Baseline,
                    digest: [2; 32],
                },
                CacheBundleReference {
                    name: format!("variant-f0-x86_64_v3-{namespace}.o"),
                    role: NativeMultiversionObjectRole::Variant {
                        root: FunctionId::from_index(0),
                        tier: KirMultiversionTierId::X86_64V3,
                    },
                    digest: [3; 32],
                },
                CacheBundleReference {
                    name: format!("dispatch-runtime-{namespace}.o"),
                    role: NativeMultiversionObjectRole::DispatchRuntime,
                    digest: dispatch_runtime_digest,
                },
            ],
        }
    }

    #[test]
    fn native_cache_bundle_index_should_round_trip_exact_order_roles_and_digests() {
        let index = bundle();
        let bytes = encode_bundle_index(&index).expect("encode bundle");
        assert_eq!(decode_bundle_index(&bytes).expect("decode bundle"), index);
    }

    #[test]
    fn native_cache_bundle_index_should_reject_missing_extra_reordered_and_redirected_objects() {
        let valid = bundle();
        let mut mutations = Vec::new();
        let mut missing = valid.clone();
        missing.objects.remove(0);
        mutations.push(missing);
        let mut extra = valid.clone();
        extra.objects.insert(1, extra.objects[1].clone());
        mutations.push(extra);
        let mut reordered = valid.clone();
        reordered.objects.swap(0, 1);
        mutations.push(reordered);
        let mut redirected = valid.clone();
        redirected.objects[1].name = "../variant.o".to_string();
        mutations.push(redirected);
        let mut digest = valid.clone();
        digest.objects.last_mut().expect("runtime").digest = [9; 32];
        mutations.push(digest);
        for mutation in mutations {
            assert!(
                encode_bundle_index(&mutation).is_err(),
                "accepted {mutation:?}"
            );
        }
        let mut trailing = encode_bundle_index(&valid).expect("valid bundle");
        trailing.push(0);
        assert!(decode_bundle_index(&trailing).is_err());
    }
}
