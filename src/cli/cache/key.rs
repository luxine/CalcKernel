use sha2::{Digest, Sha256};

const KEY_MAGIC: &[u8] = b"CKC-CACHE-KEY\0";
pub(in crate::cli) const KEY_SCHEMA: u32 = 4;

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
    pub(in crate::cli) kir_contract_version: u32,
    pub(in crate::cli) sanitizer_mode: u8,
    pub(in crate::cli) target_profile_digest: String,
    pub(in crate::cli) vector_cost_model_schema: u32,
    pub(in crate::cli) vector_proof_schema: u32,
    pub(in crate::cli) vector_budget_identity: String,
    pub(in crate::cli) cpu: String,
    pub(in crate::cli) features: String,
    pub(in crate::cli) codegen_contract: String,
    pub(in crate::cli) runtime_sha256: [String; 5],
    pub(in crate::cli) profile_identity: String,
    pub(in crate::cli) artifact_identity: String,
    pub(in crate::cli) pgo_identity: String,
    pub(in crate::cli) multiversion_identity: String,
    pub(in crate::cli) dispatch_identity: String,
    pub(in crate::cli) budget_identity: String,
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
    field(&mut output, 15, &input.kir_contract_version.to_be_bytes());
    field(&mut output, 16, &[input.sanitizer_mode]);
    field(&mut output, 17, input.target_profile_digest.as_bytes());
    field(
        &mut output,
        18,
        &input.vector_cost_model_schema.to_be_bytes(),
    );
    field(&mut output, 19, &input.vector_proof_schema.to_be_bytes());
    for (index, hash) in input.runtime_sha256.iter().enumerate() {
        field(&mut output, 20 + index as u16, hash.as_bytes());
    }
    field(&mut output, 30, input.vector_budget_identity.as_bytes());
    field(&mut output, 31, input.profile_identity.as_bytes());
    field(&mut output, 32, input.artifact_identity.as_bytes());
    field(&mut output, 33, input.pgo_identity.as_bytes());
    field(&mut output, 34, input.multiversion_identity.as_bytes());
    field(&mut output, 35, input.dispatch_identity.as_bytes());
    field(&mut output, 36, input.budget_identity.as_bytes());
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
            kir_contract_version: 1,
            sanitizer_mode: 0,
            target_profile_digest: "31".repeat(32),
            vector_cost_model_schema: 1,
            vector_proof_schema: 1,
            vector_budget_identity: "vector-budget-schema=1;growth=20".to_string(),
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
            profile_identity: "mode=use;format=1;contract=1;digest=41".to_string(),
            artifact_identity: "kind=dynamic;topology=native-library".to_string(),
            pgo_identity: "confidence=95/100;hot=900/1000;site=3;cost=3".to_string(),
            multiversion_identity: "target-set=51;variants=baseline,x86-64-v3".to_string(),
            dispatch_identity: "table=61;detector=1;thunk=1;runtime=71".to_string(),
            budget_identity: "multiversion=1;growth=100;root=25".to_string(),
        }
    }

    #[test]
    fn canonical_key_should_have_one_exact_architecture_independent_vector() {
        let input = vector();
        let bytes = canonical_key_bytes(&input);
        assert!(bytes.starts_with(b"CKC-CACHE-KEY\0\0\0\0\x04"));
        assert_eq!(
            cache_key_hex(&input),
            "4b6a7c147c891bbc12a17d1d805a1479ad3b6fe5f9a31366dc1e49b3df4d6407"
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
        changed!(kir_contract_version, 2);
        changed!(sanitizer_mode, 1);
        changed!(target_profile_digest, "32".repeat(32));
        changed!(vector_cost_model_schema, 2);
        changed!(vector_proof_schema, 2);
        changed!(
            vector_budget_identity,
            "vector-budget-schema=2;growth=20".to_string()
        );
        changed!(cpu, "generic".to_string());
        changed!(features, "+neon".to_string());
        changed!(
            codegen_contract,
            "strict-fp;entry-v2;native-cpu".to_string()
        );
        let mut runtime = baseline.runtime_sha256.clone();
        runtime[3] = "ff".repeat(32);
        changed!(runtime_sha256, runtime);
        changed!(profile_identity, "mode=off".to_string());
        changed!(
            artifact_identity,
            "kind=static;topology=native-library".to_string()
        );
        changed!(pgo_identity, "confidence=90/100".to_string());
        changed!(multiversion_identity, "target-set=52".to_string());
        changed!(dispatch_identity, "table=62".to_string());
        changed!(budget_identity, "multiversion=2".to_string());
        for (name, mutation) in mutations {
            assert_ne!(cache_key_hex(&mutation), expected, "unchanged {name}");
        }
    }
}
