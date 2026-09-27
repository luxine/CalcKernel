use crate::KirFunction;

/// Fixed knobs for deterministic scalar analysis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScalarAnalysisConfig {
    max_steps_override: Option<u32>,
    pub widening_after: u32,
    pub narrowing_iterations: u32,
}

impl ScalarAnalysisConfig {
    #[must_use]
    pub const fn with_max_steps(max_steps: u32) -> Self {
        Self {
            max_steps_override: Some(max_steps),
            widening_after: 2,
            narrowing_iterations: 2,
        }
    }
}

impl Default for ScalarAnalysisConfig {
    fn default() -> Self {
        Self {
            max_steps_override: None,
            widening_after: 2,
            narrowing_iterations: 2,
        }
    }
}

/// Budget identity derived only from KIR size and fixed configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScalarAnalysisBudget {
    kir_units: u32,
    max_steps: u32,
    widening_after: u32,
    narrowing_iterations: u32,
}

impl ScalarAnalysisBudget {
    #[must_use]
    pub fn for_function(function: &KirFunction, config: ScalarAnalysisConfig) -> Self {
        let kir_units = function
            .blocks
            .iter()
            .fold(function.params.len(), |count, block| {
                count + block.params.len() + block.instructions.len() + 1
            });
        let kir_units = u32::try_from(kir_units).unwrap_or(u32::MAX);
        let derived = kir_units.saturating_mul(32).saturating_add(64);
        Self {
            kir_units,
            max_steps: config.max_steps_override.unwrap_or(derived),
            widening_after: config.widening_after,
            narrowing_iterations: config.narrowing_iterations,
        }
    }

    #[must_use]
    pub const fn kir_units(self) -> u32 {
        self.kir_units
    }

    #[must_use]
    pub const fn max_steps(self) -> u32 {
        self.max_steps
    }

    #[must_use]
    pub const fn widening_after(self) -> u32 {
        self.widening_after
    }

    #[must_use]
    pub const fn narrowing_iterations(self) -> u32 {
        self.narrowing_iterations
    }

    #[must_use]
    pub const fn used_wall_clock(self) -> bool {
        false
    }
}
