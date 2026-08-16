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
    - { from: n, to: end }
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

/// The third per-item binding form: the empty one. A target that declares no
/// input fields — a flow with no `inputs:`, a no-argument tool — takes no part
/// of the item, and `input: {}` is how a dispatch says so. The whole-item
/// default does not fit such a target, and `input: {}` is what the diagnostic
/// that refuses it names (grammar 8.6 rule 12, 3.9).
#[test]
fn an_empty_per_item_binding_dispatches_a_target_with_no_inputs() {
    accepts(
        "per-item-empty-binding",
        r#"
state:
  tasks:
    type: array
    max_items: 5
    items: { type: string }
tool.ping:
  description: Ping the endpoint once.
  input: {}
  output: {}
  exec:
    command: ping
flow.sink:
  outputs: {}
  nodes:
    p:
      function: tool.ping
  edges:
    - { from: start, to: p }
    - { from: p, to: end }
flow.f:
  outputs: {}
  nodes:
    fan:
      map:
        over: "state.tasks"
        node: flow.sink
        max_concurrency: 2
        input: {}
    direct:
      map:
        over: "state.tasks"
        node: tool.ping
        max_concurrency: 2
        input: {}
  edges:
    - { from: start, to: fan }
    - { from: fan, to: direct }
    - { from: direct, to: end }
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

/// With **two** variants unrouted, the `default:` route still sees every field
/// they both declare — the case where "the fields every unrouted variant
/// declares" is a real intersection rather than one variant's whole payload
/// (grammar 8.6 rule 4, Decision D30).
///
/// The two declarations of `summary` are two source regions, so nothing here
/// works unless the narrowing compares the *types* rather than the
/// declarations. `finding.of`, which only one of the two declares, is the
/// rejecting half and is pinned by
/// `invalid-check/map-default-route-selects-an-unshared-field`.
#[test]
fn a_default_route_narrowed_to_two_unrouted_variants() {
    accepts(
        "default-route-intersection",
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
          needs_human:
            summary: { type: string }
            severity: { enum: [low, high] }
          duplicate:
            summary: { type: string }
            of: { type: string }
agent.fixer:
  model: model.m
  prompt: Fix.
  input:
    file: { type: string }
  output:
    patch: { type: string }
agent.reviewer:
  model: model.m
  prompt: Review.
  input:
    kind: { type: string }
    summary: { type: string }
  output:
    verdict: { type: string }
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
          node: agent.reviewer
          input:
            kind: "finding.kind"
            summary: "finding.summary"
  edges:
    - { from: start, to: classify }
    - { from: classify, to: dispatch }
    - { from: dispatch, to: end }
"#,
    );
}

/// A detached dispatch is legal — it is writing state that is not (grammar 8.6
/// rule 7, Decisions D31, D94). The sink's result field shares no name with any
/// channel, so nothing it produces lands in shared state and the fire-and-forget
/// dispatch stands.
#[test]
fn a_detached_dispatch_that_writes_no_state() {
    accepts(
        "detached-dispatch",
        r#"
state:
  tasks:
    type: array
    max_items: 5
    items: { type: string }
  note:
    type: array
    max_items: 5
    items: { type: string }
    reduce: append
agent.sink:
  model: model.m
  prompt: Record.
  input:
    text: { type: string }
  output:
    receipt: { type: string }
flow.f:
  outputs: {}
  nodes:
    fan:
      map:
        over: "state.tasks"
        node: agent.sink
        detach: true
        max_concurrency: 2
        input: { text: "item" }
  edges:
    - { from: start, to: fan }
    - { from: fan, to: end }
"#,
    );
}

/// A detached instance may fan out further, and a store write is not a state
/// write: what rule 7 refuses is a channel write, and nothing here makes one.
/// `flow.leaf` writes `store.docs` from an item-derived key inside two nested
/// fan-outs, and the only agent result — `receipt` — names no channel (grammar
/// 8.6 rules 5, 7, 11.4).
#[test]
fn a_detached_dispatch_that_fans_out_without_writing_state() {
    accepts(
        "detached-dispatch-fans-out",
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
state:
  tasks:
    type: array
    max_items: 5
    items: { type: string }
  note:
    type: array
    max_items: 5
    items: { type: string }
    reduce: append
agent.sink:
  model: model.m
  prompt: Record.
  input:
    text: { type: string }
  output:
    receipt: { type: string }
flow.leaf:
  inputs:
    text: { type: string }
  outputs: {}
  nodes:
    save:
      store: store.docs
      op: upsert
      key: "input.text"
      value: "input.text"
  edges:
    - { from: start, to: save }
    - { from: save, to: end }
flow.sink:
  inputs:
    text: { type: string }
  outputs: {}
  nodes:
    inner:
      map:
        over: "state.tasks"
        node: flow.leaf
        max_concurrency: 2
        input: { text: "item" }
    record:
      map:
        over: "state.tasks"
        node: agent.sink
        max_concurrency: 2
        input: { text: "item" }
  edges:
    - { from: start, to: inner }
    - { from: inner, to: record }
    - { from: record, to: end }
flow.f:
  outputs: {}
  nodes:
    fan:
      map:
        over: "state.tasks"
        node: flow.sink
        detach: true
        max_concurrency: 2
        input: { text: "item" }
  edges:
    - { from: start, to: fan }
    - { from: fan, to: end }
"#,
    );
}

/// The idiomatic subflow fan-in: a `map` dispatches a `flow.*`, whose node
/// writes the channel the subflow's own `outputs:` materialize from, and whose
/// result the dispatch remaps into an `append` channel of the dispatching flow
/// (grammar 8.6 rules 5, 6, 10.2's fan-in note).
///
/// The channel set is composition-global in **shape** and each flow instance
/// holds its own **values** (grammar 10.1), so `scratch` inside `flow.ingest`
/// is that instance's own — one writer, no race — and what crosses is the
/// instance's `outputs:` through the dispatch's write map (7.6.4 rule 3). Rule
/// 5 is about the crossing write, and `results` is `append` as it requires.
/// Reading it over the subflow's internals instead would make this shape
/// unwritable: `reduce:` on `scratch` is what it would ask for, and an
/// `append` `scratch` no longer satisfies a `string` output field (7.5).
#[test]
fn a_dispatched_subflow_writes_its_own_channels_and_returns_one_element() {
    accepts(
        "subflow-fan-in",
        r#"
state:
  tasks:
    type: array
    max_items: 5
    items: { type: string }
  scratch: { type: string, default: "" }
  results:
    type: array
    max_items: 5
    items: { type: string }
    reduce: append
agent.worker:
  model: model.m
  prompt: Work.
  input:
    text: { type: string }
  output:
    scratch: { type: string }
flow.ingest:
  inputs:
    text: { type: string }
  outputs:
    scratch: { type: string }
  nodes:
    n:
      agent: agent.worker
      input: { text: "input.text" }
  edges:
    - { from: start, to: n }
    - { from: n, to: end }
flow.f:
  outputs: {}
  nodes:
    fan:
      map:
        over: "state.tasks"
        node: flow.ingest
        max_concurrency: 4
        input: { text: "item" }
        writes: { scratch: results }
  edges:
    - { from: start, to: fan }
    - { from: fan, to: end }
"#,
    );
}

/// The same isolation under `detach: true`. What rule 7 refuses is a write to
/// the *dispatching* flow's state — `flow.sink` returns nothing, so nothing
/// crosses — and the channels its own nodes write, at any depth and through a
/// fan-out of its own, belong to the instance that writes them (grammar 8.6
/// rule 7, 10.1, Decision D94).
#[test]
fn a_detached_subflow_dispatch_whose_nodes_write_their_own_channels() {
    accepts(
        "detached-subflow-internals",
        r#"
state:
  tasks:
    type: array
    max_items: 5
    items: { type: string }
  scratch: { type: string, default: "" }
  note:
    type: array
    max_items: 5
    items: { type: string }
    reduce: append
agent.worker:
  model: model.m
  prompt: Work.
  input:
    text: { type: string }
  output:
    scratch: { type: string }
agent.recorder:
  model: model.m
  prompt: Record.
  input:
    text: { type: string }
  output:
    note: { type: string }
flow.sink:
  inputs:
    text: { type: string }
  outputs: {}
  nodes:
    n:
      agent: agent.worker
      input: { text: "input.text" }
    inner:
      map:
        over: "state.tasks"
        node: agent.recorder
        max_concurrency: 2
        input: { text: "item" }
  edges:
    - { from: start, to: n }
    - { from: n, to: inner }
    - { from: inner, to: end }
flow.f:
  outputs: {}
  nodes:
    fan:
      map:
        over: "state.tasks"
        node: flow.sink
        detach: true
        max_concurrency: 2
        input: { text: "item" }
  edges:
    - { from: start, to: fan }
    - { from: fan, to: end }
"#,
    );
}

/// An agent may attach both a store and tools; what grammar 11.5 refuses is a
/// synthesized name that collides with an attached one, and none of these do —
/// including `prefs_search`, which a `kv` store does not synthesize, and
/// `prefs_set`, which `agent_access: read` does not.
#[test]
fn attached_store_tools_that_do_not_collide() {
    accepts(
        "store-tool-names",
        r#"
store.prefs:
  kind: kv
  scope: global
  agent_access: read
  value_schema:
    theme: { type: string }
tool.prefs_search:
  description: Search the preference catalogue.
  input:
    query: { type: string }
  output:
    hits: { type: string }
  exec:
    command: prefs-search
tool.prefs_set:
  description: Set a preference out of band.
  input:
    key: { type: string }
  output:
    ok: { type: boolean }
  exec:
    command: prefs-set
agent.a:
  model: model.m
  prompt: Decide.
  tools: [tool.prefs_search, tool.prefs_set]
  stores: [store.prefs]
  output:
    verdict: { type: string }
flow.f:
  outputs: {}
  nodes:
    n:
      agent: agent.a
      input: "'x'"
  edges:
    - { from: start, to: n }
    - { from: n, to: end }
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

/// `execution.item_index` derives a key the way the unkeyed-write help says it
/// does: from inside a string-valued expression. The index itself is an integer
/// (grammar 4.1) and a `key` is a string (grammar 11.4), so a bare
/// `key: "execution.item_index"` is a type error however item-derived it is —
/// what satisfies both is an expression that *reads* the index and evaluates to
/// a string.
#[test]
fn an_item_index_keyed_store_write_inside_a_fan_out() {
    accepts(
        "map-store-write-keyed-by-the-index",
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
state:
  tasks:
    type: array
    max_items: 5
    items: { type: string }
  topic: { type: string, default: "" }
flow.ingest:
  inputs:
    doc_id: { type: string }
    text: { type: string }
  outputs: {}
  nodes:
    save:
      store: store.docs
      op: upsert
      key: "state.tasks[execution.item_index]"
      value: "input.text"
  edges:
    - { from: start, to: save }
    - { from: save, to: end }
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
          doc_id: "state.topic"
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

/// A property with a `default:` is optional **at its surface** — a binding need
/// not supply it, and grammar 8.0's step 4 then does — so the value the
/// declaration describes carries it either way and the channel stays readable
/// by name (grammar 3.6, 8.0).
///
/// The half that a `default:` is *not* is grammar 3.4's `optional:`, which is
/// what makes a property absent from a value at run time (Decision D110); the
/// rejecting half of that is `check_invalid`'s
/// `channel-does-not-satisfy-the-field-it-lands-in` and the reading below.
#[test]
fn a_defaulted_property_is_not_an_absent_one() {
    accepts(
        "defaulted-property",
        r#"
state:
  author:
    type: object
    properties:
      name: { type: string }
      email: { type: string, default: "nobody@example.com" }
  writer:
    type: object
    properties:
      name: { type: string }
      email: { type: string, default: "nobody@example.com" }
  partial:
    type: object
    properties:
      name: { type: string }
      email: { type: string }
    optional: [email]
agent.a:
  model: model.m
  prompt: Do it.
  input:
    author:
      type: object
      properties:
        name: { type: string }
        email: { type: string }
    other:
      type: object
      properties:
        name: { type: string }
        email: { type: string }
    third:
      type: object
      properties:
        name: { type: string }
        email: { type: string, default: "nobody@example.com" }
  output:
    result: { type: string }
flow.f:
  outputs: {}
  nodes:
    n:
      agent: agent.a
      input:
        other: "state.writer"
        third: "state.partial"
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

/// A **mixed-guard** node stays exhaustive: the sibling whose guard never
/// mentions `verdict` contributes nothing to its coverage and does not exempt
/// the node, and the `==`/`!=` pair covers the enum between them (grammar 7.3.1
/// clause 2, Decision D82).
#[test]
fn a_mixed_guard_node_covered_by_an_inequality() {
    accepts(
        "mixed-guards",
        r#"
state:
  feedback: { type: string, default: "" }
agent.a:
  model: model.m
  prompt: Do it.
  output:
    verdict: { enum: [approve, revise, escalate] }
flow.f:
  outputs: {}
  nodes:
    review: { agent: agent.a, input: "'x'" }
    publish: { agent: agent.a, input: "'y'" }
    rework: { agent: agent.a, input: "'z'" }
    notify: { agent: agent.a, input: "'w'" }
  edges:
    - { from: start, to: review }
    - { from: review, to: publish, when: "review.output.verdict == 'approve'" }
    - { from: review, to: rework, when: "review.output.verdict != 'approve'" }
    - { from: review, to: notify, when: "size(state.feedback) > 0" }
    - { from: publish, to: end }
    - { from: rework, to: end }
    - { from: notify, to: end }
"#,
    );
}

/// A cycle bounded by a CEL exit condition alone — a guarded back-edge beside an
/// `else: true` escape, which is the spelling grammar 7.4 calls usual and which
/// clause 2's earlier wording rejected (Decision D98).
#[test]
fn a_cycle_bounded_by_an_else_exit() {
    accepts(
        "else-exit-cycle",
        r#"
agent.a:
  model: model.m
  prompt: Do it.
  output:
    verdict: { enum: [approve, revise] }
flow.f:
  outputs: {}
  nodes:
    write: { agent: agent.a, input: "'x'" }
    review: { agent: agent.a, input: "'y'" }
  edges:
    - { from: start, to: write }
    - { from: write, to: review }
    - { from: review, to: write, when: "review.output.verdict == 'revise'" }
    - { from: review, to: end, else: true }
"#,
    );
}

/// A guarded shortcut **inside one concurrent branch** is not an unbalanced
/// convergence: `c` is reached from the fork at depths 2 and 3 through one edge
/// of the pair and at none through the other, and balance compares one distance
/// from *each* edge rather than the union of one side (grammar 7.6.2,
/// Decision D112). `a`'s own two out-edges are exclusive, so at most one of the
/// two paths is taken on a pass and `c` never receives two deliveries.
#[test]
fn a_guarded_shortcut_inside_one_concurrent_branch() {
    accepts(
        "guarded-shortcut",
        r#"
agent.a:
  model: model.m
  prompt: Do it.
  output:
    verdict: { enum: [approve, revise] }
flow.f:
  outputs: {}
  nodes:
    fork: { agent: agent.a, input: "'x'" }
    a: { agent: agent.a, input: "'a'" }
    b: { agent: agent.a, input: "'b'" }
    c: { agent: agent.a, input: "'c'" }
    d: { agent: agent.a, input: "'d'" }
  edges:
    - { from: start, to: fork }
    - { from: fork, to: a }
    - { from: fork, to: b }
    - { from: a, to: c, when: "a.output.verdict == 'approve'" }
    - { from: a, to: d, else: true }
    - { from: d, to: c }
    - { from: c, to: end }
    - { from: b, to: end }
"#,
    );
}

/// Two sibling edges that land on **one** node start one branch, not two.
/// Grammar 7.6's P2 runs a node targeted by several edges taken in the same step
/// exactly once, so `x` runs once and takes exactly one of *its* two exclusive
/// out-edges: `d` receives one delivery per pass however the two `plan -> x`
/// guards came out. The distances `{2, 3}` that reach `d` are one branch's own,
/// and comparing them is the comparison inside one side Decision D112 refuses —
/// here reached from the other end, since both sides of the pair are the *same*
/// set of paths. Making the two `plan -> x` guards exclusive would silence a
/// diagnostic while changing no runtime behaviour, which is how it reads as
/// over-rejection rather than as strictness (grammar 7.6, 7.6.2, D99, D112).
#[test]
fn two_sibling_edges_that_deliver_to_one_node() {
    accepts(
        "same-target-pair",
        r#"
agent.a:
  model: model.m
  prompt: Do it.
  output:
    verdict: { enum: [approve, revise] }
agent.planner:
  model: model.m
  prompt: Plan.
  output:
    need_draft: { type: boolean }
    need_research: { type: boolean }
flow.f:
  outputs: {}
  nodes:
    plan: { agent: agent.planner, input: "'p'" }
    x: { agent: agent.a, input: "'x'" }
    m: { agent: agent.a, input: "'m'" }
    d: { agent: agent.a, input: "'d'" }
  edges:
    - { from: start, to: plan }
    - { from: plan, to: x, when: "plan.output.need_draft" }
    - { from: plan, to: x, when: "plan.output.need_research" }
    - { from: x, to: d, when: "x.output.verdict == 'approve'" }
    - { from: x, to: m, when: "x.output.verdict == 'revise'" }
    - { from: m, to: d }
    - { from: d, to: end }
"#,
    );
}

/// The concurrent-write half of the same reading (grammar 7.6.1, 10.2). `p` and
/// `q` sit behind the two *exclusive* out-edges of `x`, and `x` is the single
/// node both `plan -> x` edges deliver to — so one pass runs one of them and
/// they never race for `verdict`. Reading the pair as two branches makes every
/// node of `x`'s branch concurrent with every other, and demands a `reduce:`
/// policy on a channel nothing can race for; whether two nodes *inside* that
/// branch are concurrent is its own fork's question, and `x`'s pair is exclusive.
#[test]
fn writers_behind_one_node_two_sibling_edges_share() {
    accepts(
        "same-target-writers",
        r#"
agent.a:
  model: model.m
  prompt: Do it.
  output:
    verdict: { enum: [approve, revise] }
agent.planner:
  model: model.m
  prompt: Plan.
  output:
    need_draft: { type: boolean }
    need_research: { type: boolean }
state:
  verdict:
    description: the last verdict reached
    enum: [approve, revise]
flow.f:
  outputs: {}
  nodes:
    plan: { agent: agent.planner, input: "'p'" }
    x: { agent: agent.a, input: "'x'" }
    p: { agent: agent.a, input: "'p'" }
    q: { agent: agent.a, input: "'q'" }
  edges:
    - { from: start, to: plan }
    - { from: plan, to: x, when: "plan.output.need_draft" }
    - { from: plan, to: x, when: "plan.output.need_research" }
    - { from: x, to: p, when: "x.output.verdict == 'approve'" }
    - { from: x, to: q, when: "x.output.verdict == 'revise'" }
    - { from: p, to: end }
    - { from: q, to: end }
"#,
    );
}

/// A node no edge targets, reached only through `on_error: { fallback: … }`.
/// Reachability counts the two control-transfer positions, so a dedicated
/// error-handling node is live code rather than an unreachable one — and it
/// still needs an outgoing edge of its own (grammar 7.8, 7.6.3 rule 1,
/// Decision D95).
#[test]
fn a_node_reached_only_through_a_fallback() {
    accepts(
        "fallback-only-node",
        r#"
agent.a:
  model: model.m
  prompt: Do it.
  output:
    verdict: { enum: [approve, revise] }
flow.f:
  outputs: {}
  nodes:
    n:
      agent: agent.a
      input: "'x'"
      on_error: { fallback: cleanup }
    cleanup: { agent: agent.a, input: "'y'" }
  edges:
    - { from: start, to: n }
    - { from: n, to: end }
    - { from: cleanup, to: end }
"#,
    );
}

/// The accepting half of the same reading of grammar 7.6.1 that
/// `tests/fixtures/invalid-check/a-fallback-target-races-the-other-branch`
/// pins the rejecting half of: a branch holds what its control transfers can
/// schedule, and "neither is reachable from the other" is read over that same
/// relation.
///
/// `cleanup` is `merge`'s fallback, and `merge` is the convergence both branches
/// of the fork at `plan` deliver to — so `cleanup` sits on *both* branches and
/// gets crossed with `left`, which writes the same unreduced channel. It runs
/// only where `merge` failed, and `merge` runs only after `left` completed, so
/// the two are sequential and no `reduce:` policy is owed. Reading a branch over
/// control transfers while reading "reachable from the other" over edges alone
/// would demand one here — a fallback on a convergence being an ordinary shape,
/// that is the over-rejection the pairing of the two readings avoids
/// (grammar 7.6.1, 7.8, 9.2, 10.2, Decision D32).
#[test]
fn a_fallback_below_a_convergence_is_sequential_with_the_branches() {
    accepts(
        "fallback-below-a-convergence",
        r#"
agent.a:
  model: model.m
  prompt: Do it.
  output:
    verdict: { enum: [approve, revise] }
agent.n:
  model: model.m
  prompt: Note.
  output:
    note: { type: string }
state:
  note: { type: string, default: "" }
flow.f:
  outputs: {}
  nodes:
    plan: { agent: agent.a, input: "'p'" }
    left: { agent: agent.n, input: "'l'" }
    right: { agent: agent.a, input: "'r'" }
    merge:
      agent: agent.a
      input: "'m'"
      on_error: { fallback: cleanup }
    cleanup: { agent: agent.n, input: "'c'" }
  edges:
    - { from: start, to: plan }
    - { from: plan, to: left }
    - { from: plan, to: right }
    - { from: left, to: merge }
    - { from: right, to: merge }
    - { from: merge, to: end }
    - { from: cleanup, to: end }
"#,
    );
}

/// Grammar 7.4's escape rule reads over the source of **every** bounded edge,
/// and outside a cycle only clause (b) is left to satisfy: a node alone in its
/// component is left by every outgoing edge it has, so an `else: true` sibling
/// discharges the rule on its own. The rejecting half is
/// `tests/fixtures/invalid-check/bounded-edge-outside-every-cycle-has-no-escape`,
/// where the same shape carries only guarded siblings and a fallback runs the
/// source a second time (grammar 7.4, 9.2, Decisions D19, D90).
#[test]
fn a_bounded_edge_outside_every_cycle_escapes_through_an_else() {
    accepts(
        "acyclic-budget",
        r#"
agent.a:
  model: model.m
  prompt: Do it.
  output:
    verdict: { enum: [approve, revise] }
flow.f:
  outputs: {}
  nodes:
    a: { agent: agent.a, input: "'x'" }
    b: { agent: agent.a, input: "'y'" }
    c: { agent: agent.a, input: "'z'" }
  edges:
    - { from: start, to: a }
    - { from: a, to: b, when: "a.output.verdict == 'approve'", max_iterations: 3 }
    - { from: a, to: c, else: true }
    - { from: b, to: end }
    - { from: c, to: end }
"#,
    );
}

/// The rule is stated over the source node of the bounded edge and is checked
/// there, whatever else bounds the SCC (grammar 7.4): `p` sits upstream of a
/// loop it never joins, and its own `else: true` sibling is what discharges the
/// rule — not the loop's `else:` exit, which belongs to `review`.
#[test]
fn a_bounded_edge_upstream_of_a_cycle_escapes_through_an_else() {
    accepts(
        "budget-above-a-cycle",
        r#"
agent.a:
  model: model.m
  prompt: Do it.
  output:
    verdict: { enum: [approve, revise] }
flow.f:
  outputs: {}
  nodes:
    p: { agent: agent.a, input: "'p'" }
    q: { agent: agent.a, input: "'q'" }
    write: { agent: agent.a, input: "'w'" }
    review: { agent: agent.a, input: "'r'" }
  edges:
    - { from: start, to: p }
    - { from: p, to: q, when: "p.output.verdict == 'approve'", max_iterations: 3 }
    - { from: p, to: write, else: true }
    - { from: q, to: end }
    - { from: write, to: review }
    - { from: review, to: write, when: "review.output.verdict == 'revise'" }
    - { from: review, to: end, else: true }
"#,
    );
}

/// A node the loop routes to *does* carry the escape rule, and an `else: true`
/// sibling discharges it: the pass that exhausts the budget takes that edge
/// instead of dead-ending, which is the whole content of Decision D19. The
/// rejecting half is the fixture named above (grammar 7.4, 7.3 rules 4 and 5).
#[test]
fn a_bounded_edge_below_a_cycle_escapes_through_an_else() {
    accepts(
        "budget-below-a-cycle",
        r#"
agent.a:
  model: model.m
  prompt: Do it.
  output:
    verdict: { enum: [approve, revise, escalate] }
flow.f:
  outputs: {}
  nodes:
    a: { agent: agent.a, input: "'a'" }
    b: { agent: agent.a, input: "'b'" }
    n: { agent: agent.a, input: "'n'" }
    x: { agent: agent.a, input: "'x'" }
    y: { agent: agent.a, input: "'y'" }
  edges:
    - { from: start, to: a }
    - { from: a, to: b }
    - { from: b, to: a, when: "b.output.verdict == 'revise'" }
    - { from: b, to: n, when: "b.output.verdict in ['approve', 'revise', 'escalate']" }
    - { from: n, to: x, when: "n.output.verdict == 'approve'", max_iterations: 3 }
    - { from: n, to: y, else: true }
    - { from: x, to: end }
    - { from: y, to: end }
"#,
    );
}
