use std::{error::Error, fmt};

/// Native C ABI families shipped by CalcKernel 0.10.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NativeAbiTarget {
    SysvX86_64,
    DarwinX86_64,
    Aapcs64Linux,
    Aapcs64Darwin,
    WindowsX86_64,
    WindowsArm64,
}

impl NativeAbiTarget {
    pub const ALL: [Self; 6] = [
        Self::SysvX86_64,
        Self::DarwinX86_64,
        Self::Aapcs64Linux,
        Self::Aapcs64Darwin,
        Self::WindowsX86_64,
        Self::WindowsArm64,
    ];

    /// Maps a normalized LLVM target triple to a supported Native C ABI.
    pub fn from_triple(triple: &str) -> Result<Self, NativeAbiError> {
        let normalized = triple.to_ascii_lowercase();
        let x86_64 = normalized.starts_with("x86_64-");
        let arm64 = normalized.starts_with("aarch64-") || normalized.starts_with("arm64-");
        let darwin = normalized.contains("apple-darwin");
        let windows = normalized.contains("windows") || normalized.contains("win32");
        let linux = normalized.contains("linux");
        match (x86_64, arm64, darwin, windows, linux) {
            (true, false, false, false, true) => Ok(Self::SysvX86_64),
            (true, false, true, false, false) => Ok(Self::DarwinX86_64),
            (false, true, false, false, true) => Ok(Self::Aapcs64Linux),
            (false, true, true, false, false) => Ok(Self::Aapcs64Darwin),
            (true, false, false, true, false) => Ok(Self::WindowsX86_64),
            (false, true, false, true, false) => Ok(Self::WindowsArm64),
            _ => Err(NativeAbiError::new(format!(
                "unsupported Native C ABI target triple '{triple}'"
            ))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeAbiError {
    message: String,
}

impl NativeAbiError {
    pub(super) fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for NativeAbiError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for NativeAbiError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeAbiLayout {
    pub size: u32,
    pub alignment: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeAbiRegisterClass {
    Integer,
    Floating,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeAbiRegister {
    pub class: NativeAbiRegisterClass,
    pub bits: u16,
}

impl NativeAbiRegister {
    #[must_use]
    pub const fn integer(bits: u16) -> Self {
        Self {
            class: NativeAbiRegisterClass::Integer,
            bits,
        }
    }

    #[must_use]
    pub const fn floating(bits: u16) -> Self {
        Self {
            class: NativeAbiRegisterClass::Floating,
            bits,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeAbiExtension {
    None,
    Zero,
    Sign,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeAbiPassMode {
    Direct { registers: Vec<NativeAbiRegister> },
    Indirect { by_value: bool, alignment: u32 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeAbiValue {
    pub layout: NativeAbiLayout,
    pub mode: NativeAbiPassMode,
    pub extension: NativeAbiExtension,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeAbiArgumentRole {
    Source(usize),
    SliceData(usize),
    SliceLength(usize),
    CheckedResult,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeAbiArgument {
    pub role: NativeAbiArgumentRole,
    pub value: NativeAbiValue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeAbiHiddenResult {
    pub alignment: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeAbiFunction {
    pub return_value: NativeAbiValue,
    pub parameters: Vec<NativeAbiArgument>,
    pub hidden_result: Option<NativeAbiHiddenResult>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum NativeAbiPosition {
    Parameter,
    Return,
}
