//! Compiler core for `agent-compose`: parsing, resolution, validation, and
//! the flat IR. The CLI in the `agent-compose` crate is a thin wrapper over
//! this library.
//!
//! [`diag`] holds the [`Diagnostic`](diag::Diagnostic) type every pass reports
//! through: a stable machine-readable code, a severity, a message anchored to a
//! byte/line/column [`Span`](diag::Span), labelled secondary spans, and help
//! text. Rendering belongs to whatever reports them.

pub mod diag;

pub use diag::{Diagnostic, DiagnosticCode, Diagnostics, Severity, Span, Spanned};

/// The spec versions this compiler build supports, as accepted values of a
/// spec file's required `version:` field.
pub const SUPPORTED_SPEC_VERSIONS: &[&str] = &["0.1"];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supports_the_initial_spec_version() {
        assert!(SUPPORTED_SPEC_VERSIONS.contains(&"0.1"));
    }
}
