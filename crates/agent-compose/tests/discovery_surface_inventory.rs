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
//! It lives in this crate rather than in `compose-core` because half the binds
//! need the **command line**: the clap command list and the reports the binary
//! writes, which only the binary has, and the sources of both crates.
//!
//! # The binds
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
//! The explanations cite the grammar as well — a closing
//! `Grammar: docs/grammar.md §8.6, Decisions D31, D94` — through the same
//! parser, and every section and every decision **they** name is held to
//! existing too. Only *coverage* is the topics' alone: an explanation owes a
//! pointer that resolves, not a share of the curriculum. Without this, a
//! renumbered subsection or a retired decision leaves fifty-odd embedded
//! documents pointing at nothing, which is the drift class this file exists
//! for, one layer down.
//!
//! **(c) The CLI's own vocabulary, both ways round.** The `cli` topic names
//! every verb the binary has, read out of `--help` so it is clap's list rather
//! than a second one; every environment variable
//! `compose_core::docs::ENVIRONMENT` declares, which is itself held to naming
//! every `AGENT_COMPOSE_*` variable the sources mention; and every top-level key
//! `--format json` writes, read out of the reports themselves. The skill is held
//! to the verb half of the same rule, and to listing the curriculum. `--help`'s
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
//! clap makes required, read off that subcommand's own usage line and in the
//! two kinds that line has — bare positionals, and the flags whose value is
//! required, which is what `worker` takes and nothing else does. Kept apart
//! rather than totalled, because a flag's value fills that flag's slot and no
//! other: `agent-compose serve --port 8080` is a usage error, and a count that
//! summed words would read the `8080` as the path it is missing.
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
//!
//! **(d) The PRD pointer at the top of the stack.** The grammar's decision log
//! closes its entries with a rationale — `**Rationale**: PRD resolved q30` —
//! and the sources and fixtures cite the same questions to say what authority
//! a check is implementing. `prd.md` is this project's single source of truth
//! for design decisions, and its §9 log is the inventory those pointers resolve
//! against, so every `resolved q<n>` written anywhere under `docs/` or
//! `crates/` names an entry that log records. This is bind (b) one layer up:
//! a citation of a question the PRD never asked sends a reader who follows it
//! to a document that says no such decision was ever taken — and it is the
//! shape a feature implemented ahead of its ratification leaves behind, which
//! `CLAUDE.md` forbids outright ("never implement against an unresolved
//! question").
//!
//! **(e) One statement, written out three times.** A rule an author can meet in
//! the grammar, in a diagnostic's explanation and in a comment of the emitted
//! code is three copies of one claim, and an amendment that corrects one leaves
//! the other two contradicting the section they sit in — green, because a
//! document's fenced specs are run and its prose is not. The copies of Decision
//! D141's boundary are bound to each other here: after PRD resolved q58 what
//! stops at a coder node's boundary is the failover ladder, not the connection,
//! and no copy may go back to saying otherwise.

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

/// The argument slots clap **requires** of one verb, in the two kinds it has.
///
/// Kept apart rather than summed, because they are filled from different places
/// in a written invocation and a total cannot tell one from the other: a
/// document writing `agent-compose serve --port 8080` fills an *option's* slot
/// and leaves the positional empty, which is the usage error this whole bind
/// exists to catch, and a single count would read the `8080` as the path.
struct Required {
    /// How many bare positionals — `<PATH>`, and `run`'s `<PATH> <FLOW>`.
    positionals: usize,
    /// The flags whose value is required — `worker`'s `--hub` and `--token-env`.
    options: Vec<String>,
}

impl Required {
    /// Whether this verb can be written with the program name and no more.
    fn nothing(&self) -> bool {
        self.positionals == 0 && self.options.is_empty()
    }
}

/// What one verb requires, read out of that subcommand's own usage line.
///
/// `Usage: agent-compose build [OPTIONS] <PATH>` and `Usage: agent-compose
/// worker [OPTIONS] --hub <URL> --token-env <VAR>`: clap writes a required
/// positional in angle brackets, an optional one in square, and a required
/// option as the flag followed by its value in the same angle brackets — which
/// is why the two kinds are told apart *here*, by whether a flag precedes the
/// placeholder, rather than left to a reader of the count. Reading the line
/// rather than listing the answer here is the same discipline as [`verbs`]: a
/// second inventory is a second thing to keep in step, and adding a required
/// argument to a shipped verb is exactly the change that would make the
/// documents wrong without touching them.
fn required_arguments(verb: &str) -> Required {
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
    let words: Vec<&str> = usage.split_whitespace().collect();
    let mut required = Required {
        positionals: 0,
        options: Vec::new(),
    };
    let mut at = 0;
    while at < words.len() {
        let word = words[at];
        let placeholder = |held: &str| held.starts_with('<') && held.ends_with('>');
        if word.starts_with("--") && words.get(at + 1).is_some_and(|next| placeholder(next)) {
            required.options.push(word.to_string());
            at += 2;
            continue;
        }
        if placeholder(word) {
            required.positionals += 1;
        }
        at += 1;
    }
    required
}

/// The argument slots one written invocation fills, in the same two kinds.
///
/// **Positionals** are the leading run of words after the verb that are not a
/// flag, an optional group, or a trailing comment — because that is where a
/// positional goes in every form these documents write, and stopping at the
/// first flag is what keeps `agent-compose serve --port 8080` from reading the
/// `8080` as the path it is missing. A placeholder counts: `<path>` is a
/// document saying *a path goes here*, which is the thing being checked.
///
/// **Flags** are every long flag the invocation names, wherever it stands, with
/// the optional group's brackets and a repetition's ellipsis trimmed off it and
/// an `--flag=value` spelling cut at the `=`. Only the name is kept, because
/// what a required option owes is that the document names it at all — its value
/// is the placeholder beside it, and a flag written without one is a different
/// usage error this bind does not claim to catch.
struct Supplied {
    positionals: usize,
    flags: BTreeSet<String>,
}

fn supplied_arguments(words: &[String]) -> Supplied {
    let mut supplied = Supplied {
        positionals: 0,
        flags: BTreeSet::new(),
    };
    let mut leading = true;
    for word in words.iter().skip(1) {
        let word = word.as_str();
        if word == "#" {
            break;
        }
        let bare = word.trim_start_matches('[').trim_end_matches(['.', ']']);
        if bare.starts_with("--") {
            supplied
                .flags
                .insert(bare.split('=').next().unwrap_or(bare).to_string());
        }
        if word.starts_with('-') || word.starts_with('[') {
            leading = false;
            continue;
        }
        if leading {
            supplied.positionals += 1;
        }
    }
    supplied
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

/// Every decision the grammar's decision log records, as `D1`, `D2`, ….
///
/// Appendix A writes one per heading — `### D1. Imports are entrypoint-only and
/// non-transitive` — so the log itself is the inventory, and no list here can
/// disagree with it.
fn grammar_decisions() -> BTreeSet<String> {
    let text =
        fs::read_to_string(repository().join("docs/grammar.md")).expect("the grammar is readable");
    let mut found = BTreeSet::new();
    for line in text.lines() {
        let depth = line.len() - line.trim_start_matches('#').len();
        if depth < 2 {
            continue;
        }
        let Some(rest) = line[depth..].strip_prefix(' ') else {
            continue;
        };
        let Some(rest) = rest.strip_prefix('D') else {
            continue;
        };
        let number: String = rest.chars().take_while(char::is_ascii_digit).collect();
        if !number.is_empty() {
            found.insert(format!("D{number}"));
        }
    }
    assert!(
        found.len() >= 100,
        "the decision parser still finds the log's entries, found {}",
        found.len()
    );
    found
}

/// Every decision `text` names.
///
/// A `D` that starts a word and is followed by digits — which is how both
/// "Decision D43" and "Decisions D58, D111" are written, and which the word
/// `Decisions` itself does not match.
fn decisions(text: &str) -> BTreeSet<String> {
    let held: Vec<char> = text.chars().collect();
    let mut found = BTreeSet::new();
    for (at, character) in held.iter().enumerate() {
        if *character != 'D' || held[..at].last().is_some_and(|c| c.is_alphanumeric()) {
            continue;
        }
        let number: String = held[at + 1..]
            .iter()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        if !number.is_empty() {
            found.insert(format!("D{number}"));
        }
    }
    found
}

/// One explanation's closing cross-reference: its `Grammar:` line and whatever
/// follows.
///
/// Taken as the trailing block rather than as one line because the citation
/// list is prose that wraps, and a `§` can land on either side of the wrap.
fn cross_reference<'a>(code: &str, body: &'a str) -> &'a str {
    assert_eq!(
        body.matches("\nGrammar: ").count(),
        1,
        "`{code}`'s explanation closes with exactly one `Grammar:` line"
    );
    let at = body.find("\nGrammar: ").expect("the count above found one") + 1;
    &body[at..]
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
    sections(&format!("`{topic}`"), closing)
}

/// Every grammar section `text` names, expanding ranges.
///
/// Shared by the two documents that cite the grammar: a topic's
/// `Normative source:` line and an explanation's `Grammar:` line. One parser
/// rather than two, so a citation form one of them accepts is a citation the
/// other is held to as well.
fn sections(source: &str, text: &str) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    for token in text.split('§').skip(1) {
        let raw: String = token
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == '-' || *c == '\u{2013}')
            .collect();
        let raw = raw.trim_end_matches(['.', '-', '\u{2013}']);
        let (start, end) = match raw.split_once(['-', '\u{2013}']) {
            Some((start, end)) => (start, Some(end)),
            None => (raw, None),
        };
        assert!(!start.is_empty(), "{source} claims an empty section");
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
            "{source}'s range `{start}–{end}` spans two depths"
        );
        assert_eq!(
            head[..head.len() - 1],
            tail[..tail.len() - 1],
            "{source}'s range `{start}–{end}` is not between siblings"
        );
        let first: u32 = head[head.len() - 1]
            .parse()
            .unwrap_or_else(|_| panic!("{source}'s range starts at a number"));
        let last: u32 = tail[tail.len() - 1]
            .parse()
            .unwrap_or_else(|_| panic!("{source}'s range ends at a number"));
        assert!(
            first < last,
            "{source}'s range `{start}–{end}` does not go forwards"
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

/// (b) …and so does every section and decision an **explanation** cites.
///
/// The explanations close the same way the topics do — `Grammar:
/// `docs/grammar.md` §8.6, Decisions D31, D94` — and until this ran, that line
/// was bound by nothing: a renumbered subsection or a retired decision would
/// leave 57 embedded documents pointing at nothing, with the suite green. The
/// coverage half stays the topics' alone, because that is what the curriculum
/// is for; what an explanation owes is that the pointer it hands a reader
/// resolves.
#[test]
fn every_grammar_reference_an_explanation_makes_exists() {
    let headings = grammar_headings();
    let recorded = grammar_decisions();
    let mut cited = 0;
    for code in DiagnosticCode::ALL {
        let name = code.as_str();
        let source = format!("`{name}`'s explanation");
        let closing = cross_reference(name, docs::explanation(*code));

        let claimed = sections(&source, closing);
        assert!(
            !claimed.is_empty(),
            "{source} cites the grammar section its check comes from"
        );
        for claim in &claimed {
            assert!(
                headings.contains(claim),
                "{source} cites `§{claim}`, which `docs/grammar.md` does not have"
            );
        }
        for decision in decisions(closing) {
            assert!(
                recorded.contains(&decision),
                "{source} cites `{decision}`, which `docs/grammar.md`'s decision log does not have"
            );
        }
        cited += claimed.len();
    }
    // Every explanation cites at least one section and most cite several, so a
    // scan that found a handful would mean the parser, not the documents,
    // changed.
    assert!(
        cited >= 100,
        "the scan still finds the explanations' cross-references, found {cited}"
    );
}

/// (c) The `cli` topic names every verb the binary has.
#[test]
fn the_cli_topic_names_every_verb() {
    let topic = docs::topic("cli").expect("the curriculum has a `cli` topic");
    for verb in verbs() {
        assert!(
            names_command(topic.body, &verb),
            "the `cli` topic does not name `agent-compose {verb}`"
        );
    }
}

/// Whether `document` names `agent-compose <verb>` **as that verb**.
///
/// A bare substring would let an existing verb document a future one that is a
/// prefix of it: a verb named `doc` would be satisfied by the topic's own
/// `agent-compose docs`, and the topic would ship claiming to cover every verb
/// without a word written about it. So the match ends where a name ends —
/// [`is_a_name`] spells a name as lowercase letters and hyphens, and anything
/// else after the verb is the end of it.
fn names_command(document: &str, verb: &str) -> bool {
    let needle = format!("agent-compose {verb}");
    document.match_indices(&needle).any(|(at, _)| {
        document[at + needle.len()..]
            .chars()
            .next()
            .is_none_or(|next| !next.is_ascii_lowercase() && next != '-')
    })
}

/// (c) …and the check above is the one that catches that, pinned.
///
/// The rule it enforces is about a verb this binary does not have, so the only
/// way to hold it is to ask about one: `doc` and `valid` are the prefixes the
/// shipped topic would have answered for, and neither is documented.
#[test]
fn a_verb_is_not_documented_by_a_longer_one_that_starts_with_it() {
    let topic = docs::topic("cli").expect("the curriculum has a `cli` topic");
    assert!(
        names_command(topic.body, "docs"),
        "the `cli` topic names `agent-compose docs`"
    );
    for prefix in ["doc", "valid", "ini", "s"] {
        assert!(
            !names_command(topic.body, prefix),
            "`agent-compose {prefix}` is not a command the `cli` topic names"
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
    const ACT: &[&str] = &[
        "validate",
        "plan",
        "visualize",
        "build",
        "resume",
        "run",
        "serve",
        "worker",
    ];
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
/// Both sides come from clap: what a verb requires from its own usage line,
/// what a document supplies from what the document wrote. Table cells are out of
/// scope and the module header says why.
///
/// The two **kinds** of required argument are checked separately, and that is
/// what keeps the bind honest across the verbs: `serve` requires a positional
/// and `worker` requires two options, so a check that only counted words would
/// let `agent-compose serve --port 8080` pass on the strength of the `8080`.
#[test]
fn every_command_the_documents_tell_a_reader_to_run_carries_its_arguments() {
    let arity: BTreeMap<String, Required> = verbs()
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
            if required.nothing() {
                continue;
            }
            demanding += 1;
            let supplied = supplied_arguments(&words);
            let unmet: Vec<&str> = required
                .options
                .iter()
                .filter(|flag| !supplied.flags.contains(*flag))
                .map(String::as_str)
                .collect();
            assert!(
                unmet.is_empty(),
                "{document} writes `agent-compose {}`, which exits 2: `{verb}` requires {} and \
                 this names none of {unmet:?}. Name them — a placeholder counts as the value — or, \
                 if the sentence is about the verb rather than a command to run, write the verb \
                 alone without the program name",
                words.join(" "),
                required.options.join(", ")
            );
            assert!(
                supplied.positionals >= required.positionals,
                "{document} writes `agent-compose {}`, which exits 2: `{verb}` takes {} \
                 argument(s) and this names {}. Name them — a placeholder counts — or, if the \
                 sentence is about the verb rather than a command to run, write the verb alone \
                 without the program name",
                words.join(" "),
                required.positionals,
                supplied.positionals
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

/// …and the reader that guard is built on tells the two kinds of slot apart.
///
/// A guard over documents is only as sharp as its parse, and this one has a
/// failure mode that would not show up as a red test: widen the reading of
/// "supplied" and every document keeps passing while the usage errors it exists
/// to catch walk through. So the reading itself is asserted, on the shape that
/// separates the two — a flag's **value** is not a positional, and a required
/// **option** is met by its flag being named rather than by any word being
/// counted. `agent-compose serve --port 8080` is the case: it exits 2 with `the
/// following required arguments were not provided: <PATH>`, and a count that
/// summed words would read the `8080` as that path.
#[test]
fn the_argument_reader_does_not_take_a_flags_value_for_a_positional() {
    let read = |written: &str| {
        supplied_arguments(
            &written
                .split_whitespace()
                .map(str::to_string)
                .collect::<Vec<_>>(),
        )
    };

    // The value after a flag fills that flag's slot and no other.
    assert_eq!(read("serve --port 8080").positionals, 0);
    assert_eq!(read("build --out dist").positionals, 0);
    assert_eq!(read("run --input k=v").positionals, 0);
    // A positional the document did write is counted, flags beside it or not.
    assert_eq!(read("serve main.yml --port 8080").positionals, 1);
    assert_eq!(read("run main.yml flow.triage --input k=v").positionals, 2);
    assert_eq!(
        read("build <path> [--target <name>] [--check]").positionals,
        1
    );

    // A required option is met by its flag, wherever the flag stands and
    // however the document brackets it.
    let worker = read("worker --hub <url> --claim <name>... --token-env <VAR>");
    assert_eq!(worker.positionals, 0);
    for flag in ["--hub", "--claim", "--token-env"] {
        assert!(worker.flags.contains(flag), "{flag} was not read as a flag");
    }
    assert!(read("build <path> [--check]").flags.contains("--check"));
    assert!(read("skill --agent=claude").flags.contains("--agent"));

    // And the two verbs this pair is really about, against clap's own answer.
    let serve = required_arguments("serve");
    assert_eq!((serve.positionals, serve.options.len()), (1, 0));
    let worker = required_arguments("worker");
    assert_eq!(worker.positionals, 0);
    assert_eq!(worker.options, vec!["--hub", "--token-env"]);
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

/// (c) The `cli` topic names every key the machine report writes.
///
/// `--format json` is a contract with a script, and the topic is where the
/// script's author reads it. A key the report writes and the topic does not
/// name is a field nobody knows to read; a key the topic *stops* naming — which
/// is what happened when the report grew its `warnings` array and the paragraph
/// describing it kept saying `{"diagnostics": [ … ]}` — is worse, because a
/// reader who follows the documented shape concludes an empty `diagnostics`
/// means nothing was reported.
///
/// The keys are read out of the binary's own reports rather than listed here,
/// for [`verbs`]'s reason: a list in a test is a second inventory that can
/// disagree with the first.
#[test]
fn the_cli_topic_names_every_key_the_json_report_writes() {
    let topic = docs::topic("cli").expect("the curriculum has a `cli` topic");
    let out =
        std::env::temp_dir().join(format!("agent-compose-report-keys-{}", std::process::id()));
    let _ = fs::remove_dir_all(&out);
    let reports = [
        vec![
            "validate",
            "examples/review-loop/main.yml",
            "--format",
            "json",
        ],
        // `--check` writes nothing and reports the third key beside the two.
        vec![
            "build",
            "examples/review-loop/main.yml",
            "--out",
            out.to_str().expect("a UTF-8 temporary path"),
            "--check",
            "--format",
            "json",
        ],
    ];

    let mut keys: BTreeSet<String> = BTreeSet::new();
    for arguments in reports {
        let output: Output = Command::cargo_bin("agent-compose")
            .expect("the binary under test is built")
            .current_dir(repository())
            .args(&arguments)
            .output()
            .expect("the command runs");
        let text = String::from_utf8(output.stdout).expect("the report is UTF-8");
        let report: serde_json::Value = serde_json::from_str(&text).unwrap_or_else(|error| {
            panic!("`{}` writes a JSON report: {error}", arguments.join(" "))
        });
        keys.extend(
            report
                .as_object()
                .expect("the report is one object")
                .keys()
                .cloned(),
        );
    }
    assert!(
        keys.len() >= 3,
        "the reports still carry their keys, found {keys:?}"
    );

    for key in &keys {
        assert!(
            topic.body.contains(&format!("`{key}`")) || topic.body.contains(&format!("\"{key}\"")),
            "the `cli` topic does not name the report key `{key}`: {keys:?}"
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

/// The skill's verb table names every verb the CLI has.
///
/// The skill is installed into somebody else's agent and cannot be re-published
/// from here, so a verb it does not mention is one that agent will not reach
/// for. It is held to the two **registries** the binary owns — the verbs and
/// the topics — and to nothing else on purpose: the skill teaches the loop, and
/// a check over grammar rules would be asking it to restate what it
/// deliberately does not.
///
/// Scoped to the `Verbs` section, the way
/// [`the_skill_lists_the_curriculum_in_reading_order`] is scoped to `The
/// topics`. Over the whole document the check is satisfiable by prose — "Run
/// `agent-compose docs` for the list" would keep it green with the `docs` row
/// deleted — and the table is the part a reader consults for the verb they have
/// not met.
#[test]
fn the_skill_names_every_verb() {
    let section = docs::SKILL
        .split_once("\n## Verbs\n")
        .expect("the skill has a `Verbs` section")
        .1
        .split("\n## ")
        .next()
        .expect("the section ends");
    for verb in verbs() {
        // In code voice, which is how the table writes them: bare or followed by
        // the arguments the verb takes.
        assert!(
            section.contains(&format!("`{verb}`")) || section.contains(&format!("`{verb} ")),
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

/// (d) Every PRD question this project cites is one the PRD's §9 log records.
///
/// The pointer is normative — `docs/grammar.md`'s decision entries close on it,
/// the checks name it to say whose ruling they implement, and the fixtures name
/// it to say what a diagnostic is defending — so it has to resolve. A citation
/// of a question `prd.md` never asked is worse than no citation at all: the
/// reader spends the trip, and what they find at the end is that the wording
/// every rule is justified by exists nowhere they can consult or amend.
///
/// It is also the shape an area implemented ahead of its ratification leaves
/// behind, which is the process `CLAUDE.md` states and this bind is the machine
/// half of: a question is resolved into §9 *before* the affected area is built.
#[test]
fn every_prd_question_the_project_cites_is_one_the_prd_resolved() {
    let resolved = prd_resolved_questions();
    assert!(
        resolved.len() >= 29,
        "the PRD's §9 log is the inventory this bind reads, and it parsed as {resolved:?}"
    );

    let mut cited: BTreeMap<u32, String> = BTreeMap::new();
    for root in ["docs", "crates"] {
        prd_citations(&repository().join(root), &mut cited);
    }
    assert!(
        cited.len() >= 10,
        "the scan still finds this project's PRD citations, found {cited:?}"
    );

    for (question, file) in &cited {
        assert!(
            resolved.contains(question),
            "{file} cites `resolved q{question}`, which the PRD's §9 log does not record. \
             Resolve the question there — with the rationale in the section it belongs to — \
             before the area that cites it (`CLAUDE.md`, PRD §10). Recorded: {resolved:?}"
        );
    }
}

/// (e) The three copies of the **dropped `fallbackModel`** sentence agree about
/// what stops at a coder node's boundary (`docs/grammar.md` §8.9, Decisions
/// D141 and D143, PRD resolved q58).
///
/// One statement, written out three times because three audiences meet it in
/// three places: §8.9's reserved-options paragraph, the
/// `unknown-harness-setting` explanation an author reads after `settings: {
/// fallbackModel: … }`, and the comment over `CC_RESERVED` in the emitted
/// driver. Before resolved q58 all three said the same true thing — the
/// connection stops at the boundary — and after it the true thing is narrower:
/// the **failover ladder** stops, and the connection crosses. The amendment
/// corrected one copy and left two claiming the opposite of the section they
/// sit in, which nothing caught, because a `.md`'s fenced specs are run and its
/// prose is not.
///
/// So the sentence is bound rather than the wording: each copy has to say what
/// `fallbackModel` **is** — the failover ladder — where it says why the option
/// is dropped, and no copy of it may go back to claiming the connection stops
/// there. It is deliberately not an equality between the three texts: they are
/// written for three readers and a byte-identical sentence would be the wrong
/// bind. What must not differ is the claim.
#[test]
fn the_dropped_fallback_option_reads_as_the_ladder_in_every_copy() {
    // The claim, in the one form every copy has to be able to make: what the
    // option is. A copy that instead said the connection stops there is the
    // pre-q58 statement, and the assertion below names it.
    const LADDER: &str = "failover ladder";
    // Where the sentence is written, and the word each copy spells the option
    // with — prose in two of them, the SDK's own key in the third.
    const COPIES: &[(&str, &str)] = &[
        ("docs/grammar.md", "a fallback model"),
        (
            "crates/compose-core/src/docs/codes/unknown-harness-setting.md",
            "a fallback model",
        ),
        (
            "crates/compose-core/src/codegen/js/harness-cc.ts",
            "`fallbackModel` is",
        ),
    ];

    for (path, mention) in COPIES {
        let text = fs::read_to_string(repository().join(path))
            .unwrap_or_else(|error| panic!("{path} is readable: {error}"));
        let at = text
            .find(mention)
            .unwrap_or_else(|| panic!("{path} states why a fallback model is dropped"));
        // The sentence, not the document: a window wide enough to hold the
        // clause and narrow enough that a `failover ladder` somewhere else on
        // the page cannot satisfy it.
        let sentence = &text[at..text.len().min(at + 240)];
        assert!(
            sentence.contains(LADDER),
            "{path} says why a fallback model is dropped without saying it is the {LADDER}: \
             since PRD resolved q58 the ladder is what stops at a coder node's boundary and the \
             provider's connection crosses, so a copy that still puts the *connection* there \
             contradicts §8.9 and Decision D143 in the section that states them. The clause \
             reads: {sentence:?}"
        );
    }
}

/// Every question the PRD's resolved-questions log records.
///
/// §9 writes one per numbered list item — `29. **What does replay execute…` —
/// so the log itself is the inventory and no list here can disagree with it.
fn prd_resolved_questions() -> BTreeSet<u32> {
    let prd = fs::read_to_string(repository().join("prd.md")).expect("the PRD is readable");
    let mut inside = false;
    let mut resolved = BTreeSet::new();
    for line in prd.lines() {
        if let Some(heading) = line.strip_prefix("## ") {
            inside = heading.starts_with("9. Resolved Questions");
            continue;
        }
        if !inside {
            continue;
        }
        if let Some((number, _)) = line.split_once(". ")
            && let Ok(number) = number.parse::<u32>()
        {
            resolved.insert(number);
        }
    }
    resolved
}

/// Collect every PRD question cited under `directory`, with the first file that
/// cites it.
///
/// Only the files this project writes: a lockfile's base64 is full of `q` runs
/// followed by digits, and none of them is a claim about anything.
fn prd_citations(directory: &Path, cited: &mut BTreeMap<u32, String>) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if path.is_dir() {
            prd_citations(&path, cited);
            continue;
        }
        if !path
            .extension()
            .is_some_and(|held| matches!(held.to_str(), Some("rs" | "md" | "yml" | "yaml" | "ts")))
        {
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
        for question in questions(&text) {
            cited.entry(question).or_insert_with(|| name.clone());
        }
    }
}

/// Every PRD question `text` cites, expanding lists and ranges.
///
/// The claim forms this project writes: `resolved q30`, `resolved q19 and q20`,
/// `resolved q19/q20`, `resolved q26-q29`, and `PRD q24`. A bare `q29` inside a
/// paragraph that has already cited it is prose rather than a claim, and is
/// left alone — what is bound here is the form that says *the PRD settled this*,
/// which is the form a reader follows.
fn questions(text: &str) -> BTreeSet<u32> {
    let mut cited = BTreeSet::new();
    for marker in ["resolved q", "PRD q"] {
        for at in text.match_indices(marker).map(|(at, _)| at) {
            citation(&text[at + marker.len()..], &mut cited);
        }
    }
    cited
}

/// One citation's run of numbers, from the first digit after its marker.
fn citation(rest: &str, cited: &mut BTreeSet<u32>) {
    let mut chars = rest.chars().peekable();
    let mut ranged = false;
    let mut previous = None;
    loop {
        let mut digits = String::new();
        while chars.peek().is_some_and(char::is_ascii_digit) {
            digits.push(chars.next().expect("the digit that was just peeked"));
        }
        let Ok(number) = digits.parse::<u32>() else {
            return;
        };
        // A range covers its interior: `q26-q29` cites four questions, and the
        // two it does not name are the two a renumbering would strand.
        if ranged {
            for held in previous.unwrap_or(number)..=number {
                cited.insert(held);
            }
        }
        cited.insert(number);
        previous = Some(number);

        // What may follow one number and still belong to the same citation. The
        // `q` is required: `resolved q19 and the rest` ends at `19`, and only a
        // second `q` says another number is coming.
        let mut lookahead = chars.clone();
        let mut joiner = String::new();
        while let Some(&held) = lookahead.peek() {
            lookahead.next();
            if held == 'q' {
                break;
            }
            joiner.push(held);
            if joiner.chars().count() > 5 {
                return;
            }
        }
        if !matches!(
            joiner.as_str(),
            "-" | "\u{2013}" | "\u{2014}" | "/" | ", " | " and "
        ) {
            return;
        }
        ranged = matches!(joiner.as_str(), "-" | "\u{2013}" | "\u{2014}");
        chars = lookahead;
    }
}
