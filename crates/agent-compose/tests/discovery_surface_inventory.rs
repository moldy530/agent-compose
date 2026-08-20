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
//! **(c) The CLI's own vocabulary, both ways round.** The `cli` topic names
//! every verb the binary has, read out of `--help` so it is clap's list rather
//! than a second one; and every environment variable
//! `compose_core::docs::ENVIRONMENT` declares, which is itself held to naming
//! every `AGENT_COMPOSE_*` variable the sources mention. The skill is held to
//! the verb half of the same rule, and to listing the curriculum. `--help`'s
//! **order** is bound as well: clap prints subcommands in declaration order, so
//! that order is where the two-group split — five verbs that act on a
//! composition, then five that teach — is made rather than described, and it is
//! what a coding agent reads before it reads any of the three documents that
//! describe it.
//!
//! That direction — *the binary's vocabulary appears in the documents* — is
//! only half a bind, and the half that catches an addition. The other half
//! catches an **invention**: every `agent-compose <verb>` and
//! `agent-compose docs <topic>` written in code voice anywhere in the embedded
//! documents names something that exists. Without it a document can instruct a
//! reader to run a verb this compiler does not have, which is worse than an
//! undocumented verb — the reader spends a turn on a usage error, inside a
//! binary they cannot correct.
//!
//! A verb that exists is not yet a command that runs, so the same direction
//! also checks **arity**: an invocation names at least as many arguments as
//! clap makes required, counted from that subcommand's own usage line.
//! `agent-compose build` is every bit as much a usage error as
//! `agent-compose migrate` would be, and it is the likelier mistake, because
//! the verb is real and the sentence around it reads fine.
//!
//! Arity is checked where a reader is being told what to *type*, which is
//! everywhere in these documents except a **table cell**. English names a verb
//! as a noun — the `human` topic's "the terminal of an `agent-compose run`",
//! the `trace` topic's row for an "`agent-compose serve` status" — and where it
//! does, the invocation is the row's subject rather than a line to run; the
//! table's other columns say what about it. That carve is a structural proxy
//! for an intent, so it has a hole with a name: an invocation written inside a
//! table *is* instructing and misses its arguments. The hole is bounded — a
//! table cell is a poor place to put a command a reader should type — and the
//! alternative, requiring the arguments everywhere, would force the referential
//! sentences into prose that names the verb alone and reads worse for it.
//! Everything else, tables included, is bound: a table cell still cannot name a
//! verb that does not exist.
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
//! A topic may claim documents other than the grammar, and two do: `trace`
//! claims `docs/trace.md` **alone**, so it carries no `§` and contributes
//! nothing to coverage; `cli` claims `docs/plan.md` and `docs/trace.md` *and*
//! `§8.7, §14`, so it counts like any other topic. Coverage is therefore owned
//! by every topic except `trace` — which is what the failure below reports,
//! rather than a count in this comment that a new topic would falsify.

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
    let found: BTreeSet<String> = listed_verbs().into_iter().collect();
    assert!(
        found.len() >= 8,
        "the help parser still finds the verbs, found {found:?}"
    );
    found
}

/// The same verbs, **in the order `--help` prints them**.
///
/// Which is clap's declaration order, and is therefore a fact about the enum
/// that a reader of the enum has no reason to think anything depends on. One
/// thing does: the order is the grouping every document describes, and
/// [`the_help_lists_the_verbs_that_act_before_the_verbs_that_teach`] is what
/// holds the two together.
fn listed_verbs() -> Vec<String> {
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
    commands
        .lines()
        .filter_map(|line| line.split_whitespace().next())
        .filter(|verb| *verb != "help")
        .map(str::to_string)
        .collect()
}

/// How many positional arguments clap **requires** of one verb.
///
/// Read out of that subcommand's own usage line — `Usage: agent-compose build
/// [OPTIONS] <PATH>` — where clap writes a required positional in angle
/// brackets and an optional one in square. Reading it rather than listing it
/// here is the same discipline as [`verbs`]: a second inventory is a second
/// thing to keep in step, and adding a required argument to a shipped verb is
/// exactly the change that would make the documents wrong without touching
/// them.
fn required_arguments(verb: &str) -> usize {
    let output: Output = Command::cargo_bin("agent-compose")
        .expect("the binary under test is built")
        .args([verb, "--help"])
        .output()
        .expect("the command runs");
    let text = String::from_utf8(output.stdout).expect("help is UTF-8");
    let usage = text
        .lines()
        .find(|line| line.starts_with("Usage:"))
        .unwrap_or_else(|| panic!("`agent-compose {verb} --help` prints a usage line"));
    usage
        .split_whitespace()
        .filter(|word| word.starts_with('<') && word.ends_with('>'))
        .count()
}

/// How many positional arguments one written invocation supplies.
///
/// The leading run of words after the verb that are not flags, an optional
/// group, or a trailing comment — because that is where a positional goes in
/// every form these documents write, and stopping at the first flag keeps
/// `agent-compose build --check` from counting `--check` as the path it is
/// missing. A placeholder counts: `<path>` is a document saying *a path goes
/// here*, which is the thing being checked.
fn supplied_arguments(words: &[String]) -> usize {
    words
        .iter()
        .skip(1)
        .take_while(|word| {
            let word = word.as_str();
            !word.starts_with('-') && !word.starts_with('[') && word != "#"
        })
        .count()
}

/// A small count as the documents spell it.
///
/// They spell it in words — "Five verbs act on a composition" — so a bind on
/// that sentence has to as well. The panic is the honest failure for a surface
/// that outgrew the spelling: somebody rewrites the sentence, and this decides
/// what it may say.
fn spelled(count: usize) -> &'static str {
    match count {
        1 => "one",
        2 => "two",
        3 => "three",
        4 => "four",
        5 => "five",
        6 => "six",
        7 => "seven",
        8 => "eight",
        9 => "nine",
        10 => "ten",
        other => panic!("the documents count the verb groups in words, and {other} has no word"),
    }
}

/// `document` with its table rows blanked out.
///
/// A table cell is where these documents name a verb as a noun — "the terminal
/// of an `agent-compose run`" — so it is where an invocation is the subject
/// under discussion rather than a line to type. Blanked rather than deleted so
/// nothing runs together, and only outside a fence, where a leading `|` is
/// somebody's YAML rather than a table.
fn outside_tables(document: &str) -> String {
    let mut kept = String::new();
    let mut fenced = false;
    for line in document.lines() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
        } else if !fenced && line.trim_start().starts_with('|') {
            kept.push('\n');
            continue;
        }
        kept.push_str(line);
        kept.push('\n');
    }
    kept
}

/// Every document the binary carries, by the name a failure should call it.
///
/// The **embedded** text rather than the files, because that is what ships: a
/// file the registry does not carry is caught by the registry bind above, and
/// this one is about what a reader is told. The scaffold is YAML rather than
/// Markdown and is wrapped in a fence, which is the honest description of it —
/// the whole document is code voice, comments included.
fn embedded_documents() -> Vec<(String, String)> {
    let mut found: Vec<(String, String)> = docs::TOPICS
        .iter()
        .map(|topic| {
            (
                format!("the `{}` topic", topic.name),
                topic.body.to_string(),
            )
        })
        .collect();
    found.push(("the skill".to_string(), docs::SKILL.to_string()));
    found.push((
        "the `init` scaffold".to_string(),
        format!("```yaml\n{}\n```", docs::SCAFFOLD),
    ));
    for code in DiagnosticCode::ALL {
        found.push((
            format!("`{}`'s explanation", code.as_str()),
            docs::explanation(*code).to_string(),
        ));
    }
    found
}

/// Every fragment of `document` written in **code voice**: fenced lines, and
/// inline spans.
///
/// The distinction is the documents' own discipline and the reason a scan over
/// them is worth anything: a command a reader is meant to type is in a span or
/// a fence, and prose *about* the product — "an agent-compose project is YAML"
/// — is not. Scanning the prose too would fail on English sentences.
///
/// A fenced block is kept line by line, so an invocation never runs on into the
/// next command; the prose is flattened first, so an inline span that wrapped
/// across a line — `` `agent-compose docs\n<topic>` `` — is still one fragment.
fn code_voice(document: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut prose = String::new();
    let mut fenced = false;
    for line in document.lines() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
            continue;
        }
        if fenced {
            found.push(line.to_string());
        } else {
            prose.push_str(line);
            prose.push(' ');
        }
    }
    assert!(!fenced, "every fence in the document is closed");
    assert!(
        prose.matches('`').count().is_multiple_of(2),
        "every inline span in the document is closed"
    );
    found.extend(
        prose
            .split('`')
            .enumerate()
            .filter(|(index, _)| index % 2 == 1)
            .map(|(_, span)| span.to_string()),
    );
    found
}

/// Every `agent-compose …` invocation in `document`, as the words after the
/// program name.
fn invocations(document: &str) -> Vec<Vec<String>> {
    let mut found = Vec::new();
    for fragment in code_voice(document) {
        let words: Vec<&str> = fragment.split_whitespace().collect();
        for (at, word) in words.iter().enumerate() {
            if *word == "agent-compose" {
                found.push(
                    words[at + 1..]
                        .iter()
                        .map(|held| held.to_string())
                        .collect(),
                );
            }
        }
    }
    found
}

/// Whether `word` is a name this compiler could answer to, rather than a
/// placeholder or a flag.
///
/// `<topic>`, `[<dir>]` and `--version` are all things a document writes where
/// a name would go, and none of them is a claim that a name exists. A claim
/// looks like a name: a lowercase letter, then lowercase letters and hyphens,
/// which is the spelling every verb and every topic has.
///
/// Flags are therefore outside this bind, and stay outside it: a document
/// mostly names a flag in prose about it — "add `--format json` to any verb
/// that reports" — rather than inside an invocation, so binding the invocations
/// would check the few and miss the many. What is bound here is the two
/// **registries**: the verbs and the topics.
fn is_a_name(word: &str) -> bool {
    word.starts_with(|c: char| c.is_ascii_lowercase())
        && word.chars().all(|c| c.is_ascii_lowercase() || c == '-')
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

/// (c) …and `--help` prints them in the grouping the documents describe: the
/// five that act on a composition, then the five that teach.
///
/// `--help` is the first thing a coding agent reads, and clap prints
/// subcommands in **declaration order** — so `Command`'s variant order is the
/// only place the grouping is *made* rather than described. Three documents
/// describe it: this crate's module header, the `cli` topic's opening sentence,
/// and the skill's verb table. None of the three would go red if a verb were
/// declared in the wrong place, and a reader who meets `serve` among the
/// teaching verbs has been told two different things about the surface before
/// running a command.
///
/// The two lists here are not a second inventory: their union is held to the
/// verb registry, so a new verb fails this test until somebody says which group
/// it belongs to — the discipline
/// [`every_grammar_section_is_claimed_by_a_topic`] applies to the grammar.
#[test]
fn the_help_lists_the_verbs_that_act_before_the_verbs_that_teach() {
    const ACT: &[&str] = &["validate", "plan", "build", "run", "serve"];
    const TEACH: &[&str] = &["docs", "explain", "init", "schema", "skill"];

    let mut claimed: Vec<String> = ACT
        .iter()
        .chain(TEACH.iter())
        .map(|verb| (*verb).to_string())
        .collect();
    claimed.sort();
    let known: Vec<String> = verbs().into_iter().collect();
    assert_eq!(
        claimed, known,
        "every verb is in one of the two groups the documents describe; a new one goes in the \
         list it belongs to, and into the sentence in `docs/topics/cli.md` that counts them"
    );

    // And that sentence is held to the counts, or the instruction above is one
    // nothing enforces: a shipped topic would open by miscounting the surface
    // an agent reads before anything else.
    let counted = format!(
        "{} verbs act on a composition; {} teach",
        spelled(ACT.len()),
        spelled(TEACH.len())
    );
    let cli = docs::topic("cli").expect("the curriculum has a `cli` topic");
    assert!(
        cli.body.to_lowercase().contains(&counted),
        "the `cli` topic opens by counting the two groups and the count is now \"{counted}\""
    );

    let listed = listed_verbs();
    let last_acting = listed
        .iter()
        .rposition(|verb| ACT.contains(&verb.as_str()))
        .expect("`--help` lists the verbs that act on a composition");
    let first_teaching = listed
        .iter()
        .position(|verb| TEACH.contains(&verb.as_str()))
        .expect("`--help` lists the verbs that teach");
    assert!(
        last_acting < first_teaching,
        "`--help` prints {listed:?}, which interleaves the two groups; clap lists subcommands in \
         declaration order, so the fix is the order of `Command`'s variants"
    );
}

/// (c) …and every verb the documents name is one the binary has.
///
/// The other direction of the same bind, and the one that catches an
/// invention rather than an omission. `agent-compose migrate` shipped in two
/// explanations under a suite that only checked docs ⊇ clap: every verb was
/// documented, and one documented command was not a verb.
#[test]
fn the_documents_name_only_verbs_the_binary_has() {
    let verbs = verbs();
    for (document, text) in embedded_documents() {
        for words in invocations(&text) {
            let Some(verb) = words.first() else { continue };
            if !is_a_name(verb) {
                continue;
            }
            assert!(
                verbs.contains(verb),
                "{document} tells a reader to run `agent-compose {verb}`, which is not a verb \
                 this binary has: {verbs:?}"
            );
        }
    }
}

/// (c) …and every command they tell a reader to run is one that would run.
///
/// A verb that exists still exits `2` when its required arguments are missing,
/// and that failure is the likelier of the two: the verb is real, the sentence
/// reads fine, and nothing but clap notices. The skill shipped
/// `agent-compose build` in the middle of a loop step whose two siblings —
/// `agent-compose run main.yml flow.<name>` and `agent-compose serve main.yml`
/// — both carried their path, so it read as an invocation and was one word
/// short of being one. The skill is installed into somebody else's agent, so
/// there is no correcting it afterwards.
///
/// Both numbers come from clap: the required count from the verb's usage line,
/// the supplied count from what the document wrote. Table cells are out of
/// scope and the module header says why.
#[test]
fn every_command_the_documents_tell_a_reader_to_run_carries_its_arguments() {
    let arity: BTreeMap<String, usize> = verbs()
        .into_iter()
        .map(|verb| {
            let required = required_arguments(&verb);
            (verb, required)
        })
        .collect();
    let mut demanding = 0;
    for (document, text) in embedded_documents() {
        for words in invocations(&outside_tables(&text)) {
            let Some(verb) = words.first() else { continue };
            if !is_a_name(verb) {
                continue;
            }
            // A verb this binary does not have is the sibling test's failure,
            // reported there rather than as a confusing arity one here.
            let Some(required) = arity.get(verb) else {
                continue;
            };
            if *required == 0 {
                continue;
            }
            demanding += 1;
            let supplied = supplied_arguments(&words);
            assert!(
                supplied >= *required,
                "{document} writes `agent-compose {}`, which exits 2: `{verb}` takes {required} \
                 argument(s) and this names {supplied}. Name them — a placeholder counts — or, if \
                 the sentence is about the verb rather than a command to run, write the verb \
                 alone without the program name",
                words.join(" ")
            );
        }
    }
    // Every loop the documents teach ends in a verb that takes a path, so a
    // scan that found a handful would mean the scan, not the documents,
    // changed.
    assert!(
        demanding >= 15,
        "the scan still finds the commands that take arguments, found {demanding}"
    );
}

/// (c) …and every topic the documents point at is one the curriculum has.
///
/// The same hole over the other registry. Every explanation and every topic
/// closes with an `agent-compose docs <topic>` pointer, and a renamed topic
/// would leave those pointers exiting `2` in a released binary with the suite
/// green.
#[test]
fn the_documents_name_only_topics_the_curriculum_has() {
    let topics: BTreeSet<&str> = docs::topics::names().into_iter().collect();
    let mut checked = 0;
    for (document, text) in embedded_documents() {
        for words in invocations(&text) {
            if words.first().map(String::as_str) != Some("docs") {
                continue;
            }
            let Some(topic) = words.get(1) else { continue };
            if !is_a_name(topic) {
                continue;
            }
            assert!(
                topics.contains(topic.as_str()),
                "{document} points at `agent-compose docs {topic}`, which the curriculum does not \
                 have: {topics:?}"
            );
            checked += 1;
        }
    }
    // Every explanation and most topics close with one, so a scan that found a
    // handful would mean the reader stopped seeing them.
    assert!(
        checked >= 50,
        "the scan still finds the documents' topic pointers, found {checked}"
    );
}

/// (c) …and so does every command the **sources** name.
///
/// The documents are not the only place this compiler tells somebody to run
/// something: a `help:` line does it too, and a diagnostic's help is read far
/// more often than a topic is. `parse::spec_version` carried
/// "run `agent-compose migrate` to update a spec written for another version",
/// which is the same failure one layer earlier — the message a reader gets is
/// itself a document nobody can correct.
///
/// Code voice in a source file is a backtick, in a Rust doc comment and in a
/// diagnostic's own text alike, and this project writes every command that way.
/// Prose about the product — "run agent-compose specs" — carries none, so the
/// scan reads the marked half and leaves the sentences alone.
#[test]
fn the_sources_name_only_verbs_and_topics_that_exist() {
    let verbs = verbs();
    let topics: BTreeSet<&str> = docs::topics::names().into_iter().collect();
    let mut found = Vec::new();
    for crate_name in ["agent-compose", "compose-core"] {
        quoted_commands(
            &repository().join("crates").join(crate_name).join("src"),
            &mut found,
        );
    }
    assert!(
        found.len() >= 40,
        "the source scan still finds this project's own commands, found {}",
        found.len()
    );

    for (file, words) in &found {
        let Some(verb) = words.first() else { continue };
        if !is_a_name(verb) {
            continue;
        }
        assert!(
            verbs.contains(verb),
            "{file} names `agent-compose {verb}`, which is not a verb this binary has: {verbs:?}"
        );
        if verb != "docs" {
            continue;
        }
        let Some(topic) = words.get(1) else { continue };
        if !is_a_name(topic) {
            continue;
        }
        assert!(
            topics.contains(topic.as_str()),
            "{file} points at `agent-compose docs {topic}`, which the curriculum does not have: \
             {topics:?}"
        );
    }
}

/// Collect every backticked `agent-compose …` under `directory`, with the file
/// that wrote it.
fn quoted_commands(directory: &Path, found: &mut Vec<(String, Vec<String>)>) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if path.is_dir() {
            quoted_commands(&path, found);
            continue;
        }
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        let name = path
            .strip_prefix(repository())
            .unwrap_or(&path)
            .display()
            .to_string();
        // Line by line: a span that wrapped is a span whose first line names no
        // verb, and reading on into the next line of a source file would read a
        // sentence rather than the rest of the command.
        for line in text.lines() {
            for at in line.match_indices("`agent-compose ").map(|(at, _)| at) {
                let rest = &line[at + "`agent-compose ".len()..];
                let command = rest.split('`').next().unwrap_or_default();
                found.push((
                    name.clone(),
                    command
                        .split_whitespace()
                        .map(str::to_string)
                        .collect::<Vec<_>>(),
                ));
            }
        }
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
/// for. It is held to the two **registries** the binary owns — the verbs and
/// the topics — and to nothing else on purpose: the skill teaches the loop, and
/// a check over grammar rules would be asking it to restate what it
/// deliberately does not.
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

/// The skill lists the curriculum, in the curriculum's own order.
///
/// The list is written out in the document because the skill is one static
/// document — but it is a **registry** the binary owns rather than a rule the
/// grammar owns, so it costs one assertion to hold it exactly. Set equality and
/// order both: a reader is told these are "in reading order", and the index the
/// binary prints is what that order means.
#[test]
fn the_skill_lists_the_curriculum_in_reading_order() {
    let section = docs::SKILL
        .split_once("\n## The topics\n")
        .expect("the skill has a `The topics` section")
        .1
        .split("\n## ")
        .next()
        .expect("the section ends");
    let flattened = section.split_whitespace().collect::<Vec<_>>().join(" ");
    let listed: Vec<&str> = flattened
        .split('`')
        .enumerate()
        .filter(|(index, _)| index % 2 == 1)
        .map(|(_, span)| span)
        // The section names the verb that prints the list as well as the list.
        .filter(|span| !span.starts_with("agent-compose"))
        .collect();
    assert_eq!(
        listed,
        docs::topics::names(),
        "the skill's topic list and the curriculum have drifted apart"
    );
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
///
/// The sentence **count** is asserted rather than left to the name of this
/// test. A length bound alone lets the value grow into a paragraph under a test
/// whose name says it cannot, which is a doc comment and a check disagreeing
/// about the same constant.
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

    // A terminator with anything after it starts a second sentence. `main.yml`
    // is not one: the test is a terminator followed by a space, which is what
    // separates sentences and never what separates a stem from a suffix.
    let interior: Vec<usize> = description
        .char_indices()
        .filter(|(_, held)| matches!(held, '.' | '!' | '?'))
        .filter(|(at, _)| description[at + 1..].starts_with(' '))
        .map(|(at, _)| at)
        .collect();
    assert!(
        interior.is_empty(),
        "it is one sentence, and a terminator at {interior:?} ends another: `{description}`"
    );
}
