use sha2::{Digest, Sha256};

const KEY_MAGIC: &[u8] = b"CKC-CACHE-KEY\0";
const KEY_SCHEMA: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::cli) struct CacheKeyInput {
    pub(in crate::cli) source: Vec<u8>,
    pub(in crate::cli) compiler_version: String,
    pub(in crate::cli) native_abi: u32,
    pub(in crate::cli) runtime_abi: u32,
    pub(in crate::cli) bridge_abi: u32,
    pub(in crate::cli) llvm_version: String,
    pub(in crate::cli) llvm_manifest_sha256: String,
    pub(in crate::cli) target_triple: String,
    pub(in crate::cli) optimization_level: u8,
    pub(in crate::cli) overflow_mode: u8,
    pub(in crate::cli) bounds_mode: u8,
    pub(in crate::cli) cpu: String,
    pub(in crate::cli) features: String,
    pub(in crate::cli) codegen_contract: String,
    pub(in crate::cli) runtime_sha256: [String; 5],
}

pub(super) fn canonical_key_bytes(input: &CacheKeyInput) -> Vec<u8> {
    let mut output = Vec::with_capacity(input.source.len() + 768);
    output.extend_from_slice(KEY_MAGIC);
    output.extend_from_slice(&KEY_SCHEMA.to_be_bytes());
    field(&mut output, 1, &input.source);
    field(&mut output, 2, input.compiler_version.as_bytes());
    field(&mut output, 3, &input.native_abi.to_be_bytes());
    field(&mut output, 4, &input.runtime_abi.to_be_bytes());
    field(&mut output, 5, &input.bridge_abi.to_be_bytes());
    field(&mut output, 6, input.llvm_version.as_bytes());
    field(&mut output, 7, input.llvm_manifest_sha256.as_bytes());
    field(&mut output, 8, input.target_triple.as_bytes());
    field(&mut output, 9, &[input.optimization_level]);
    field(&mut output, 10, &[input.overflow_mode]);
    field(&mut output, 11, &[input.bounds_mode]);
    field(&mut output, 12, input.cpu.as_bytes());
    field(&mut output, 13, input.features.as_bytes());
    field(&mut output, 14, input.codegen_contract.as_bytes());
    for (index, hash) in input.runtime_sha256.iter().enumerate() {
        field(&mut output, 20 + index as u16, hash.as_bytes());
    }
    output
}

pub(in crate::cli) fn cache_key_hex(input: &CacheKeyInput) -> String {
    let digest = Sha256::digest(canonical_key_bytes(input));
    let mut output = String::with_capacity(64);
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for byte in digest {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

fn field(output: &mut Vec<u8>, tag: u16, value: &[u8]) {
    output.extend_from_slice(&tag.to_be_bytes());
    output.extend_from_slice(&(value.len() as u64).to_be_bytes());
    output.extend_from_slice(value);
}

#[cfg(test)]
mod tests {
    use super::{CacheKeyInput, cache_key_hex, canonical_key_bytes};

    fn vector() -> CacheKeyInput {
        CacheKeyInput {
            source: b"fn main() -> i32 { return 42; }".to_vec(),
            compiler_version: "0.10.0".to_string(),
            native_abi: 1,
            runtime_abi: 1,
            bridge_abi: 1,
            llvm_version: "22.1.8".to_string(),
            llvm_manifest_sha256: "11".repeat(32),
            target_triple: "aarch64-apple-darwin".to_string(),
            optimization_level: 3,
            overflow_mode: 1,
            bounds_mode: 0,
            cpu: "apple-m1".to_string(),
            features: "+aes,+crc,+neon".to_string(),
            codegen_contract: "strict-fp;entry-v1;native-cpu".to_string(),
            runtime_sha256: [
                "21".repeat(32),
                "22".repeat(32),
                "23".repeat(32),
                "24".repeat(32),
                "25".repeat(32),
            ],
        }
    }

    #[test]
    fn canonical_key_should_have_one_exact_architecture_independent_vector() {
        let input = vector();
        let bytes = canonical_key_bytes(&input);
        assert!(bytes.starts_with(b"CKC-CACHE-KEY\0\0\0\0\x01"));
        assert_eq!(
            cache_key_hex(&input),
            "0f2608d415f216c7c32559da6565eaf621b42449b72ec06eb92589f7d907fc48"
        );
    }

    #[test]
    fn every_object_affecting_input_should_change_the_key() {
        let baseline = vector();
        let expected = cache_key_hex(&baseline);
        let mut mutations = Vec::new();
        macro_rules! changed {
            ($field:ident, $value:expr) => {{
                let mut value = baseline.clone();
                value.$field = $value;
                mutations.push((stringify!($field), value));
            }};
        }
        changed!(source, b"fn main() -> i32 { return 43; }".to_vec());
        changed!(compiler_version, "0.10.1".to_string());
        changed!(native_abi, 2);
        changed!(runtime_abi, 2);
        changed!(bridge_abi, 2);
        changed!(llvm_version, "22.1.9".to_string());
        changed!(llvm_manifest_sha256, "12".repeat(32));
        changed!(target_triple, "x86_64-apple-darwin".to_string());
        changed!(optimization_level, 2);
        changed!(overflow_mode, 0);
        changed!(bounds_mode, 1);
        changed!(cpu, "generic".to_string());
        changed!(features, "+neon".to_string());
        changed!(
            codegen_contract,
            "strict-fp;entry-v2;native-cpu".to_string()
        );
        let mut runtime = baseline.runtime_sha256.clone();
        runtime[3] = "ff".repeat(32);
        changed!(runtime_sha256, runtime);
        for (name, mutation) in mutations {
            assert_ne!(cache_key_hex(&mutation), expected, "unchanged {name}");
        }
    }
}
