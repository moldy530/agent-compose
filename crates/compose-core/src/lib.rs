//! Compiler core for `agent-compose`: parsing, resolution, validation, the flat
//! IR, and codegen. The CLI in the `agent-compose` crate is a thin wrapper over
//! this library.
//!
//! The pipeline, in order:
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
//!   reads, and what is wrong with it (grammar 4.1);
//! * [`check`] — the validator: schema compatibility, bindings and wiring,
//!   fan-out shapes, store ops, provider settings and capabilities, trigger
//!   bindings, and the graph analyses — routing exhaustiveness, cycle
//!   termination, convergence, reachability, recursion, and interrupt-freedom;
//! * [`codegen`] — [`emit`], the pure function from a validated artifact to a
//!   deterministic set of `(path, contents)` files: a LangGraph TypeScript
//!   project, pinned per compiler release (PRD 5.12).
//!
//! Everything the parser reports is decidable from a single file, and
//! everything the resolver reports is decidable from names, files, and
//! addresses. Everything [`check`] reports needs the whole artifact: its
//! schemas and expressions, or its graphs. `docs/grammar.md` Appendix B is the
//! normative account of the split, and
//! `crates/compose-core/tests/static_check_inventory.rs` is that account made executable
//! — every static check PRD §7 M0 promises, mapped to the pass that decides it
//! and the diagnostic codes it reports through.
//!
//! [`codegen`] reports nothing at all: everything it could refuse, [`check`] has
//! already refused with a span to point at, so `build` validates first and emits
//! only on a clean report.
//!
//! The CLI over all of it is the `agent-compose` crate: `agent-compose validate
//! <path> [--target <name>] [--format human|json]`, which is the product's core
//! loop (PRD §7 M0), and `agent-compose build <path> [--target <name>]
//! [--out <dir>] [--check]`, which is codegen (PRD §7 M1).

pub mod ast;
pub mod cel;
pub mod check;
pub mod codegen;
pub mod diag;
pub mod ir;
pub mod parse;
pub mod resolve;
pub mod yaml;

pub use check::check;
pub use codegen::{COMPILER_VERSION, GeneratedFile, GeneratedProject, emit};
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
