//! What a plan may be silent about, stated as a property rather than as a
//! golden.
//!
//! Every other test of this command asserts a *particular* report: this pair of
//! specs produces these lines. That is the right shape for what a plan says, and
//! it is structurally blind to what a plan **fails** to say — a rule that
//! swallows a whole class of edit produces no line to assert against, so no
//! golden anywhere fails and CI is green on a diff that reports two compositions
//! agree when they do not. `docs/plan.md` §12 promises a machine surface; a
//! reader branching on it needs the silence to be exhaustively accounted for,
//! not merely spot-checked.
//!
//! So this file states the account itself:
//!
//! > A plan names a **subject** — a component, a node, a flow's edges, a
//! > channel, the policy defaults, a caller-visible surface — **if and only if**
//! > the two artifacts differ in the part of them that subject owns, once
//! > everything `docs/plan.md` §11 excuses is taken out.
//!
//! Both directions are load bearing, and each fails a different way:
//!
//! * **left to right** is completeness — a difference the plan says nothing
//!   about, which is the plan being wrong about the composition. An author's
//!   literal array of objects compared as though its entries were a keyed map is
//!   exactly this: reverse `default: [{name: one}, {name: two}]` and every
//!   caller is handed a different value, with nothing in the report;
//!   [`compared`] does not sort that array, so the property fails on it;
//! * **right to left** is quiet — a line about an edit nobody made. Reordering
//!   an `env:` map, a node's `input:` bindings or a flow's `nodes:` changes the
//!   *layout* of the generated project and nothing it decides, and §11 is where
//!   each of those is accounted for. [`compared`] canonicalizes exactly those,
//!   so a plan that started reporting one would fail the property here.
//!
//! # Why it is stated per subject
//!
//! The weaker reading — *the three structural sections are empty exactly when
//! the two artifacts agree* — is asserted too ([`holds`]), and on its own it has
//! almost no grip. It can only fail on a pair that differs in **nothing else**:
//! a rule that dropped every `description:` in the composition is invisible on
//! any pair that also moved an edge, because the edge's own record keeps the
//! count off zero and the biconditional is satisfied by the wrong record. Every
//! pair of a corpus chosen for its variety is such a pair, so the whole class of
//! "a field nobody compares any more" would sit under a green suite.
//!
//! Naming the subject removes the hiding place. The definition whose description
//! moved still differs; the plan still says nothing about **it**; the sets do not
//! match, and the failure names the address. That is why [`subjects`] restates
//! §4–§6's partition — which record type owns which part of a definition, of a
//! trigger, of a flow — beside [`compared`]'s restatement of §11, and it is what
//! holds every field of every component and of every node to something, rather
//! than only the fields some pair of the corpus happens to differ in twice.
//!
//! # `compared` and `subjects` are written twice on purpose
//!
//! Both restate `docs/plan.md` independently of
//! `crates/compose-core/src/plan/`. [`compared`] is §11 — take the source
//! regions out, sort the sets, sort the arrays the generated code looks up by
//! name, drop the keys no section owns, drop a section declared with no entries
//! — and [`subjects`] is §4–§6, the partition that says a flow's `inputs:` is
//! read by `interfaces` and its `description:` by `components`. Both are written
//! from the document rather than shared with the code under test, which is what
//! keeps the property from agreeing with a bug by construction. Which is also
//! how they can be wrong: a clause the document states and this file omits makes
//! the property fail on documented behavior and name the compiler for it, and no
//! golden anywhere would notice. [`CASES`] is where a clause is held to an actual
//! edit rather than assumed.
//!
//! # Where the validation section is
//!
//! Outside the biconditional, deliberately. `validation` is the difference
//! between two check-phase reports **plus** what resolving each side said, and a
//! resolution warning is not part of the artifact — so a pair whose artifacts
//! agree can legitimately differ there. What is asserted about it instead is
//! that it never *saves* a case: the three structural sections are what the
//! property is stated over, so an edit hidden from them fails here whatever the
//! validator happened to notice.
//!
//! # What the corpora do not reach
//!
//! The property is a biconditional over the artifacts two corpora **produce**,
//! and that is narrower than a biconditional over the grammar. Nearly every
//! field of the IR is written `skip_serializing_if` — 135 of them across
//! `crates/compose-core/src/ir/` — so a construct no spec of either corpus
//! declares is no key of any artifact here: [`subjects`] never sees it,
//! [`differing_keys`] never names it, and
//! [`every_key_of_the_base_composition_is_moved_by_some_pair`] never asks for
//! it. `schemas/agent-compose.schema.json` names 164 distinct optional keys —
//! every property some object in it declares and does not require — and dozens
//! of them are declared by neither [`BASE`] nor any pair of either corpus. A
//! large share are a deploy backend's plugin config and a provider's server-tool
//! config, which `docs/plan.md` §11 excuses from a plan outright; the rest are
//! real — a model's `top_p:`, `seed:` and `thinking:`, a schema's `min_items:`,
//! `multiple_of:` and its two exclusive bounds, a store op's `filter:`,
//! `metadata:` and `top_k:`, and the whole of an `http` trigger's authentication
//! surface: `auth:`, `callback_auth:`, `callback_allow:` and the `header:`,
//! `algorithm:`, `encoding:` and `prefix:` under them.
//!
//! The two counts are stated rather than asserted, so they are the one thing
//! here that rots quietly: both are recomputable — `skip_serializing_if` over
//! `crates/compose-core/src/ir/`, and the schema's properties minus each
//! object's `required` — and a change that grows either without touching this
//! paragraph leaves a reader budgeting against a number that has moved.
//!
//! It is stated per **construct** rather than per key name, which is what makes
//! it easy to under-read: a `description:` is declared by every definition and
//! by every kind of subject *except* a trigger, so dropping `"description"` from
//! `plan::sections`'s `TRIGGER_IDENTITY` — which hands a trigger's documentation
//! to `interfaces` instead of to `components`, against §4 — leaves this file
//! green, while the same edit at any of the three places a definition or a node
//! declares one fails it.
//!
//! Closing that is a matter of growing what the corpora declare — a key added to
//! [`BASE`] needs a case moving it, because
//! [`every_key_of_the_base_composition_is_moved_by_some_pair`] demands one —
//! rather than anything about the property. `docs/plan.md` §11 carries the same
//! statement for readers of the format.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use compose_core::plan::{FieldChange, TopologyKind};
use compose_core::{Composition, Ir, Plan, plan, resolve};
use serde_json::{Map, Value};

/// The repository root.
fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("the manifest directory has a grandparent")
        .to_path_buf()
}

/// A scratch directory of this test's own, cleaned out before use.
fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("plan-completeness-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("can create a scratch directory");
    dir
}

// ---------------------------------------------------------------------------
// The document's §11, restated over the artifact.
// ---------------------------------------------------------------------------

/// Top-level keys of the artifact no section of a plan owns (`docs/plan.md`
/// §11, §2.2).
///
/// `sources` is about file layout, `entrypoint` and `ir_version` are the same on
/// both sides of nearly every comparison and §2.2 carries the useful ones
/// instead, `spec_version` is carried rather than compared, and `target` is
/// §2.2's as well.
const UNOWNED: &[&str] = &[
    "entrypoint",
    "ir_version",
    "sources",
    "spec_version",
    "target",
];

/// Keys of the deploy layer no section owns: the file a layer was read from, the
/// target §2.2 carries, and the one section `local` cannot declare.
const UNOWNED_DEPLOY: &[&str] = &["source", "storage_backends", "target"];

/// The keys whose arrays are memberships, whose order is never compared
/// (`docs/plan.md` §3, §11).
const SETS: &[&str] = &["expect_exit", "expect_status", "optional", "route_on"];

/// The keys whose arrays the generated code looks entries up in **by name**, so
/// that a reordering is a change to the layout of the emitted project and to
/// nothing the composition decides (`docs/plan.md` §5, §11).
///
/// Each is paired with the key its entries name themselves by. `fields` and
/// `variants` are deliberately **not** here: both reach the JSON Schema the
/// model is handed as written, so their order is reported and this must not
/// canonicalize it away.
const BY_NAME: &[(&str, &[&str])] = &[
    ("entries", &["name", "field"]),
    ("env", &["name"]),
    ("headers", &["name"]),
    ("nodes", &["id"]),
    ("routes", &["tag"]),
];

/// Where each section of the artifact sits, for [`compared`]'s last clause
/// (`docs/plan.md` §11, final bullet).
///
/// A section written with no entries reaches the artifact as a `Section` whose
/// `entries` map is empty; an absent one is no key at all (`crate::ir::Section`).
/// §11 excuses the difference — a plan reports the channels, the triggers and
/// the placements themselves rather than the sections that hold them — so this
/// restatement has to excuse it too, or the property would blame the compiler
/// for behaving exactly as documented.
///
/// Stated over the **place** rather than over the key, for the reason §3 gives
/// about arrays: `settings: { state: {} }` is an author's own empty object under
/// a key spelled like a section's, and canonicalizing that away would make a
/// real edit read as no change. `storage_backends:` is a section too and is not
/// here: [`UNOWNED_DEPLOY`] has already removed it.
const SECTIONS: &[&[&str]] = &[
    &["state"],
    &["triggers"],
    &["deploy", "placements"],
    &["deploy", "event_sources"],
];

/// The artifact with everything `docs/plan.md` §11 excuses taken out of it.
///
/// Two artifacts whose `compared` forms agree differ in nothing a plan reports;
/// two whose forms differ must produce at least one structural record. That is
/// the whole property, and everything in this function is one clause of §11.
///
/// # The one clause this restatement inherits rather than states
///
/// §11's bullet on **the key order of an author's own mapping** is the one line
/// of the document with no code of its own here. `serde_json::to_value` builds
/// every object as a `BTreeMap`, so a `default: {alpha: 1, beta: 2}` and a
/// `default: {beta: 2, alpha: 1}` are already one value by the time the first
/// line below runs — and `plan::diff::semantic` opens with the same call, so the
/// compiler is silent about it for the same mechanical reason. The two therefore
/// agree here **by construction**, which is precisely the shape of agreement the
/// module docs above say keeps nothing honest: this function cannot fail on that
/// clause in either direction.
///
/// So the clause is carried by [`CASES`] instead — "an author's literal mapping
/// reordered" plants the edit in a real composition and pins the silence to
/// behavior — and the neighbouring clause it is easiest to confuse it with, an
/// author's own *array* reordered, is planted right beside it and comes out the
/// other way.
fn compared(ir: &Ir) -> Value {
    let mut value = serde_json::to_value(ir).expect("the artifact is representable as JSON");
    if let Value::Object(map) = &mut value {
        for key in UNOWNED {
            map.remove(*key);
        }
        if let Some(Value::Object(deploy)) = map.get_mut("deploy") {
            for key in UNOWNED_DEPLOY {
                deploy.remove(*key);
            }
        }
    }
    let mut value = canonical("", value);
    for place in SECTIONS {
        emptied(&mut value, place);
    }
    value
}

/// Drop the key at `place` when what it holds is an object with nothing left in
/// it — which, a section's span having been taken out already, is a section
/// declared with no entries. See [`SECTIONS`].
fn emptied(value: &mut Value, place: &[&str]) {
    let Some((key, above)) = place.split_last() else {
        return;
    };
    let mut at = value;
    for step in above {
        let Some(next) = at.get_mut(*step) else {
            return;
        };
        at = next;
    }
    let Value::Object(map) = at else {
        return;
    };
    if map
        .get(*key)
        .is_some_and(|held| held.as_object().is_some_and(Map::is_empty))
    {
        map.remove(*key);
    }
}

/// The same value with its source regions removed and its excused orders made
/// canonical. `key` is the object key this value was found under, which is what
/// classifies an array here.
fn canonical(key: &str, value: Value) -> Value {
    match value {
        Value::Object(map) => {
            // A `Spanned<T>` reaches the artifact as `{value, span}`, and is
            // unwrapped so that a path names `model` rather than `model.value`.
            if map.len() == 2
                && let Some(inner) = map.get("value")
                && map.get("span").and_then(Value::as_str).is_some_and(is_span)
            {
                return canonical(key, inner.clone());
            }
            let mut kept = Map::new();
            for (key, value) in map {
                if key == "span" && value.as_str().is_some_and(is_span) {
                    continue;
                }
                let value = canonical(&key, value);
                kept.insert(key, value);
            }
            Value::Object(kept)
        }
        Value::Array(items) => {
            let mut items: Vec<Value> =
                items.into_iter().map(|item| canonical(key, item)).collect();
            if SETS.contains(&key) && flat(&items) {
                items.sort_by_key(Value::to_string);
            } else if let Some((_, names)) = BY_NAME.iter().find(|(held, _)| *held == key) {
                items.sort_by_key(|item| (named(item, names), item.to_string()));
            } else if key == "edges" {
                items = grouped(items);
            }
            Value::Array(items)
        }
        scalar => scalar,
    }
}

/// Whether every element is a scalar of one kind — all strings, or all numbers.
///
/// Every membership of [`SETS`] the grammar can produce is exactly that:
/// `optional:` and `route_on:` hold names, `expect_exit:` and `expect_status:`
/// hold integers. The document states the rule over the **key** an array sits
/// under (§3, §11), and the one place the key alone is not enough is an author's
/// own array written under one of the four names inside a `settings:` or a
/// `default:`, which is data rather than one of the grammar's memberships.
///
/// The compiler draws that line at a uniform flat array
/// (`crates/compose-core/src/plan/diff.rs`), and this restatement draws it in the
/// same place on purpose. The two are meant to be independent, not to disagree:
/// a mixed `optional: [1, "a"]` is unreachable through the grammar, so on every
/// composition the two readings decide alike — and a weaker line here (anything
/// that is not an object, say) would report that author's array as canonicalized
/// while the code compares it by position, failing the property over code
/// behaving exactly as documented.
fn flat(items: &[Value]) -> bool {
    items.iter().all(Value::is_string) || items.iter().all(Value::is_number)
}

/// The first of these keys this entry carries as a string, for sorting.
fn named(item: &Value, names: &[&str]) -> String {
    names
        .iter()
        .find_map(|name| item.get(*name).and_then(Value::as_str))
        .unwrap_or_default()
        .to_string()
}

/// A flow's edges with the **groups** put in one order and the order *within*
/// each group left alone.
///
/// Grammar 7.3 rule 1 evaluates a node's outgoing edges in declaration order,
/// and that order is what a trace's routing decision for the node is written in
/// (`docs/trace.md` §4). A swap between two edges of one node is reported
/// (`docs/plan.md` §5) — this must not hide it. An edge moved past an edge of a
/// *different* node leaves both nodes' decisions reading as they did and is not
/// reported, which is what putting the groups in `from` order excuses.
fn grouped(items: Vec<Value>) -> Vec<Value> {
    let mut groups: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    for item in items {
        let from = item
            .get("from")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        groups.entry(from).or_default().push(item);
    }
    groups.into_values().flatten().collect()
}

/// Whether this string is a source region as `crate::ir::leaf` writes one:
/// `<file>:<line>:<col>..<line>:<col>`, with a non-empty file.
///
/// Written out here rather than borrowed from the compiler, so that the property
/// does not inherit the code's own idea of what a span looks like. A file name
/// may hold a `:`, so the coordinates are taken from the right.
fn is_span(text: &str) -> bool {
    let Some((start, end)) = text.rsplit_once("..") else {
        return false;
    };
    let Some((start, start_column)) = start.rsplit_once(':') else {
        return false;
    };
    let Some((file, start_line)) = start.rsplit_once(':') else {
        return false;
    };
    let Some((end_line, end_column)) = end.split_once(':') else {
        return false;
    };
    !file.is_empty()
        && [start_line, start_column, end_line, end_column]
            .iter()
            .all(|held| held.parse::<u32>().is_ok())
}

// ---------------------------------------------------------------------------
// The document's §4–§6, restated as a partition of the artifact into subjects.
// ---------------------------------------------------------------------------

/// The three structural sections, spelled the way a plan names them.
const COMPONENTS: &str = "components";
const TOPOLOGY: &str = "topology";
const INTERFACES: &str = "interfaces";

/// One subject of a plan: the section that owns it (`docs/plan.md` §4–§6) and
/// the address it is named by (§8).
type Subject = (&'static str, String);

/// One subject's own part of the artifact, and what has to hold for a plan to
/// say anything about it at all.
struct Slice {
    /// The component whose presence on **both** sides is what makes this subject
    /// comparable — §3's rule that a component which arrived brings nothing with
    /// it, so a flow added to the composition is one line rather than one line
    /// per node it declares. `None` when the subject *is* a component, which is
    /// reported whichever side declares it.
    owner: Option<String>,
    /// What the plan compares for this subject.
    value: Value,
}

impl Slice {
    /// A subject that stands on its own: a definition, a trigger, a placement,
    /// an event source, a channel, the defaults.
    fn own(value: Value) -> Self {
        Self { owner: None, value }
    }

    /// A subject only compared where its component sits on both sides: a node,
    /// a flow's edges, either caller-visible surface.
    fn under(owner: &str, value: Value) -> Self {
        Self {
            owner: Some(owner.to_string()),
            value,
        }
    }
}

/// The artifact cut into the subjects a plan reports on, keyed the way a plan
/// names them.
///
/// This is `docs/plan.md` §4–§6 restated: which record type owns which part of
/// a definition, of a trigger, of a flow. Every part of [`compared`] lands in
/// exactly one slice — the partition is what the plan's own module docs call
/// "every change is reported once", read from the other end — so a field that
/// stopped being compared leaves its subject differing with nothing to name it.
///
/// Two spellings here are this file's rather than the document's, and both are
/// only ever compared against themselves: a section is named by the string a
/// plan writes it under, and a flow's edges are one subject `"<flow> edges"`
/// because §5 matches edges by identity rather than by address — restating that
/// pairing would be copying the algorithm, while the *set* of a flow's edges is
/// exactly what [`compared`]'s `grouped` already canonicalizes.
fn subjects(ir: &Ir) -> BTreeMap<Subject, Slice> {
    let artifact = compared(ir);
    accounted(
        &artifact,
        &["defaults", "state", "triggers", "definitions", "deploy"],
        "the artifact",
    );
    let mut found = BTreeMap::new();

    // `defaults:` and the `state:` channels are the composition's own, and §5
    // owns both.
    if let Some(defaults) = artifact.get("defaults") {
        found.insert(
            (TOPOLOGY, "defaults".to_string()),
            Slice::own(defaults.clone()),
        );
    }
    for (name, channel) in section(&artifact, &["state"]) {
        found.insert(
            (TOPOLOGY, format!("state.{name}")),
            Slice::own(without(channel, &["name"])),
        );
    }

    // A trigger is split: which flow it runs and what it is for are §4's, and
    // its whole delivery surface is §6's.
    for (name, trigger) in section(&artifact, &["triggers"]) {
        let address = format!("trigger.{name}");
        found.insert(
            (COMPONENTS, address.clone()),
            Slice::own(only(trigger.clone(), &["flow", "description"])),
        );
        found.insert(
            (INTERFACES, address.clone()),
            Slice::under(&address, without(trigger, &["flow", "description", "name"])),
        );
    }

    // The three sections `local` admits are §4's entirely. `hub:` is a singleton
    // rather than a map of named entries, so it is one subject at the address
    // `hub`, and nothing inside it repeats a key.
    for (name, placement) in section(&artifact, &["deploy", "placements"]) {
        found.insert(
            (COMPONENTS, format!("placement.{name}")),
            Slice::own(without(placement, &["name"])),
        );
    }
    if let Some(hub) = artifact.pointer("/deploy/hub") {
        found.insert((COMPONENTS, "hub".to_string()), Slice::own(hub.clone()));
    }
    // …and `trace_sink:` the same way, for the same reason: one block of three
    // keys, at one address (`docs/plan.md` §4, §8).
    if let Some(sink) = artifact.pointer("/deploy/trace_sink") {
        found.insert(
            (COMPONENTS, "trace_sink".to_string()),
            Slice::own(sink.clone()),
        );
    }
    // …and `package_registry:` the same way again: one block at one address
    // (`docs/plan.md` §4, §8). Its `scopes:` are *inside* it rather than
    // subjects of their own — one deployment has one place it installs from.
    if let Some(registry) = artifact.pointer("/deploy/package_registry") {
        found.insert(
            (COMPONENTS, "package_registry".to_string()),
            Slice::own(registry.clone()),
        );
    }
    for (name, source) in section(&artifact, &["deploy", "event_sources"]) {
        found.insert(
            (COMPONENTS, format!("event_source.{name}")),
            Slice::own(without(source, &["name"])),
        );
    }
    accounted(
        artifact.get("deploy").unwrap_or(&Value::Null),
        &[
            "hub",
            "placements",
            "package_registry",
            "trace_sink",
            "event_sources",
        ],
        "the deploy layer",
    );

    // A definition is §4's, except that a flow's graph is §5's and its declared
    // I/O is §6's.
    let Some(Value::Object(definitions)) = artifact.get("definitions") else {
        panic!("the artifact declares its definitions");
    };
    for (address, definition) in definitions {
        if definition.get("namespace") != Some(&Value::from("flow")) {
            found.insert(
                (COMPONENTS, address.clone()),
                Slice::own(without(definition.clone(), &["address"])),
            );
            continue;
        }
        found.insert(
            (COMPONENTS, address.clone()),
            Slice::own(without(
                definition.clone(),
                &["address", "inputs", "outputs", "nodes", "edges"],
            )),
        );
        found.insert(
            (INTERFACES, address.clone()),
            Slice::under(address, only(definition.clone(), &["inputs", "outputs"])),
        );
        for node in items(definition, "nodes") {
            let id = node
                .get("id")
                .and_then(Value::as_str)
                .expect("a node of the artifact carries its id");
            found.insert(
                (TOPOLOGY, format!("{address}.{id}")),
                Slice::under(address, without(node.clone(), &["id"])),
            );
        }
        found.insert(
            (TOPOLOGY, format!("{address} edges")),
            Slice::under(
                address,
                definition.get("edges").cloned().unwrap_or(Value::Null),
            ),
        );
    }

    found
}

/// Every key of `value`, held to the ones a clause of [`subjects`] places.
///
/// A partition is only a partition if it covers what it partitions, so a key
/// added to the IR that no section of `docs/plan.md` accounts for is a failure
/// *of this file* — the property would otherwise go quiet about that key in both
/// directions, which is the hole it exists to close.
fn accounted(value: &Value, keys: &[&str], what: &str) {
    let Value::Object(map) = value else {
        panic!("{what} is an object of the artifact");
    };
    let extra: Vec<&str> = map
        .keys()
        .map(String::as_str)
        .filter(|key| !keys.contains(key))
        .collect();
    assert!(
        extra.is_empty(),
        "{what} holds {extra:?}, which no clause of `subjects` places. Either a section of \
         `docs/plan.md` owns it — and this restatement has to say which — or `docs/plan.md` §11 \
         excuses it and `compared` has to take it out."
    );
}

/// The named entries of a section of the artifact at `place`, which may not have
/// been declared at all.
///
/// A declared section reaches the artifact as `{entries, span}` and an empty one
/// as no key at all once [`compared`] is through with it, so a missing key here
/// is a section with nothing in it either way.
fn section<'a>(artifact: &'a Value, place: &[&str]) -> Vec<(&'a str, Value)> {
    let mut at = artifact;
    for step in place {
        let Some(next) = at.get(*step) else {
            return Vec::new();
        };
        at = next;
    }
    accounted(
        at,
        &["entries"],
        &format!("the `{}` section", place.join(".")),
    );
    let Some(Value::Object(entries)) = at.get("entries") else {
        return Vec::new();
    };
    entries
        .iter()
        .map(|(name, value)| (name.as_str(), value.clone()))
        .collect()
}

/// The array under this key, or nothing when the key holds no array.
fn items<'a>(value: &'a Value, key: &str) -> &'a [Value] {
    value
        .get(key)
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice)
}

/// The same object without these top-level keys — the ones another section of a
/// plan owns, or the one repeating the key the entry sits under.
fn without(value: Value, keys: &[&str]) -> Value {
    let Value::Object(mut map) = value else {
        return value;
    };
    for key in keys {
        map.remove(*key);
    }
    Value::Object(map)
}

/// The same object with only these top-level keys.
fn only(value: Value, keys: &[&str]) -> Value {
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

/// Every subject the two artifacts disagree about.
///
/// A subject whose component is missing from either side is not one of them:
/// §3's rule is that what a component which arrived *contains* is not a diff, it
/// is the spec, so its nodes, its edges and its surfaces are the flow's own
/// arrival to report and nothing else's.
fn differing(
    before: &BTreeMap<Subject, Slice>,
    after: &BTreeMap<Subject, Slice>,
) -> BTreeSet<Subject> {
    let both: BTreeSet<&str> = components(before)
        .intersection(&components(after))
        .copied()
        .collect();
    let mut found = BTreeSet::new();
    for key in before.keys().chain(after.keys()) {
        let owner = before
            .get(key)
            .or_else(|| after.get(key))
            .and_then(|slice| slice.owner.as_deref());
        if owner.is_some_and(|owner| !both.contains(owner)) {
            continue;
        }
        match (before.get(key), after.get(key)) {
            (Some(old), Some(new)) if old.value == new.value => {}
            _ => {
                found.insert(key.clone());
            }
        }
    }
    found
}

/// The addresses of the components one side declares, which is what §3's rule is
/// stated over.
fn components(subjects: &BTreeMap<Subject, Slice>) -> BTreeSet<&str> {
    subjects
        .keys()
        .filter(|(section, _)| *section == COMPONENTS)
        .map(|(_, address)| address.as_str())
        .collect()
}

/// Every subject a plan names, keyed the way [`subjects`] keys them, and for
/// each the **places of the subject** its records reach into.
///
/// A path is `docs/plan.md` §3's — keys joined with `.`, array elements as
/// `[i]`, named entries as `[<name>]` — and it is taken as far as its **object
/// keys** go: `exec.command.text` in full, `exec.args[0].text` cut back to
/// `exec.args`. That is the granularity a plan can be held to from outside.
///
/// The cut is at the first `[` and not before, because the two sides of it are
/// different kinds of thing. Descending through an object key needs nothing but
/// the key: [`differing_keys`] reads the same key off the artifact, and the two
/// agree or the property fails. Descending *into an array* would mean deciding
/// which entry of one side is which entry of the other — by name for six keys of
/// the grammar, by identity for a flow's edges, by position for everything else
/// — and that pairing is precisely the algorithm this file exists not to share
/// with the code under test (see [`subjects`]). So an array is one place, named
/// by the key it sits under, and what a plan says *inside* it is held by
/// [`CASES`] instead.
///
/// Cutting only at the first step, which is what this took before, left every
/// key below one — `exec.command`, an `http:`'s `url`, a node's block, a
/// schema's internals — with no pair anywhere forcing a record: a rule that
/// swallowed one reported at the subject's first step all the same, and the sets
/// matched.
fn named_subjects(plan: &Plan) -> BTreeMap<Subject, BTreeSet<String>> {
    let mut found: BTreeMap<Subject, BTreeSet<String>> = BTreeMap::new();
    for change in &plan.components {
        found
            .entry((COMPONENTS, change.address.clone()))
            .or_default()
            .extend(heads(&change.fields));
    }
    for change in &plan.topology {
        let address = if change.site == TopologyKind::Edge {
            let flow = change
                .flow
                .as_deref()
                .expect("an edge belongs to a flow (`docs/plan.md` §5)");
            format!("{flow} edges")
        } else {
            change.address.clone()
        };
        found
            .entry((TOPOLOGY, address))
            .or_default()
            .extend(heads(&change.fields));
    }
    for change in &plan.interfaces {
        found
            .entry((INTERFACES, change.address.clone()))
            .or_default()
            .extend(heads(&change.fields));
    }
    found
}

/// Each of these paths cut back to its object keys: everything up to the first
/// `[`, which is where an array's entries begin. See [`named_subjects`].
fn heads(fields: &[FieldChange]) -> BTreeSet<String> {
    fields
        .iter()
        .map(|field| {
            let path = field.path.as_str();
            path[..path.find('[').unwrap_or(path.len())].to_string()
        })
        .collect()
}

/// Which places of one subject the two artifacts differ at.
///
/// The mirror of [`heads`], read off the artifact instead of off the report, and
/// cut at the same place: two objects are descended into key by key, and
/// anything that is not an object on both sides — an array, a scalar, a key one
/// side does not declare at all — is the place itself. A key on one side only
/// counts, and counts *there* rather than below: `docs/plan.md` §3 makes an
/// undeclared field a change like any other, because an absent `timeout:`
/// inherits and a declared one does not, and a plan reports it at the key rather
/// than at each key inside it.
///
/// Two unequal objects always differ at some key, so descending never loses a
/// difference: the recursion turns one place into at least one place, never into
/// none.
fn differing_keys(before: &Map<String, Value>, after: &Map<String, Value>) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    differing_below("", before, after, &mut found);
    found
}

fn differing_below(
    path: &str,
    before: &Map<String, Value>,
    after: &Map<String, Value>,
    found: &mut BTreeSet<String>,
) {
    // Over the union of the keys, each once: a key both sides declare would
    // otherwise be descended into twice, and twice again one level down.
    let keys: BTreeSet<&String> = before.keys().chain(after.keys()).collect();
    for key in keys {
        let at = below(path, key);
        match (before.get(key), after.get(key)) {
            (Some(old), Some(new)) if old == new => {}
            (Some(Value::Object(old)), Some(Value::Object(new))) => {
                differing_below(&at, old, new, found);
            }
            _ => {
                found.insert(at);
            }
        }
    }
}

/// Every place of one subject, whether or not any pair moves it — the same walk
/// as [`differing_below`], against nothing.
fn places(value: &Map<String, Value>) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    places_below("", value, &mut found);
    found
}

fn places_below(path: &str, value: &Map<String, Value>, found: &mut BTreeSet<String>) {
    for (key, held) in value {
        let at = below(path, key);
        match held {
            Value::Object(inner) if !inner.is_empty() => places_below(&at, inner, found),
            _ => {
                found.insert(at);
            }
        }
    }
}

/// A child place: the key on its own at the top of a subject, joined with a `.`
/// below it.
///
/// The same spelling `plan::diff` writes a path in, including what it does with
/// a key holding a `.` of its own — the two open surfaces admit one — which is
/// written as it stands and read as a route rather than parsed back into keys.
fn below(path: &str, key: &str) -> String {
    if path.is_empty() {
        key.to_string()
    } else {
        format!("{path}.{key}")
    }
}

// ---------------------------------------------------------------------------
// The property.
// ---------------------------------------------------------------------------

/// How many records the three structural sections hold.
fn structural(plan: &Plan) -> usize {
    plan.components.len() + plan.topology.len() + plan.interfaces.len()
}

/// The artifact of a project that must resolve cleanly enough to be planned.
///
/// Check-phase diagnostics are content rather than a refusal (`docs/plan.md`
/// §1), so a composition the validator rejects still has an artifact; what has
/// to succeed here is resolution.
fn artifact(entrypoint: &Path) -> Option<Ir> {
    resolve(entrypoint).ir
}

/// The property, over one pair: a plan names a subject exactly when the two
/// artifacts differ in the part of them that subject owns, and — the weaker
/// reading, which follows and is asserted anyway because it is stated over the
/// **whole** artifact rather than over the partition — the three structural
/// sections are empty exactly when the two artifacts agree on everything §11
/// does not excuse.
#[track_caller]
fn holds(what: &str, before: &Ir, after: &Ir) -> usize {
    let plan = plan(
        Composition {
            entrypoint: "before/main.yml",
            ir: before,
            resolution: &[],
        },
        Composition {
            entrypoint: "after/main.yml",
            ir: after,
            resolution: &[],
        },
    );

    let old = subjects(before);
    let new = subjects(after);
    let named = named_subjects(&plan);
    let differing = differing(&old, &new);
    let spoken: BTreeSet<Subject> = named.keys().cloned().collect();
    let silent: Vec<&Subject> = differing.difference(&spoken).collect();
    let invented: Vec<&Subject> = spoken.difference(&differing).collect();
    assert!(
        silent.is_empty(),
        "{what}: a plan said two compositions agree about {silent:?} when they do not — the two \
         artifacts differ there in something `docs/plan.md` §11 does not excuse, and no record of \
         the owning section names it."
    );
    assert!(
        invented.is_empty(),
        "{what}: a plan named {invented:?}, which the two artifacts do not differ in — either \
         `docs/plan.md` §11 excuses the difference and no record may name it, or §4–§6 give the \
         field to a different section than `subjects` does."
    );

    // …and, for a subject both sides declare, down to which of its keys. The
    // edges of a flow are one subject and not an object, so they are held to the
    // paragraph above and no further: an edge is matched by identity rather than
    // by address (§5), and reading its records back onto the artifact would mean
    // restating that pairing here.
    for (key, fields) in &named {
        let (Some(old), Some(new)) = (old.get(key), new.get(key)) else {
            continue;
        };
        let (Value::Object(old), Value::Object(new)) = (&old.value, &new.value) else {
            continue;
        };
        let differing = differing_keys(old, new);
        assert_eq!(
            fields, &differing,
            "{what}: a plan reports {key:?} changed at {fields:?}, and the two artifacts differ \
             at {differing:?}. A key of the artifact missing from the report is a field nobody \
             compares any more; a key of the report missing from the artifact is a line about an \
             edit nobody made."
        );
    }

    let reported = structural(&plan);
    let agree = compared(before) == compared(after);
    assert_eq!(
        reported == 0,
        agree,
        "{what}: {reported} structural record(s) against artifacts that {}. {}",
        if agree { "agree" } else { "differ" },
        if agree {
            "A plan reported an edit nobody made: `docs/plan.md` §11 says this difference is not \
             compared, so no record may name it."
        } else {
            "A plan said two compositions agree when they do not: the artifacts differ in \
             something `docs/plan.md` §11 does not excuse, and no section named it."
        }
    );
    reported
}

// ---------------------------------------------------------------------------
// Corpus 1: every pair the CLI's own golden corpus is built from.
// ---------------------------------------------------------------------------

/// The plan command's fixture corpus, which is `crates/agent-compose/tests`'.
///
/// Shared rather than duplicated: those pairs are chosen one per sentence of
/// `docs/plan.md`, so they are exactly the pairs whose reports are worth holding
/// to the property that produced them.
fn corpus() -> PathBuf {
    repository().join("crates/agent-compose/tests/projects/plan")
}

/// Every pair of the corpus, by name.
fn corpus_pairs() -> Vec<String> {
    let mut found: Vec<String> = fs::read_dir(corpus())
        .expect("the plan corpus is readable")
        .map(|entry| entry.expect("a directory entry").file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .collect();
    found.sort();
    found
}

#[test]
fn every_pair_of_the_corpus_reports_exactly_the_differences_it_has() {
    let pairs = corpus_pairs();
    assert!(pairs.len() >= 10, "the corpus is populated: {pairs:?}");

    let mut skipped = Vec::new();
    let mut checked = 0usize;
    for name in &pairs {
        let before = artifact(&corpus().join(name).join("before/main.yml"));
        let after = artifact(&corpus().join(name).join("after/main.yml"));
        match (before, after) {
            (Some(before), Some(after)) => {
                holds(name, &before, &after);
                // …and the property is symmetric, because the diff is.
                holds(&format!("{name} (reversed)"), &after, &before);
                checked += 1;
            }
            _ => skipped.push(name.clone()),
        }
    }

    // A pair is skipped only because it exists to test a spec with no artifact,
    // and those are named for it. Anything else silently dropping out of this
    // corpus is the failure this assertion is here for.
    assert!(
        skipped.iter().all(|name| name.starts_with("unresolvable-")),
        "only the deliberately unresolvable pairs have no artifact, and these were skipped: \
         {skipped:?}"
    );
    assert_eq!(
        checked + skipped.len(),
        pairs.len(),
        "every pair is either checked or accounted for"
    );
}

// ---------------------------------------------------------------------------
// Corpus 2: one composition, edited one construct at a time.
// ---------------------------------------------------------------------------

/// One case: the two sides as edits to the composition they both start from.
struct Case {
    /// What the case is about, and what a failure names.
    what: &'static str,
    /// Applied to the before side.
    before: &'static [Edit],
    /// Applied to the after side.
    after: &'static [Edit],
    /// Whether the two are expected to differ in anything a plan reports.
    ///
    /// The property is a biconditional and would hold with both sides equal, so
    /// a mutation that silently failed to apply would pass it. This is what says
    /// which way each case is supposed to come out.
    differs: bool,
}

/// One textual edit to one file of the composition: exactly one occurrence,
/// replaced.
type Edit = (&'static str, &'static str, &'static str);

/// The composition every case starts from: `examples/triage-fanout`, which is
/// the widest one in the tree — a discriminator-routed `map` with a catch-all, a
/// `human` node, an inline `exec:` and an inline `http:`, kv and vector stores,
/// reduced channels, four kinds of trigger, and a deploy layer with placements.
const BASE: &str = "examples/triage-fanout";

/// A channel whose default is an author's literal array of objects, and the
/// same channel with the two elements the other way round.
///
/// Reversing it hands every caller a different value — the emitted schema reads
/// `.default([{"name":"beta",…},{"name":"alpha",…}])` — so it is a change. What
/// makes it worth a case of its own is that its elements are shaped exactly like
/// a field map's: a rule that classified an array by its elements rather than by
/// the key above it reports nothing here.
const SEEDS: &str = r#"  seeds:
    type: array
    max_items: 4
    items:
      type: object
      properties:
        name: { type: string }
        weight: { type: integer }
    default:
      - { name: alpha, weight: 1 }
      - { name: beta, weight: 2 }

  human_decision:"#;
const SEEDS_SWAPPED: &str = r#"  seeds:
    type: array
    max_items: 4
    items:
      type: object
      properties:
        name: { type: string }
        weight: { type: integer }
    default:
      - { name: beta, weight: 2 }
      - { name: alpha, weight: 1 }

  human_decision:"#;

/// The same channel a third time, with the two **keys** of one of those elements
/// written the other way round rather than the two elements.
///
/// The pair to read against [`SEEDS_SWAPPED`], because the two edits look alike
/// and come out opposite ways. An array's positions are part of its value, so
/// reversing the elements hands every caller a different one. A mapping's keys
/// are not: `{name: alpha, weight: 1}` and `{weight: 1, name: alpha}` answer
/// every lookup identically, and `docs/plan.md` §11 accounts for the one thing
/// that does read differently — the order the literal is *written out* in the
/// emitted project.
///
/// This is the clause [`compared`] inherits from `serde_json` rather than
/// stating, so the case is the only place it is held to an edit at all.
const SEEDS_REMAPPED: &str = r#"  seeds:
    type: array
    max_items: 4
    items:
      type: object
      properties:
        name: { type: string }
        weight: { type: integer }
    default:
      - { weight: 1, name: alpha }
      - { name: beta, weight: 2 }

  human_decision:"#;

/// A node's two input bindings, and the same two swapped. Dispatched on by name.
const BINDINGS: &str = r#"        pattern: "input.pattern"
        max_matches: "25""#;
const BINDINGS_SWAPPED: &str = r#"        max_matches: "25"
        pattern: "input.pattern""#;

/// An `exec:`'s environment, planted with a second entry so that there is an
/// order to reorder, and the same two swapped. Looked up by name by the child
/// process, so the order is nothing the composition decides.
const ENV: &str = r#"        env:
          CI: "true"
          TZ: "UTC""#;
const ENV_SWAPPED: &str = r#"        env:
          TZ: "UTC"
          CI: "true""#;

/// The same for an `http:` block's headers, which the request builder also looks
/// up by name. The `body:` below is what tells `announce`'s block from
/// `escalate`'s.
const ONE_HEADER: &str = r#"        headers:
          authorization: "Bearer ${QUEUE_TOKEN}"
        body:
          report: "state.report_normalized""#;
const HEADERS: &str = r#"        headers:
          authorization: "Bearer ${QUEUE_TOKEN}"
          x-triage-source: "compose"
        body:
          report: "state.report_normalized""#;
const HEADERS_SWAPPED: &str = r#"        headers:
          x-triage-source: "compose"
          authorization: "Bearer ${QUEUE_TOKEN}"
        body:
          report: "state.report_normalized""#;

/// The `dispatch` map's two named routes, and the same two swapped.
const ROUTES: &str = r#"          auto_fixable:
            node: agent.fixer
            max_concurrency: 4
            input:
              file: "finding.file"
              patch_hint: "finding.patch_hint"
            writes:
              patch: patches
          needs_human:
            node: tool.review_queue
            input:
              summary: "finding.summary"
              severity: "finding.severity""#;
const ROUTES_SWAPPED: &str = r#"          needs_human:
            node: tool.review_queue
            input:
              summary: "finding.summary"
              severity: "finding.severity"
          auto_fixable:
            node: agent.fixer
            max_concurrency: 4
            input:
              file: "finding.file"
              patch_hint: "finding.patch_hint"
            writes:
              patch: patches"#;

/// Two nodes of `flow.triage`, and the same two swapped. The graph is the
/// edges', not this mapping's (`docs/plan.md` §5).
const NODES: &str = r#"    scan:
      function: tool.repo_grep
      input:
        pattern: "input.pattern"
        max_matches: "25"

    classify:
      agent: agent.triage
      input:
        report: "state.report_normalized"
        matches: "state.matches""#;
const NODES_SWAPPED: &str = r#"    classify:
      agent: agent.triage
      input:
        report: "state.report_normalized"
        matches: "state.matches"

    scan:
      function: tool.repo_grep
      input:
        pattern: "input.pattern"
        max_matches: "25""#;

/// Two variants of `agent.triage`'s union, and the same two swapped. This order
/// is the order of the `oneOf` the model is handed.
const VARIANTS: &str = r#"          auto_fixable:
            file: { type: string }
            patch_hint: { type: string }
          needs_human:
            summary: { type: string }
            severity: { enum: [low, high, critical] }"#;
const VARIANTS_SWAPPED: &str = r#"          needs_human:
            summary: { type: string }
            severity: { enum: [low, high, critical] }
          auto_fixable:
            file: { type: string }
            patch_hint: { type: string }"#;

/// Two fields of `flow.triage`'s input surface, and the same two swapped. This
/// order is the order of a JSON Schema's `properties` and `required`.
const INPUTS: &str = r#"    report:
      description: The raw bug report.
      type: string
      min_length: 1
    pattern:
      description: Repository search pattern used to ground the triage agent.
      type: string"#;
const INPUTS_SWAPPED: &str = r#"    pattern:
      description: Repository search pattern used to ground the triage agent.
      type: string
    report:
      description: The raw bug report.
      type: string
      min_length: 1"#;

/// Two outgoing edges of one node, and the same two swapped. Grammar 7.3 rule 1
/// evaluates them in this order and a trace records the node's decision in it
/// (`docs/plan.md` §5), so the swap is reported — it does not decide which of
/// the two fires, which is rule 6's multicast.
const SIBLINGS: &str = r#"    - { from: approve, to: end, when: "approve.output.decision == 'approve'" }
    - { from: approve, to: escalate, when: "approve.output.decision == 'reject'" }"#;
const SIBLINGS_SWAPPED: &str = r#"    - { from: approve, to: escalate, when: "approve.output.decision == 'reject'" }
    - { from: approve, to: end, when: "approve.output.decision == 'approve'" }"#;

/// The `hub:` block of the `local` deploy layer, and the same file with an event
/// source planted under it so that there is one to edit — `local` declares none,
/// and `event_sources:` is the reserved section it admits beside the two live
/// ones (`docs/plan.md` §11).
///
/// The anchor is the hub rather than a placement, because `local` places
/// nothing: a placed component is dispatched to whichever worker claims its
/// placement (`docs/distributed.md` §3), so a target whose runs are
/// single-process leaves `placements:` to `deploy/staging.yml` — which is where
/// this corpus's placement coverage comes from.
const HUB: &str = r#"hub:
  join_token: ${MESH_JOIN_TOKEN}"#;
const EVENT_SOURCE: &str = r#"hub:
  join_token: ${MESH_JOIN_TOKEN}

event_sources:
  bug_reports:
    kind: redis_streams
    url: ${REDIS_URL}
    stream: bug-reports
    consumer_group: agent-compose"#;
const EVENT_SOURCE_EMPTY: &str = r#"hub:
  join_token: ${MESH_JOIN_TOKEN}

event_sources: {}"#;
const EVENT_SOURCE_MOVED: &str = r#"hub:
  join_token: ${MESH_JOIN_TOKEN}

event_sources:
  bug_reports:
    kind: redis_streams
    url: ${REDIS_URL}
    stream: triage-reports
    consumer_group: agent-compose"#;

/// A trace sink planted under the same anchor, and the two edits a plan has to
/// report about one (grammar §14.5, `docs/plan.md` §4).
///
/// Planted for [`EVENT_SOURCE`]'s reason and one of its own: `local` admits the
/// section — a laptop's `run` settles executions like any other target — but the
/// example carries no sink under `local`, and a section a plan stopped reporting
/// would let a deployment repoint every trace it emits, or switch the wire
/// format under a collector, with nothing in the review.
///
/// The second edit moves `format:` rather than `url:` on purpose: the default is
/// applied in the IR, so `format: envelope` and no `format:` at all are the same
/// artifact, and the pair that has to be reported is the one where the *decided*
/// value moves.
const TRACE_SINK: &str = r#"hub:
  join_token: ${MESH_JOIN_TOKEN}

trace_sink:
  url: "https://collector.internal.example/v1/traces"
  auth:
    bearer:
      token: ${TRACE_SINK_TOKEN}"#;
const TRACE_SINK_REPOINTED: &str = r#"hub:
  join_token: ${MESH_JOIN_TOKEN}

trace_sink:
  url: "https://collector.eu.internal.example/v1/traces"
  auth:
    bearer:
      token: ${TRACE_SINK_TOKEN}"#;
const TRACE_SINK_AS_OTLP: &str = r#"hub:
  join_token: ${MESH_JOIN_TOKEN}

trace_sink:
  url: "https://collector.internal.example/v1/traces"
  format: otlp
  auth:
    bearer:
      token: ${TRACE_SINK_TOKEN}"#;

/// A package registry planted under the same anchor, and the edits a plan has to
/// report about one (grammar §14.6, `docs/plan.md` §4).
///
/// Planted for [`TRACE_SINK`]'s reason exactly: `local` admits the section — a
/// project built on a laptop is one somebody runs `bun install` in, on the same
/// network — and the example carries none under `local`. A section a plan
/// stopped reporting would let a deployment repoint where every machine
/// downloads its code from, or move which variable authenticates that download,
/// with nothing in the review.
const PACKAGE_REGISTRY: &str = r#"hub:
  join_token: ${MESH_JOIN_TOKEN}

package_registry:
  url: "https://npm.internal.example/repository/npm-group/"
  token: ${NPM_MIRROR_TOKEN}"#;
const PACKAGE_REGISTRY_REPOINTED: &str = r#"hub:
  join_token: ${MESH_JOIN_TOKEN}

package_registry:
  url: "https://npm.eu.internal.example/repository/npm-group/"
  token: ${NPM_MIRROR_TOKEN}"#;
const PACKAGE_REGISTRY_SCOPED: &str = r#"hub:
  join_token: ${MESH_JOIN_TOKEN}

package_registry:
  url: "https://npm.internal.example/repository/npm-group/"
  token: ${NPM_MIRROR_TOKEN}
  scopes:
    "@corp":
      url: "https://npm.internal.example/repository/corp/"
      token: ${NPM_CORP_TOKEN}"#;

/// A placement planted under the same anchor, and the two edits a plan has to
/// report about one (grammar §14.1, `docs/plan.md` §4).
///
/// Planted rather than read out of the example, for [`EVENT_SOURCE`]'s reason:
/// the target this corpus resolves under is `local`, whose runs are
/// single-process and which therefore places nothing. The section still has to
/// be covered — a `plan` that stopped reporting a placement would let a
/// deployment move a component to another machine with nothing in the review —
/// so the pairs bring their own.
const PLACEMENT: &str = r#"hub:
  join_token: ${MESH_JOIN_TOKEN}

placements:
  patchers:
    members: [agent.fixer]
    description: The machine holding a checkout."#;
const PLACEMENT_WIDENED: &str = r#"hub:
  join_token: ${MESH_JOIN_TOKEN}

placements:
  patchers:
    members: [agent.fixer, tool.repo_grep]
    description: The machine holding a checkout."#;
const PLACEMENT_REWORDED: &str = r#"hub:
  join_token: ${MESH_JOIN_TOKEN}

placements:
  patchers:
    members: [agent.fixer]
    description: The machine with a checkout on it."#;

/// Two edges of **different** nodes, and the same two swapped. Neither node's
/// own outgoing order moves, so neither node's routing decision reads
/// differently.
const STRANGERS: &str = r#"    - { from: dispatch, to: verify }
    - { from: announce, to: verify }"#;
const STRANGERS_SWAPPED: &str = r#"    - { from: announce, to: verify }
    - { from: dispatch, to: verify }"#;

/// The `dispatch` map's `routes:` key and everything under it, and the
/// homogeneous dispatch that replaces it.
///
/// Rewriting the block the other way is the only edit that moves `map.dispatch`,
/// the tag `MapDispatch` is written under: the two forms are `route_by:` +
/// `routes:` and a single `node:`, and nothing an author can type moves one to
/// the other without moving the whole block (grammar 8.6 rule 2).
const ROUTED: &str = r#"        routes:
          auto_fixable:
            node: agent.fixer
            max_concurrency: 4
            input:
              file: "finding.file"
              patch_hint: "finding.patch_hint"
            writes:
              patch: patches
          needs_human:
            node: tool.review_queue
            input:
              summary: "finding.summary"
              severity: "finding.severity""#;
const HOMOGENEOUS: &str = r#"        input:
          file: "finding.file""#;

/// The catch-all route, with the newline that follows it — taken out whole by
/// the same case, because a `default:` is narrowed against the routes it is the
/// complement of and there are none left.
const CATCH_ALL: &str = r#"        default:
          node: tool.dead_letter
          detach: false
          input:
            kind: "finding.kind"
            payload: "finding.of"
"#;

/// The catch-all's own bindings, and the one unnamed value that replaces them —
/// which is the other form a `map` dispatch's `input:` takes (Decision D88).
const CATCH_ALL_BINDINGS: &str = r#"          input:
            kind: "finding.kind"
            payload: "finding.of""#;
const CATCH_ALL_VALUE: &str = r#"          input: "finding.of""#;

/// A node's own bindings, and the same as one unnamed value — legal on an
/// inline `exec:` node, whose stdin is where it goes (Decision D88).
const STDIN_BINDINGS: &str = r#"      input:
        patches: "state.patches""#;
const STDIN_VALUE: &str = r#"      input: "state.patches""#;

/// A `kv set`'s field map of expressions, and the single expression that
/// replaces it — the two forms of a store op's `value:` (grammar 11.4).
///
/// The op moves with it, because the form is the op's: a `kv set` writes a field
/// map matching the store's `value_schema`, and only a `vector upsert` or a
/// `blob put` writes one expression. `key:` stays, which is what keeps the pair
/// down to the three places the form itself moves.
const STORE_FIELDS: &str = r#"      store: store.triage_memory
      op: set
      key: "execution.session_key"
      value:
        last_report: "input.report"
        patch_count: "size(state.patches)""#;
const STORE_EXPRESSION: &str = r#"      store: store.docs
      op: upsert
      key: "execution.session_key"
      value: "input.report""#;

/// The `matches` channel's element type, which is where a schema's *inside* is
/// edited: `patches:` declares the same two lines under a `max_items:`, so the
/// anchor is the pair rather than the `items:` line on its own.
const ELEMENT: &str = r#"    type: array
    items: { type: string }"#;
const ELEMENT_RETYPED: &str = r#"    type: array
    items: { type: integer }"#;
const ELEMENT_REFORMED: &str = r#"    type: array
    items: { enum: [hit, miss] }"#;
const NO_ELEMENTS: &str = r#"    type: string"#;

/// `verify`'s failure mode, anchored to the key above it because three other
/// nodes of the flow declare the same one.
const VERIFY_ON_ERROR: &str = r#"      timeout: 5m
      on_error: skip"#;
const VERIFY_ON_ERROR_FAILS: &str = r#"      timeout: 5m
      on_error: fail"#;

/// `escalate`'s request body, anchored to the key above it because `approve`
/// binds the same channel to the same name.
const ESCALATION_BODY: &str = r#"        body:
          summary: "state.summary""#;
const ESCALATION_BODY_MOVED: &str = r#"        body:
          summary: "state.report_normalized""#;

/// One edit per construct whose comparison rule is not the obvious one — and,
/// beside each, the edit that looks like it and must come out the other way.
const CASES: &[Case] = &[
    // --- The two open surfaces, where the author chooses the keys. -----------
    Case {
        what: "an author's literal array of objects reordered",
        before: &[("main.yml", "  human_decision:", SEEDS)],
        after: &[("main.yml", "  human_decision:", SEEDS_SWAPPED)],
        differs: true,
    },
    Case {
        what: "an author's literal mapping reordered",
        before: &[("main.yml", "  human_decision:", SEEDS)],
        after: &[("main.yml", "  human_decision:", SEEDS_REMAPPED)],
        differs: false,
    },
    Case {
        what: "a literal scalar default changed",
        before: &[],
        after: &[("main.yml", "    default: approve", "    default: reject")],
        differs: true,
    },
    // --- The four memberships, whose order is never compared. ----------------
    Case {
        what: "a set of accepted exit statuses reordered",
        before: &[],
        after: &[(
            "flows/triage.yml",
            "        expect_exit: [0, 1]",
            "        expect_exit: [1, 0]",
        )],
        differs: false,
    },
    Case {
        what: "a set of accepted exit statuses grown",
        before: &[],
        after: &[(
            "flows/triage.yml",
            "        expect_exit: [0, 1]",
            "        expect_exit: [0, 1, 2]",
        )],
        differs: true,
    },
    // --- The arrays the generated code looks up by name. ---------------------
    Case {
        what: "two node input bindings swapped",
        before: &[],
        after: &[("flows/triage.yml", BINDINGS, BINDINGS_SWAPPED)],
        differs: false,
    },
    Case {
        what: "two map routes swapped",
        before: &[],
        after: &[("flows/triage.yml", ROUTES, ROUTES_SWAPPED)],
        differs: false,
    },
    Case {
        what: "two nodes swapped in the declaring mapping",
        before: &[],
        after: &[("flows/triage.yml", NODES, NODES_SWAPPED)],
        differs: false,
    },
    Case {
        what: "two environment entries swapped",
        before: &[(
            "flows/triage.yml",
            "        env:\n          CI: \"true\"",
            ENV,
        )],
        after: &[(
            "flows/triage.yml",
            "        env:\n          CI: \"true\"",
            ENV_SWAPPED,
        )],
        differs: false,
    },
    Case {
        what: "two request headers swapped",
        before: &[("flows/triage.yml", ONE_HEADER, HEADERS)],
        after: &[("flows/triage.yml", ONE_HEADER, HEADERS_SWAPPED)],
        differs: false,
    },
    Case {
        what: "an environment value changed",
        before: &[],
        after: &[(
            "flows/triage.yml",
            "          CI: \"true\"",
            "          CI: \"false\"",
        )],
        differs: true,
    },
    // --- The two orders that reach the model. --------------------------------
    Case {
        what: "two union variants swapped",
        before: &[],
        after: &[("agents/triage.yml", VARIANTS, VARIANTS_SWAPPED)],
        differs: true,
    },
    Case {
        what: "two fields of a flow's input surface swapped",
        before: &[],
        after: &[("flows/triage.yml", INPUTS, INPUTS_SWAPPED)],
        differs: true,
    },
    // --- Edges, whose reported order is per source node. ----------------------
    Case {
        what: "two edges of one node swapped",
        before: &[],
        after: &[("flows/triage.yml", SIBLINGS, SIBLINGS_SWAPPED)],
        differs: true,
    },
    Case {
        what: "two edges of different nodes swapped",
        before: &[],
        after: &[("flows/triage.yml", STRANGERS, STRANGERS_SWAPPED)],
        differs: false,
    },
    Case {
        what: "an edge retargeted",
        before: &[],
        after: &[(
            "flows/triage.yml",
            "    - { from: verify, to: summarize }",
            "    - { from: verify, to: remember }",
        )],
        differs: true,
    },
    // --- A default written out, which the artifact holds and a plan reports. --
    Case {
        what: "a default written out as the value the grammar already supplies",
        before: &[],
        after: &[(
            "agents/triage.yml",
            "  stores: [store.docs]",
            "  stores: [store.docs]\n  max_tool_iterations: 8",
        )],
        differs: true,
    },
    // --- The three leaves the artifact keeps as source text (§11). ------------
    Case {
        what: "a duration respelled",
        before: &[],
        after: &[("flows/triage.yml", "timeout: 24h", "timeout: 1440m")],
        differs: true,
    },
    Case {
        what: "a guard respelled",
        before: &[],
        after: &[(
            "flows/triage.yml",
            "when: \"approve.output.decision == 'reject'\"",
            "when: \"approve.output.decision=='reject'\"",
        )],
        differs: true,
    },
    // --- Files, which are authoring UX and nothing the composition does. ------
    Case {
        what: "an import list reordered",
        before: &[],
        after: &[(
            "main.yml",
            "  - providers.yml\n  - models.yml",
            "  - models.yml\n  - providers.yml",
        )],
        differs: false,
    },
    Case {
        what: "a comment and a blank line inserted",
        before: &[],
        after: &[(
            "flows/triage.yml",
            "  edges:",
            "  # Every edge, in the order grammar 7.3 evaluates them.\n\n  edges:",
        )],
        differs: false,
    },
    // --- A section written with nothing in it, which §11's last bullet excuses.
    Case {
        what: "a section declared with no entries where the other side declares none",
        before: &[],
        after: &[("deploy/local.yml", HUB, EVENT_SOURCE_EMPTY)],
        differs: false,
    },
    // --- One field of one component, with nothing else moving. ----------------
    //
    // A section that stopped comparing a whole class of field reports nothing
    // for a component whose *only* edit is one of that class — and every pair of
    // the corpus above moves several constructs at once, so the record some
    // other construct produces would cover for it. Each of these five is one
    // key of one component and nothing else, so the class has nowhere to hide.
    Case {
        what: "a definition's description reworded",
        before: &[],
        after: &[(
            "agents/triage.yml",
            "  description: Classifies bug-report findings into routable variants.",
            "  description: Sorts incoming bug-report findings into routable variants.",
        )],
        differs: true,
    },
    Case {
        what: "a model's id retargeted",
        before: &[],
        after: &[(
            "models.yml",
            "  id: qwen3-coder-30b",
            "  id: qwen3-coder-14b",
        )],
        differs: true,
    },
    Case {
        what: "a model setting retuned",
        before: &[],
        after: &[("models.yml", "    temperature: 0.2", "    temperature: 0.3")],
        differs: true,
    },
    Case {
        what: "a store's agent access widened",
        before: &[],
        after: &[(
            "stores/docs.yml",
            "  agent_access: read\n",
            "  agent_access: read_write\n",
        )],
        differs: true,
    },
    Case {
        what: "an event source's stream repointed",
        before: &[("deploy/local.yml", HUB, EVENT_SOURCE)],
        after: &[("deploy/local.yml", HUB, EVENT_SOURCE_MOVED)],
        differs: true,
    },
    Case {
        what: "a trace sink repointed at another collector",
        before: &[("deploy/local.yml", HUB, TRACE_SINK)],
        after: &[("deploy/local.yml", HUB, TRACE_SINK_REPOINTED)],
        differs: true,
    },
    Case {
        what: "a trace sink switched to the OTLP wire format",
        before: &[("deploy/local.yml", HUB, TRACE_SINK)],
        after: &[("deploy/local.yml", HUB, TRACE_SINK_AS_OTLP)],
        differs: true,
    },
    Case {
        what: "a trace sink added to a target that had none",
        before: &[],
        after: &[("deploy/local.yml", HUB, TRACE_SINK)],
        differs: true,
    },
    Case {
        what: "a package registry repointed at another mirror",
        before: &[("deploy/local.yml", HUB, PACKAGE_REGISTRY)],
        after: &[("deploy/local.yml", HUB, PACKAGE_REGISTRY_REPOINTED)],
        differs: true,
    },
    Case {
        what: "a package registry given a scope of its own",
        before: &[("deploy/local.yml", HUB, PACKAGE_REGISTRY)],
        after: &[("deploy/local.yml", HUB, PACKAGE_REGISTRY_SCOPED)],
        differs: true,
    },
    Case {
        what: "a package registry added to a target that had none",
        before: &[],
        after: &[("deploy/local.yml", HUB, PACKAGE_REGISTRY)],
        differs: true,
    },
    // --- Every remaining key of one component, swept a component at a time. ---
    //
    // The property is stated over the **set** of keys a record names against the
    // set the two artifacts differ at, so a case moving eight keys of one
    // component holds all eight: swallow any one of them and the two sets differ
    // by that key, whatever the other seven still report. One case per component
    // kind is therefore what it takes to reach the keys a single-edit case would
    // need a case each for.
    Case {
        what: "a store rewritten across the keys it declares",
        before: &[],
        after: &[
            ("stores/docs.yml", "  scope: global", "  scope: session"),
            (
                "stores/docs.yml",
                "  backend: docs_db",
                "  backend: project_docs",
            ),
            (
                "stores/docs.yml",
                "    model: text-embedding-3-small",
                "    model: text-embedding-3-large",
            ),
            (
                "stores/docs.yml",
                "    provider: provider.local",
                "    provider: provider.anthropic",
            ),
            (
                "stores/docs.yml",
                "    dimensions: 1536",
                "    dimensions: 768",
            ),
            (
                "stores/docs.yml",
                "    source: { type: string }",
                "    source: { type: string, min_length: 1 }",
            ),
            (
                "stores/docs.yml",
                "  description: Project documentation, chunked, for grounding triage decisions.",
                "  description: Project documentation for grounding triage decisions.",
            ),
            (
                "stores/triage_memory.yml",
                "      minimum: 0",
                "      minimum: 1",
            ),
        ],
        differs: true,
    },
    Case {
        what: "a provider re-keyed",
        before: &[],
        after: &[
            ("providers.yml", "  kind: anthropic", "  kind: openai"),
            (
                "providers.yml",
                "  api_key: ${ANTHROPIC_API_KEY}",
                "  api_key: ${OPENAI_API_KEY}",
            ),
            (
                "providers.yml",
                "  base_url: ${LOCAL_LLM_URL}",
                "  base_url: ${LLM_URL}",
            ),
        ],
        differs: true,
    },
    Case {
        what: "a model re-provisioned",
        before: &[],
        after: &[(
            "models.yml",
            "  provider: provider.local",
            "  provider: provider.anthropic",
        )],
        differs: true,
    },
    // A provider's server-tool suite is what the model is offered inside every
    // call the connection serves (grammar 12.1, Decision D122), so tightening it
    // changes what the graph can do without touching a node — exactly the kind
    // of edit a plan exists to surface.
    Case {
        what: "a server tool's budget narrowed",
        before: &[],
        after: &[("providers.yml", "      max_uses: 3", "      max_uses: 1")],
        differs: true,
    },
    // A built-in's `workspace:` is the whole of what bounds it (grammar 6.1,
    // Decision D135), so moving it changes what an agent may reach without
    // touching a node, a prompt or a schema — the same shape of edit as the one
    // above, on the side of the wire this runtime dispatches.
    Case {
        what: "a built-in's workspace widened",
        before: &[],
        after: &[(
            "tools/checkout.yml",
            "  workspace: \"${REPO_ROOT}\"",
            "  workspace: \"${REPO_ROOT}/src\"",
        )],
        differs: true,
    },
    // …and the same bound moved to a *different* machine fact, which is the
    // half of a class-2 surface a plan reads separately: the env refs the
    // workspace names, rather than the text around them (grammar 4.3).
    Case {
        what: "a built-in's workspace read from another variable",
        before: &[],
        after: &[(
            "tools/checkout.yml",
            "  workspace: \"${REPO_ROOT}\"",
            "  workspace: \"${CHECKOUT_ROOT}\"",
        )],
        differs: true,
    },
    // …and which built-in a tool binds is the capability itself: swapping the
    // file editor for a shell is the widest edit this grammar admits without a
    // node changing, so a plan that did not report it would hide the one thing
    // PRD resolved q54 asks to be said loudly.
    Case {
        what: "a built-in tool rebound to the shell",
        before: &[],
        after: &[("tools/checkout.yml", "  builtin: files", "  builtin: bash")],
        differs: true,
    },
    Case {
        what: "a tool's signature and its binding retuned",
        before: &[],
        after: &[
            (
                "tools/repo_grep.yml",
                "  description: Search the repository for a pattern and return matching lines.",
                "  description: Search the repository and return matching lines.",
            ),
            (
                "tools/repo_grep.yml",
                "      maximum: 200",
                "      maximum: 100",
            ),
            (
                "tools/repo_grep.yml",
                "      max_items: 200",
                "      max_items: 150",
            ),
            (
                "tools/review_queue.yml",
                "    expect_status: [201]",
                "    expect_status: [200, 201]",
            ),
        ],
        differs: true,
    },
    Case {
        what: "an agent given a store and a tool",
        before: &[],
        after: &[
            (
                "agents/triage.yml",
                "  stores: [store.docs]",
                "  stores: [store.docs]\n  tools: [tool.repo_grep]",
            ),
            (
                "agents/fixer.yml",
                "  model: model.fast",
                "  model: model.fast\n  stores: [store.docs]",
            ),
        ],
        differs: true,
    },
    Case {
        what: "a trigger's delivery surface rewritten",
        before: &[],
        after: &[
            (
                "triggers.yml",
                "    cron: \"0 3 * * *\"",
                "    cron: \"0 4 * * *\"",
            ),
            (
                "triggers.yml",
                "    timezone: UTC",
                "    timezone: Europe/Berlin",
            ),
            (
                "triggers.yml",
                "    source: bug_reports",
                "    source: triage_reports",
            ),
            (
                "triggers.yml",
                "    dedupe_key: \"payload.id\"",
                "    dedupe_key: \"payload.message_id\"",
            ),
            (
                "triggers.yml",
                "    callback: \"payload.body.callback_url\"",
                "    callback: \"payload.body.done_url\"",
            ),
            (
                "triggers.yml",
                "      report: \"payload.body.text\"",
                "      report: \"payload.body.summary\"",
            ),
        ],
        differs: true,
    },
    Case {
        what: "a trigger pointed at another flow",
        before: &[],
        after: &[(
            "triggers.yml",
            "  cli:\n    type: manual\n    flow: flow.triage",
            "  cli:\n    type: manual\n    flow: flow.enrich",
        )],
        differs: true,
    },
    Case {
        what: "the policy defaults given another failure mode",
        before: &[],
        after: &[("main.yml", "  on_error: fail", "  on_error: skip")],
        differs: true,
    },
    Case {
        what: "a flow's description reworded",
        before: &[],
        after: &[(
            "flows/triage.yml",
            "  description: Triage a bug report, fan out fixes, and get a human sign-off.",
            "  description: Triage a bug report and get a sign-off.",
        )],
        differs: true,
    },
    // --- The tag keys, which a construct's whole shape hangs off. -------------
    //
    // Each of these is one key of the artifact and a different construct on the
    // two sides, so the record has to name the tag *and* every key that arrived
    // or left with it. `namespace` is the one tag no case can move: it follows
    // from the address (grammar 2.2), and two definitions are compared only
    // where they sit at the same address.
    Case {
        what: "a bound model made a failover route",
        before: &[],
        after: &[(
            "models.yml",
            "model.offline:\n  provider: provider.local\n  id: qwen3-coder-30b\n  settings:\n    temperature: 0.2",
            "model.offline:\n  route: [model.fast, model.smart]\n  route_on: [rate_limit]",
        )],
        differs: true,
    },
    Case {
        what: "a kv store made a blob store",
        before: &[],
        after: &[
            (
                "stores/triage_memory.yml",
                "  kind: kv\n  scope: session",
                "  kind: blob\n  scope: session",
            ),
            (
                "stores/triage_memory.yml",
                "  value_schema:\n    last_report:\n      description: The most recent report triaged in this session.\n      type: string\n    patch_count:\n      description: How many patches the last run produced.\n      type: integer\n      minimum: 0\n",
                "",
            ),
        ],
        differs: true,
    },
    Case {
        what: "an exec-bound tool made an http-bound one",
        before: &[],
        after: &[(
            "tools/repo_grep.yml",
            "  exec:\n    command: repo-grep\n    args: [\"--format\", \"json\"]\n    cwd: \"${REPO_ROOT}\"\n    env:\n      RIPGREP_CONFIG_PATH: \"${RG_CONFIG_PATH}\"",
            "  http:\n    method: POST\n    url: \"https://${QUEUE_HOST}/v1/grep\"\n    body:\n      pattern: \"input.pattern\"",
        )],
        differs: true,
    },
    Case {
        what: "a manual trigger made an http one",
        before: &[],
        after: &[(
            "triggers.yml",
            "  cli:\n    type: manual",
            "  cli:\n    type: http\n    path: /cli\n    method: POST",
        )],
        differs: true,
    },
    Case {
        what: "a subgraph node pointed at another flow",
        before: &[],
        after: &[(
            "flows/triage.yml",
            "      flow: flow.enrich",
            "      flow: flow.triage",
        )],
        differs: true,
    },
    Case {
        what: "a store-op node's operation changed",
        before: &[],
        after: &[(
            "flows/triage.yml",
            "      op: set\n      key: \"execution.session_key\"\n      value:\n        last_report: \"input.report\"\n        patch_count: \"size(state.patches)\"",
            "      op: get\n      key: \"execution.session_key\"",
        )],
        differs: true,
    },
    // --- One field of one node, the same way. ---------------------------------
    Case {
        what: "a node re-kinded, and the rest of the graph's kinds moved with it",
        before: &[],
        after: &[
            (
                "flows/triage.yml",
                "    summarize:\n      agent: agent.summarizer",
                "    summarize:\n      function: tool.repo_grep",
            ),
            (
                "flows/triage.yml",
                "      context: isolated",
                "      context: inherit",
            ),
            (
                "flows/triage.yml",
                "      store: store.triage_memory",
                "      store: store.docs",
            ),
            (
                "flows/triage.yml",
                "        report: \"input.report\"",
                "        report: \"state.report_normalized\"",
            ),
            (
                "flows/triage.yml",
                "        timeout: 30s",
                "        timeout: 45s",
            ),
            (
                "flows/triage.yml",
                "      retry: { max: 3, backoff: 1s }",
                "      retry: { max: 2, backoff: 1s }",
            ),
            (
                "flows/triage.yml",
                "\"https://${QUEUE_HOST}/v1/escalations\"",
                "\"https://${QUEUE_HOST}/v1/escalate\"",
            ),
        ],
        differs: true,
    },
    Case {
        what: "a node's write remapped",
        before: &[],
        after: &[(
            "flows/triage.yml",
            "        decision: human_decision",
            "        decision: summary",
        )],
        differs: true,
    },
    Case {
        what: "a store-op node rekeyed",
        before: &[],
        after: &[(
            "flows/triage.yml",
            "      key: \"execution.session_key\"",
            "      key: \"input.report\"",
        )],
        differs: true,
    },
    Case {
        what: "a node's description reworded",
        before: &[],
        after: &[(
            "flows/triage.yml",
            "    summarize:\n      agent: agent.summarizer",
            "    summarize:\n      description: Say what the run did.\n      agent: agent.summarizer",
        )],
        differs: true,
    },
    // --- The rest of the surfaces, one edit each. -----------------------------
    Case {
        what: "a policy default retimed",
        before: &[],
        after: &[("main.yml", "  timeout: 90s", "  timeout: 45s")],
        differs: true,
    },
    Case {
        what: "a fan-out bound lowered",
        before: &[],
        after: &[(
            "flows/triage.yml",
            "max_concurrency: 8",
            "max_concurrency: 6",
        )],
        differs: true,
    },
    Case {
        what: "an argv entry added",
        before: &[],
        after: &[(
            "flows/triage.yml",
            "args: [\"--fast\"]",
            "args: [\"--fast\", \"--quiet\"]",
        )],
        differs: true,
    },
    Case {
        what: "a trigger route moved",
        before: &[],
        after: &[("triggers.yml", "path: /reports", "path: /v1/reports")],
        differs: true,
    },
    Case {
        what: "a placement given a second member",
        before: &[("deploy/local.yml", HUB, PLACEMENT)],
        after: &[("deploy/local.yml", HUB, PLACEMENT_WIDENED)],
        differs: true,
    },
    Case {
        what: "a placement's description reworded",
        before: &[("deploy/local.yml", HUB, PLACEMENT)],
        after: &[("deploy/local.yml", HUB, PLACEMENT_REWORDED)],
        differs: true,
    },
    // The `hub:` block is the deploy layer's one singleton, so both of its keys
    // are swept here rather than by a section walk: `join_token:` is the
    // variable a worker's credential is read from, and `public_url:` is the base
    // every ingress URL derives from, so repointing either is a deployment
    // change a reviewer has to see (grammar 14.2).
    Case {
        what: "the hub's join token read from another variable",
        before: &[],
        after: &[(
            "deploy/local.yml",
            "  join_token: ${MESH_JOIN_TOKEN}",
            "  join_token: ${LAPTOP_JOIN_TOKEN}",
        )],
        differs: true,
    },
    Case {
        what: "the hub given a public base",
        before: &[],
        after: &[(
            "deploy/local.yml",
            "  join_token: ${MESH_JOIN_TOKEN}",
            "  join_token: ${MESH_JOIN_TOKEN}\n  public_url: \"http://localhost:8080\"",
        )],
        differs: true,
    },
    Case {
        what: "a channel bound narrowed",
        before: &[],
        after: &[("main.yml", "    max_items: 200", "    max_items: 100")],
        differs: true,
    },
    Case {
        what: "a channel's reduce policy changed",
        before: &[],
        after: &[("main.yml", "    reduce: append", "    reduce: last_wins")],
        differs: true,
    },
    Case {
        what: "a flow's output surface documented",
        before: &[],
        after: &[(
            "flows/triage.yml",
            "    summary:\n      type: string",
            "    summary:\n      description: What the run concluded.\n      type: string",
        )],
        differs: true,
    },
    // --- Inside a block: the places below a subject's first key. --------------
    //
    // Every case above moves a key at the **top** of its subject, and until this
    // section existed that was the whole of what any pair forced: a plan's
    // records were read back only as far as their first step, so a rule that
    // swallowed a tool's `exec.command` while still comparing its `exec.args`
    // named the tool at `exec` all the same and the sets matched.
    //
    // [`named_subjects`] now reads a path as far as its object keys go, so each
    // place below one needs its own forcing pair for the same reason a key at
    // the top does. What is still one place — an array, and everything the walk
    // pairs up inside it — is the line that file's docs draw and this section
    // does not cross.
    Case {
        what: "an exec-bound tool's command respelled",
        before: &[],
        after: &[(
            "tools/repo_grep.yml",
            "    command: repo-grep",
            "    command: repo-search",
        )],
        differs: true,
    },
    Case {
        what: "an exec-bound tool's argv, directory and environment retuned",
        before: &[],
        after: &[
            (
                "tools/repo_grep.yml",
                "    args: [\"--format\", \"json\"]",
                "    args: [\"--format\", \"jsonl\"]",
            ),
            (
                "tools/repo_grep.yml",
                "    cwd: \"${REPO_ROOT}\"",
                "    cwd: \"${WORKSPACE_ROOT}\"",
            ),
            (
                "tools/repo_grep.yml",
                "      RIPGREP_CONFIG_PATH: \"${RG_CONFIG_PATH}\"",
                "      RIPGREP_CONFIG_PATH: \"${RG_CONFIG}\"",
            ),
        ],
        differs: true,
    },
    Case {
        what: "an http-bound tool's method changed",
        before: &[],
        after: &[(
            "tools/review_queue.yml",
            "    method: POST",
            "    method: PUT",
        )],
        differs: true,
    },
    Case {
        what: "an http-bound tool's endpoint, headers and body repointed",
        before: &[],
        after: &[
            (
                "tools/review_queue.yml",
                "    url: \"https://${QUEUE_HOST}/v1/tickets\"",
                "    url: \"https://${TICKET_HOST}/v1/tickets\"",
            ),
            (
                "tools/review_queue.yml",
                "      authorization: \"Bearer ${QUEUE_TOKEN}\"",
                "      authorization: \"Bearer ${TICKET_TOKEN}\"",
            ),
            (
                "tools/review_queue.yml",
                "      severity: \"input.severity\"",
                "      severity: \"input.summary\"",
            ),
        ],
        differs: true,
    },
    Case {
        what: "a model's token budget lowered",
        before: &[],
        after: &[("models.yml", "    max_tokens: 8000", "    max_tokens: 6000")],
        differs: true,
    },
    Case {
        what: "the policy defaults' retry rebuilt",
        before: &[],
        after: &[(
            "main.yml",
            "  retry: { max: 1, backoff: 2s }",
            "  retry: { max: 2, backoff: 5s }",
        )],
        differs: true,
    },
    // --- A schema's own insides, which are a subject's places like any other. -
    Case {
        what: "a channel's element type changed",
        before: &[],
        after: &[("main.yml", ELEMENT, ELEMENT_RETYPED)],
        differs: true,
    },
    Case {
        what: "a channel's element form changed",
        before: &[],
        after: &[("main.yml", ELEMENT, ELEMENT_REFORMED)],
        differs: true,
    },
    Case {
        what: "a channel re-formed",
        before: &[],
        after: &[("main.yml", ELEMENT, NO_ELEMENTS)],
        differs: true,
    },
    Case {
        what: "a channel's variants widened",
        before: &[],
        after: &[(
            "main.yml",
            "    enum: [approve, reject]",
            "    enum: [approve, reject, defer]",
        )],
        differs: true,
    },
    // --- The inline blocks a node declares. -----------------------------------
    Case {
        what: "an inline subprocess's command respelled",
        before: &[],
        after: &[(
            "flows/triage.yml",
            "        command: \"${OPS_BIN}/log-event\"",
            "        command: \"${OPS_HOME}/log-event\"",
        )],
        differs: true,
    },
    Case {
        what: "an inline subprocess's directory moved",
        before: &[],
        after: &[(
            "flows/triage.yml",
            "        cwd: \"${REPO_ROOT}\"",
            "        cwd: \"${CHECK_ROOT}\"",
        )],
        differs: true,
    },
    Case {
        what: "an inline subprocess's declared output bounded",
        before: &[],
        after: &[(
            "flows/triage.yml",
            "          stdout: { type: string }",
            "          stdout: { type: string, max_length: 8192 }",
        )],
        differs: true,
    },
    Case {
        what: "an inline request rewritten across the keys it declares",
        before: &[],
        after: &[
            (
                "flows/enrich.yml",
                "        method: POST",
                "        method: PUT",
            ),
            (
                "flows/enrich.yml",
                "        url: \"https://${TRIAGE_HOST}/v1/normalize\"",
                "        url: \"https://${NORMALIZE_HOST}/v1/normalize\"",
            ),
            (
                "flows/enrich.yml",
                "          authorization: \"Bearer ${TRIAGE_TOKEN}\"",
                "          authorization: \"Bearer ${NORMALIZE_TOKEN}\"",
            ),
            (
                "flows/enrich.yml",
                "        expect_status: [200]",
                "        expect_status: [200, 202]",
            ),
            (
                "flows/enrich.yml",
                "          report_normalized: { type: string }",
                "          report_normalized: { type: string, max_length: 4000 }",
            ),
        ],
        differs: true,
    },
    Case {
        what: "an inline request's body repointed",
        before: &[],
        after: &[("flows/triage.yml", ESCALATION_BODY, ESCALATION_BODY_MOVED)],
        differs: true,
    },
    Case {
        what: "an inline node's retry envelope retuned",
        before: &[],
        after: &[(
            "flows/enrich.yml",
            "      retry: { max: 2, backoff: 1s, multiplier: 3.0, max_backoff: 20s }",
            "      retry: { max: 2, backoff: 3s, multiplier: 2.5, max_backoff: 30s }",
        )],
        differs: true,
    },
    Case {
        what: "a human pause's two surfaces and its expiry route rewritten",
        before: &[],
        after: &[
            (
                "flows/triage.yml",
                "          summary: { type: string }",
                "          summary: { type: string, max_length: 500 }",
            ),
            (
                "flows/triage.yml",
                "          note: { type: string }",
                "          note: { type: string, max_length: 200 }",
            ),
            (
                "flows/triage.yml",
                "        on_timeout: escalate",
                "        on_timeout: announce_failed",
            ),
        ],
        differs: true,
    },
    Case {
        what: "a node's bindings given as one unnamed value",
        before: &[],
        after: &[("flows/triage.yml", STDIN_BINDINGS, STDIN_VALUE)],
        differs: true,
    },
    Case {
        what: "a store-op node's value repointed",
        before: &[],
        after: &[(
            "flows/triage.yml",
            "        last_report: \"input.report\"",
            "        last_report: \"input.pattern\"",
        )],
        differs: true,
    },
    Case {
        what: "a store-op node's value given as one expression",
        before: &[],
        after: &[("flows/triage.yml", STORE_FIELDS, STORE_EXPRESSION)],
        differs: true,
    },
    // --- The `map:` block, whose insides are a construct of their own. --------
    Case {
        what: "a map route's own bound lowered",
        before: &[],
        after: &[(
            "flows/triage.yml",
            "            max_concurrency: 4",
            "            max_concurrency: 2",
        )],
        differs: true,
    },
    Case {
        what: "a map's source array and discriminator repointed",
        before: &[],
        after: &[
            (
                "flows/triage.yml",
                "        over: classify.output.findings",
                "        over: state.patches",
            ),
            (
                "flows/triage.yml",
                "        route_by: kind",
                "        route_by: tag",
            ),
        ],
        differs: true,
    },
    Case {
        what: "a map's item binding renamed",
        before: &[],
        after: &[(
            "flows/triage.yml",
            "        as: finding",
            "        as: item",
        )],
        differs: true,
    },
    Case {
        what: "a map's per-item retry retuned",
        before: &[],
        after: &[(
            "flows/triage.yml",
            "          retry: { max: 2, backoff: 2s }",
            "          retry: { max: 3, backoff: 4s }",
        )],
        differs: true,
    },
    Case {
        what: "a map's per-item error strategy replaced",
        before: &[],
        after: &[(
            "flows/triage.yml",
            "        on_item_error:\n          retry: { max: 2, backoff: 2s }",
            "        on_item_error: skip",
        )],
        differs: true,
    },
    Case {
        what: "a map's catch-all rerouted, detached, and rebound",
        before: &[],
        after: &[
            (
                "flows/triage.yml",
                "          node: tool.dead_letter",
                "          node: tool.review_queue",
            ),
            (
                "flows/triage.yml",
                "          detach: false",
                "          detach: true",
            ),
            (
                "flows/triage.yml",
                "            kind: \"finding.kind\"",
                "            kind: \"finding.summary\"",
            ),
        ],
        differs: true,
    },
    Case {
        what: "a map's catch-all given one unnamed value",
        before: &[],
        after: &[("flows/triage.yml", CATCH_ALL_BINDINGS, CATCH_ALL_VALUE)],
        differs: true,
    },
    Case {
        what: "a routed map made a homogeneous one",
        before: &[],
        after: &[
            (
                "flows/triage.yml",
                "        route_by: kind",
                "        node: agent.fixer",
            ),
            ("flows/triage.yml", ROUTED, HOMOGENEOUS),
            ("flows/triage.yml", CATCH_ALL, ""),
        ],
        differs: true,
    },
    // --- The three places a node says what to do when it fails. ---------------
    Case {
        what: "a node's own failure mode changed",
        before: &[],
        after: &[("flows/triage.yml", VERIFY_ON_ERROR, VERIFY_ON_ERROR_FAILS)],
        differs: true,
    },
    Case {
        what: "a node's fallback retargeted",
        before: &[],
        after: &[(
            "flows/triage.yml",
            "      on_error: { fallback: announce_failed }",
            "      on_error: { fallback: verify }",
        )],
        differs: true,
    },
    Case {
        what: "a subgraph instantiation's policy given another failure mode",
        before: &[],
        after: &[(
            "flows/triage.yml",
            "        on_error: fail",
            "        on_error: skip",
        )],
        differs: true,
    },
];

/// Copy the base composition into `into`, then apply the edits.
fn planted(into: &Path, edits: &[Edit]) {
    planted_from(BASE, into, edits);
}

/// …and the same over any composition of the tree, for the one property whose
/// construct [`BASE`] does not hold (see
/// [`a_coder_node_arriving_says_which_harness_it_binds`]).
fn planted_from(base: &str, into: &Path, edits: &[Edit]) {
    copy(&repository().join(base), into);
    for (file, from, to) in edits {
        let path = into.join(file);
        let text = fs::read_to_string(&path).expect("a file of the base composition is readable");
        assert_eq!(
            text.matches(from).count(),
            1,
            "{file}: the text to edit appears exactly once:\n{from}"
        );
        fs::write(&path, text.replace(from, to)).expect("can write the edited file");
    }
}

/// Copy a directory tree.
fn copy(from: &Path, to: &Path) {
    fs::create_dir_all(to).expect("can create a directory");
    for entry in fs::read_dir(from).expect("the source directory is readable") {
        let entry = entry.expect("a directory entry");
        let target = to.join(entry.file_name());
        if entry.file_type().expect("a file type").is_dir() {
            copy(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), &target).expect("can copy a file");
        }
    }
}

/// The two artifacts one case describes, planted and resolved.
///
/// `under` names the caller, because the tests of this file run in one process
/// and [`scratch`] is keyed by that process: two of them planting the same case
/// into one directory would race.
fn sides(under: &str, at: usize, case: &Case) -> (Ir, Ir) {
    let before = scratch(&format!("{under}-{at}-before"));
    let after = scratch(&format!("{under}-{at}-after"));
    planted(&before, case.before);
    planted(&after, case.after);
    let old = artifact(&before.join("main.yml"))
        .unwrap_or_else(|| panic!("{}: the before side resolves", case.what));
    let new = artifact(&after.join("main.yml"))
        .unwrap_or_else(|| panic!("{}: the after side resolves", case.what));
    let _ = fs::remove_dir_all(&before);
    let _ = fs::remove_dir_all(&after);
    (old, new)
}

/// A built-in reaching an agent by its **shorthand** is named in the plan (PRD
/// resolved q54, `docs/plan.md` §3, §4).
///
/// The three cases above move a built-in's bounds, which is enough for the
/// completeness walk and not enough for the property q54 asks for: what a
/// built-in grants is the widest capability this grammar hands out, so an author
/// reading a plan has to *see* one arrive. Asserted on the records' addresses
/// rather than on the rendered text, because the addresses are the machine
/// surface `docs/plan.md` fixes and the rendering follows them. The configured
/// spelling is the test below.
#[test]
fn a_builtin_arriving_at_an_agent_is_named_in_the_plan() {
    let case = Case {
        what: "an agent given both built-ins",
        before: &[],
        after: &[(
            "agents/fixer.yml",
            "    - tool.checkout",
            "    - tool.checkout\n    - builtin.bash",
        )],
        differs: true,
    };
    let (old, new) = sides("builtin-arrival", 0, &case);
    let plan = plan(
        Composition {
            entrypoint: "before/main.yml",
            ir: &old,
            resolution: &[],
        },
        Composition {
            entrypoint: "after/main.yml",
            ir: &new,
            resolution: &[],
        },
    );
    let agent = plan
        .components
        .iter()
        .find(|change| change.address == "agent.fixer")
        .expect("the agent that gained a built-in is a component change");
    let fields: Vec<&str> = agent
        .fields
        .iter()
        .map(|field| field.path.as_str())
        .collect();
    assert!(
        fields.contains(&"builtins"),
        "a shorthand built-in reached an agent and the plan named no `builtins` field: \
         {fields:?}"
    );
    let said = agent
        .fields
        .iter()
        .find(|field| field.path == "builtins")
        .map(|field| format!("{:?}", field.after))
        .unwrap_or_default();
    assert!(
        said.contains("builtin.bash"),
        "the plan does not say which built-in arrived, which is the whole of what it grants: \
         {said}"
    );
}

/// …and the **configured** spelling, which is a definition arriving rather than
/// a list growing (PRD resolved q54, `docs/plan.md` §3).
///
/// The shorthand says what it grants in the agent's own `builtins` field. A
/// `tool.*` carrying a `builtin:` binding says it nowhere unless the arrival
/// carries it: an added definition reports no fields, so `+ tool.sandbox` would
/// read exactly like `+ tool.repo_grep` — and the configured form is now the
/// only way to grant a *bounded* shell, so it is the spelling a plan reader most
/// needs told. One field, `builtin`, holding the name.
#[test]
fn a_configured_builtin_arriving_says_what_it_binds() {
    let case = Case {
        what: "a tool binding a shell, added and attached",
        before: &[],
        after: &[
            (
                "tools/checkout.yml",
                "tool.checkout:",
                "tool.sandbox:\n  builtin: bash\n  workspace: \"${REPO_ROOT}\"\n  timeout: 90s\ntool.checkout:",
            ),
            (
                "agents/fixer.yml",
                "    - tool.checkout",
                "    - tool.checkout\n    - tool.sandbox",
            ),
        ],
        differs: true,
    };
    let (old, new) = sides("builtin-definition", 0, &case);
    let arrived = plan(
        Composition {
            entrypoint: "before/main.yml",
            ir: &old,
            resolution: &[],
        },
        Composition {
            entrypoint: "after/main.yml",
            ir: &new,
            resolution: &[],
        },
    );
    let tool = arrived
        .components
        .iter()
        .find(|change| change.address == "tool.sandbox")
        .expect("the tool that arrived is a component change");
    let said: Vec<(&str, String)> = tool
        .fields
        .iter()
        .map(|field| (field.path.as_str(), format!("{:?}", field.after)))
        .collect();
    assert!(
        said.iter()
            .any(|(path, after)| *path == "builtin" && after.contains("bash")),
        "a `tool.*` binding a shell arrived and the plan does not say it binds one, so it reads \
         like any other tool arriving: {said:?}"
    );
    // …and the same tool leaving says the same thing, on the other side: the two
    // artifacts compared the other way round.
    let left = plan(
        Composition {
            entrypoint: "before/main.yml",
            ir: &new,
            resolution: &[],
        },
        Composition {
            entrypoint: "after/main.yml",
            ir: &old,
            resolution: &[],
        },
    );
    let tool = left
        .components
        .iter()
        .find(|change| change.address == "tool.sandbox")
        .expect("the tool that left is a component change");
    let said: Vec<(&str, String)> = tool
        .fields
        .iter()
        .map(|field| (field.path.as_str(), format!("{:?}", field.before)))
        .collect();
    assert!(
        said.iter()
            .any(|(path, before)| *path == "builtin" && before.contains("bash")),
        "a capability leaving is as much of a plan as one arriving: {said:?}"
    );
}

/// A **coder node** arriving says which harness it binds, and so does one
/// leaving (`docs/plan.md` §3, PRD resolved q57 ruling c).
///
/// [`a_configured_builtin_arriving_says_what_it_binds`] one construct along,
/// and for a sharper version of its reason. A `builtin:` binding hands the
/// model a program inside this runtime's tool surface; a `coder:` node hands a
/// harness the program *and the loop*, inside the harness's own tool surface,
/// where none of §5.5's bounds apply. That is the widest capability this
/// grammar grants, so `+ flow.patch.implement` alone — which is what a plan
/// prints for a `function:` node arriving — is not a plan for it.
///
/// The edit is a **rename**, which is one node leaving and another arriving in
/// a single comparison, so both directions are read off one plan. The base is
/// `examples/patch-pipeline` rather than [`BASE`]: the corpus above is the
/// widest composition in the tree and it has no coder node in it, which is the
/// same reason the built-in pair plants its own.
#[test]
fn a_coder_node_arriving_says_which_harness_it_binds() {
    const RENAMED: &[Edit] = &[
        (
            "flows/patch.yml",
            "    implement:\n      coder:",
            "    build:\n      coder:",
        ),
        (
            "flows/patch.yml",
            "{ from: start, to: implement }",
            "{ from: start, to: build }",
        ),
        (
            "flows/patch.yml",
            "{ from: implement, to: review }",
            "{ from: build, to: review }",
        ),
        ("flows/patch.yml", "      to: implement", "      to: build"),
    ];
    let before = scratch("coder-harness-before");
    let after = scratch("coder-harness-after");
    planted_from("examples/patch-pipeline", &before, RENAMED);
    planted_from("examples/patch-pipeline", &after, &[]);
    let old = artifact(&before.join("main.yml")).expect("the renamed side resolves");
    let new = artifact(&after.join("main.yml")).expect("the example resolves");
    let _ = fs::remove_dir_all(&before);
    let _ = fs::remove_dir_all(&after);

    let plan = plan(
        Composition {
            entrypoint: "before/main.yml",
            ir: &old,
            resolution: &[],
        },
        Composition {
            entrypoint: "after/main.yml",
            ir: &new,
            resolution: &[],
        },
    );
    let said = |address: &str, take: fn(&FieldChange) -> &Option<Value>| {
        let change = plan
            .topology
            .iter()
            .find(|change| change.address == address)
            .unwrap_or_else(|| panic!("`{address}` is a node change of this plan"));
        change
            .fields
            .iter()
            .map(|field| (field.path.clone(), format!("{:?}", take(field))))
            .collect::<Vec<_>>()
    };

    let arrived = said("flow.patch.implement", |field| &field.after);
    assert!(
        arrived
            .iter()
            .any(|(path, after)| path == "coder.harness" && after.contains("cc")),
        "a coder node arrived and the plan does not say which harness it binds, so it reads \
         like any other node arriving: {arrived:?}"
    );
    // …and the same node leaving, on the other side of the same plan: a
    // capability leaving is as much of a plan as one arriving.
    let left = said("flow.patch.build", |field| &field.before);
    assert!(
        left.iter()
            .any(|(path, before)| path == "coder.harness" && before.contains("cc")),
        "a coder node left and the plan does not say which harness it bound: {left:?}"
    );
}

#[test]
fn one_composition_edited_one_construct_at_a_time_reports_every_edit_and_no_other() {
    for (at, case) in CASES.iter().enumerate() {
        let (old, new) = sides("reports", at, case);

        let reported = holds(case.what, &old, &new);
        assert_eq!(
            reported > 0,
            case.differs,
            "{}: {reported} structural record(s), and the case says the two {}",
            case.what,
            if case.differs { "differ" } else { "agree" }
        );
        holds(&format!("{} (reversed)", case.what), &new, &old);
    }
}

// ---------------------------------------------------------------------------
// …and the corpora are the coverage, so what they reach is asserted too.
// ---------------------------------------------------------------------------

/// The places of the artifact no pair of specs can move, so none is asked to.
///
/// Both entries are tags the compiler writes from **where** a construct sits
/// rather than from anything an author typed, at a place two comparable specs
/// reach the same way:
///
/// * `namespace` is the tag a definition's body is written under, and it follows
///   from the address (grammar 2.2). Two definitions are compared only where
///   they sit at the same address (`docs/plan.md` §4), so it is equal on both
///   sides of every comparison there is;
/// * `surface` is which of grammar 3.5's and 3.6's opposite rules apply inside a
///   field map (`crate::ir::schema::FieldMap`), and it follows from the key the
///   map is written under — an `input:` is an input surface and an `output:` a
///   result surface, wherever either is declared. So it is spelled out per
///   place: every one of them is a field map the grammar fixes the surface of,
///   and a **new** place holding a `surface` has to be added here deliberately
///   rather than excused by the key's name.
const CONSTRUCTED: &[&str] = &[
    "exec.output.surface",
    "http.output.surface",
    "human.input.surface",
    "human.output.surface",
    "input.surface",
    "inputs.surface",
    "metadata_schema.surface",
    "namespace",
    "output.surface",
    "outputs.surface",
    "value_schema.surface",
];

/// Which kind of subject an address names, which is what coverage is counted
/// over: every node of every flow is one bucket, and so is every agent.
fn kind_of(subject: &Subject) -> String {
    let (section, address) = subject;
    format!(
        "{section}/{}",
        address.split('.').next().unwrap_or_default()
    )
}

/// Every place the two artifacts differ at, into `found`, bucketed by
/// [`kind_of`].
fn moved(before: &Ir, after: &Ir, found: &mut BTreeMap<String, BTreeSet<String>>) {
    let old = subjects(before);
    let new = subjects(after);
    for key in old.keys().chain(new.keys()) {
        let (Some(one), Some(two)) = (old.get(key), new.get(key)) else {
            continue;
        };
        let (Value::Object(one), Value::Object(two)) = (&one.value, &two.value) else {
            continue;
        };
        found
            .entry(kind_of(key))
            .or_default()
            .extend(differing_keys(one, two));
    }
}

/// Every place of the base composition's artifact is moved by some pair, so the
/// property above is **exercised** at each of them rather than merely stated
/// over them.
///
/// [`holds`] compares the places a plan's records name against the places the
/// two artifacts differ at, place for place — so it catches a field that stopped
/// being compared exactly on the pairs whose two sides differ in *that field*.
/// Which makes the two corpora the coverage, and this is the assertion that says
/// so: a place of the artifact no pair moves is a place the property is silent
/// about, and a refactor could stop comparing it under a green suite. That is
/// the hole this whole file exists to close, one level up.
///
/// So a key added to the IR needs a case that moves it, the way a check added to
/// the compiler needs a fixture (`static_check_inventory.rs`). The failure names
/// the place and the kind of subject it sits on.
///
/// # What it reaches
///
/// The base composition's own places, and only those. Nearly every field of the
/// IR is written `skip_serializing_if`, so a key [`BASE`] declares nowhere is no
/// key of its artifact and nothing here asks for it. The module docs' final
/// section states that limitation, counts it, and says what closes it.
#[test]
fn every_key_of_the_base_composition_is_moved_by_some_pair() {
    let mut found: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for name in corpus_pairs() {
        let before = artifact(&corpus().join(&name).join("before/main.yml"));
        let after = artifact(&corpus().join(&name).join("after/main.yml"));
        if let (Some(before), Some(after)) = (before, after) {
            moved(&before, &after, &mut found);
        }
    }
    for (at, case) in CASES.iter().enumerate() {
        let (old, new) = sides("moves", at, case);
        moved(&old, &new, &mut found);
    }

    let base = scratch("base");
    planted(&base, &[]);
    let ir = artifact(&base.join("main.yml")).expect("the base composition resolves");
    let _ = fs::remove_dir_all(&base);

    let mut missed: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (subject, slice) in subjects(&ir) {
        let Value::Object(map) = &slice.value else {
            continue;
        };
        let kind = kind_of(&subject);
        let reached = found.get(&kind).cloned().unwrap_or_default();
        for key in places(map) {
            if !reached.contains(&key) && !CONSTRUCTED.contains(&key.as_str()) {
                missed.entry(kind.clone()).or_default().push(key);
            }
        }
    }
    for keys in missed.values_mut() {
        keys.sort();
        keys.dedup();
    }
    assert!(
        missed.is_empty(),
        "no pair of either corpus moves {missed:?}, so nothing here would notice if a plan \
         stopped comparing one of them. Add a case to `CASES` that edits it — beside the one \
         for the construct it sits in — or, if it cannot differ between two comparable specs, \
         name it in `CONSTRUCTED` and say why."
    );
}
