#![allow(dead_code)]

use sha2::{Digest, Sha256};

pub const MAX_MATRIX_BYTES: u64 = 1_073_741_824;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrozenSplit {
    pub name: &'static str,
    pub n: u32,
    pub seed: u64,
    pub expected_digest: &'static str,
}

pub const TRAINING: FrozenSplit = FrozenSplit {
    name: "training",
    n: 128,
    seed: 113,
    expected_digest: "d6105453012eedb8a8db812555f116dd69ca6a8e0242faf81f10038947581608",
};
pub const VALIDATION: FrozenSplit = FrozenSplit {
    name: "validation",
    n: 256,
    seed: 127,
    expected_digest: "e21128a8623d0c072111b02c8a3f1ce3309d12f8bf0b3beddc1e0f6342dfc6c8",
};
pub const RELEASE: FrozenSplit = FrozenSplit {
    name: "release-held-out",
    n: 1_024,
    seed: 131,
    expected_digest: "4d9a1612967ec78ffb3d0ecc035929bb8217cefc13dfeb3fb2e21989186b055d",
};

#[derive(Debug, Clone, PartialEq)]
pub struct PredicatedMatrix {
    pub n: u32,
    pub values: Vec<f64>,
}

impl PredicatedMatrix {
    pub fn generate(n: u32, seed: u64) -> Result<Self, String> {
        let cells = checked_cells(n)?;
        let mut values = Vec::with_capacity(cells);
        for i in 0..n {
            for j in 0..n {
                values.push(cell(i, j, n, seed));
            }
        }
        Ok(Self { n, values })
    }

    pub fn scalar_floyd(&mut self) -> Result<(), String> {
        let n = usize::try_from(self.n).map_err(|_| "matrix dimension is not representable")?;
        for k in 0..n {
            let k_row = k.checked_mul(n).ok_or("Floyd k-row overflow")?;
            for i in 0..n {
                let i_row = i.checked_mul(n).ok_or("Floyd i-row overflow")?;
                let dik = self.values[i_row.checked_add(k).ok_or("Floyd i+k overflow")?];
                for j in 0..n {
                    let index = i_row.checked_add(j).ok_or("Floyd i+j overflow")?;
                    let candidate =
                        dik + self.values[k_row.checked_add(j).ok_or("Floyd k+j overflow")?];
                    let old = self.values[index];
                    if candidate < old {
                        self.values[index] = candidate;
                    }
                }
            }
        }
        Ok(())
    }

    pub fn result_digest(&self) -> Result<[u8; 32], String> {
        if self.values.iter().any(|value| !value.is_finite()) {
            return Err("predicated Floyd result contains a nonfinite value".to_string());
        }
        let mut digest = Sha256::new();
        digest.update(b"CK-V014-PRED-RESULT\0");
        digest.update(self.n.to_be_bytes());
        digest.update(
            u64::try_from(self.values.len())
                .map_err(|_| "matrix cell count overflow")?
                .to_be_bytes(),
        );
        for value in &self.values {
            digest.update(value.to_bits().to_be_bytes());
        }
        Ok(digest.finalize().into())
    }

    pub fn canonical_result_bytes(&self) -> Result<Vec<u8>, String> {
        let capacity = self
            .values
            .len()
            .checked_mul(std::mem::size_of::<u64>())
            .ok_or("result byte count overflow")?;
        let mut bytes = Vec::with_capacity(capacity);
        for value in &self.values {
            bytes.extend_from_slice(&value.to_bits().to_be_bytes());
        }
        Ok(bytes)
    }
}

pub fn split(name: &str) -> Option<FrozenSplit> {
    match name {
        "training" => Some(TRAINING),
        "validation" => Some(VALIDATION),
        "release-held-out" => Some(RELEASE),
        _ => None,
    }
}

pub fn checked_invocation_bytes(n: u32, iterations: u64) -> Result<u64, String> {
    if iterations == 0 {
        return Err("predicated iteration count must be positive".to_string());
    }
    let cells = u64::from(n)
        .checked_mul(u64::from(n))
        .ok_or("predicated matrix cell count overflow")?;
    let one = cells
        .checked_mul(u64::try_from(std::mem::size_of::<f64>()).unwrap_or(8))
        .ok_or("predicated matrix byte count overflow")?;
    let total = one
        .checked_mul(iterations)
        .ok_or("predicated invocation byte count overflow")?;
    if total > MAX_MATRIX_BYTES {
        return Err("predicated invocation exceeds the 1 GiB matrix cap".to_string());
    }
    Ok(total)
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn checked_cells(n: u32) -> Result<usize, String> {
    if n == 0 || n > 1_024 {
        return Err("predicated matrix dimension must be in 1..=1024".to_string());
    }
    let cells = u64::from(n)
        .checked_mul(u64::from(n))
        .ok_or("predicated matrix cell count overflow")?;
    let bytes = cells
        .checked_mul(u64::try_from(std::mem::size_of::<f64>()).unwrap_or(8))
        .ok_or("predicated matrix byte count overflow")?;
    if bytes > MAX_MATRIX_BYTES {
        return Err("predicated matrix exceeds the 1 GiB cap".to_string());
    }
    usize::try_from(cells)
        .map_err(|_| "predicated matrix cell count is not representable".to_string())
}

fn cell(i: u32, j: u32, n: u32, seed: u64) -> f64 {
    if i == j {
        return 0.0;
    }
    let r = splitmix64(seed ^ (u64::from(i) << 32) ^ u64::from(j));
    if j == i.wrapping_add(1) % n {
        return (1 + r % 16) as f64;
    }
    if (r >> 8).is_multiple_of(4) {
        return f64::INFINITY;
    }
    (1 + r % 1_024) as f64
}

fn splitmix64(value: u64) -> u64 {
    let mut z = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}
