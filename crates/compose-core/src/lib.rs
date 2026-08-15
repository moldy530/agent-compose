//! Compiler core for `agent-compose`: parsing, resolution, validation, and
//! the flat IR. The CLI in the `agent-compose` crate is a thin wrapper over
//! this library.
//!
//! What exists today is the front half of the pipeline:
//!
//! * [`diag`] — the [`Diagnostic`](diag::Diagnostic) type every pass reports
//!   through, with stable machine-readable codes and byte/line/column spans;
//! * [`yaml`] — the YAML 1.2 profile of `docs/grammar.md` §1.1, loaded into a
//!   fully spanned tree;
//! * [`ast`] — the typed, span-carrying shape of one spec or deploy file;
//! * [`parse`] — [`parse_file`]/[`parse_str`], turning one file into that AST
//!   plus diagnostics;
//! * [`resolve`] — [`resolve`](resolve()), following the entrypoint's
//!   `imports:` and binding every name to what it names;
//! * [`ir`] — the flat, self-contained JSON artifact resolution produces
//!   (PRD 5.1);
//! * [`cel`] — the expression front-end: one CEL string in, its type, what it
//!   reads, and what is wrong with it (grammar 4.1).
//!
//! Everything the parser reports is decidable from a single file, and
//! everything the resolver reports is decidable from names, files, and
//! addresses. The static checks that read the artifact — schema compatibility,
//! CEL typing, and then the graph analyses — are the pass after it;
//! `docs/grammar.md` Appendix B is the normative account of that split.

pub mod ast;
pub mod cel;
pub mod diag;
pub mod ir;
pub mod parse;
pub mod resolve;
pub mod yaml;

pub use diag::{Diagnostic, DiagnosticCode, Diagnostics, Severity, Span, Spanned};
pub use ir::{IR_VERSION, Ir};
pub use parse::{ParsedFile, parse_file, parse_str};
pub use resolve::{DEFAULT_TARGET, Resolution, resolve, resolve_with_target};

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
