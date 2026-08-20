//! The four sections of a plan, and the one rule that keeps them apart.
//!
//! # Every change is reported once
//!
//! The sections are four lenses on one pair of artifacts, and three of them can
//! see the same field: a flow's `inputs:` is part of the flow definition
//! (components), part of what a caller passes (interfaces), and nothing to do
//! with its graph (topology). Reporting a field wherever it is visible would
//! print one edit three times, and a reader of a plan is reading it to find out
//! what changed — not to find out how many sections mention it.
//!
//! So the sections **partition** the artifact, and the partition is stated once,
//! here:
//!
//! | Section | What it owns |
//! |---|---|
//! | components | every definition and trigger arriving or leaving, and every field of one that neither section below owns |
//! | topology | a flow's `nodes:` and `edges:`, the `state:` channels, and the composition's `defaults:` |
//! | interfaces | a flow's `inputs:`/`outputs:`, and everything about a trigger except which flow it names |
//!
//! `docs/plan.md` is the normative statement of it, and
//! [`delegated`]/[`TRIGGER_IDENTITY`] are where the code says the same thing.
//!
//! # A component that arrived brings nothing with it
//!
//! Topology and interfaces report on subjects present in **both** specs. A flow
//! added to the composition is one line in `components`, not one line per node
//! it declares: its whole graph is new, the reader already knows that from the
//! line that says the flow is new, and a plan that expanded it would bury the
//! two edges that moved in the flow next to it under forty that did not move at
//! all. The same holds in reverse for a flow that was deleted.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use serde_json::Value;

use crate::diag::{Diagnostic, Diagnostics};
use crate::ir::definition::DefinitionBody;
use crate::ir::flow::Flow;
use crate::ir::{Definition, Ir};

use super::Composition;
use super::diff::{changes, only, semantic, without};
use super::document::{
    ChangeKind, ComponentChange, ComponentKind, Finding, InterfaceChange, InterfaceKind,
    TopologyChange, TopologyKind, Validation,
};

/// Keys of a definition that another section owns, plus the one key that can
/// never differ.
///
/// `address` is the definition's own address repeated beside the key it sits
/// under, so that a definition pulled out of the artifact still names itself
/// (`crate::ir::definition`). Two definitions are compared here only when they
/// sit at the same address, so it is the one field that is equal by
/// construction — and reporting it would be noise on every change.
fn delegated(component: ComponentKind) -> &'static [&'static str] {
    match component {
        ComponentKind::Flow => &["address", "inputs", "outputs", "nodes", "edges"],
        _ => &["address"],
    }
}

/// What a trigger is to `components`: which flow it runs, and its
/// documentation. Its whole delivery surface — the route, the response mode,
/// the session key, the input bindings — is what a caller meets, and belongs to
/// `interfaces`.
const TRIGGER_IDENTITY: &[&str] = &["flow", "description"];

/// The `name` a trigger and a channel repeat from the key they sit under, which
/// is equal by construction wherever one is compared.
const REPEATED_NAME: &[&str] = &["name"];

/// Definitions, triggers, and the deploy layer's own entries.
pub(super) fn components(before: &Ir, after: &Ir) -> Vec<ComponentChange> {
    let mut found = Vec::new();

    let addresses: BTreeSet<&str> = before
        .definitions
        .keys()
        .chain(after.definitions.keys())
        .map(String::as_str)
        .collect();
    for address in addresses {
        match (before.definition(address), after.definition(address)) {
            (Some(old), Some(new)) => {
                let component = kind(&new.body);
                let keys = delegated(component);
                let fields = changes(&without(semantic(old), keys), &without(semantic(new), keys));
                if !fields.is_empty() {
                    found.push(ComponentChange {
                        change: ChangeKind::Changed,
                        component,
                        address: address.to_string(),
                        fields,
                        span: new.span.clone(),
                    });
                }
            }
            (None, Some(new)) => found.push(arrival(ChangeKind::Added, address, new)),
            (Some(old), None) => found.push(arrival(ChangeKind::Removed, address, old)),
            (None, None) => unreachable!("the address came from one of the two artifacts"),
        }
    }

    let triggers: BTreeSet<&str> = names(before.triggers.as_ref())
        .chain(names(after.triggers.as_ref()))
        .collect();
    for name in triggers {
        let old = before.triggers.as_ref().and_then(|held| held.get(name));
        let new = after.triggers.as_ref().and_then(|held| held.get(name));
        let address = format!("trigger.{name}");
        match (old, new) {
            (Some(old), Some(new)) => {
                let fields = changes(
                    &only(semantic(old), TRIGGER_IDENTITY),
                    &only(semantic(new), TRIGGER_IDENTITY),
                );
                if !fields.is_empty() {
                    found.push(ComponentChange {
                        change: ChangeKind::Changed,
                        component: ComponentKind::Trigger,
                        address,
                        fields,
                        span: new.span.clone(),
                    });
                }
            }
            (None, Some(new)) => found.push(ComponentChange {
                change: ChangeKind::Added,
                component: ComponentKind::Trigger,
                address,
                fields: Vec::new(),
                span: new.span.clone(),
            }),
            (Some(old), None) => found.push(ComponentChange {
                change: ChangeKind::Removed,
                component: ComponentKind::Trigger,
                address,
                fields: Vec::new(),
                span: old.span.clone(),
            }),
            (None, None) => unreachable!("the name came from one of the two artifacts"),
        }
    }

    let placements: BTreeSet<&str> = names(before.deploy.placements.as_ref())
        .chain(names(after.deploy.placements.as_ref()))
        .collect();
    for name in placements {
        let old = before
            .deploy
            .placements
            .as_ref()
            .and_then(|held| held.get(name));
        let new = after
            .deploy
            .placements
            .as_ref()
            .and_then(|held| held.get(name));
        entry(
            &mut found,
            ComponentKind::Placement,
            &format!("placement.{name}"),
            old.map(|held| (semantic(held), held.span.clone())),
            new.map(|held| (semantic(held), held.span.clone())),
            &["address"],
        );
    }

    let sources: BTreeSet<&str> = names(before.deploy.event_sources.as_ref())
        .chain(names(after.deploy.event_sources.as_ref()))
        .collect();
    for name in sources {
        let old = before
            .deploy
            .event_sources
            .as_ref()
            .and_then(|held| held.get(name));
        let new = after
            .deploy
            .event_sources
            .as_ref()
            .and_then(|held| held.get(name));
        entry(
            &mut found,
            ComponentKind::EventSource,
            &format!("event_source.{name}"),
            old.map(|held| (semantic(held), held.span.clone())),
            new.map(|held| (semantic(held), held.span.clone())),
            REPEATED_NAME,
        );
    }

    found.sort_by(|left, right| left.address.cmp(&right.address));
    found
}

/// One component whose two sides are already JSON, pushed if it changed at all.
fn entry(
    found: &mut Vec<ComponentChange>,
    component: ComponentKind,
    address: &str,
    old: Option<(Value, crate::diag::Span)>,
    new: Option<(Value, crate::diag::Span)>,
    keys: &[&str],
) {
    match (old, new) {
        (Some((old, _)), Some((new, span))) => {
            let fields = changes(&without(old, keys), &without(new, keys));
            if !fields.is_empty() {
                found.push(ComponentChange {
                    change: ChangeKind::Changed,
                    component,
                    address: address.to_string(),
                    fields,
                    span,
                });
            }
        }
        (None, Some((_, span))) => found.push(ComponentChange {
            change: ChangeKind::Added,
            component,
            address: address.to_string(),
            fields: Vec::new(),
            span,
        }),
        (Some((_, span)), None) => found.push(ComponentChange {
            change: ChangeKind::Removed,
            component,
            address: address.to_string(),
            fields: Vec::new(),
            span,
        }),
        (None, None) => {}
    }
}

/// A definition that arrived or left: the component itself is the change, so it
/// carries no fields.
fn arrival(change: ChangeKind, address: &str, definition: &Definition) -> ComponentChange {
    ComponentChange {
        change,
        component: kind(&definition.body),
        address: address.to_string(),
        fields: Vec::new(),
        span: definition.span.clone(),
    }
}

const fn kind(body: &DefinitionBody) -> ComponentKind {
    match body {
        DefinitionBody::Agent(_) => ComponentKind::Agent,
        DefinitionBody::Tool(_) => ComponentKind::Tool,
        DefinitionBody::Flow(_) => ComponentKind::Flow,
        DefinitionBody::Store(_) => ComponentKind::Store,
        DefinitionBody::Provider(_) => ComponentKind::Provider,
        DefinitionBody::Model(_) => ComponentKind::Model,
    }
}

/// The entry names of a section that may not have been declared at all.
fn names<T>(section: Option<&crate::ir::Section<T>>) -> impl Iterator<Item = &str> {
    section
        .into_iter()
        .flat_map(|held| held.entries.keys().map(String::as_str))
}

/// Graphs, channels, and the policy defaults.
pub(super) fn topology(before: &Ir, after: &Ir) -> Vec<TopologyChange> {
    let mut found = Vec::new();

    match (&before.defaults, &after.defaults) {
        (Some(old), Some(new)) => {
            let fields = changes(&semantic(old), &semantic(new));
            if !fields.is_empty() {
                found.push(TopologyChange {
                    change: ChangeKind::Changed,
                    site: TopologyKind::Defaults,
                    address: "defaults".to_string(),
                    flow: None,
                    fields,
                    span: new.span.clone(),
                });
            }
        }
        (None, Some(new)) => found.push(TopologyChange {
            change: ChangeKind::Added,
            site: TopologyKind::Defaults,
            address: "defaults".to_string(),
            flow: None,
            fields: Vec::new(),
            span: new.span.clone(),
        }),
        (Some(old), None) => found.push(TopologyChange {
            change: ChangeKind::Removed,
            site: TopologyKind::Defaults,
            address: "defaults".to_string(),
            flow: None,
            fields: Vec::new(),
            span: old.span.clone(),
        }),
        (None, None) => {}
    }

    let channels: BTreeSet<&str> = names(before.state.as_ref())
        .chain(names(after.state.as_ref()))
        .collect();
    for name in channels {
        let old = before.state.as_ref().and_then(|held| held.get(name));
        let new = after.state.as_ref().and_then(|held| held.get(name));
        let address = format!("state.{name}");
        match (old, new) {
            (Some(old), Some(new)) => {
                let fields = changes(
                    &without(semantic(old), REPEATED_NAME),
                    &without(semantic(new), REPEATED_NAME),
                );
                if !fields.is_empty() {
                    found.push(TopologyChange {
                        change: ChangeKind::Changed,
                        site: TopologyKind::Channel,
                        address,
                        flow: None,
                        fields,
                        span: new.span.clone(),
                    });
                }
            }
            (None, Some(new)) => found.push(TopologyChange {
                change: ChangeKind::Added,
                site: TopologyKind::Channel,
                address,
                flow: None,
                fields: Vec::new(),
                span: new.span.clone(),
            }),
            (Some(old), None) => found.push(TopologyChange {
                change: ChangeKind::Removed,
                site: TopologyKind::Channel,
                address,
                flow: None,
                fields: Vec::new(),
                span: old.span.clone(),
            }),
            (None, None) => {}
        }
    }

    for (address, definition) in &after.definitions {
        let DefinitionBody::Flow(new) = &definition.body else {
            continue;
        };
        let Some(DefinitionBody::Flow(old)) = before.definition(address).map(|held| &held.body)
        else {
            continue;
        };
        graph(&mut found, address, old, new);
    }

    // By the site's scope, then nodes before edges, then by address — so one
    // flow's changes read together, its nodes before the edges between them,
    // and `defaults` and the channels sort into the same one order every run
    // (`docs/plan.md` §Ordering).
    found.sort_by_key(|change| {
        (
            change
                .flow
                .clone()
                .unwrap_or_else(|| change.address.clone()),
            u8::from(change.site == TopologyKind::Edge),
            change.address.clone(),
        )
    });
    found
}

/// One flow's nodes and edges, into `found`.
fn graph(found: &mut Vec<TopologyChange>, address: &str, before: &Flow, after: &Flow) {
    let ids: BTreeSet<&str> = before
        .nodes
        .iter()
        .chain(&after.nodes)
        .map(|node| node.id.value.as_str())
        .collect();
    let old_nodes: BTreeMap<&str, _> = before
        .nodes
        .iter()
        .map(|node| (node.id.value.as_str(), node))
        .collect();
    let new_nodes: BTreeMap<&str, _> = after
        .nodes
        .iter()
        .map(|node| (node.id.value.as_str(), node))
        .collect();
    for id in ids {
        let site = TopologyKind::Node;
        let at = format!("{address}.{id}");
        match (old_nodes.get(id), new_nodes.get(id)) {
            (Some(old), Some(new)) => {
                let fields = changes(
                    &without(semantic(old), &["id"]),
                    &without(semantic(new), &["id"]),
                );
                if !fields.is_empty() {
                    found.push(TopologyChange {
                        change: ChangeKind::Changed,
                        site,
                        address: at,
                        flow: Some(address.to_string()),
                        fields,
                        span: new.span.clone(),
                    });
                }
            }
            (None, Some(new)) => found.push(TopologyChange {
                change: ChangeKind::Added,
                site,
                address: at,
                flow: Some(address.to_string()),
                fields: Vec::new(),
                span: new.span.clone(),
            }),
            (Some(old), None) => found.push(TopologyChange {
                change: ChangeKind::Removed,
                site,
                address: at,
                flow: Some(address.to_string()),
                fields: Vec::new(),
                span: old.span.clone(),
            }),
            (None, None) => {}
        }
    }

    edges(found, address, before, after);
}

/// One flow's edges, matched by identity before they are compared.
///
/// An edge has no name to be matched by: what identifies it is where it runs
/// from, where it runs to, and what guards it — and an edit changes exactly one
/// of those. So the match runs in three passes over what is still unpaired,
/// each pass a different answer to "is this the same edge":
///
/// 1. **identical** — every field agrees. Nothing to report, and matching these
///    first keeps a duplicated `from`/`to` pair from stealing its twin's
///    partner;
/// 2. **same `from` and `to`** — the guard, the `else:`, or the iteration
///    budget moved. That is a changed edge, not a removed one beside an added
///    one;
/// 3. **same `from` and guard** — the edge was **retargeted**, which is the
///    edit `docs/plan.md` names and the one a pair of add/remove lines hides.
///
/// Whatever is left is an edge that really arrived or really left. Each pass is
/// an index rather than a scan, so a flow with thousands of edges costs the
/// sort rather than the square (`tests/plan_scale.rs`).
fn edges(found: &mut Vec<TopologyChange>, flow: &str, before: &Flow, after: &Flow) {
    let old = ordered(before);
    let new = ordered(after);
    let paired = pair(&old, &new);

    let mut held: Vec<TopologyChange> = Vec::new();
    for (at, edge) in after.edges.iter().enumerate() {
        let address = edge_address(flow, &new[at]);
        match paired[at] {
            Some(from) => {
                let fields = changes(&old[from], &new[at]);
                if !fields.is_empty() {
                    held.push(TopologyChange {
                        change: ChangeKind::Changed,
                        site: TopologyKind::Edge,
                        address,
                        flow: Some(flow.to_string()),
                        fields,
                        span: edge.span.clone(),
                    });
                }
            }
            None => held.push(TopologyChange {
                change: ChangeKind::Added,
                site: TopologyKind::Edge,
                address,
                flow: Some(flow.to_string()),
                fields: Vec::new(),
                span: edge.span.clone(),
            }),
        }
    }
    let taken: BTreeSet<usize> = paired.iter().flatten().copied().collect();
    for (at, edge) in before.edges.iter().enumerate() {
        if taken.contains(&at) {
            continue;
        }
        held.push(TopologyChange {
            change: ChangeKind::Removed,
            site: TopologyKind::Edge,
            address: edge_address(flow, &old[at]),
            flow: Some(flow.to_string()),
            fields: Vec::new(),
            span: edge.span.clone(),
        });
    }

    disambiguate(&mut held);
    found.append(&mut held);
}

/// One flow's edges as JSON, each carrying the one thing the artifact records
/// as a position rather than as a key: `order`.
///
/// Grammar 7.3 evaluates **a node's outgoing edges in declaration order**, and
/// takes the first whose guard passes — so swapping two guarded edges from one
/// node changes which one fires, on a composition where every edge is otherwise
/// untouched. The IR carries that as the position of the edge in `edges:`, which
/// a diff matching edges by identity would never see.
///
/// `order` is the plan's own key for it, and it counts **per source node**
/// rather than over the whole list, because that is what the rule is stated
/// over: an edge inserted between two edges of a different node changes nobody's
/// precedence, and reporting it as though it had would make every insertion look
/// like a routing change.
fn ordered(flow: &Flow) -> Vec<Value> {
    let mut counted: BTreeMap<String, u64> = BTreeMap::new();
    flow.edges
        .iter()
        .map(|edge| {
            let mut value = semantic(edge);
            let from = text(&value, "from").to_string();
            let at = counted.entry(from).or_insert(0);
            if let Value::Object(map) = &mut value {
                map.insert("order".to_string(), Value::from(*at));
            }
            *at += 1;
            value
        })
        .collect()
}

/// `<flow>.<from>-><to>`, which is what a reader looks an edge up by.
fn edge_address(flow: &str, edge: &Value) -> String {
    format!("{flow}.{}->{}", text(edge, "from"), text(edge, "to"))
}

/// A string-valued key of a normalized edge, without its JSON quotes.
fn text<'a>(edge: &'a Value, key: &str) -> &'a str {
    edge.get(key).and_then(Value::as_str).unwrap_or_default()
}

/// Two edges of one flow can run between the same pair of nodes under different
/// guards, so the address is not unique on its own. The second and further
/// entries carrying one address take a `#2`, `#3` suffix, in the order the
/// entries were built — after edges in declaration order, then before edges in
/// theirs — which is fixed by the artifact rather than by the diff.
fn disambiguate(held: &mut [TopologyChange]) {
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    for change in held {
        let count = seen.entry(change.address.clone()).or_insert(0);
        *count += 1;
        if *count > 1 {
            change.address = format!("{}#{count}", change.address);
        }
    }
}

/// For each after edge, the before edge it is the same edge as. See [`edges`].
fn pair(before: &[Value], after: &[Value]) -> Vec<Option<usize>> {
    let old: Vec<[String; 3]> = before.iter().map(keys).collect();
    let new: Vec<[String; 3]> = after.iter().map(keys).collect();
    let mut taken = vec![false; before.len()];
    let mut paired: Vec<Option<usize>> = vec![None; after.len()];
    for pass in 0..3 {
        let mut index: BTreeMap<&str, VecDeque<usize>> = BTreeMap::new();
        for (at, keys) in old.iter().enumerate() {
            if !taken[at] {
                index.entry(keys[pass].as_str()).or_default().push_back(at);
            }
        }
        for (at, keys) in new.iter().enumerate() {
            if paired[at].is_some() {
                continue;
            }
            if let Some(queue) = index.get_mut(keys[pass].as_str())
                && let Some(found) = queue.pop_front()
            {
                paired[at] = Some(found);
                taken[found] = true;
            }
        }
    }
    paired
}

/// The three keys one edge is matched on, one per pass of [`pair`].
fn keys(edge: &Value) -> [String; 3] {
    let from = part(edge, "from");
    let to = part(edge, "to");
    let when = part(edge, "when");
    [
        edge.to_string(),
        format!("{from}\u{1f}{to}"),
        format!("{from}\u{1f}{when}"),
    ]
}

/// One key of an edge as JSON text, which is `null` when it declares none — a
/// spelling no identifier, guard or pseudo-node can collide with.
fn part(edge: &Value, key: &str) -> String {
    edge.get(key).unwrap_or(&Value::Null).to_string()
}

/// A flow's declared I/O, and a trigger's delivery surface.
pub(super) fn interfaces(before: &Ir, after: &Ir) -> Vec<InterfaceChange> {
    let mut found = Vec::new();

    for (address, definition) in &after.definitions {
        let DefinitionBody::Flow(_) = &definition.body else {
            continue;
        };
        let Some(old) = before.definition(address) else {
            continue;
        };
        let DefinitionBody::Flow(_) = &old.body else {
            continue;
        };
        let keys = &["inputs", "outputs"];
        let fields = changes(
            &only(semantic(old), keys),
            &only(semantic(definition), keys),
        );
        if !fields.is_empty() {
            found.push(InterfaceChange {
                change: ChangeKind::Changed,
                surface: InterfaceKind::Flow,
                address: address.clone(),
                fields,
                span: definition.span.clone(),
            });
        }
    }

    for name in names(after.triggers.as_ref()) {
        let Some(new) = after.triggers.as_ref().and_then(|held| held.get(name)) else {
            continue;
        };
        let Some(old) = before.triggers.as_ref().and_then(|held| held.get(name)) else {
            continue;
        };
        let mut keys: Vec<&str> = TRIGGER_IDENTITY.to_vec();
        keys.extend_from_slice(REPEATED_NAME);
        let fields = changes(
            &without(semantic(old), &keys),
            &without(semantic(new), &keys),
        );
        if !fields.is_empty() {
            found.push(InterfaceChange {
                change: ChangeKind::Changed,
                surface: InterfaceKind::Trigger,
                address: format!("trigger.{name}"),
                fields,
                span: new.span.clone(),
            });
        }
    }

    found.sort_by(|left, right| left.address.cmp(&right.address));
    found
}

/// What each side is told that the other is not.
///
/// The full report on each side — resolution's own diagnostics, and every static
/// check — then matched **by code and message** rather than by location. A
/// diagnostic's span moves when anything above it in the file does, and a plan
/// that keyed on one would report an inserted comment as a validation error
/// resolved and the same error introduced two lines down. What a diagnostic
/// says is what identifies it: the message names the construct it is about
/// (`node \`merge\` of \`flow.diamond\` …`), and the code names the failure
/// class.
///
/// Matched as a **multiset**, so two diagnostics that really do say the same
/// thing twice are two, and a spec that grew a third reports one introduced.
fn findings(before: &Composition<'_>, after: &Composition<'_>) -> Validation {
    let old = reported(before);
    let new = reported(after);

    let mut index: BTreeMap<(crate::diag::DiagnosticCode, &str), VecDeque<usize>> = BTreeMap::new();
    for (at, diagnostic) in old.iter().enumerate() {
        index
            .entry((diagnostic.code, diagnostic.message.as_str()))
            .or_default()
            .push_back(at);
    }
    let mut introduced = Vec::new();
    let mut matched = BTreeSet::new();
    for diagnostic in &new {
        match index
            .get_mut(&(diagnostic.code, diagnostic.message.as_str()))
            .and_then(VecDeque::pop_front)
        {
            Some(at) => {
                matched.insert(at);
            }
            None => introduced.push(finding(diagnostic)),
        }
    }
    let resolved = old
        .iter()
        .enumerate()
        .filter(|(at, _)| !matched.contains(at))
        .map(|(_, diagnostic)| finding(diagnostic))
        .collect();

    Validation {
        introduced,
        resolved,
    }
}

/// Everything one side is told: what resolving it reported — which is warnings,
/// since a composition with an error has no artifact to plan over — and the
/// whole check phase.
fn reported(composition: &Composition<'_>) -> Vec<Diagnostic> {
    let mut held = Diagnostics::new();
    held.extend(composition.resolution.iter().cloned());
    held.extend(crate::check::check(composition.ir));
    held.sort();
    held.into_vec()
}

fn finding(diagnostic: &Diagnostic) -> Finding {
    Finding {
        code: diagnostic.code,
        severity: diagnostic.severity,
        message: diagnostic.message.clone(),
        span: diagnostic.span.clone(),
    }
}

/// The validation section.
pub(super) fn validation(before: &Composition<'_>, after: &Composition<'_>) -> Validation {
    findings(before, after)
}
