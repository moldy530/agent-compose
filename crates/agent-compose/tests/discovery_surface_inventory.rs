//! The discovery surface's inventory: every document the binary teaches from,
//! mapped to the thing it describes.
//!
//! `tests/acceptance_inventory.rs`, `compose-core`'s
//! `tests/static_check_inventory.rs`, `tests/trace_format_inventory.rs` and
//! `tests/plan_format_inventory.rs` are the same idea over their own surfaces.
//! This one is over the surface PRD §7 M2 calls progressive discovery
//! (resolved q23), and it exists because that surface is **derivative**: the
//! topics are derived from `docs/grammar.md`, the explanations from the
//! diagnostic registry, the skill from the CLI. A derivative drifts silently,
//! and a released binary is the one artifact a reader cannot correct.
//!
//! It lives in this crate rather than in `compose-core` because two of the
//! three binds need the **command line**: the clap command list, which only the
//! binary has, and the sources of both crates.
//!
//! # The three binds
//!
//! **(a) Registry against directory.** Every file under `docs/topics/` is
//! registered in `compose_core::docs::topics::TOPICS` and every registration
//! has a file; every file under `src/docs/codes/` is reachable from
//! `docs::explanation` and every code's explanation is one of those files. The
//! second direction of the explanations is a compile error already — the
//! `match` is exhaustive — so what is added here is the *first*: an orphan file
//! nothing reads, which compiles fine and ships as dead bytes.
//!
//! **(b) Grammar coverage.** Every numbered heading of `docs/grammar.md` is
//! claimed by at least one topic's `Normative source` line, and every claim
//! names a heading that exists. A section may be claimed by several topics —
//! `§8.7` belongs to `human` and to `cli` alike — and the failure names the
//! unclaimed ones, so a new grammar subsection forces a decision about which
//! topic teaches it rather than quietly falling out of the curriculum.
//!
//! **(c) The CLI's own vocabulary.** The `cli` topic names every verb the
//! binary has, read out of `--help` so it is clap's list rather than a second
//! one; and every environment variable `compose_core::docs::ENVIRONMENT`
//! declares, which is itself held to naming every `AGENT_COMPOSE_*` variable
//! the sources mention. The skill is held to the verb half of the same rule.
//!
//! # What a claim looks like
//!
//! A topic's last non-empty line is
//!
//! ```text
//! Normative source: `docs/grammar.md` §1, §1.1–1.7, §2, §2.1–2.3, §2.5
//! ```
//!
//! A claim is one heading number, or a **range of siblings** written with an en
//! dash or a hyphen — `§1.1–1.7` is `1.1` through `1.7`. A range expands over
//! the last component only, so `§7.6.1–7.6.4` is four headings and `§1–2` is
//! two. Ranges keep the line readable without weakening the bind, because a
//! claim naming a heading that does not exist is itself a failure: an
//! aspirational `§8.1–8.20` fails on `8.9`.
//!
//! Two topics claim documents other than the grammar (`docs/trace.md`,
//! `docs/plan.md`) and carry no `§` at all. That is legal and contributes
//! nothing to coverage, which is why the other sixteen have to cover the whole
//! of it between them.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Output;

use assert_cmd::Command;
use compose_core::DiagnosticCode;
use compose_core::docs;

/// The repository root.
fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("the manifest directory has a grandparent")
        .to_path_buf()
}

/// Every `.md` file directly under `directory`, by file stem.
fn documents(directory: &Path) -> BTreeMap<String, String> {
    fs::read_dir(directory)
        .unwrap_or_else(|error| panic!("{} is readable: {error}", directory.display()))
        .map(|entry| entry.expect("a directory entry").path())
        .filter(|path| path.extension().is_some_and(|held| held == "md"))
        .map(|path| {
            let name = path
                .file_stem()
                .expect("a file name")
                .to_string_lossy()
                .into_owned();
            (
                name,
                fs::read_to_string(&path).expect("a readable document"),
            )
        })
        .collect()
}

/// What the binary's own `--help` says its verbs are.
///
/// Read from the command line rather than from a list in a test, because a list
/// in a test is a second inventory to keep in step — and the one thing this
/// file exists to prevent is two inventories that can disagree. `help` is
/// clap's own and is dropped: it is not a verb of this compiler.
fn verbs() -> BTreeSet<String> {
    let output: Output = Command::cargo_bin("agent-compose")
        .expect("the binary under test is built")
        .arg("--help")
        .output()
        .expect("the command runs");
    let text = String::from_utf8(output.stdout).expect("help is UTF-8");
    let commands = text
        .split("Commands:\n")
        .nth(1)
        .expect("the help lists commands")
        .split("\n\n")
        .next()
        .expect("the command list ends");
    let found: BTreeSet<String> = commands
        .lines()
        .filter_map(|line| line.split_whitespace().next())
        .filter(|verb| *verb != "help")
        .map(str::to_string)
        .collect();
    assert!(
        found.len() >= 8,
        "the help parser still finds the verbs, found {found:?}"
    );
    found
}

/// Every numbered heading of the grammar, as it is written after the `#`s.
///
/// A heading counts when its text starts with a dotted decimal number followed
/// by a space — `## 1. Document model`, `### 7.6.1 Exclusive edges`. The
/// appendices are named rather than numbered and are correctly excluded: they
/// hold decision entries and editor notes, which are cross-referenced from
/// topics but are not a section of the language.
fn grammar_headings() -> BTreeSet<String> {
    let text =
        fs::read_to_string(repository().join("docs/grammar.md")).expect("the grammar is readable");
    let mut found = BTreeSet::new();
    for line in text.lines() {
        // `##`, `###` and `####` all carry sections; `#` is the document title.
        let depth = line.len() - line.trim_start_matches('#').len();
        if depth < 2 {
            continue;
        }
        let Some(rest) = line[depth..].strip_prefix(' ') else {
            continue;
        };
        let Some((number, _)) = rest.split_once(' ') else {
            continue;
        };
        let number = number.trim_end_matches('.');
        if !number.is_empty()
            && number
                .split('.')
                .all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_digit()))
        {
            found.insert(number.to_string());
        }
    }
    assert!(
        found.len() >= 60,
        "the heading parser still finds the grammar's sections, found {}",
        found.len()
    );
    found
}

/// The grammar sections one topic claims on its `Normative source` line.
///
/// Returns nothing for a topic whose normative source is another document.
fn claims(topic: &str, body: &str) -> BTreeSet<String> {
    let line = body
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .unwrap_or_default();
    let closing = body
        .lines()
        .rev()
        .find(|line| line.starts_with("Normative source:"))
        .unwrap_or_else(|| panic!("`{topic}` ends with a `Normative source:` line"));
    assert_eq!(
        closing, line,
        "`{topic}`'s `Normative source:` line is its last line"
    );

    let mut found = BTreeSet::new();
    for token in closing.split('§').skip(1) {
        let raw: String = token
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == '-' || *c == '\u{2013}')
            .collect();
        let raw = raw.trim_end_matches(['.', '-', '\u{2013}']);
        let (start, end) = match raw.split_once(['-', '\u{2013}']) {
            Some((start, end)) => (start, Some(end)),
            None => (raw, None),
        };
        assert!(!start.is_empty(), "`{topic}` claims an empty section");
        found.insert(start.to_string());
        let Some(end) = end else { continue };

        // A range expands over its last component: `§7.6.1–7.6.4` is four
        // headings, and the two ends must be siblings for that to mean
        // anything.
        let head: Vec<&str> = start.split('.').collect();
        let tail: Vec<&str> = end.split('.').collect();
        assert_eq!(
            head.len(),
            tail.len(),
            "`{topic}`'s range `{start}–{end}` spans two depths"
        );
        assert_eq!(
            head[..head.len() - 1],
            tail[..tail.len() - 1],
            "`{topic}`'s range `{start}–{end}` is not between siblings"
        );
        let first: u32 = head[head.len() - 1]
            .parse()
            .unwrap_or_else(|_| panic!("`{topic}`'s range starts at a number"));
        let last: u32 = tail[tail.len() - 1]
            .parse()
            .unwrap_or_else(|_| panic!("`{topic}`'s range ends at a number"));
        assert!(
            first < last,
            "`{topic}`'s range `{start}–{end}` does not go forwards"
        );
        let prefix = head[..head.len() - 1].join(".");
        for number in first..=last {
            found.insert(if prefix.is_empty() {
                number.to_string()
            } else {
                format!("{prefix}.{number}")
            });
        }
    }
    found
}

/// (a) Every topic file is registered, and every registration has a file.
#[test]
fn the_topic_registry_and_the_topics_directory_are_the_same_set() {
    let on_disk = documents(&repository().join("docs/topics"));
    let registered: BTreeSet<&str> = docs::topics::names().into_iter().collect();
    let files: BTreeSet<&str> = on_disk.keys().map(String::as_str).collect();
    assert_eq!(
        registered, files,
        "`docs/topics/` and `compose_core::docs::topics::TOPICS` name different topics"
    );

    // And the registration carries that file, not another one: a copy-paste in
    // the registry would otherwise leave two names serving one document.
    for (name, body) in &on_disk {
        let held = docs::topic(name).expect("a registered topic");
        assert_eq!(
            held.body, body,
            "`{name}` is registered against a different document"
        );
    }
}

/// (a) Every explanation file is reachable from the registry.
///
/// The other direction is a compile error — `docs::explanation` is exhaustive
/// over `DiagnosticCode` — so this closes the half that compiles: a file under
/// `src/docs/codes/` that nothing includes, which ships as bytes nobody can
/// reach.
#[test]
fn the_explanation_registry_and_the_codes_directory_are_the_same_set() {
    let on_disk = documents(&repository().join("crates/compose-core/src/docs/codes"));
    let files: BTreeSet<&str> = on_disk.keys().map(String::as_str).collect();
    let codes: BTreeSet<&str> = DiagnosticCode::ALL
        .iter()
        .map(|code| code.as_str())
        .collect();
    assert_eq!(
        codes, files,
        "`src/docs/codes/` and the diagnostic registry name different codes"
    );

    for code in DiagnosticCode::ALL {
        let name = code.as_str();
        assert_eq!(
            docs::explanation(*code),
            on_disk[name],
            "`{name}` is registered against a different document"
        );
    }
}

/// (b) Every numbered section of the grammar is claimed by some topic.
///
/// The failure names the unclaimed sections, which is the point: a grammar
/// addition should read as "nothing teaches §8.9 yet", not as a number that
/// went down.
#[test]
fn every_grammar_section_is_claimed_by_a_topic() {
    let headings = grammar_headings();
    let mut claimed: BTreeSet<String> = BTreeSet::new();
    for topic in docs::TOPICS {
        claimed.extend(claims(topic.name, topic.body));
    }

    let unclaimed: Vec<&String> = headings.difference(&claimed).collect();
    assert!(
        unclaimed.is_empty(),
        "no topic claims these grammar sections: {unclaimed:?} — add each to the `Normative \
         source:` line of the topic that teaches it, or write the topic that should"
    );
}

/// (b) Every claim names a section the grammar has.
///
/// Without this the coverage check above is trivially satisfiable: a topic
/// could claim `§1–99` and cover everything, including the sections nobody
/// wrote.
#[test]
fn every_claimed_grammar_section_exists() {
    let headings = grammar_headings();
    for topic in docs::TOPICS {
        for claim in claims(topic.name, topic.body) {
            assert!(
                headings.contains(&claim),
                "`{}` claims `§{claim}`, which `docs/grammar.md` does not have",
                topic.name
            );
        }
    }
}

/// (c) The `cli` topic names every verb the binary has.
#[test]
fn the_cli_topic_names_every_verb() {
    let topic = docs::topic("cli").expect("the curriculum has a `cli` topic");
    for verb in verbs() {
        assert!(
            topic.body.contains(&format!("agent-compose {verb}")),
            "the `cli` topic does not name `agent-compose {verb}`"
        );
    }
}

/// (c) The `cli` topic names every environment variable this project reads.
#[test]
fn the_cli_topic_names_every_environment_variable() {
    let topic = docs::topic("cli").expect("the curriculum has a `cli` topic");
    for (name, _) in docs::ENVIRONMENT {
        assert!(
            topic.body.contains(name),
            "the `cli` topic does not name `{name}`"
        );
    }
}

/// (c) …and the list it is held to names every variable the sources mention.
///
/// The other half of the environment bind, and the half that makes the first
/// one worth having: a new `AGENT_COMPOSE_*` variable would otherwise be
/// documented by nobody and checked by nothing. Only this project's own
/// namespace can be enumerated — `NO_COLOR` is a convention this compiler
/// honours rather than one it defines, so it is in the list by hand.
#[test]
fn the_environment_list_names_every_variable_the_sources_read() {
    let declared: BTreeSet<&str> = docs::ENVIRONMENT.iter().map(|(name, _)| *name).collect();
    let mut mentioned = BTreeSet::new();
    for crate_name in ["agent-compose", "compose-core"] {
        walk(
            &repository().join("crates").join(crate_name).join("src"),
            &mut mentioned,
        );
    }
    assert!(
        !mentioned.is_empty(),
        "the source scan still finds this project's environment variables"
    );
    let undeclared: Vec<&String> = mentioned
        .iter()
        .filter(|name| !declared.contains(name.as_str()))
        .collect();
    assert!(
        undeclared.is_empty(),
        "these environment variables are read but not in `docs::ENVIRONMENT`: {undeclared:?}"
    );
}

/// Collect every `AGENT_COMPOSE_*` token under `directory`.
fn walk(directory: &Path, found: &mut BTreeSet<String>) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if path.is_dir() {
            walk(&path, found);
            continue;
        }
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        let mut rest = text.as_str();
        while let Some(at) = rest.find("AGENT_COMPOSE_") {
            let tail = &rest[at..];
            let name: String = tail
                .chars()
                .take_while(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || *c == '_')
                .collect();
            // Prose about the namespace — "every `AGENT_COMPOSE_*` variable" —
            // stops at the prefix and names no variable.
            if name.len() > "AGENT_COMPOSE_".len() {
                found.insert(name);
            }
            rest = &tail[1..];
        }
    }
}

/// The skill names every verb the CLI has.
///
/// The skill is installed into somebody else's agent and cannot be re-published
/// from here, so a verb it does not mention is one that agent will not reach
/// for. It is held to the verbs and to nothing else on purpose: the skill
/// teaches the loop, and a check over grammar rules would be asking it to
/// restate what it deliberately does not.
#[test]
fn the_skill_names_every_verb() {
    for verb in verbs() {
        // In code voice, which is how the skill's verb table writes them. A
        // looser match would be satisfied by prose: `run` appears in an
        // ordinary English sentence three times before the table.
        assert!(
            docs::SKILL.contains(&format!("`{verb}`"))
                || docs::SKILL.contains(&format!("`{verb} ")),
            "the skill's verb table does not name `{verb}`"
        );
    }
}

/// A guard on the two coverage checks: they are only worth something if the two
/// sets they compare are the size the documents actually are.
///
/// A heading parser that silently found four would make coverage trivial, and a
/// claim reader that silently found none would make the existence check
/// vacuous. Both counts are asserted as floors rather than as snapshots, so a
/// new grammar section is not a failure here — only in the check that names it.
#[test]
fn the_two_sides_of_the_coverage_check_are_both_populated() {
    let headings = grammar_headings();
    assert!(
        headings.len() >= 80,
        "the grammar has its sections, found {}",
        headings.len()
    );
    let mut claimed = BTreeSet::new();
    let mut claiming_topics = 0;
    for topic in docs::TOPICS {
        let held = claims(topic.name, topic.body);
        if !held.is_empty() {
            claiming_topics += 1;
        }
        claimed.extend(held);
    }
    assert!(
        claiming_topics >= 15,
        "most topics derive from the grammar, found {claiming_topics}"
    );
    assert!(
        claimed.len() >= headings.len(),
        "the claims cover the headings, {} against {}",
        claimed.len(),
        headings.len()
    );
}

/// The skill's frontmatter description stays one sentence of a length a skill
/// loader is happy with.
///
/// It is what decides when an agent reaches for the skill, and it is read as a
/// single line of YAML: a newline would end the value early, and a paragraph
/// would be a description nobody's picker shows.
#[test]
fn the_skill_description_is_one_short_sentence() {
    let description = docs::skill::DESCRIPTION;
    assert!(!description.contains('\n'), "it is one line");
    assert!(
        description.len() <= 1024,
        "it is within a skill description's conventional length, found {}",
        description.len()
    );
    assert!(
        description.ends_with('.'),
        "it reads as a sentence, found `{description}`"
    );
    assert!(
        description.contains("agent-compose"),
        "it names the product a user would mention"
    );
}
