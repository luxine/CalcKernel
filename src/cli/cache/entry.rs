use sha2::{Digest, Sha256};

const ENTRY_MAGIC: &[u8; 8] = b"CKCOBJ01";
const MANIFEST_MAGIC: &[u8] = b"CKC-MANIFEST\0";
const MANIFEST_SCHEMA: u32 = 2;
const MAX_MANIFEST_BYTES: usize = 16 * 1024;
const MAX_OBJECT_BYTES: usize = 256 * 1024 * 1024;

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
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct DecodedCacheEntry<'bytes> {
    pub(super) manifest: CacheManifest,
    pub(super) object: &'bytes [u8],
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
    let native_abi = reader.u32()?;
    let runtime_abi = reader.u32()?;
    let bridge_abi = reader.u32()?;
    let optimization_level = reader.u8()?;
    let overflow_mode = reader.u8()?;
    let bounds_mode = reader.u8()?;
    let kir_contract_version = reader.u32()?;
    let sanitizer_mode = reader.u8()?;
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
    use super::{CacheManifest, decode_entry, encode_entry};

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
}
