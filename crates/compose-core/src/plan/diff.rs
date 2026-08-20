//! The structural diff: one IR fragment against another, field by field.
//!
//! # Why JSON
//!
//! The IR is already a JSON document (PRD 5.1) and every part of it is
//! [`Serialize`], so the comparison is written over
//! [`Value`](serde_json::Value) rather than over sixty pairs of match arms. It
//! is the same substrate the artifact itself is written in, so a path this
//! module reports — `output.fields[0].type.form` — is a path into the artifact a
//! reader can follow, and a field added to the IR is diffed the day it is added
//! rather than the day somebody remembers to extend a walker.
//!
//! # Why spans come out first
//!
//! A plan is a diff of **compositions**, not of files: PRD §2's problem is
//! "reviewing what changed in the topology", and a comment inserted above a
//! definition changes nothing about the topology. Every construct in the IR
//! carries the region it was written in (`crate::ir::leaf`), and those regions
//! move whenever anything above them does — so a comparison that read them
//! would call an added blank line a change to every definition below it.
//!
//! [`semantic`] is therefore the first step of every comparison here: the
//! fragment as JSON, with its source coordinates taken out. Two shapes carry
//! them and both are handled:
//!
//! * a `span` key, written by every construct that has a region of its own;
//! * a `{"value": …, "span": …}` object, which is how a
//!   [`Spanned`](crate::diag::Spanned) reaches the artifact — unwrapped to the
//!   value, so the path a change is reported at is `model` rather than
//!   `model.value`.
//!
//! Both are recognized by the **span string itself** parsing as one
//! ([`parse_span`](crate::ir::leaf::parse_span)), not by the key's name alone.
//! Two surfaces of the IR hold author-written data with author-chosen keys — a
//! model's `settings:` and a schema's `default:`, both `Literal`, and a deploy
//! backend's plugin config — so `span:` is a key an author can write. Requiring
//! it to hold something of the form `file:line:col..line:col` is what keeps a
//! setting called `span` from being mistaken for a source region. The residual
//! is a setting called `span` whose value is spelled exactly like a source
//! region, which loses that one key from the comparison; nothing else in the
//! artifact can reach either shape by accident.
//!
//! # Depth
//!
//! [`normalize`] and [`walk`] recurse, and what they recurse over is the depth
//! of one subject's JSON — which is the nesting depth of a schema, not the size
//! of a flow: a graph's nodes and edges are arrays of flat objects, and
//! `check_scale.rs`'s deeply nested composition is a chain of *flows*, each one a
//! definition of its own. Schema depth is already the parser's recursion
//! (`parse::schema`) and the serializer's, so nothing here adds a failure class
//! a composition could not already reach one pass earlier.

use std::collections::BTreeSet;

use serde::Serialize;
use serde_json::{Map, Value};

use super::document::FieldChange;

/// One IR fragment as JSON, with its source coordinates taken out.
///
/// # Panics
///
/// If the fragment cannot be represented as JSON. Nothing in the IR can be:
/// `Ir::to_json` writes the whole artifact through the same serializer, and the
/// four surfaces that carry a float are checked where they are written. A
/// fragment that somehow could not be would otherwise compare equal to
/// everything, which is a silent "no changes" on a real difference — so it is a
/// panic rather than a fallback.
pub(super) fn semantic<T: Serialize>(value: &T) -> Value {
    normalize(
        serde_json::to_value(value)
            .expect("every part of the IR is representable as JSON (see `Ir::to_json`)"),
    )
}

/// The same value with every source region removed and every spanned value
/// unwrapped. See the module docs for what is recognized and why.
fn normalize(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            if map.len() == 2
                && let Some(inner) = map.get("value")
                && let Some(Value::String(text)) = map.get("span")
                && crate::ir::leaf::parse_span(text).is_some()
            {
                return normalize(inner.clone());
            }
            let mut held = Map::new();
            for (key, value) in map {
                if key == "span"
                    && let Value::String(text) = &value
                    && crate::ir::leaf::parse_span(text).is_some()
                {
                    continue;
                }
                held.insert(key, normalize(value));
            }
            Value::Object(held)
        }
        Value::Array(items) => Value::Array(items.into_iter().map(normalize).collect()),
        scalar => scalar,
    }
}

/// The same object without these top-level keys — the ones another section of
/// the plan owns.
pub(super) fn without(value: Value, keys: &[&str]) -> Value {
    let Value::Object(mut map) = value else {
        return value;
    };
    for key in keys {
        map.remove(*key);
    }
    Value::Object(map)
}

/// The same object with only these top-level keys.
pub(super) fn only(value: Value, keys: &[&str]) -> Value {
    let Value::Object(map) = value else {
        return Value::Object(Map::new());
    };
    let mut held = Map::new();
    for key in keys {
        if let Some(value) = map.get(*key) {
            held.insert((*key).to_string(), value.clone());
        }
    }
    Value::Object(held)
}

/// Every field of `before` that `after` does not agree with, deepest first.
///
/// Both values are already [`semantic`]. The walk descends while it can name
/// what it descended into, and reports the pair where it can no longer:
///
/// * two objects are compared key by key, over the union of their keys, so a
///   key on one side only is one change at that key rather than a rewrite of
///   the whole object;
/// * two arrays whose elements each **name themselves** — a field map's
///   `fields`, a binding list's `entries`, a union's `variants`, a routed map's
///   `routes` — are matched on that name and compared entry by entry, so a
///   schema that gained a property is one change at that property rather than a
///   rewrite of the whole map. The trade is that their declaration order is not
///   reported, which is right for the four lists that have one: a JSON object's
///   properties, a mapping's bindings, and a discriminator's variants are all
///   dispatched on by name;
/// * two other arrays are compared element by element **when they are the same
///   length**, so the ones whose order *is* semantic — a model's `route:`
///   (failover order), an `exec:`'s `args:` (argv order), an `enum:`'s variants
///   (the order they reach a structured-output schema in) — report a move. The
///   three that are really sets and not sequences (`optional:`,
///   `expect_exit:`, `expect_status:`) report a reordering nobody meant, which
///   is the direction to be wrong in: a line to skim, against a silently
///   reordered failover route;
/// * when the lengths differ and nothing names itself, the array is one change:
///   an element inserted in the middle shifts every index after it, and
///   reporting each shifted element would bury the insertion that caused them.
///   A flow's `edges:` is the list where that would matter most, and topology
///   matches those by identity before they ever reach this walk (see
///   `sections::edges`);
/// * anything else is reported as it stands.
///
/// The result is in path order, which is the key order of a
/// [`Map`](serde_json::Map) — sorted — so the same pair of fragments always
/// produces the same list.
pub(super) fn changes(before: &Value, after: &Value) -> Vec<FieldChange> {
    let mut found = Vec::new();
    walk("", before, after, &mut found);
    found
}

fn walk(path: &str, before: &Value, after: &Value, found: &mut Vec<FieldChange>) {
    if before == after {
        return;
    }
    match (before, after) {
        (Value::Object(old), Value::Object(new)) => {
            let keys: BTreeSet<&String> = old.keys().chain(new.keys()).collect();
            for key in keys {
                let path = join(path, key);
                match (old.get(key), new.get(key)) {
                    (Some(old), Some(new)) => walk(&path, old, new, found),
                    (old, new) => found.push(FieldChange {
                        path,
                        before: old.cloned(),
                        after: new.cloned(),
                    }),
                }
            }
        }
        (Value::Array(old), Value::Array(new)) if identity(old, new).is_some() => {
            let key = identity(old, new).expect("the arm matched on it");
            let names: BTreeSet<&str> = old
                .iter()
                .chain(new)
                .filter_map(|item| name(item, key))
                .collect();
            for held in names {
                let path = format!("{path}[{held}]");
                match (entry(old, key, held), entry(new, key, held)) {
                    (Some(old), Some(new)) => walk(&path, old, new, found),
                    (old, new) => found.push(FieldChange {
                        path,
                        before: old.cloned(),
                        after: new.cloned(),
                    }),
                }
            }
        }
        (Value::Array(old), Value::Array(new)) if old.len() == new.len() => {
            for (at, (old, new)) in old.iter().zip(new).enumerate() {
                walk(&format!("{path}[{at}]"), old, new, found);
            }
        }
        _ => found.push(FieldChange {
            path: path.to_string(),
            before: Some(before.clone()),
            after: Some(after.clone()),
        }),
    }
}

/// The keys an array's elements can name themselves by, in the order they are
/// tried.
///
/// The four spellings the IR uses for "what this entry is called": `name` on a
/// schema field, a binding, and an interpolated entry; `field` on a `writes:`
/// remap; `tag` on a union variant and a routed map's route; `id` on a node.
const IDENTITY: &[&str] = &["name", "field", "tag", "id"];

/// The key two arrays' elements can be matched on, if there is one.
///
/// Every element of both must carry it as a **string**, and no two elements of
/// one array may carry the same one: an array where a name repeats is not an
/// array a name identifies an element of. An empty array qualifies vacuously,
/// which is what makes the first field added to `{}` one change at that field.
fn identity(old: &[Value], new: &[Value]) -> Option<&'static str> {
    IDENTITY
        .iter()
        .copied()
        .find(|key| named(old, key) && named(new, key))
}

fn named(items: &[Value], key: &str) -> bool {
    let mut seen = BTreeSet::new();
    items
        .iter()
        .all(|item| name(item, key).is_some_and(|held| seen.insert(held)))
}

fn name<'a>(item: &'a Value, key: &str) -> Option<&'a str> {
    item.get(key).and_then(Value::as_str)
}

fn entry<'a>(items: &'a [Value], key: &str, held: &str) -> Option<&'a Value> {
    items.iter().find(|item| name(item, key) == Some(held))
}

/// A child path: the key on its own at the root, joined with a `.` below it.
///
/// A key holding a `.` of its own — which the two open surfaces of the IR admit,
/// a model's `settings:` and a plugin config — is written as it stands, so a
/// path is read as a route through the artifact rather than parsed back into
/// one.
fn join(path: &str, key: &str) -> String {
    if path.is_empty() {
        key.to_string()
    } else {
        format!("{path}.{key}")
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_span_key_and_a_spanned_value_both_come_out() {
        let value = json!({
            "model": { "value": "model.smart", "span": "main.yml:3:5..3:16" },
            "span": "main.yml:1:1..9:2",
            "tools": [{ "value": "tool.search", "span": "main.yml:5:7..5:18" }],
        });
        assert_eq!(
            normalize(value),
            json!({ "model": "model.smart", "tools": ["tool.search"] })
        );
    }

    /// A `settings:` key called `span`, or a two-key literal mapping spelled
    /// `{value, span}`, is data rather than a source region — so both survive,
    /// and a change to either is still a change.
    #[test]
    fn author_written_data_named_like_a_region_survives() {
        let literal = json!({ "value": 1, "span": 2 });
        assert_eq!(normalize(literal.clone()), literal);

        let named = json!({ "span": "every 5 minutes", "id": "some-model" });
        assert_eq!(normalize(named.clone()), named);

        assert_eq!(
            changes(
                &normalize(json!({ "settings": { "value": 1, "span": 2 } })),
                &normalize(json!({ "settings": { "value": 1, "span": 3 } })),
            ),
            [FieldChange {
                path: "settings.span".to_string(),
                before: Some(json!(2)),
                after: Some(json!(3)),
            }]
        );
    }

    #[test]
    fn a_key_on_one_side_only_is_one_change_at_that_key() {
        let found = changes(
            &json!({ "kept": 1, "gone": { "deep": true } }),
            &json!({ "kept": 1, "new": "x" }),
        );
        assert_eq!(
            found,
            [
                FieldChange {
                    path: "gone".to_string(),
                    before: Some(json!({ "deep": true })),
                    after: None,
                },
                FieldChange {
                    path: "new".to_string(),
                    before: None,
                    after: Some(json!("x")),
                },
            ]
        );
    }

    #[test]
    fn equal_length_arrays_are_compared_element_by_element() {
        assert_eq!(
            changes(
                &json!({ "route": ["model.a", "model.b"] }),
                &json!({ "route": ["model.a", "model.c"] }),
            ),
            [FieldChange {
                path: "route[1]".to_string(),
                before: Some(json!("model.b")),
                after: Some(json!("model.c")),
            }]
        );
    }

    /// …and an array whose length changed is one change, not one per shifted
    /// element.
    #[test]
    fn an_array_that_grew_is_one_change() {
        let found = changes(
            &json!({ "args": ["a", "b"] }),
            &json!({ "args": ["x", "a", "b"] }),
        );
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].path, "args");
    }

    #[test]
    fn only_and_without_partition_the_top_level() {
        let value = json!({ "inputs": 1, "outputs": 2, "nodes": 3 });
        assert_eq!(
            only(value.clone(), &["inputs", "outputs"]),
            json!({ "inputs": 1, "outputs": 2 })
        );
        assert_eq!(
            without(value, &["inputs", "outputs"]),
            json!({ "nodes": 3 })
        );
    }
}
