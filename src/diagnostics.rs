use std::fmt;

use crate::{SourceFile, SourceSpan};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticSeverity {
    Error,
}

impl fmt::Display for DiagnosticSeverity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Error => formatter.write_str("error"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticCode {
    Ck0001,
    Ck1001,
    Ck2001,
    Ck2002,
    Ck2003,
    Ck2004,
    Ck2005,
    Ck2006,
    Ck2007,
    Ck2008,
    Ck2009,
    Ck2010,
}

impl fmt::Display for DiagnosticCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let code = match self {
            Self::Ck0001 => "CK0001",
            Self::Ck1001 => "CK1001",
            Self::Ck2001 => "CK2001",
            Self::Ck2002 => "CK2002",
            Self::Ck2003 => "CK2003",
            Self::Ck2004 => "CK2004",
            Self::Ck2005 => "CK2005",
            Self::Ck2006 => "CK2006",
            Self::Ck2007 => "CK2007",
            Self::Ck2008 => "CK2008",
            Self::Ck2009 => "CK2009",
            Self::Ck2010 => "CK2010",
        };
        formatter.write_str(code)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub code: DiagnosticCode,
    pub severity: DiagnosticSeverity,
    pub message: String,
    pub file_name: String,
    pub line: usize,
    pub column: usize,
    pub span: SourceSpan,
}

impl Diagnostic {
    #[must_use]
    pub fn error(
        code: DiagnosticCode,
        message: impl Into<String>,
        file_name: impl Into<String>,
        span: SourceSpan,
    ) -> Self {
        Self {
            code,
            severity: DiagnosticSeverity::Error,
            message: message.into(),
            file_name: file_name.into(),
            line: span.start.line,
            column: span.start.column,
            span,
        }
    }
}

#[must_use]
pub fn format_diagnostic(source_file: &SourceFile, diagnostic: &Diagnostic) -> String {
    let source_line = source_file
        .text
        .lines()
        .nth(diagnostic.line.saturating_sub(1))
        .unwrap_or("");
    let marker_width = diagnostic_marker_width(diagnostic, source_line);
    let caret = format!(
        "{}{}",
        " ".repeat(diagnostic.column.saturating_sub(1)),
        "^".repeat(marker_width)
    );

    format!(
        "{}:{}:{}: {} {}: {}\n{}\n{}\n",
        diagnostic.file_name,
        diagnostic.line,
        diagnostic.column,
        diagnostic.severity,
        diagnostic.code,
        diagnostic.message,
        source_line,
        caret
    )
}

#[must_use]
pub fn format_diagnostics(source_file: &SourceFile, diagnostics: &[Diagnostic]) -> String {
    diagnostics
        .iter()
        .map(|diagnostic| format_diagnostic(source_file, diagnostic))
        .collect()
}

fn diagnostic_marker_width(diagnostic: &Diagnostic, source_line: &str) -> usize {
    if diagnostic.span.start.line == diagnostic.span.end.line {
        return (diagnostic
            .span
            .end
            .column
            .saturating_sub(diagnostic.span.start.column))
        .max(1);
    }

    source_line
        .encode_utf16()
        .count()
        .saturating_sub(diagnostic.column)
        .saturating_add(1)
        .max(1)
}
