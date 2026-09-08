//! The curriculum: `agent-compose docs [<topic>]`.
//!
//! Topics are **curated derivatives** of `docs/grammar.md`, not slices of it,
//! and that is the load-bearing choice. The grammar is normative and is written
//! to be *complete* — every rule, every rationale, every decision cross-
//! reference — which is exactly the wrong shape for a reader with a context
//! window. A topic is example-led: one screen of runnable YAML first, then the
//! rules that example demonstrates, then a closing line naming the grammar
//! sections it derives from.
//!
//! Generating topics from the grammar was the obvious alternative and was
//! refused for the reason the derivative exists: a generated topic is the
//! grammar again, at the grammar's size.
//!
//! A derivative can drift from its source silently, so three binds hold it, all
//! in `crates/agent-compose/tests/discovery_surface_inventory.rs`: every file is
//! registered and every registration has a file; every numbered heading of
//! `docs/grammar.md` is claimed by some topic's `Normative source` line and
//! every claim names a heading that exists; and the `cli` topic names every verb
//! the binary has and every environment variable it reads.

use std::fmt::Write as _;

use crate::parse::reader::suggest;

/// One document `agent-compose docs <topic>` prints.
pub struct Topic {
    /// The name typed after `docs`.
    pub name: &'static str,
    /// What it teaches, as the topic index prints it.
    pub summary: &'static str,
    /// The document, verbatim.
    pub body: &'static str,
}

/// The curriculum, in **reading order**.
///
/// Reading order rather than alphabetical: the index is the first thing an agent
/// with no context sees, and the order it is printed in is the order the topics
/// are written to be read in — `getting-started` before `agents`, `routing`
/// before `cycles`, the two reference topics last.
pub const TOPICS: &[Topic] = &[
    Topic {
        name: "getting-started",
        summary: "project layout, `imports:`, typed addresses, and the validate loop",
        body: include_str!("../../../../docs/topics/getting-started.md"),
    },
    Topic {
        name: "schemas",
        summary: "the type language: scalars, objects, arrays, enums, unions, defaults",
        body: include_str!("../../../../docs/topics/schemas.md"),
    },
    Topic {
        name: "cel",
        summary: "the expression dialect, what each surface may read, and what is not CEL",
        body: include_str!("../../../../docs/topics/cel.md"),
    },
    Topic {
        name: "agents",
        summary: "model-backed nodes: prompts, output schemas, tools, the bounded tool loop",
        body: include_str!("../../../../docs/topics/agents.md"),
    },
    Topic {
        name: "tools",
        summary: "`exec`/`http`/`function`/`module`/`builtin` implementations, and the def/use split",
        body: include_str!("../../../../docs/topics/tools.md"),
    },
    Topic {
        name: "flows",
        summary: "flows as modules: inputs, outputs, nodes, and a flow used as a tool",
        body: include_str!("../../../../docs/topics/flows.md"),
    },
    Topic {
        name: "routing",
        summary: "edges, guards, exhaustiveness, `else:`, and concurrent branches",
        body: include_str!("../../../../docs/topics/routing.md"),
    },
    Topic {
        name: "cycles",
        summary: "loops that provably stop: `max_iterations`, escapes, reachability",
        body: include_str!("../../../../docs/topics/cycles.md"),
    },
    Topic {
        name: "maps",
        summary: "fan-out: `over:`, concurrency bounds, routed variants, joins, `detach:`",
        body: include_str!("../../../../docs/topics/maps.md"),
    },
    Topic {
        name: "human",
        summary: "pausing for a person, answering at a terminal or over HTTP, timeouts",
        body: include_str!("../../../../docs/topics/human.md"),
    },
    Topic {
        name: "state",
        summary: "channels, reduce policies, `writes:` remaps, and conversation history",
        body: include_str!("../../../../docs/topics/state.md"),
    },
    Topic {
        name: "policies",
        summary: "`retry:`, `timeout:`, `on_error:`, and the four levels they resolve through",
        body: include_str!("../../../../docs/topics/policies.md"),
    },
    Topic {
        name: "stores",
        summary: "`kv`/`vector`/`blob` stores, ops, scopes, and agent-attached store tools",
        body: include_str!("../../../../docs/topics/stores.md"),
    },
    Topic {
        name: "models",
        summary: "providers, models, settings schemas, and ordered failover routes",
        body: include_str!("../../../../docs/topics/models.md"),
    },
    Topic {
        name: "triggers",
        summary: "`manual`/`http` entrypoints, respond modes, webhook auth and signing, idempotency",
        body: include_str!("../../../../docs/topics/triggers.md"),
    },
    Topic {
        name: "targets",
        summary: "deploy files, `--target`, storage-backend aliases, and reserved grammar",
        body: include_str!("../../../../docs/topics/targets.md"),
    },
    Topic {
        name: "trace",
        summary: "what a run records, where it is delivered, and how to read it",
        body: include_str!("../../../../docs/topics/trace.md"),
    },
    Topic {
        name: "cli",
        summary: "every verb, its exit codes, and the environment variables they read",
        body: include_str!("../../../../docs/topics/cli.md"),
    },
];

/// The topic named `name`, if the curriculum has one.
#[must_use]
pub fn topic(name: &str) -> Option<&'static Topic> {
    TOPICS.iter().find(|topic| topic.name == name)
}

/// Every topic name, in reading order.
#[must_use]
pub fn names() -> Vec<&'static str> {
    TOPICS.iter().map(|topic| topic.name).collect()
}

/// The closest topic name to `name`, when one is close enough to suggest.
#[must_use]
pub fn nearest(name: &str) -> Option<&'static str> {
    suggest(name, &names())
}

/// The topic index: what `agent-compose docs` prints with no argument.
///
/// One line per topic, then **the loop**. The two closing lines are why the
/// index exists at all: an agent that has read a list of topic names knows what
/// this compiler has, and still does not know what to do first.
#[must_use]
pub fn index() -> String {
    let width = TOPICS
        .iter()
        .map(|topic| topic.name.len())
        .max()
        .unwrap_or(0);
    let mut text = String::from("topics — print one with `agent-compose docs <topic>`\n\n");
    for topic in TOPICS {
        let _ = writeln!(text, "  {:width$}  {}", topic.name, topic.summary);
    }
    text.push_str(
        "\nthe loop: read a topic, author the spec, `agent-compose validate main.yml`, and \
         `agent-compose explain <code>` anything it reports.\n",
    );
    text.push_str(
        "then `agent-compose plan <before> <after>` to review a change, and `agent-compose run \
         main.yml flow.<name>` to run it.\n",
    );
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_index_names_every_topic_and_closes_with_the_loop() {
        let index = index();
        for topic in TOPICS {
            assert!(
                index.contains(topic.name),
                "the index omits `{}`",
                topic.name
            );
            assert!(
                index.contains(topic.summary),
                "the index omits what `{}` teaches",
                topic.name
            );
        }
        assert!(index.contains("validate"), "the loop names `validate`");
        assert!(index.contains("explain"), "the loop names `explain`");
        assert!(index.contains("plan"), "the loop names `plan`");
        assert!(index.contains("run"), "the loop names `run`");
    }

    /// The index is where an agent with no context decides what to read, so the
    /// `tools` summary names every implementation binding a tool can carry.
    ///
    /// The summaries are free-form prose and this is the one of them with a
    /// **closed set** behind it (`parse::definition::TOOL_IMPLEMENTATIONS`). A
    /// binding added to the language and not to this line is a surface the
    /// discovery path never mentions: the agent scanning the index for where
    /// hand-written tool code goes reads three names, finds no fourth, and never
    /// opens the topic that has it. Pinned here rather than in the topic body's
    /// own tests because the body is what a reader gets *after* choosing, and
    /// this is the sentence the choice is made on.
    #[test]
    fn the_tools_summary_names_every_implementation_binding() {
        let summary = topic("tools").expect("a `tools` topic").summary;
        for binding in crate::parse::definition::TOOL_IMPLEMENTATIONS {
            assert!(
                summary.contains(&format!("`{binding}`")),
                "the `tools` summary does not name `{binding}`: {summary}"
            );
        }
    }

    /// The registry is a lookup table and a suggestion source; both have to
    /// answer for every name it holds.
    #[test]
    fn every_registered_topic_is_reachable_by_name() {
        for name in names() {
            let held = topic(name).expect("a registered topic answers to its own name");
            assert_eq!(held.name, name);
            assert!(!held.body.is_empty(), "`{name}` has a document");
            assert!(!held.summary.is_empty(), "`{name}` has a summary");
        }
        assert!(topic("no-such-topic").is_none());
        assert_eq!(nearest("routng"), Some("routing"));
        assert_eq!(nearest("qqqqqqqqqqqq"), None);
    }

    /// Names are typed at a shell, so they are lowercase and hyphenated — the
    /// spelling the index prints and the one a suggestion can reach.
    #[test]
    fn topic_names_are_lowercase_and_unique() {
        let mut seen = std::collections::BTreeSet::new();
        for name in names() {
            assert!(
                name.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
                "`{name}` is not a lowercase, hyphenated name"
            );
            assert!(seen.insert(name), "`{name}` is registered twice");
        }
    }
}
