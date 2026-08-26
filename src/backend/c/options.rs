#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverflowMode {
    Unchecked,
    Checked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoundsMode {
    Unchecked,
    Checked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EmitCOptions {
    pub overflow_mode: OverflowMode,
    pub bounds_mode: BoundsMode,
    pub opt_level: u8,
}

impl Default for EmitCOptions {
    fn default() -> Self {
        Self {
            overflow_mode: OverflowMode::Unchecked,
            bounds_mode: BoundsMode::Unchecked,
            opt_level: 0,
        }
    }
}
