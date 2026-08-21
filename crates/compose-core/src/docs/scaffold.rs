//! What `agent-compose init` writes.
//!
//! One file, because a project *is* one file until it outgrows one (grammar
//! 1.6): `imports:` exists so that a spec can be split, not so that it must
//! start split, and a scaffold that arrived as six files would teach the
//! layout before it taught the language.
//!
//! Every line that carries a decision carries a comment naming the grammar
//! section that decided it, so the file doubles as the first topic an agent
//! reads — it is the artifact in front of them, and a pointer from there into
//! `agent-compose docs <topic>` costs one line.

/// The scaffold's file name: the conventional entrypoint (grammar 1.6).
pub const SCAFFOLD_NAME: &str = "main.yml";

/// The spec `init` writes.
///
/// It **validates clean under the built-in `local` target**, which is the whole
/// contract:
/// `crates/compose-core/tests/documented_examples_validate.rs` runs it through
/// the same resolver and checker `validate` runs, and
/// `crates/agent-compose/tests/discovery_cli.rs` runs the two commands back to
/// back through the real binary. A scaffold that did not validate would make an
/// agent's first `validate` a report about this compiler's own file.
pub const SCAFFOLD: &str = include_str!("scaffold.yml");

#[cfg(test)]
mod tests {
    use super::*;

    /// The comments are the teaching, so they are held to pointing somewhere
    /// real: the two commands a reader is told to run next.
    #[test]
    fn the_scaffold_names_the_commands_it_tells_the_reader_to_run() {
        assert!(SCAFFOLD.contains("agent-compose validate main.yml"));
        assert!(SCAFFOLD.contains("agent-compose docs"));
        assert!(
            SCAFFOLD.contains("flow.summarize"),
            "the `run` line names a flow the file defines"
        );
    }
}
