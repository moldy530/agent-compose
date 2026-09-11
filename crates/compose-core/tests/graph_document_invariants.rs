//! The structural promises `docs/graph.md` §9.1 makes, over a corpus.
//!
//! §9.1 is the list of things a reader of a graph document MAY rely on, and
//! three of them are about the graph being a graph rather than about any one
//! field: `id` is unique within a flow, every `from` and `to` names a node that
//! flow's `nodes` array holds, and a `map_route` edge's `to` is the `node` of
//! one of that map's `routes`.
//!
//! They are checked here rather than left to the goldens because a golden pins
//! what one composition renders to, and these are promises about **every**
//! composition — including the ones whose shape nobody drew the format for. The
//! corpus is therefore the two worked examples plus the two fixtures written
//! for shapes an example does not reach:
//!
//! * `omitted-graph-keys`, whose map is homogeneous — one satellite, no variant;
//! * `default-tagged-variant`, whose union declares a variant tagged `default`
//!   beside a catch-all `default:` route. Both dispatch somewhere, and a
//!   satellite id derived from the variant tag alone would spell the two the
//!   same way: one node, two routes pointing at it, the other target drawn
//!   nowhere on a canvas whose whole promise is that what you see is every path
//!   the validator proved (PRD resolved q56).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use compose_core::graph::{EdgeClass, GraphDocument};
use compose_core::resolve;

/// Every project a document is built from here.
const CORPUS: &[&str] = &[
    "examples/triage-fanout",
    "examples/review-loop",
    "crates/compose-core/tests/projects/omitted-graph-keys",
    "crates/compose-core/tests/projects/default-tagged-variant",
];

fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("the manifest directory has a grandparent")
        .to_path_buf()
}

/// One project's graph document, insisting the composition is valid: `visualize`
/// emits only on a clean report.
fn document(project: &str) -> GraphDocument {
    let resolution = resolve(repository().join(project).join("main.yml"));
    let ir = resolution.ir.unwrap_or_else(|| {
        panic!(
            "`{project}` does not resolve: {:#?}",
            resolution.diagnostics
        )
    });
    let diagnostics = compose_core::check(&ir);
    assert!(
        diagnostics.is_empty(),
        "`{project}` does not validate: {diagnostics:#?}"
    );
    compose_core::graph(&ir)
}

/// No two nodes of one flow share an `id`.
///
/// The invariant everything downstream is built on: the template indexes its
/// nodes by id (`byId[n.id] = n`), so a duplicate is last-wins — one node drawn
/// at another's place, one edge arriving at the wrong end, and one dispatch
/// target on no canvas at all.
#[test]
fn every_node_id_is_unique_within_its_flow() {
    for project in CORPUS {
        for flow in &document(project).flows {
            let mut seen = BTreeSet::new();
            for node in &flow.nodes {
                assert!(
                    seen.insert(node.id.as_str()),
                    "`{project}`'s `{}` draws two nodes called `{}`",
                    flow.address,
                    node.id
                );
            }
        }
    }
}

/// Every edge arrives somewhere, and every edge leaves somewhere.
#[test]
fn every_edge_names_nodes_its_own_flow_holds() {
    for project in CORPUS {
        for flow in &document(project).flows {
            let known: BTreeSet<&str> = flow.nodes.iter().map(|node| node.id.as_str()).collect();
            for edge in &flow.edges {
                for (end, id) in [("from", &edge.from), ("to", &edge.to)] {
                    assert!(
                        known.contains(id.as_str()),
                        "`{project}`'s `{}` has an edge whose `{end}` is `{id}`, which is not one \
                         of its nodes",
                        flow.address
                    );
                }
            }
        }
    }
}

/// A `map_route` edge arrives at the satellite its route names, and each route
/// has one of its own.
///
/// `default-tagged-variant` is the composition that makes this more than a
/// restatement: its `routes:` names a variant tagged `default` and its
/// `default:` catches what is left, so the two satellites are the two ids a
/// derivation has to keep apart.
#[test]
fn every_map_route_edge_arrives_at_that_routes_own_satellite() {
    let mut routed = 0;
    for project in CORPUS {
        for flow in &document(project).flows {
            let mut arrivals: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
            for edge in &flow.edges {
                if edge.class == EdgeClass::MapRoute {
                    arrivals.entry(&edge.from).or_default().push(&edge.to);
                }
            }
            for node in &flow.nodes {
                let Some(map) = node.map.as_ref() else {
                    continue;
                };
                routed += 1;
                let targets: Vec<&str> =
                    map.routes.iter().map(|route| route.node.as_str()).collect();
                let mut distinct: BTreeSet<&str> = BTreeSet::new();
                for target in &targets {
                    assert!(
                        distinct.insert(target),
                        "`{project}`'s `{}`.`{}` dispatches two routes to one satellite, `{target}`",
                        flow.address,
                        node.id
                    );
                }
                assert_eq!(
                    arrivals.get(node.id.as_str()).cloned().unwrap_or_default(),
                    targets,
                    "`{project}`'s `{}`.`{}` draws map_route edges the routes do not name",
                    flow.address,
                    node.id
                );
                for target in targets {
                    let satellite = flow
                        .nodes
                        .iter()
                        .find(|held| held.id == target)
                        .unwrap_or_else(|| panic!("`{target}` is drawn"));
                    assert_eq!(
                        satellite.dispatch.as_ref().map(|held| held.map.as_str()),
                        Some(node.id.as_str()),
                        "`{target}` is not marked as dispatched by `{}`",
                        node.id
                    );
                }
            }
        }
    }
    assert!(
        routed >= 3,
        "the corpus still carries fan-outs, found {routed}"
    );
}

/// The satellites of a `default`-tagged variant and of the catch-all beside it
/// are two nodes, and each is the one its route dispatches to.
///
/// The regression this whole fixture exists for, asserted on the ids rather
/// than only on their uniqueness: a reader opening the page sees `agent.worker`
/// under the variant's route and `agent.third` under the catch-all, and a
/// derivation that spelled both `work/default` would draw one of them at the
/// far-left of the canvas with no edge reaching it.
#[test]
fn a_variant_tagged_default_and_the_catch_all_get_separate_satellites() {
    let document = document("crates/compose-core/tests/projects/default-tagged-variant");
    let flow = document.flow("flow.bare").expect("the fixture's one flow");
    let map = flow
        .nodes
        .iter()
        .find(|node| node.id == "work")
        .and_then(|node| node.map.as_ref())
        .expect("the fan-out");

    let drawn: Vec<(&str, Option<&str>, &str)> = map
        .routes
        .iter()
        .map(|route| {
            (
                route.node.as_str(),
                route.variant.as_deref(),
                route.target.as_str(),
            )
        })
        .collect();
    assert_eq!(
        drawn,
        vec![
            ("work/default", Some("default"), "agent.worker"),
            ("work/other", Some("other"), "agent.second"),
            ("work/(default)", None, "agent.third"),
        ],
        "the variant tagged `default` and the `default:` route are two satellites"
    );

    let catch_all = flow
        .nodes
        .iter()
        .find(|node| node.id == "work/(default)")
        .expect("the catch-all's satellite is drawn");
    assert_eq!(catch_all.binding.as_deref(), Some("agent.third"));
    assert_eq!(
        catch_all
            .dispatch
            .as_ref()
            .expect("it is a satellite")
            .covers,
        vec!["third".to_string()],
        "the catch-all is narrowed to the variant no named route claims"
    );
}
