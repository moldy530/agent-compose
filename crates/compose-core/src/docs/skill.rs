//! The installable agent skill: `agent-compose skill [--agent <name>]`.
//!
//! PRD §7 M2's exit criterion is that a user downloads a released binary,
//! **hands their coding agent the skill**, and builds and runs flows locally.
//! This is that document.
//!
//! # It teaches the loop, never the grammar
//!
//! The skill is installed into somebody else's agent, in somebody else's
//! repository, and this project cannot re-publish it when the grammar moves. A
//! grammar rule restated here is therefore a copy of a normative sentence in
//! the one artifact that can never be corrected — so the skill names topics
//! instead: it says `agent-compose docs routing`, and the binary the user has
//! answers with the routing rules that binary implements.
//!
//! What it does carry is the part that does not drift: what the product is, how
//! to verify the install, the authoring loop, a verb table, the exit codes, and
//! the pointer that the topics are the curriculum.
//!
//! # The two install postures
//!
//! They differ because the two agents' conventions do, not because one is
//! better served.
//!
//! * **Claude Code** reads `.claude/skills/<name>/SKILL.md`, a directory whose
//!   whole content is skills. So [`claude`] is **written**: there is no file of
//!   the user's to damage, and a refusal on a *differing* file plus a no-op on
//!   an identical one covers both the customization and the re-install.
//! * **Codex** reads `AGENTS.md`, a file the user writes, frequently holding
//!   instructions this compiler knows nothing about. So [`codex`] is
//!   **printed**, under a header saying where to put it. A compiler that edited
//!   `AGENTS.md` would be writing into a document it does not own.

/// The name the skill installs under.
pub const NAME: &str = "agent-compose";

/// The one sentence that decides when a coding agent reaches for the skill.
///
/// It names the artifacts a *user's request* would mention — a spec, a
/// `main.yml` agent graph, the CLI — rather than describing the skill. A
/// description that said "teaches the agent-compose CLI" would trigger only for
/// a user who already knows the product's name, which is the one user who does
/// not need it.
///
/// **One** sentence, and short enough for the conventions of a skill
/// description: `crates/agent-compose/tests/discovery_surface_inventory.rs`
/// holds it to both, the sentence count included, so the value and this comment
/// cannot drift apart.
pub const DESCRIPTION: &str = "Author, validate, and run agent-compose specs — the YAML DSL for \
                               agent graphs that compiles to LangGraph TypeScript — for any work \
                               on an agent-compose project, a main.yml agent graph, or the \
                               agent-compose CLI.";

/// The skill itself, agent-agnostic.
///
/// No frontmatter: frontmatter is one agent's convention rather than the
/// document's own, so the bare `skill` verb prints something a reader can paste
/// anywhere, and [`claude`] prepends the header its own loader wants.
pub const SKILL: &str = include_str!("../../../../docs/skill.md");

/// The agents `--agent` accepts.
pub const AGENTS: &[&str] = &["claude", "codex"];

/// The path `--agent claude` writes, relative to the current directory or to
/// `$HOME`.
pub const CLAUDE_PATH: &str = ".claude/skills/agent-compose/SKILL.md";

/// The closest supported agent to `name`, when one is close enough to suggest.
#[must_use]
pub fn nearest(name: &str) -> Option<&'static str> {
    crate::parse::reader::suggest(name, AGENTS)
}

/// The skill as Claude Code installs it: its frontmatter, then the document.
#[must_use]
pub fn claude() -> String {
    format!("---\nname: {NAME}\ndescription: {DESCRIPTION}\n---\n\n{SKILL}")
}

/// The skill as Codex takes it: the document under a header saying where it
/// goes.
///
/// Printed, never written — see the module header.
#[must_use]
pub fn codex() -> String {
    format!(
        "<!--\n\
         agent-compose skill, for Codex.\n\
         \n\
         Codex reads `AGENTS.md` from the repository root. Either paste the document below into\n\
         a section of your `AGENTS.md`, or save it beside it — say as\n\
         `docs/agent-compose-skill.md` — and add one line to `AGENTS.md` pointing at it:\n\
         \n\
         \x20   When working with agent-compose specs, follow `docs/agent-compose-skill.md`.\n\
         \n\
         This command writes nothing: `AGENTS.md` is yours.\n\
         -->\n\
         \n\
         {SKILL}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The frontmatter is generated, so the two things a loader reads out of it
    /// have to be there and have to be one line each.
    #[test]
    fn the_claude_install_carries_a_frontmatter_a_loader_can_read() {
        let installed = claude();
        assert!(installed.starts_with("---\n"), "frontmatter comes first");
        let header = installed
            .split("\n---\n")
            .next()
            .expect("a frontmatter block");
        assert!(header.contains(&format!("name: {NAME}")));
        assert!(header.contains(&format!("description: {DESCRIPTION}")));
        assert!(
            !DESCRIPTION.contains('\n'),
            "the description is one line, or the frontmatter stops being YAML"
        );
        assert!(
            installed.ends_with(SKILL),
            "the document itself is unchanged by the install"
        );
    }

    /// The Codex posture's whole content is the instruction, so it has to name
    /// the file it is telling the reader to edit — and has to say it will not
    /// edit it.
    #[test]
    fn the_codex_install_says_where_the_document_goes() {
        let printed = codex();
        assert!(printed.contains("AGENTS.md"), "it names Codex's surface");
        assert!(
            printed.contains("writes nothing"),
            "it says this command does not edit that file"
        );
        assert!(printed.ends_with(SKILL), "the document itself follows");
    }

    /// The bare document is the one every posture is built from, so it carries
    /// no agent's header of its own.
    #[test]
    fn the_bare_skill_belongs_to_no_agent() {
        assert!(
            !SKILL.starts_with("---"),
            "the agent-agnostic document carries no frontmatter"
        );
        assert!(!SKILL.contains("AGENTS.md"), "and names no agent's surface");
        assert!(!SKILL.contains(".claude/"), "nor another's");
    }
}
