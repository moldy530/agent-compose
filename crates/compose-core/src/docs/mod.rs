//! Progressive discovery: everything the binary knows how to teach about
//! itself.
//!
//! PRD §7 M2's exit criterion is that a user downloads a released binary, hands
//! their coding agent the skill, and builds and runs flows locally — **no clone
//! of this repository**. Nothing in this module therefore reads a file at run
//! time: every document it serves is `include_str!`d into the binary at build
//! time, which is the single-static-binary posture of PRD 5.12 and also why
//! `validate` keeps its millisecond budget — a document costs binary size, and
//! nothing is parsed until the verb that prints it is the one that was typed.
//!
//! The surfaces, one submodule each (resolved q23):
//!
//! * [`schema`] — the published JSON Schema, byte for byte.
//! * [`scaffold`] — the project `init` writes.
//! * [`codes`] — one expanded explanation per diagnostic code, bound to the
//!   registry in [`crate::diag`] by an exhaustive `match`.
//! * [`topics`] — the curriculum: curated derivatives of `docs/grammar.md`,
//!   sized for a reader with a context window.
//! * [`skill`] — the installable skill, which teaches the loop rather than the
//!   grammar.

pub mod schema;

pub use schema::SCHEMA;
