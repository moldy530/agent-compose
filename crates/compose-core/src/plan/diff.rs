//! The structural diff: one IR fragment against another, field by field.
//!
//! # Why JSON
//!
//! The IR is already a JSON document (PRD 5.1) and every part of it is
//! [`Serialize`], so the comparison is written over
//! [`Value`](serde_json::Value) rather than over sixty pairs of match arms. It
//! is the same substrate the artifact itself is written in, so a path this
//! module reports — `output.fields[verdict].type.form` — is a path into the
//! artifact a reader can follow, and a field added to the IR is diffed the day
//! it is added rather than the day somebody remembers to extend a walker.
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
//! # Which arrays carry an order
//!
//! Some of the artifact's arrays are **sequences** whose order the composition
//! behaves differently for, and some are **sets** whose order carries nothing. A
//! plan has to tell them apart in both directions: an order it reports where
//! none exists is a line about an edit nobody made, and an order it hides where
//! one exists is the plan saying two compositions agree when they do not.
//!
//! The line is drawn at what a reordering *does*, and the key an array sits
//! under is what decides it:
//!
//! * three keys are sets. `optional:` (grammar 3.4) names the properties an
//!   object does not require, `expect_exit:` (6.1, 8.2) the exit statuses a
//!   process may end with, and `expect_status:` (6.1, 8.3) the response statuses
//!   a request may return. Each is parsed as a **distinct** membership and
//!   membership-tested at run time, so [`normalize`] sorts all three: their
//!   order never reaches a comparison at all;
//! * `fields:` and `variants:` are sequences, and the ones that would otherwise
//!   be missed. Both are matched by name — so a schema that gained a property is
//!   one change at that property — and *both* orders reach the JSON Schema the
//!   model is handed: a field map's order is the order of `properties` and of
//!   `required`, a union's is the order of `oneOf`. So [`walk`] reports a move
//!   through the same synthesized `order` key `sections::ordered` gives an edge;
//! * every other array is compared by position already, which is right for the
//!   ones whose order is plainly the author's: a model's `route:` (failover
//!   order), an `exec:`'s `args:` (argv order), an `enum:`'s variants (the order
//!   they reach a structured-output schema in).
//!
//! What is left silent on purpose is a reordering that changes the *layout* of
//! the emitted project and nothing it does: a node `input:` binding map, a
//! `writes:` remap, an `env:` or `headers:` map, a `map:`'s `routes:`. All of
//! them are dispatched on by name — the generated code looks each up by the name
//! it is written under — and a plan is a diff of compositions, not of files
//! (`docs/plan.md` §11). `agent-compose build --check` is the command that
//! notices a generated file whose bytes moved.
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

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;
use serde_json::{Map, Value};

use super::document::FieldChange;

/// The key a plan reports a declaration position under.
///
/// Not a key of the artifact: the IR carries a position as a position, and this
/// is the plan's name for it — here for a named array's entries, and in
/// [`sections::ordered`](super::sections) for a flow's edges.
pub(super) const ORDER: &str = "order";

/// The keys whose arrays are **sets**, canonicalized by [`normalize`] so that
/// their order never reaches a comparison. See the module docs.
const SETS: &[&str] = &["expect_exit", "expect_status", "optional"];

/// The keys whose **named** arrays are sequences: matched by name, and reporting
/// a move through [`ORDER`]. See the module docs.
const SEQUENCES: &[&str] = &["fields", "variants"];

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

/// The same value with every source region removed, every spanned value
/// unwrapped, and every set-valued array sorted. See the module docs for what is
/// recognized and why.
fn normalize(value: Value) -> Value {
    held("", value)
}

/// The same, carrying the key the value was found under — which is what says
/// whether an array below it is one of [`SETS`].
fn held(key: &str, value: Value) -> Value {
    match value {
        Value::Object(map) => {
            if map.len() == 2
                && let Some(inner) = map.get("value")
                && let Some(Value::String(text)) = map.get("span")
                && crate::ir::leaf::parse_span(text).is_some()
            {
                return held(key, inner.clone());
            }
            let mut kept = Map::new();
            for (key, value) in map {
                if key == "span"
                    && let Value::String(text) = &value
                    && crate::ir::leaf::parse_span(text).is_some()
                {
                    continue;
                }
                let value = held(&key, value);
                kept.insert(key, value);
            }
            Value::Object(kept)
        }
        Value::Array(items) => {
            let mut items: Vec<Value> = items.into_iter().map(|item| held(key, item)).collect();
            if SETS.contains(&key) && scalars(&items) {
                items.sort_by(order);
            }
            Value::Array(items)
        }
        scalar => scalar,
    }
}

/// Whether every element is a scalar of one kind — all strings, or all numbers.
///
/// The three set keys hold exactly that: `optional:` a list of identifiers,
/// `expect_exit:` and `expect_status:` a list of integers. Requiring it keeps
/// the canonicalization off the surfaces where an author chooses the keys — a
/// model's `settings:`, a schema's `default:`, a deploy backend's plugin config
/// — where an array of objects written under a key spelled `optional` is data
/// rather than one of the grammar's sets. The residual, an author's own flat
/// array of scalars under one of the three names, is sorted like the set it is
/// spelled as: the same class of residual the `span` rule above leaves, and the
/// same reason it is tolerable.
fn scalars(items: &[Value]) -> bool {
    items.iter().all(Value::is_string) || items.iter().all(Value::is_number)
}

/// A total order over the scalars a set holds: numbers by their value, anything
/// else by its JSON text, ties broken by that text so that the sort is decided
/// by the values alone rather than by the order they arrived in.
fn order(left: &Value, right: &Value) -> Ordering {
    match (left.as_f64(), right.as_f64()) {
        (Some(one), Some(two)) => one.partial_cmp(&two).unwrap_or(Ordering::Equal),
        _ => Ordering::Equal,
    }
    .then_with(|| left.to_string().cmp(&right.to_string()))
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
///   rewrite of the whole map. Where the module docs say the order of one of
///   those is the composition's rather than the file's ([`SEQUENCES`]), the
///   entry also carries its position as [`ORDER`], so a swap is a change at the
///   two entries that swapped;
/// * two other arrays are compared element by element **when they are the same
///   length**, so the ones whose order *is* semantic — a model's `route:`
///   (failover order), an `exec:`'s `args:` (argv order), an `enum:`'s variants
///   (the order they reach a structured-output schema in) — report a move. The
///   three arrays that are sets rather than sequences never reach this arm in
///   two orders, because [`normalize`] has already sorted them;
/// * when the lengths differ and nothing names itself, the array is one change:
///   an element inserted in the middle shifts every index after it, and
///   reporting each shifted element would bury the insertion that caused them.
///   A flow's `edges:` is the list where that would matter most, and topology
///   matches those by identity before they ever reach this walk (see
///   `sections::edges`);
/// * anything else is reported as it stands.
///
/// The result is in path order — an object's keys sorted, an array's entries in
/// index order for a positional array and in sorted name order for a named one —
/// so the same pair of fragments always produces the same list.
pub(super) fn changes(before: &Value, after: &Value) -> Vec<FieldChange> {
    let mut found = Vec::new();
    walk("", "", before, after, &mut found);
    found
}

/// `key` is the object key this pair was found under, which is what says whether
/// an array here carries an order a plan reports ([`SEQUENCES`]). It is passed
/// on unchanged into an array's elements, so that the key of the array is what
/// classifies it however the elements are shaped.
fn walk(path: &str, key: &str, before: &Value, after: &Value, found: &mut Vec<FieldChange>) {
    if before == after {
        return;
    }
    match (before, after) {
        (Value::Object(old), Value::Object(new)) => {
            let keys: BTreeSet<&String> = old.keys().chain(new.keys()).collect();
            for key in keys {
                let path = join(path, key);
                match (old.get(key), new.get(key)) {
                    (Some(old), Some(new)) => walk(&path, key, old, new, found),
                    (old, new) => found.push(FieldChange {
                        path,
                        before: old.cloned(),
                        after: new.cloned(),
                    }),
                }
            }
        }
        (Value::Array(old), Value::Array(new)) if identity(old, new).is_some() => {
            let id = identity(old, new).expect("the arm matched on it");
            let names: BTreeSet<&str> = old
                .iter()
                .chain(new)
                .filter_map(|item| name(item, id))
                .collect();
            let places = places(key, old, new, id);
            for held in names {
                let path = format!("{path}[{held}]");
                match (entry(old, id, held), entry(new, id, held)) {
                    (Some(old), Some(new)) => match places.as_ref().and_then(|at| at.moved(held)) {
                        Some((one, two)) => {
                            walk(&path, key, &placed(old, one), &placed(new, two), found);
                        }
                        None => walk(&path, key, old, new, found),
                    },
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
                walk(&format!("{path}[{at}]"), key, old, new, found);
            }
        }
        _ => found.push(FieldChange {
            path: path.to_string(),
            before: Some(before.clone()),
            after: Some(after.clone()),
        }),
    }
}

/// Where each side declares the entries **both** of them declare.
///
/// Counted over the common names rather than over each array on its own, which
/// is what keeps an insertion from reading as a move: a field added at the front
/// of a schema shifts every index below it, and the entries that shifted are in
/// the same order relative to each other as they were. A move is the case where
/// that is no longer true, and it is reported at the entries that moved.
struct Places<'a> {
    before: BTreeMap<&'a str, u64>,
    after: BTreeMap<&'a str, u64>,
}

impl Places<'_> {
    /// This entry's two positions, when it has one on each side and they differ.
    fn moved(&self, held: &str) -> Option<(u64, u64)> {
        let one = *self.before.get(held)?;
        let two = *self.after.get(held)?;
        (one != two).then_some((one, two))
    }
}

/// The positions to report for a named array, or `None` when this one carries no
/// order a plan reports.
///
/// The second refusal is the collision guard: [`ORDER`] is the plan's key rather
/// than the artifact's, so an array whose entries already carry one is author
/// data (a model's `settings:`, a plugin config) shaped like a named list, and
/// is compared as it stands rather than overwritten with a position.
fn places<'a>(key: &str, old: &'a [Value], new: &'a [Value], id: &str) -> Option<Places<'a>> {
    if !SEQUENCES.contains(&key) {
        return None;
    }
    if old.iter().chain(new).any(|item| item.get(ORDER).is_some()) {
        return None;
    }
    let held: BTreeSet<&str> = new.iter().filter_map(|item| name(item, id)).collect();
    let common: BTreeSet<&str> = old
        .iter()
        .filter_map(|item| name(item, id))
        .filter(|at| held.contains(at))
        .collect();
    Some(Places {
        before: counted(old, id, &common),
        after: counted(new, id, &common),
    })
}

fn counted<'a>(items: &'a [Value], id: &str, common: &BTreeSet<&str>) -> BTreeMap<&'a str, u64> {
    items
        .iter()
        .filter_map(|item| name(item, id))
        .filter(|held| common.contains(held))
        .enumerate()
        .map(|(at, held)| (held, at as u64))
        .collect()
}

/// The same entry carrying its position, so that a move is one field of it.
fn placed(item: &Value, at: u64) -> Value {
    let mut held = item.clone();
    if let Value::Object(map) = &mut held {
        map.insert(ORDER.to_string(), Value::from(at));
    }
    held
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

    /// A field map's order reaches the schema the model is handed, so a swap is
    /// a change — at the two entries that swapped, and nowhere else.
    #[test]
    fn a_named_sequence_reports_the_entries_that_moved() {
        let before = json!({ "fields": [{ "name": "a" }, { "name": "b" }] });
        let after = json!({ "fields": [{ "name": "b" }, { "name": "a" }] });
        assert_eq!(
            changes(&before, &after),
            [
                FieldChange {
                    path: "fields[a].order".to_string(),
                    before: Some(json!(0)),
                    after: Some(json!(1)),
                },
                FieldChange {
                    path: "fields[b].order".to_string(),
                    before: Some(json!(1)),
                    after: Some(json!(0)),
                },
            ]
        );
    }

    /// …and an entry inserted at the front is one change, not one per entry it
    /// pushed down: the positions are counted over the names both sides declare.
    #[test]
    fn an_insertion_into_a_named_sequence_moves_nothing() {
        let before = json!({ "variants": [{ "tag": "a" }, { "tag": "b" }] });
        let after = json!({ "variants": [{ "tag": "x" }, { "tag": "a" }, { "tag": "b" }] });
        let found = changes(&before, &after);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].path, "variants[x]");
    }

    /// A named array whose order is dispatched on by name rather than read as a
    /// sequence keeps reporting nothing about it.
    #[test]
    fn a_named_map_reports_no_order() {
        assert_eq!(
            changes(
                &json!({ "entries": [{ "name": "a" }, { "name": "b" }] }),
                &json!({ "entries": [{ "name": "b" }, { "name": "a" }] }),
            ),
            []
        );
    }

    /// The three set-valued keys are canonicalized, so a reordering of one is
    /// not a change — and a membership edit still is.
    #[test]
    fn a_set_is_compared_by_membership_rather_than_by_position() {
        for key in ["optional", "expect_exit", "expect_status"] {
            let one = semantic(&serde_json::json!({ key: ["b", "a"] }));
            let two = semantic(&serde_json::json!({ key: ["a", "b"] }));
            assert_eq!(changes(&one, &two), [], "{key}");

            let three = semantic(&serde_json::json!({ key: ["a", "c"] }));
            let found = changes(&two, &three);
            assert_eq!(found.len(), 1, "{key}: {found:?}");
            assert_eq!(found[0].path, format!("{key}[1]"), "{key}");
        }
    }

    /// A set of numbers sorts by its numbers rather than by their text, so
    /// `[2, 10]` and `[10, 2]` are one set and the reported one reads as one.
    #[test]
    fn a_set_of_numbers_is_canonicalized_numerically() {
        let one = semantic(&serde_json::json!({ "expect_exit": [10, 2] }));
        let two = semantic(&serde_json::json!({ "expect_exit": [2, 10] }));
        assert_eq!(changes(&one, &two), []);
        assert_eq!(one, json!({ "expect_exit": [2, 10] }));
    }

    /// The set rule is stated over flat scalars, so an author's own array of
    /// objects under one of the three names is left as it stands.
    #[test]
    fn an_authors_array_of_objects_is_not_taken_for_a_set() {
        let value = json!({ "settings": { "optional": [{ "b": 1 }, { "a": 1 }] } });
        assert_eq!(normalize(value.clone()), value);
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
