use num_bigint::BigInt;

use super::ScalarDomainError;

/// Congruence `value ≡ remainder (mod modulus)`; modulus zero denotes an exact value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScalarCongruence {
    modulus: BigInt,
    remainder: BigInt,
}

impl ScalarCongruence {
    pub fn new(modulus: BigInt, remainder: BigInt) -> Result<Self, ScalarDomainError> {
        if modulus <= BigInt::from(0_u8) {
            return Err(ScalarDomainError::new(
                "congruence modulus must be a positive integer",
            ));
        }
        let remainder = mathematical_mod(remainder, &modulus);
        Ok(Self { modulus, remainder })
    }

    pub(super) fn exact(value: BigInt) -> Self {
        Self {
            modulus: BigInt::from(0_u8),
            remainder: value,
        }
    }

    pub(super) fn top() -> Self {
        Self {
            modulus: BigInt::from(1_u8),
            remainder: BigInt::from(0_u8),
        }
    }

    #[must_use]
    pub fn contains(&self, value: &BigInt) -> bool {
        if self.modulus == BigInt::from(0_u8) {
            return value == &self.remainder;
        }
        mathematical_mod(value - &self.remainder, &self.modulus) == BigInt::from(0_u8)
    }

    #[must_use]
    pub fn add(&self, other: &Self) -> Self {
        combine(self, other, false)
    }

    #[must_use]
    pub fn sub(&self, other: &Self) -> Self {
        combine(self, other, true)
    }

    pub(super) fn multiply(&self, other: &Self) -> Self {
        if self.modulus == BigInt::from(0_u8) && other.modulus == BigInt::from(0_u8) {
            return Self::exact(&self.remainder * &other.remainder);
        }
        Self::top()
    }
}

fn combine(left: &ScalarCongruence, right: &ScalarCongruence, subtract: bool) -> ScalarCongruence {
    let remainder = if subtract {
        &left.remainder - &right.remainder
    } else {
        &left.remainder + &right.remainder
    };
    if left.modulus == BigInt::from(0_u8) && right.modulus == BigInt::from(0_u8) {
        return ScalarCongruence::exact(remainder);
    }
    let modulus = gcd_nonnegative(left.modulus.clone(), right.modulus.clone());
    if modulus == BigInt::from(0_u8) {
        ScalarCongruence::exact(remainder)
    } else {
        ScalarCongruence {
            remainder: mathematical_mod(remainder, &modulus),
            modulus,
        }
    }
}

fn gcd_nonnegative(mut left: BigInt, mut right: BigInt) -> BigInt {
    if left < BigInt::from(0_u8) {
        left = -left;
    }
    if right < BigInt::from(0_u8) {
        right = -right;
    }
    while right != BigInt::from(0_u8) {
        let remainder = left % &right;
        left = right;
        right = remainder;
    }
    left
}

pub(super) fn mathematical_mod(value: BigInt, modulus: &BigInt) -> BigInt {
    let remainder = value % modulus;
    if remainder < BigInt::from(0_u8) {
        remainder + modulus
    } else {
        remainder
    }
}
