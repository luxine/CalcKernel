use num_bigint::{BigInt, Sign};

use super::{IntegerType, mathematical_mod};

/// Internal known-zero/known-one bit mask for CK's at-most-64-bit integers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScalarKnownBits {
    known_zero: u64,
    known_one: u64,
}

impl ScalarKnownBits {
    pub(super) const fn unknown() -> Self {
        Self {
            known_zero: 0,
            known_one: 0,
        }
    }

    pub(super) fn exact(value: &BigInt, type_node: IntegerType) -> Self {
        let modulus = BigInt::from(1_u8) << type_node.bits();
        let unsigned = mathematical_mod(value.clone(), &modulus);
        let (_, digits) = unsigned.to_u64_digits();
        let raw = digits.first().copied().unwrap_or(0);
        let mask = type_node.bit_mask();
        Self {
            known_zero: (!raw) & mask,
            known_one: raw & mask,
        }
    }

    #[must_use]
    pub fn matches(self, value: &BigInt, type_node: IntegerType) -> bool {
        if value.sign() == Sign::NoSign {
            return self.known_one == 0;
        }
        let exact = Self::exact(value, type_node);
        (exact.known_one & self.known_zero) == 0
            && (exact.known_one & self.known_one) == self.known_one
    }
}
