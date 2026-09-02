use sha2::{Digest, Sha256};

use super::{TuneCacheDomain, TuneCacheKey};

const MAGIC: &[u8; 8] = b"CKTCACH1";
const SCHEMA: u32 = 1;
const HEADER: usize = 8 + 4 + 1 + 32 + 8;
const TRAILER: usize = 32;
const MAX_PAYLOAD: u64 = 1024 * 1024 * 1024;

pub(super) struct DecodedEntry<'a> {
    pub payload: &'a [u8],
    pub digest: [u8; 32],
}

pub(super) fn encode(
    domain: TuneCacheDomain,
    key: TuneCacheKey,
    payload: &[u8],
) -> Result<(Vec<u8>, [u8; 32]), String> {
    let length = u64::try_from(payload.len()).map_err(|_| "tune cache payload overflow")?;
    if length > MAX_PAYLOAD {
        return Err("tune cache payload exceeds 1 GiB entry bound".to_string());
    }
    let mut bytes = Vec::with_capacity(HEADER + payload.len() + TRAILER);
    bytes.extend_from_slice(MAGIC);
    bytes.extend_from_slice(&SCHEMA.to_be_bytes());
    bytes.push(domain as u8);
    bytes.extend_from_slice(key.as_bytes());
    bytes.extend_from_slice(&length.to_be_bytes());
    bytes.extend_from_slice(payload);
    let digest = entry_digest(&bytes);
    bytes.extend_from_slice(&digest);
    Ok((bytes, digest))
}

pub(super) fn decode(
    expected_domain: TuneCacheDomain,
    expected_key: TuneCacheKey,
    bytes: &[u8],
) -> Result<DecodedEntry<'_>, String> {
    if bytes.len() < HEADER + TRAILER || bytes.get(..8) != Some(MAGIC) {
        return Err("invalid tune cache entry header".to_string());
    }
    let schema = u32::from_be_bytes(
        bytes[8..12]
            .try_into()
            .map_err(|_| "truncated tune cache schema")?,
    );
    if schema != SCHEMA || bytes[12] != expected_domain as u8 {
        return Err("tune cache schema or domain mismatch".to_string());
    }
    if bytes[13..45] != *expected_key.as_bytes() {
        return Err("tune cache key mismatch".to_string());
    }
    let length = u64::from_be_bytes(
        bytes[45..53]
            .try_into()
            .map_err(|_| "truncated tune cache length")?,
    );
    if length > MAX_PAYLOAD {
        return Err("tune cache payload exceeds bound".to_string());
    }
    let payload_end = HEADER
        .checked_add(usize::try_from(length).map_err(|_| "tune cache length overflow")?)
        .ok_or("tune cache length overflow")?;
    let exact_end = payload_end
        .checked_add(TRAILER)
        .ok_or("tune cache length overflow")?;
    if exact_end != bytes.len() {
        return Err("tune cache entry length mismatch".to_string());
    }
    let digest: [u8; 32] = bytes[payload_end..]
        .try_into()
        .map_err(|_| "truncated tune cache digest")?;
    if entry_digest(&bytes[..payload_end]) != digest {
        return Err("tune cache entry digest mismatch".to_string());
    }
    Ok(DecodedEntry {
        payload: &bytes[HEADER..payload_end],
        digest,
    })
}

fn entry_digest(bytes: &[u8]) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(b"CK-TUNE-CACHE-ENTRY\0");
    digest.update(bytes);
    digest.finalize().into()
}
