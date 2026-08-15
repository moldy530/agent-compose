//! Legal shapes the example corpus does not happen to use.
//!
//! `check_examples.rs` proves the data checks accept the two worked projects;
//! this file proves they accept the rest of what the grammar admits.
//! Over-rejection is the failure mode a negative corpus cannot catch — a rule
//! written slightly too tight makes a legal spec unwritable, and nothing else
//! in the suite would notice. Every case here is one the grammar names, and
//! several are the *accepting* half of a rule the negative corpus pins the
//! rejecting half of.

use std::fs;
use std::path::{Path, PathBuf};

use compose_core::resolve;

/// The provider and model every case needs, and nothing more.
const BACKEND: &str = r#"
provider.p:
  kind: anthropic
  api_key: ${K}
model.m:
  provider: provider.p
  id: some-model
"#;

#[track_caller]
fn accepts(name: &str, body: &str) {
    let dir = scratch(name);
    let source = format!("version: \"0.1\"\n{BACKEND}{body}");
    fs::write(dir.join("main.yml"), &source).expect("can write the project");
    let resolution = resolve(dir.join("main.yml"));
    assert!(
        resolution.diagnostics.is_empty(),
        "{name} should resolve cleanly, got:\n{}",
        render(&resolution.diagnostics)
    );
    let ir = resolution
        .ir
        .expect("a clean resolution produces an artifact");
    let diagnostics = compose_core::check(&ir);
    assert!(
        diagnostics.is_empty(),
        "{name} should check cleanly, got:\n{}",
        render(&diagnostics)
    );
}

fn render(diagnostics: &[compose_core::Diagnostic]) -> String {
    diagnostics
        .iter()
        .map(|diagnostic| {
            format!(
                "  {} [{}] {}",
                diagnostic.span, diagnostic.code, diagnostic.message
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// A scratch directory of this test's own, cleaned out before use — named by
/// the process as well as by the case, for the reason `resolve_examples.rs`
/// gives.
fn scratch(name: &str) -> PathBuf {
    let dir =
        Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("check-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("can create a scratch directory");
    dir
}

/// Grammar 4.1's supported surface: the comprehension macros, `has()`, the
/// string predicates, indexing, and arithmetic, all over declared schemas.
#[test]
fn the_whole_supported_expression_surface() {
    accepts(
        "expression-surface",
        r#"
state:
  tasks:
    type: array
    items: { type: string }
  totals:
    type: object
    properties:
      fixed: { type: integer }
    reduce: merge
agent.a:
  model: model.m
  prompt: Do it.
  input:
    ok: { type: boolean }
    count: { type: integer }
    first: { type: string }
  output:
    verdict: { enum: [approve, revise] }
flow.f:
  inputs:
    goal: { type: string }
  outputs: {}
  nodes:
    n:
      agent: agent.a
      input:
        ok: "state.tasks.exists(t, t.startsWith('a')) && has(state.totals.fixed)"
        count: "size(state.tasks) + 1"
        first: "state.tasks.filter(t, t.contains('x'))[0]"
  edges:
    - { from: start, to: n }
    - { from: n, to: end, when: "n.output.verdict in ['approve', 'revise']" }
    - { from: n, to: end, else: true }
"#,
    );
}

/// An edge leaving `start` reads `input`/`state`/`execution` and no node
/// output, because `start` has none (grammar 4.1, 2.4).
#[test]
fn a_guard_on_an_edge_leaving_start() {
    accepts(
        "start-guard",
        r#"
agent.a:
  model: model.m
  prompt: Do it.
  input:
    text: { type: string }
  output:
    result: { type: string }
flow.f:
  inputs:
    goal: { type: string }
  outputs: {}
  nodes:
    n:
      agent: agent.a
      input: { text: "input.goal" }
  edges:
    - { from: start, to: n, when: "size(input.goal) > 0" }
    - { from: start, to: n }
"#,
    );
}

/// A `merge` channel takes a partial object, and an `append` channel one
/// element (grammar 10.2, Decision D58).
#[test]
fn partial_and_element_wise_writes() {
    accepts(
        "reduced-writes",
        r#"
state:
  totals:
    type: object
    properties:
      fixed: { type: integer }
      skipped: { type: integer }
    reduce: merge
  patches:
    type: array
    items: { type: string }
    reduce: append
agent.a:
  model: model.m
  prompt: Do it.
  input:
    text: { type: string }
  output:
    totals:
      type: object
      properties:
        fixed: { type: integer }
    patches: { type: string }
flow.f:
  outputs: {}
  nodes:
    n:
      agent: agent.a
      input: { text: "'x'" }
  edges:
    - { from: start, to: n }
    - { from: n, to: end }
"#,
    );
}

/// A dispatch that passes the whole item, and one that selects a field from it
/// for a string-in agent (grammar 8.6 rule 12, Decision D75).
#[test]
fn both_per_item_binding_forms() {
    accepts(
        "per-item-forms",
        r#"
state:
  tasks:
    type: array
    max_items: 5
    items:
      type: object
      properties:
        title: { type: string }
  results:
    type: array
    items: { type: string }
    reduce: append
agent.worker:
  model: model.m
  prompt: Work.
  input:
    title: { type: string }
  output:
    results: { type: string }
agent.summarizer:
  model: model.m
  prompt: Summarize.
  output:
    results: { type: string }
flow.f:
  outputs: {}
  nodes:
    whole:
      map:
        over: "state.tasks"
        node: agent.worker
        max_concurrency: 4
    scalar:
      map:
        over: "state.tasks"
        as: task
        node: agent.summarizer
        input: "task.title"
        max_concurrency: 4
  edges:
    - { from: start, to: whole }
    - { from: whole, to: scalar }
    - { from: scalar, to: end }
"#,
    );
}

/// A routed map's `default:` sees the discriminator and the fields every
/// unrouted variant declares (grammar 8.6 rule 4, Decision D30).
#[test]
fn a_default_route_narrowed_to_the_unrouted_variants() {
    accepts(
        "default-route",
        r#"
agent.triage:
  model: model.m
  prompt: Triage.
  input:
    text: { type: string }
  output:
    findings:
      type: array
      max_items: 10
      items:
        discriminator: kind
        variants:
          auto_fixable:
            file: { type: string }
          duplicate:
            of: { type: string }
agent.fixer:
  model: model.m
  prompt: Fix.
  input:
    file: { type: string }
  output:
    patch: { type: string }
tool.dead_letter:
  description: Record an unroutable finding.
  input:
    kind: { type: string }
    payload: { type: string }
  output:
    accepted: { type: boolean }
  http:
    method: POST
    url: "https://${H}/dead-letter"
    body:
      kind: "input.kind"
      payload: "input.payload"
flow.f:
  outputs: {}
  nodes:
    classify:
      agent: agent.triage
      input: { text: "'x'" }
    dispatch:
      map:
        over: "classify.output.findings"
        as: finding
        route_by: kind
        max_concurrency: 4
        routes:
          auto_fixable:
            node: agent.fixer
            input: { file: "finding.file" }
        default:
          node: tool.dead_letter
          input:
            kind: "finding.kind"
            payload: "finding.of"
  edges:
    - { from: start, to: classify }
    - { from: classify, to: dispatch }
    - { from: dispatch, to: end }
"#,
    );
}

/// The two legal forms of a store write inside a fan-out: an item-derived key,
/// and a keyed `kv` write whose key is constant across instances (grammar 11.4,
/// Decisions D67, D83).
#[test]
fn both_legal_store_writes_inside_a_fan_out() {
    accepts(
        "map-store-writes",
        r#"
provider.local:
  kind: openai_compatible
  base_url: ${U}
store.docs:
  kind: vector
  scope: global
  embed:
    model: text-embedding-3-small
    provider: provider.local
store.memory:
  kind: kv
  scope: session
  value_schema:
    last: { type: string }
state:
  tasks:
    type: array
    max_items: 5
    items: { type: string }
flow.ingest:
  inputs:
    doc_id: { type: string }
    text: { type: string }
  outputs: {}
  nodes:
    save:
      store: store.docs
      op: upsert
      key: "input.doc_id"
      value: "input.text"
    remember:
      store: store.memory
      op: set
      key: "execution.session_key"
      value:
        last: "input.text"
  edges:
    - { from: start, to: save }
    - { from: save, to: remember }
    - { from: remember, to: end }
flow.f:
  outputs: {}
  nodes:
    work:
      map:
        over: "state.tasks"
        as: task
        node: flow.ingest
        max_concurrency: 4
        input:
          doc_id: "task"
          text: "task"
  edges:
    - { from: start, to: work }
    - { from: work, to: end }
"#,
    );
}

/// A store node reached by no `map` is not subject to the keying rule at all,
/// and its derived output is written by name like any other (grammar 11.4).
#[test]
fn a_store_node_outside_every_fan_out() {
    accepts(
        "store-outside-a-map",
        r#"
provider.local:
  kind: openai_compatible
  base_url: ${U}
store.docs:
  kind: vector
  scope: global
  embed:
    model: text-embedding-3-small
    provider: provider.local
  metadata_schema:
    source: { type: string }
state:
  matches:
    type: array
    max_items: 5
    items:
      type: object
      properties:
        id: { type: string }
        score: { type: number }
        text: { type: string }
        metadata:
          type: object
          properties:
            source: { type: string }
flow.f:
  inputs:
    query: { type: string }
  outputs:
    matches:
      type: array
      max_items: 5
      items:
        type: object
        properties:
          id: { type: string }
          score: { type: number }
          text: { type: string }
          metadata:
            type: object
            properties:
              source: { type: string }
  nodes:
    find:
      store: store.docs
      op: search
      query: "input.query"
      top_k: 5
      filter:
        source: "'docs'"
  edges:
    - { from: start, to: find }
    - { from: find, to: end, when: "size(find.output.matches) > 0" }
    - { from: find, to: end, else: true }
"#,
    );
}

/// An inline node's bindings build an ad-hoc object, so its names are its own
/// and its envelope fields are the kind's (grammar 8.2, 8.3).
#[test]
fn inline_nodes_bind_ad_hoc_objects() {
    accepts(
        "inline-nodes",
        r#"
state:
  draft: { type: string, default: "" }
  status: { type: integer, default: 0 }
flow.f:
  outputs: {}
  nodes:
    run:
      exec:
        command: run-checks
        expect_exit: [0, 1]
      input: "state.draft"
    notify:
      http:
        method: POST
        url: "https://${H}/notify"
        body:
          draft: "state.draft"
        output:
          status: { type: integer }
  edges:
    - { from: start, to: run }
    - { from: run, to: notify, when: "run.output.exit_code == 0" }
    - { from: run, to: notify, else: true }
    - { from: notify, to: end }
"#,
    );
}

/// A `get` whose result is written by name, and a guard that routes on the
/// companion `found` rather than on the value it may not carry (grammar 11.4,
/// Decision D110).
#[test]
fn a_get_that_may_miss() {
    accepts(
        "kv-get",
        r#"
store.prefs:
  kind: kv
  scope: session
  value_schema:
    theme: { type: string }
state:
  value:
    type: object
    properties:
      theme: { type: string }
    default: { theme: "dark" }
triggers:
  cli:
    type: manual
    flow: flow.f
flow.f:
  outputs: {}
  nodes:
    load:
      store: store.prefs
      op: get
      key: "execution.session_key"
  edges:
    - { from: start, to: load }
    - { from: load, to: end, when: "load.output.found" }
    - { from: load, to: end, else: true }
"#,
    );
}

/// A trigger binding whose CEL result is a decoded body member — a `Dyn` value
/// no declaration types, which every field accepts (grammar 13.3).
#[test]
fn a_trigger_binding_from_a_decoded_body() {
    accepts(
        "trigger-bindings",
        r#"
agent.a:
  model: model.m
  prompt: Do it.
  input:
    text: { type: string }
  output:
    result: { type: string }
triggers:
  on_request:
    type: http
    flow: flow.f
    input:
      goal: "payload.body.goal"
      urgent: "payload.query['urgent'] == 'yes'"
    session_key: "payload.headers['x-session-id']"
  nightly:
    type: schedule
    flow: flow.f
    cron: "0 3 * * *"
    input:
      goal: "'nightly sweep'"
      urgent: "false"
flow.f:
  inputs:
    goal: { type: string }
    urgent: { type: boolean }
  outputs: {}
  nodes:
    n:
      agent: agent.a
      input: { text: "input.goal" }
  edges:
    - { from: start, to: n }
    - { from: n, to: end }
"#,
    );
}

/// A `GET` trigger reading `payload.body` **whole** stays legal: it binds `{}`,
/// and whether a field accepts an empty object is an ordinary type check
/// (grammar 13.3, Decision D117).
#[test]
fn a_get_trigger_reading_the_body_whole() {
    accepts(
        "get-trigger",
        r#"
agent.a:
  model: model.m
  prompt: Do it.
  input:
    text: { type: string }
  output:
    result: { type: string }
triggers:
  on_request:
    type: http
    flow: flow.f
    method: GET
    input:
      envelope: "payload.body"
      goal: "payload.query['goal']"
flow.f:
  inputs:
    goal: { type: string }
    envelope:
      type: object
      properties:
        note: { type: string }
      optional: [note]
  outputs: {}
  nodes:
    n:
      agent: agent.a
      input: { text: "input.goal" }
  edges:
    - { from: start, to: n }
    - { from: n, to: end }
"#,
    );
}

/// Name-based wiring in both directions, with nothing between the two
/// declarations (grammar 8.0 steps 2–3, 7.5, Decision D111).
#[test]
fn name_based_wiring_in_both_directions() {
    accepts(
        "name-based-wiring",
        r#"
state:
  goal: { type: string, min_length: 1, default: "x" }
  draft: { type: string, max_length: 100, default: "" }
agent.a:
  model: model.m
  prompt: Do it.
  input:
    goal: { type: string }
  output:
    draft: { type: string, max_length: 10 }
flow.f:
  outputs:
    draft: { type: string, max_length: 100 }
  nodes:
    n:
      agent: agent.a
  edges:
    - { from: start, to: n }
    - { from: n, to: end }
"#,
    );
}
