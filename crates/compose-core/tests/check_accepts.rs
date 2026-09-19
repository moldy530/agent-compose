//! Legal shapes the example corpus does not happen to use.
//!
//! `check_examples.rs` proves the data checks accept the two worked projects;
//! this file proves they accept the rest of what the grammar admits.
//! Over-rejection is the failure mode a negative corpus cannot catch — a rule
//! written slightly too tight makes a legal spec unwritable, and nothing else
//! in the suite would notice. Every case here is one the grammar names, and
//! several are the *accepting* half of a rule the negative corpus pins the
//! rejecting half of.
//!
//! Each case is also **emitted**, because over-rejection has a second form the
//! validator alone cannot show. A composition this file accepts is one `build`,
//! `run` and `serve` all owe a project to; an emitter that reached for a schema
//! nobody wrote, or indexed a list a legal shape leaves empty, refuses it as a
//! panic and exit 101 rather than as a diagnostic — a spec that validates clean
//! and cannot be compiled. The bytes are the golden corpus's subject; that the
//! emitter answers *at all* over the whole legal surface is this one's, since
//! the legal-but-unusual compositions live here.

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

    // The emitter's own over-rejection: a legal shape it never learned to write.
    //
    // Every case here is a composition `validate` accepts, and `build`, `run`
    // and `serve` all owe one a project — so the emitter reaching an
    // `unreachable!` or an indexing panic over a shape this corpus admits is the
    // same failure this file exists to catch, arriving as exit 101 instead of as
    // a diagnostic. The emitted bytes are the goldens' subject and not this
    // file's; that the emitter *answers at all* is this one's, because the
    // corpus of legal-but-unusual compositions is here rather than there.
    //
    // A case binding a `module:` implementation is emitted the way `build`
    // emits one: the stub is scaffolded into the project first and then read
    // back, because the artifact carries the authored file (PRD resolved q49)
    // and `emit` is a pure function that is handed those bytes. Scaffolding
    // rather than inventing content is what keeps this the command's own order.
    for scaffold in compose_core::codegen::authored::scaffolds(&ir) {
        let path = dir.join(scaffold.path.replace('/', std::path::MAIN_SEPARATOR_STR));
        fs::create_dir_all(path.parent().expect("a project-relative path has a parent"))
            .expect("can write the scaffold");
        fs::write(&path, &scaffold.contents).expect("can write the scaffold");
    }
    let authored = compose_core::Authored::read(&ir, &dir)
        .unwrap_or_else(|error| panic!("{name} references a module that cannot be read: {error}"));
    let project = compose_core::emit(&ir, &authored);
    assert!(
        !project.files().is_empty(),
        "{name} validates, so `build` owes it a project, and it emitted no files"
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

/// The same target from the *other* module boundary: a `flow:` node
/// instantiating a flow that declares no `inputs:` (grammar 7.5, 8.5).
///
/// `inputs:` is optional, so the instantiation binds nothing and writes no
/// `input:` at all — and it is still total over the parameter list, because the
/// list is empty (Decision D68). Legal, ordinary — `flow.tenant_recall` in the
/// store acceptance fixture is an inputs-less flow — and reached by nothing in
/// the example corpus, which is what left the emitter with a surface it never
/// learned to look up: nobody writes `inputs: {}` down, so no `<flow>.inputs`
/// schema is emitted for such a flow, and asking for one panicked `build`,
/// `run` and `serve` on a composition `validate` had just accepted. The `map`
/// case above is the position that already answered.
#[test]
fn a_flow_node_instantiates_a_flow_with_no_inputs() {
    accepts(
        "flow-node-no-inputs",
        r#"
state:
  seen:
    type: string
    default: ""
flow.inner:
  outputs:
    seen: { type: string }
  nodes:
    stamp:
      exec:
        command: printf
        args: ["%s", "hi"]
        output:
          stdout: { type: string }
      writes:
        stdout: seen
  edges:
    - { from: start, to: stamp }
    - { from: stamp, to: end }
flow.f:
  outputs:
    seen: { type: string }
  nodes:
    sub:
      flow: flow.inner
      writes:
        seen: seen
  edges:
    - { from: start, to: sub }
    - { from: sub, to: end }
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

/// The accepting half of the delivery-slot rule, on the side of the *dispatch*:
/// an `exec:` sink may declare an `idempotency_key` input field, and every
/// dispatch of it whose outcome is **observed** may bind it. Grammar 9.4 gives a
/// key to a detached dispatch and to nothing else, so a joined one leaves the
/// `IDEMPOTENCY_KEY` variable to the field and there is no second writer for
/// Decision D66 to refuse.
#[test]
fn a_joined_dispatch_binds_an_exec_sinks_own_idempotency_key_field() {
    accepts(
        "joined-dispatch-binds-an-idempotency-key-field",
        r#"
state:
  findings:
    type: array
    max_items: 5
    items: { type: string }
tool.audit_log:
  description: Append one finding to the audit log.
  input:
    finding: { type: string }
    idempotency_key: { type: string }
  output: {}
  exec:
    command: audit-log
flow.f:
  outputs: {}
  nodes:
    fan:
      map:
        over: "state.findings"
        node: tool.audit_log
        max_concurrency: 2
        input:
          finding: "item"
          idempotency_key: "item"
  edges:
    - { from: start, to: fan }
    - { from: fan, to: end }
"#,
    );
}

/// …and the accepting half on the side of the *target*: only one of grammar
/// 9.4's three delivery surfaces shares a namespace with the sink's declared
/// input. An `http:` target reads the key out of the `Idempotency-Key` **header**
/// and a `function:` target out of its **invocation context**, both of which sit
/// beside the input object — so a detached dispatch to either may bind a field
/// of that name, and the rule stays the `exec:`-only rule grammar 6.1's
/// environment makes it.
#[test]
fn a_detached_dispatch_to_a_sink_whose_key_rides_beside_its_input() {
    accepts(
        "detached-dispatch-keyed-beside-the-input",
        r#"
state:
  findings:
    type: array
    max_items: 5
    items: { type: string }
tool.file_ticket:
  description: File one finding as a ticket.
  input:
    finding: { type: string }
    idempotency_key: { type: string }
  output: {}
  http:
    method: POST
    url: "https://example.invalid/tickets"
    body:
      title: "input.finding"
      dedupe: "input.idempotency_key"
tool.audit_log:
  description: Append one finding to the audit log.
  input:
    finding: { type: string }
    idempotency_key: { type: string }
  output: {}
  function: { name: audit_log }
flow.f:
  outputs: {}
  nodes:
    ticket:
      map:
        over: "state.findings"
        node: tool.file_ticket
        detach: true
        max_concurrency: 2
        input:
          finding: "item"
          idempotency_key: "item"
    record:
      map:
        over: "state.findings"
        node: tool.audit_log
        detach: true
        max_concurrency: 2
        input:
          finding: "item"
          idempotency_key: "item"
  edges:
    - { from: start, to: ticket }
    - { from: ticket, to: record }
    - { from: record, to: end }
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

/// A route the app's own routes only *look* like stays legal (grammar 13.3).
///
/// The accepting half of `invalid-check/an-http-trigger-claims-a-route-the-app-mounts`,
/// and the reason it is written out rather than assumed: the app mounts exactly
/// `GET /executions/:id` and `POST /executions/:id/resume`, so a check that
/// matched a *prefix*, or matched the path without the method, would refuse
/// every one of these — a longer path under the same prefix, a shorter one, the
/// reserved path at another method, and the `POST` one at the `GET` one's
/// address. Each is a route the router can tell apart from the app's, and none
/// of them is the app's.
#[test]
fn a_trigger_route_beside_the_apps_own_routes() {
    accepts(
        "routes-beside-reserved",
        r#"
agent.a:
  model: model.m
  prompt: Do it.
  input:
    text: { type: string }
  output:
    result: { type: string }
triggers:
  detail:
    type: http
    flow: flow.f
    method: GET
    path: /executions/:id/detail
    input:
      goal: "payload.query['goal']"
  index:
    type: http
    flow: flow.f
    method: GET
    path: /executions
    input:
      goal: "payload.query['goal']"
  replace:
    type: http
    flow: flow.f
    method: PUT
    path: /executions/:id
    input:
      goal: "payload.body.goal"
  restart:
    type: http
    flow: flow.f
    method: GET
    path: /executions/:id/resume
    input:
      goal: "payload.query['goal']"
flow.f:
  inputs:
    goal: { type: string }
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

/// The accepting half of
/// `invalid-check/manual-triggers-disagree-about-the-session-key`, which is
/// where the rule could go wrong in the other direction. Four `manual` triggers
/// here and none of them conflicts:
///
/// * `cli` and `also_cli` name one flow and declare the **same** remap, which is
///   one answer written twice rather than two answers;
/// * `quiet_cli` names that same flow and declares **no** `session_key:` — the
///   key's `"payload.session"` default, which is a trigger with no opinion about
///   an entry rather than a third answer for it (grammar 13.1's table);
/// * `other_cli` declares a different remap for a **different** flow, which is a
///   different CLI entry entirely.
///
/// A rule that compared any two `manual` triggers, or read an undeclared
/// `session_key:` as a declaration of the default, would refuse all of this.
#[test]
fn manual_triggers_that_agree_or_are_silent_about_the_session_key() {
    accepts(
        "manual-session-keys",
        r#"
store.memory:
  kind: kv
  scope: session
  value_schema:
    text: { type: string }
triggers:
  cli:
    type: manual
    flow: flow.f
    session_key: '"tenant/" + payload.session'
  also_cli:
    type: manual
    flow: flow.f
    session_key: '"tenant/" + payload.session'
  quiet_cli:
    type: manual
    flow: flow.f
  other_cli:
    type: manual
    flow: flow.g
    session_key: '"other/" + payload.session'
flow.f:
  inputs:
    goal: { type: string }
  outputs: {}
  nodes:
    n:
      store: store.memory
      op: set
      key: "input.goal"
      value: { text: "input.goal" }
  edges:
    - { from: start, to: n }
    - { from: n, to: end }
flow.g:
  inputs:
    goal: { type: string }
  outputs: {}
  nodes:
    n:
      store: store.memory
      op: set
      key: "input.goal"
      value: { text: "input.goal" }
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

/// A bounded cycle whose two nodes both write one **unreduced** channel.
///
/// `review`'s two out-edges are a `when:` and an `else:`, which grammar 7.3
/// rule 4 makes exclusive by construction, so the flow has no fork at all — no
/// co-takeable pair, no concurrency, and two writers that simply alternate
/// across steps. The concurrency relation is stated over the two edges of one
/// co-takeable pair (grammar 7.6.1), and a rule that read "in one cycle" as
/// "concurrent" on its own would make the PRD's flagship loop unwritable
/// without a `reduce:` policy on every channel it touches. What a cycle does
/// change is the reading of a pair that already exists, which is
/// `tests/fixtures/invalid-check/writers-deep-in-a-cycle-reach-each-other`.
#[test]
fn a_bounded_loop_with_no_fork_races_nothing() {
    accepts(
        "loop-without-a-fork",
        r#"
agent.r:
  model: model.m
  prompt: Do it.
  output:
    note: { type: string }
    verdict: { enum: [approve, revise] }
state:
  note: { type: string, default: "" }
flow.f:
  outputs: {}
  nodes:
    write: { agent: agent.r, input: "'w'" }
    review: { agent: agent.r, input: "'r'" }
  edges:
    - { from: start, to: write }
    - { from: write, to: review }
    - { from: review, to: write, when: "review.output.verdict == 'revise'", max_iterations: 3 }
    - { from: review, to: end, else: true }
"#,
    );
}

/// The accepting half of `a-fork-inside-a-bounded-cycle-races` and of
/// `writers-deep-in-a-cycle-reach-each-other`: the very same fan inside the very
/// same bounded loop, with the `reduce:` policy those two are missing.
///
/// A fan inside a review loop is an ordinary shape, and what makes it legal is
/// the declared policy rather than anything about the cycle — so the rule has to
/// stop asking the moment the policy is there (grammar 10.2, Decision D32).
#[test]
fn a_fan_inside_a_bounded_cycle_needs_only_the_policy() {
    accepts(
        "fan-inside-a-cycle",
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
  note: { type: string, reduce: last_wins, default: "" }
flow.f:
  outputs: {}
  nodes:
    fork: { agent: agent.a, input: "'w'" }
    x: { agent: agent.n, input: "'x'" }
    y: { agent: agent.n, input: "'y'" }
    j: { agent: agent.a, input: "'j'" }
  edges:
    - { from: start, to: fork }
    - { from: fork, to: x }
    - { from: fork, to: y }
    - { from: x, to: j }
    - { from: y, to: j }
    - { from: j, to: fork, when: "j.output.verdict == 'revise'", max_iterations: 3 }
    - { from: j, to: end, else: true }
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

/// Both built-ins on one agent in both spellings, beside a `tool.*` of its own
/// (grammar 5.5, 6.1, Decision D135).
///
/// The accepting half of the corpus that pins every rejection: an `input:` on a
/// built-in tool, a `timeout:` on `builtin: files`, a name outside the set, a
/// bounded mapping in a `tools:` list, a built-in listed twice. Each of those is
/// a fixture; this is the shape they are each *nearly*, and a rule written one
/// notch tighter than §6.1 would make it unwritable with nothing else noticing.
///
/// It is emitted as well as checked, which is this file's second claim and the
/// one that matters most for a construct the emitter learned last: a shorthand
/// reaches `codegen::graph` through a list every other agent leaves empty, and a
/// configured one through the attachment loop every other tool takes.
#[test]
fn every_builtin_attached_to_one_agent_beside_a_tool() {
    accepts(
        "every-builtin",
        r#"
tool.repo_grep:
  description: Search the repository.
  input:
    pattern: { type: string }
  output:
    matches: { type: string }
  exec:
    command: rg
tool.sandbox:
  builtin: bash
  workspace: "${WORKSPACE}/build"
  timeout: 30s
  env:
    PATH: "/usr/bin:/bin"
  inherit_env: false
agent.a:
  model: model.m
  prompt: Fix it.
  tools:
    - tool.repo_grep
    - tool.sandbox
    - builtin.files
  output:
    verdict: { enum: [approve, revise] }
flow.f:
  outputs: {}
  nodes:
    n: { agent: agent.a, input: "'x'" }
  edges:
    - { from: start, to: n }
    - { from: n, to: end }
"#,
    );
}

/// One built-in, on an agent carrying no `tool.*` and no `stores:` at all.
///
/// The list every other case leaves non-empty: an agent whose whole tool
/// surface is a shorthand built-in is where the emitter's "no tools at all"
/// branch and its "some tools" branch meet, and where a reader first meets the
/// construct (grammar 5.5).
#[test]
fn one_builtin_is_a_whole_tool_surface() {
    accepts(
        "one-builtin",
        r#"
agent.a:
  model: model.m
  prompt: Read it.
  tools:
    - builtin.files
  output:
    verdict: { enum: [approve, revise] }
flow.f:
  outputs: {}
  nodes:
    n: { agent: agent.a, input: "'x'" }
  edges:
    - { from: start, to: n }
    - { from: n, to: end }
"#,
    );
}

/// A built-in configured **without** bounds, and one carrying only a workspace
/// relative to the process (grammar 6.1).
///
/// Every bound is optional, so `tool.plain` below is the shorthand written the
/// long way — the shape a rule that required a workspace, or a timeout, would
/// make unwritable. The author's `description:` is the other optional key, and
/// it is the one key of a built-in tool a composition may sharpen.
#[test]
fn a_builtin_binding_takes_every_bound_or_none() {
    accepts(
        "builtin-bounds",
        r#"
tool.plain:
  builtin: bash
tool.described:
  builtin: files
  workspace: "./work"
  description: The scratch tree this run was handed.
agent.a:
  model: model.m
  prompt: Fix it.
  tools:
    - tool.plain
    - tool.described
  output:
    verdict: { enum: [approve, revise] }
flow.f:
  outputs: {}
  nodes:
    n: { agent: agent.a, input: "'x'" }
  edges:
    - { from: start, to: n }
    - { from: n, to: end }
"#,
    );
}

/// The `module:` binding in both of its spellings, and the shapes around it
/// whose rejecting half the negative corpus pins (grammar 6.1, Decisions D132,
/// D133).
///
/// The scalar form and the block form are one binding, so both are here; two
/// tools sharing a dependency **at the same version** is the accepting half of
/// D133's conflict rule, and a package the generated project pins, **at the
/// version it pins**, is the accepting half of the collision rule.
/// Over-rejecting either would make a legal composition unwritable and nothing
/// else in the suite would notice.
///
/// Nothing here needs the files to exist: whether an authored module is on disk
/// is `check_modules`'s question, and it is a pass of its own for a reason —
/// `build` scaffolds an absent one, and the refusal is pinned by
/// `tests/fixtures/invalid-check/module-binding-names-a-file-that-is-not-there`.
#[test]
fn module_bindings_in_both_forms_sharing_a_pinned_dependency() {
    accepts(
        "module-bindings",
        r#"
tool.sign:
  description: Sign a payload.
  input:
    payload: { type: string }
  output:
    signature: { type: string }
  module: ./src/tools/sign.ts
tool.verify:
  description: Verify a signature.
  input:
    signature: { type: string }
  output:
    ok: { type: boolean }
  module:
    path: ./src/tools/verify.ts
    env:
      SIGNING_KEY: "${SIGNING_KEY}"
    dependencies:
      "@noble/hashes": "1.4.0"
      zod: "4.4.3"
tool.reseal:
  description: Re-sign an envelope.
  input:
    envelope: { type: string }
  output:
    signature: { type: string }
  module:
    path: ./src/tools/reseal.ts
    dependencies:
      "@noble/hashes": "1.4.0"
agent.a:
  model: model.m
  prompt: Sign it.
  tools: [tool.sign]
  output:
    verdict: { enum: [approve, revise] }
flow.f:
  outputs: {}
  nodes:
    n: { agent: agent.a, input: "'x'" }
    check: { function: tool.verify, input: { signature: "'sig'" } }
  edges:
    - { from: start, to: n }
    - { from: n, to: check }
    - { from: check, to: end }
"#,
    );
}

/// **Every `access:` preset against every harness this release ships** — all
/// six pairings, written out.
///
/// The worked example takes two of the six: `cc` with `workspace_write` and
/// `codex` with `read_only`, because those are the two a sensible pipeline
/// wants. The other four are where a pairing goes wrong, and a preset means
/// what the *harness's own* primitive means (PRD resolved q57 ruling c fixes
/// per-harness statement, never implied equivalence), so a pairing nothing
/// writes is a pairing nothing compiles:
///
///  * `full_access` is the node asking for the *least* containment, so it is
///    where an adapter bound that quietly stops applying does the most damage —
///    and on `cc` it is the preset whose permission mode bypasses the callback,
///    which is why the pairing below also carries an `allow_tools:` list: the
///    bound the driver has to hold with no callback to lean on;
///  * `read_only` on `cc` is the reviewer's shape — a run that must read the
///    checkout and write nothing — and it is the pairing the example corpus
///    spells only under the *other* harness.
///
/// Two other legal shapes ride along, each a key the example corpus never
/// takes: `inherit_env: true` — q54 ruling b's explicit opt-in, and the only way
/// a run sees the process's own environment — and a coder node with neither
/// `input:` nor `allow_tools:` at all.
///
/// Each node's `model:` resolves through the provider kind its harness's
/// connection table speaks for — `anthropic` under `cc`, `openai` under `codex`
/// — which is why this case declares a second provider of its own: a slot is an
/// endpoint on one wire, and the pairing is checked (Decision D143, PRD resolved
/// q58 ruling b).
#[test]
fn a_coder_node_takes_every_containment_preset() {
    accepts(
        "coder-access-presets",
        r#"
provider.o:
  kind: openai
  api_key: ${OK}
model.o:
  provider: provider.o
  id: some-openai-model
flow.f:
  outputs: {}
  nodes:
    wide:
      coder:
        harness: cc
        model: model.m
        workspace: "'${ROOT}'"
        access: full_access
        prompt: Do the work.
        output:
          summary: { type: string }
        allow_tools: [Bash, Read]
        inherit_env: true
      input: "'go'"
    reading:
      coder:
        harness: cc
        model: model.m
        workspace: "'${ROOT}'"
        access: read_only
        prompt: Read the work and report on it.
        output:
          notes: { type: string }
        allow_tools: [Glob, Grep, Read]
      input: "'go'"
    narrow:
      coder:
        harness: codex
        model: model.o
        workspace: "'${ROOT}'"
        access: read_only
        prompt: Read the work.
        output:
          verdict: { enum: [approve, revise] }
      input: "'go'"
    broad:
      coder:
        harness: codex
        model: model.o
        workspace: "'${ROOT}'"
        access: full_access
        prompt: Do the work outside the sandbox.
        output:
          summary: { type: string }
      input: "'go'"
    writing:
      coder:
        harness: codex
        model: model.o
        workspace: "'${ROOT}'"
        access: workspace_write
        prompt: Edit inside the checkout.
        output:
          summary: { type: string }
      input: "'go'"
    middle:
      coder:
        harness: cc
        model: model.m
        workspace: "'${ROOT}'"
        prompt: Take the default preset.
        output:
          note: { type: string }
      input: "'go'"
  edges:
    - { from: start, to: wide }
    - { from: wide, to: reading }
    - { from: reading, to: narrow }
    - { from: narrow, to: broad }
    - { from: broad, to: writing }
    - { from: writing, to: middle }
    - { from: middle, to: end }
"#,
    );
}

/// The **accepting** half of PRD resolved q61's unwritable race (grammar 8.9,
/// Decision D147).
///
/// The negative corpus pins both refusals — a map-dispatched coder whose
/// `workspace:` reads nothing per-dispatch, and two concurrent coder nodes
/// writing one value. Over-rejection is what a negative corpus cannot see, and
/// this rule has three legal shapes it would be easy to refuse by accident:
///
///  * a fan-out whose dispatched coder takes the item's own path **whole**
///    (`workspace: "input.worktree"`), which is the repair the error names and
///    therefore the one shape that must never be refused;
///  * a fan-out whose coder reads an item-derived field **inside a larger
///    expression** (`"'${ROOT}/' + input.branch"`). The predicate is about what
///    the expression reads rather than about its shape, and an implementation
///    that matched the whole value against a binding would accept the first
///    shape and refuse this one;
///  * a fan-out whose map declares `max_concurrency: 1`, which is the second
///    repair: one run at a time is one run in the directory at a time, whatever
///    the expression says.
///
/// `workspace: fresh` rides along, since a keyword that stopped parsing — or an
/// emitter that could not write it — would fail here rather than at a golden.
#[test]
fn a_fan_out_over_a_coder_node_takes_a_directory_per_dispatch() {
    accepts(
        "coder-per-dispatch-workspaces",
        r#"
state:
  summary: { type: string, default: "" }
  summaries:
    type: array
    max_items: 4
    items: { type: string }
    reduce: append
    default: []
flow.carried:
  inputs:
    worktree: { type: string }
    branch: { type: string }
  outputs:
    summary: { type: string }
  nodes:
    build:
      coder:
        harness: cc
        model: model.m
        workspace: "input.worktree"
        prompt: Work in the checkout you were given.
        output:
          summary: { type: string }
      input: "'go'"
      writes: { summary: summary }
  edges:
    - { from: start, to: build }
    - { from: build, to: end }
flow.branched:
  inputs:
    worktree: { type: string }
    branch: { type: string }
  outputs:
    summary: { type: string }
  nodes:
    build:
      coder:
        harness: cc
        model: model.m
        workspace: "'${ROOT}/' + input.branch"
        prompt: Work under the root, in the directory this item's branch names.
        output:
          summary: { type: string }
      input: "'go'"
      writes: { summary: summary }
  edges:
    - { from: start, to: build }
    - { from: build, to: end }
flow.serial:
  inputs:
    worktree: { type: string }
    branch: { type: string }
  outputs:
    summary: { type: string }
  nodes:
    build:
      coder:
        harness: cc
        model: model.m
        workspace: "'${ROOT}'"
        prompt: Work in the one checkout, one dispatch at a time.
        output:
          summary: { type: string }
      input: "'go'"
      writes: { summary: summary }
  edges:
    - { from: start, to: build }
    - { from: build, to: end }
flow.main:
  inputs:
    worktrees:
      type: array
      max_items: 4
      items:
        type: object
        properties:
          worktree: { type: string }
          branch: { type: string }
  outputs:
    summaries:
      type: array
      max_items: 4
      items: { type: string }
  nodes:
    carried:
      map:
        over: input.worktrees
        as: task
        node: flow.carried
        max_concurrency: 4
        writes: { summary: summaries }
    branched:
      map:
        over: input.worktrees
        as: task
        node: flow.branched
        max_concurrency: 4
        writes: { summary: summaries }
    serial:
      map:
        over: input.worktrees
        as: task
        node: flow.serial
        max_concurrency: 1
        writes: { summary: summaries }
    provisioned:
      coder:
        harness: cc
        model: model.m
        workspace: fresh
        prompt: Work in the directory the runtime made.
        output:
          summary: { type: string }
      input: "'go'"
      writes: { summary: summary }
  edges:
    - { from: start, to: carried }
    - { from: carried, to: branched }
    - { from: branched, to: serial }
    - { from: serial, to: provisioned }
    - { from: provisioned, to: end }
"#,
    );
}
