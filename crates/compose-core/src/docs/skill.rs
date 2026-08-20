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
//! # One install, two roots
//!
//! Claude Code reads `.claude/skills/<name>/SKILL.md`; Codex reads
//! `.codex/skills/<name>/SKILL.md`. The format is the same portable `SKILL.md`
//! — YAML frontmatter, then the document — and both are auto-detected from a
//! directory whose whole content is skills. So there is one [`installed`]
//! document and one write, and the postures differ by the **root directory
//! alone**: `--agent` picks a row of [`AGENTS`], `--global` picks whether that
//! row's path hangs off the current directory or off `$HOME`, and a third agent
//! that adopts the convention is a row here and no new code.
//!
//! Writing is safe in both because neither path is a file of the user's to
//! damage: a refusal on a *differing* file plus a no-op on an identical one
//! covers both the customization and the re-install.

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
/// No frontmatter: frontmatter is a loader's convention rather than the
/// document's own, so the bare `skill` verb prints something a reader can paste
/// anywhere, and [`installed`] prepends the header an install wants.
pub const SKILL: &str = include_str!("../../../../docs/skill.md");

/// The agents `--agent` installs for, each beside the directory it keeps its
/// skills in — in the order a refusal lists them.
///
/// One row is the whole of a posture, because the postures differ by nothing
/// else: both agents load the same portable `SKILL.md` from
/// `<root>/skills/<name>/SKILL.md`, project-local or under `$HOME`.
pub const AGENTS: &[(&str, &str)] = &[("claude", ".claude"), ("codex", ".codex")];

/// The names `--agent` accepts, for the refusal that lists the vocabulary.
#[must_use]
pub fn agents() -> Vec<&'static str> {
    AGENTS.iter().map(|(name, _)| *name).collect()
}

/// The path an install writes for `agent`, relative to the current directory or
/// to `$HOME` — or `None` for an agent with no posture.
///
/// The one lookup: a caller that gets a path has an install to do, and a caller
/// that gets `None` has a name to refuse, so there is no third state where an
/// agent is supported but has nowhere to write.
#[must_use]
pub fn path(agent: &str) -> Option<String> {
    AGENTS
        .iter()
        .find(|(name, _)| *name == agent)
        .map(|(_, root)| format!("{root}/skills/{NAME}/SKILL.md"))
}

/// The closest supported agent to `name`, when one is close enough to suggest.
#[must_use]
pub fn nearest(name: &str) -> Option<&'static str> {
    crate::parse::reader::suggest(name, &agents())
}

/// The skill as an agent installs it: its frontmatter, then the document.
///
/// One document for every posture. The frontmatter format is portable across
/// the agents that read `SKILL.md`, so a per-agent variant would be a
/// difference this project invented and then had to keep in step.
#[must_use]
pub fn installed() -> String {
    format!("---\nname: {NAME}\ndescription: {DESCRIPTION}\n---\n\n{SKILL}")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The frontmatter is generated, so the two things a loader reads out of it
    /// have to be there and have to be one line each.
    #[test]
    fn the_install_carries_a_frontmatter_a_loader_can_read() {
        let installed = installed();
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

    /// Every supported agent has a path, every path is that agent's own root,
    /// and an unsupported name has none.
    ///
    /// This is what makes [`path`] the single lookup: an agent the CLI accepts
    /// and an agent the CLI can write for are the same set, so `--agent` can
    /// refuse on a `None` rather than consulting a second list that could
    /// disagree with this one.
    #[test]
    fn every_agent_installs_under_its_own_root() {
        let mut seen = std::collections::BTreeSet::new();
        for (name, root) in AGENTS {
            let path = path(name).expect("a supported agent has a path");
            assert_eq!(
                path,
                format!("{root}/skills/{NAME}/SKILL.md"),
                "the path shape is the same under every root"
            );
            assert!(seen.insert(root), "`{root}` is two agents' root");
        }
        assert_eq!(agents(), vec!["claude", "codex"]);
        assert!(
            path("nano").is_none(),
            "an agent with no posture has no path"
        );
    }

    /// The bare document is the one every posture is built from, so it carries
    /// no agent's header of its own.
    #[test]
    fn the_bare_skill_belongs_to_no_agent() {
        assert!(
            !SKILL.starts_with("---"),
            "the agent-agnostic document carries no frontmatter"
        );
        for (name, root) in AGENTS {
            assert!(
                !SKILL.contains(&format!("{root}/")),
                "and names no agent's skills directory, `{name}`'s included"
            );
        }
    }
}
