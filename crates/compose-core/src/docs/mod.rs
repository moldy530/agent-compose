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
//! * [`topics`] — the curriculum: curated derivatives of `docs/grammar.md`,
//!   sized for a reader with a context window.
//! * [`codes`] — one expanded explanation per diagnostic code, bound to the
//!   registry in [`crate::diag`] by an exhaustive `match`.
//! * [`schema`] — the published JSON Schema, byte for byte.
//! * [`scaffold`] — the project `init` writes.
//! * [`skill`] — the installable skill, which teaches the loop rather than the
//!   grammar.
//!
//! The three inventory tests that hold this surface to the compiler it
//! describes are `crates/agent-compose/tests/discovery_surface_inventory.rs`
//! (registries, grammar coverage, the CLI's own vocabulary) and
//! `crates/compose-core/tests/documented_examples_validate.rs` (every published
//! example, run).

pub mod codes;
pub mod scaffold;
pub mod schema;
pub mod skill;
pub mod topics;

pub use codes::explanation;
pub use scaffold::{SCAFFOLD, SCAFFOLD_NAME};
pub use schema::SCHEMA;
pub use skill::SKILL;
pub use topics::{TOPICS, Topic, index, topic};

/// Every environment variable this project's own tooling reads, and what reads
/// it.
///
/// The list exists to be *checked*, not to be printed: the `cli` topic is where
/// a reader meets these, and
/// `crates/agent-compose/tests/discovery_surface_inventory.rs` holds that topic
/// to naming every entry — plus, in the other direction, holds this list to
/// naming every `AGENT_COMPOSE_*` variable the sources mention. So a new
/// variable fails a test until somebody documents it.
///
/// Variables outside this project's own namespace are here by hand, because
/// nothing can enumerate them: `NO_COLOR` is a convention this compiler honours
/// rather than one it defines.
pub const ENVIRONMENT: &[(&str, &str)] = &[
    (
        "AGENT_COMPOSE_INTERACTIVE",
        "whether a `run` may answer a `human` pause at the terminal (grammar 8.7)",
    ),
    (
        "AGENT_COMPOSE_DATA_DIR",
        "where an emitted project keeps local stores and trace files",
    ),
    (
        "AGENT_COMPOSE_CALLBACK_RETRY",
        "the schedule a served app retries a callback delivery on (`docs/durability.md` §3.7)",
    ),
    (
        "NO_COLOR",
        "any non-empty value turns styling off on every report (no-color.org)",
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_environment_entry_is_named_and_explained() {
        let mut seen = std::collections::BTreeSet::new();
        for (name, meaning) in ENVIRONMENT {
            assert!(
                name.chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_'),
                "`{name}` is not an environment variable name"
            );
            assert!(!meaning.is_empty(), "`{name}` says what reads it");
            assert!(seen.insert(*name), "`{name}` is listed twice");
        }
    }
}
