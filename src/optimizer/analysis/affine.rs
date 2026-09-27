use std::collections::BTreeMap;

use num_bigint::BigInt;

use crate::ValueId;

/// Deterministic affine form over SSA values and mathematical integers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AffineForm {
    terms: BTreeMap<ValueId, BigInt>,
    constant: BigInt,
}

impl AffineForm {
    #[must_use]
    pub fn variable(value: ValueId) -> Self {
        Self {
            terms: BTreeMap::from([(value, BigInt::from(1_u8))]),
            constant: BigInt::from(0_u8),
        }
    }

    #[must_use]
    pub fn integer(value: BigInt) -> Self {
        Self {
            terms: BTreeMap::new(),
            constant: value,
        }
    }

    #[must_use]
    pub fn coefficient(&self, value: ValueId) -> BigInt {
        self.terms
            .get(&value)
            .cloned()
            .unwrap_or_else(|| BigInt::from(0_u8))
    }

    #[must_use]
    pub const fn constant(&self) -> &BigInt {
        &self.constant
    }

    #[must_use]
    pub fn add(&self, other: &Self) -> Self {
        let mut terms = self.terms.clone();
        for (value, coefficient) in &other.terms {
            let entry = terms.entry(*value).or_insert_with(|| BigInt::from(0_u8));
            *entry += coefficient;
        }
        terms.retain(|_, coefficient| coefficient != &BigInt::from(0_u8));
        Self {
            terms,
            constant: &self.constant + &other.constant,
        }
    }

    #[must_use]
    pub fn sub(&self, other: &Self) -> Self {
        self.add(&other.scale(&BigInt::from(-1_i8)))
    }

    #[must_use]
    pub fn scale(&self, coefficient: &BigInt) -> Self {
        Self {
            terms: self
                .terms
                .iter()
                .map(|(value, term)| (*value, term * coefficient))
                .filter(|(_, term)| term != &BigInt::from(0_u8))
                .collect(),
            constant: &self.constant * coefficient,
        }
    }

    #[must_use]
    pub fn add_constant(&self, constant: BigInt) -> Self {
        Self {
            terms: self.terms.clone(),
            constant: &self.constant + constant,
        }
    }
}
