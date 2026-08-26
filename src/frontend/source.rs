#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceFile {
    pub file_name: String,
    pub text: String,
}

impl SourceFile {
    #[must_use]
    pub fn new(file_name: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            file_name: file_name.into(),
            text: text.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SourcePosition {
    pub offset: usize,
    pub line: usize,
    pub column: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SourceSpan {
    pub start: SourcePosition,
    pub end: SourcePosition,
}
