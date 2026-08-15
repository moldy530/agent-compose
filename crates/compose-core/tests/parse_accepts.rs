//! Legal shapes the example corpus does not happen to use.
//!
//! `parse_examples.rs` proves the parser accepts the two worked projects; this
//! file proves it accepts the rest of the grammar. Over-rejection is the
//! failure mode a negative corpus cannot catch — a rule written slightly too
//! tight makes a legal spec unwritable, and nothing else in the suite would
//! notice.

use compose_core::ast::{Document, DocumentKind};
use compose_core::parse_str;

#[track_caller]
fn accepts(name: &str, source: &str) -> Document {
    let parsed = parse_str(source, name.to_string());
    assert!(
        parsed.diagnostics.is_empty(),
        "{name} should parse cleanly, got:\n{}",
        parsed
            .diagnostics
            .iter()
            .map(|d| format!("  {} [{}] {}", d.span, d.code, d.message))
            .collect::<Vec<_>>()
            .join("\n")
    );
    parsed
        .document
        .unwrap_or_else(|| panic!("{name} produced no document"))
}

#[test]
fn a_string_in_agent_bound_with_the_scalar_form() {
    accepts(
        "string-in.yml",
        r#"
agent.researcher:
  model: model.smart
  prompt: Write a draft.
  output:
    draft: { type: string }

flow.demo:
  outputs:
    draft: { type: string }
  nodes:
    write:
      agent: agent.researcher
      input: "input.goal"
  edges:
    - { from: start, to: write }
    - { from: write, to: end }
"#,
    );
}

#[test]
fn a_tool_with_a_host_function_binding_and_empty_schemas() {
    accepts(
        "function-tool.yml",
        r#"
tool.rerank:
  description: Rerank candidates in the host process.
  input: {}
  output: {}
  function:
    name: rerank_candidates
"#,
    );
}

#[test]
fn every_provider_kind_with_its_required_keys() {
    accepts(
        "providers.yml",
        r#"
provider.anthropic:
  kind: anthropic
  api_key: ${ANTHROPIC_API_KEY}
  headers:
    x-trace-id: "${TRACE_ID}"

provider.openai:
  kind: openai
  api_key: ${OPENAI_API_KEY}
  organization: acme

provider.local:
  kind: openai_compatible
  base_url: ${LOCAL_LLM_URL}

provider.azure:
  kind: azure_openai
  base_url: ${AZURE_ENDPOINT}
  api_key: ${AZURE_API_KEY}
  api_version: "2026-01-01"

provider.aws:
  kind: bedrock
  region: us-east-1
  access_key_id: ${AWS_ACCESS_KEY_ID}
  secret_access_key: ${AWS_SECRET_ACCESS_KEY}
  session_token: ${AWS_SESSION_TOKEN}

provider.gcp:
  kind: vertex
  project: acme-prod
  location: us-central1
  credentials_json: ${GOOGLE_CREDENTIALS_JSON}
"#,
    );
}

#[test]
fn a_model_route_without_route_on_and_a_direct_model_with_open_settings() {
    accepts(
        "models.yml",
        r#"
model.smart:
  provider: provider.anthropic
  id: claude-sonnet-4-6
  description: The default reasoning model.
  settings:
    max_tokens: 8000
    thinking: { budget_tokens: 4000 }
    plugin_specific_knob: [1, 2, 3]

model.default:
  route: [model.smart, model.fast]
"#,
    );
}

#[test]
fn every_store_kind_and_its_optional_keys() {
    accepts(
        "stores.yml",
        r#"
store.prefs:
  kind: kv
  scope: execution
  value_schema:
    theme: { type: string }
  backend: prefs_db
  agent_access: read

store.docs:
  kind: vector
  scope: global
  embed:
    model: text-embedding-3-small
    provider: provider.openai
    dimensions: 1536
  metadata_schema:
    source: { type: string }

store.artifacts:
  kind: blob
  scope: session
  metadata_schema:
    content_hash: { type: string }
"#,
    );
}

#[test]
fn every_store_op_with_its_parameter_row() {
    accepts(
        "store-ops.yml",
        r#"
flow.demo:
  outputs:
    done: { type: boolean }
  nodes:
    kv_get:    { store: store.prefs, op: get, key: "execution.session_key" }
    kv_set:    { store: store.prefs, op: set, key: "execution.session_key", value: { theme: "'dark'" } }
    kv_delete: { store: store.prefs, op: delete, key: "execution.session_key" }
    kv_list:   { store: store.prefs, op: list, prefix: "'user:'", limit: 100 }
    search:
      store: store.docs
      op: search
      query: "input.question"
      top_k: 5
      filter: { source: "'handbook'" }
    upsert:
      store: store.docs
      op: upsert
      key: "input.doc_id"
      value: "input.text"
      metadata: { source: "'upload'" }
    put:
      store: store.artifacts
      op: put
      key: "input.doc_id"
      value: "state.report"
      content_type: text/markdown
  edges:
    - { from: start, to: kv_get }
    - { from: kv_get, to: kv_set }
    - { from: kv_set, to: kv_delete }
    - { from: kv_delete, to: kv_list }
    - { from: kv_list, to: search }
    - { from: search, to: upsert }
    - { from: upsert, to: put }
    - { from: put, to: end }
"#,
    );
}

#[test]
fn a_detached_homogeneous_map_and_a_renamed_item_binding() {
    accepts(
        "map-detached.yml",
        r#"
flow.demo:
  outputs:
    done: { type: boolean }
  nodes:
    fan_out:
      map:
        over: plan.output.tasks
        as: task
        node: tool.notify
        input: { subject: "task.title" }
        max_concurrency: 16
        on_item_error: retry
        detach: true
      timeout: 30s
      on_error: skip
  edges:
    - { from: start, to: fan_out }
    - { from: fan_out, to: end }
"#,
    );
}

#[test]
fn a_human_node_without_a_timeout_and_a_subflow_that_inherits_context() {
    accepts(
        "human-and-subflow.yml",
        r#"
flow.demo:
  outputs:
    done: { type: boolean }
  nodes:
    approve:
      human:
        input:
          summary: { type: string }
        output:
          decision: { enum: [approve, reject] }
      writes: { decision: human_decision }
      on_error: { fallback: end }
    sub:
      flow: flow.enrich
      input: { report: "state.report" }
      context: inherit
      policy:
        retry: { max: 2, backoff: 1s, multiplier: 1.5, max_backoff: 10s, jitter: false }
        timeout: 45s
        on_error: skip
      timeout: 90s
  edges:
    - { from: start, to: approve }
    - { from: approve, to: sub }
    - { from: sub, to: end }
"#,
    );
}

#[test]
fn inline_exec_and_http_nodes_in_their_non_competing_shapes() {
    accepts(
        "inline-nodes.yml",
        r#"
flow.demo:
  outputs:
    done: { type: boolean }
  nodes:
    run:
      exec:
        command: run-checks
        args: ["--fast", "--json"]
        cwd: "${REPO_ROOT}/packages"
        env: { CI: "true", TOKEN: "${CI_TOKEN}" }
      input: { pattern: "state.filter" }
    fetch:
      http:
        method: GET
        url: "https://${API_HOST}/v1/status"
        headers: { authorization: "Bearer ${API_TOKEN}" }
        query: { verbose: "'1'" }
        expect_status: [200, 204]
        output:
          status: { type: integer }
          stderr: { type: string }
    post:
      http:
        method: POST
        url: "https://${API_HOST}/v1/tickets"
        query: { dry_run: "'false'" }
      input: { title: "state.summary" }
  edges:
    - { from: start, to: run }
    - { from: run, to: fetch }
    - { from: fetch, to: post }
    - { from: post, to: end }
"#,
    );
}

#[test]
fn a_bounded_cycle_with_an_escape_edge() {
    accepts(
        "cycle.yml",
        r#"
flow.demo:
  outputs:
    draft: { type: string }
  nodes:
    write:  { agent: agent.researcher }
    review: { agent: agent.reviewer }
  edges:
    - { from: start, to: write }
    - { from: write, to: review }
    - from: review
      to: write
      when: "review.output.verdict == 'revise'"
      max_iterations: 3
    - { from: review, to: end, else: true }
"#,
    );
}

#[test]
fn every_channel_form_with_its_reduce_policy() {
    accepts(
        "state.yml",
        r#"
state:
  draft:
    type: string
    default: ""
    reduce: last_wins
  patches:
    type: array
    items: { type: string }
    max_items: 100
    min_items: 0
    unique_items: true
    default: []
    reduce: append
  totals:
    type: object
    properties:
      fixed: { type: integer }
      skipped: { type: integer }
    optional: [skipped]
    default: {}
    reduce: merge
  verdict:
    enum: [approve, revise]
    default: approve
  finding:
    discriminator: kind
    variants:
      auto_fixable: { file: { type: string } }
      needs_human: { summary: { type: string } }
"#,
    );
}

#[test]
fn the_full_scalar_constraint_vocabulary() {
    accepts(
        "constraints.yml",
        r#"
tool.constrained:
  description: Exercises every scalar constraint.
  input:
    title: { type: string, min_length: 1, max_length: 200, pattern: "^[a-z]+$" }
    started_at: { type: string, format: date-time }
    confidence: { type: number, minimum: 0, maximum: 1, multiple_of: 0.01 }
    attempts: { type: integer, exclusive_minimum: 0, exclusive_maximum: 10 }
    urgent: { type: boolean, default: false }
  output:
    findings:
      type: array
      max_items: 50
      items:
        discriminator: kind
        variants:
          auto_fixable:
            file: { type: string, format: uri }
            nested:
              type: object
              properties:
                depth: { type: integer }
              optional: [depth]
          needs_human:
            severity: { enum: [low, high, critical] }
  exec:
    command: constrained
"#,
    );
}

#[test]
fn all_four_trigger_types_in_their_optional_shapes() {
    accepts(
        "triggers.yml",
        r#"
triggers:
  cli:
    type: manual
    flow: flow.demo
    session_key: "payload.session"
    description: The development entry point.
  sync_api:
    type: http
    flow: flow.demo
    path: /reviews/:id
    method: PUT
    respond: sync
    timeout: 30s
    input: { goal: "payload.body.goal" }
  nightly:
    type: schedule
    flow: flow.demo
    cron: "0 3 * * *"
    timezone: America/New_York
    input: { scope: "'full'" }
  ingest:
    type: event
    flow: flow.demo
    source: bug_reports
    dedupe_key: "payload.id"
"#,
    );
}

#[test]
fn a_deploy_file_with_every_section() {
    let document = accepts(
        "deploy/staging.yml",
        r#"
version: "0.1"

placements:
  agent.fixer:
    runtime: isolated
    network: none
    description: Sandboxed, no egress.
  flow.triage: { runtime: colocated }

storage_backends:
  defaults:
    kv: { provider: redis, url: "${REDIS_URL}" }
    vector: { provider: pgvector, dsn: "${PG_DSN}" }
    blob: { provider: s3, bucket: artifacts, region: us-east-1 }
  aliases:
    docs_db: { provider: chroma, url: "${CHROMA_URL}", index: project-docs }

event_sources:
  bug_reports:
    kind: sqs
    queue_url: "https://sqs.us-east-1.amazonaws.com/1/bug-reports"
    region: us-east-1
"#,
    );
    assert_eq!(document.kind(), DocumentKind::Deploy);
}

#[test]
fn the_defaults_section_and_every_error_policy_shape() {
    accepts(
        "policy.yml",
        r#"
defaults:
  retry: { max: 1, backoff: 2s }
  timeout: 90s
  on_error: fail

flow.demo:
  outputs:
    done: { type: boolean }
  nodes:
    a:
      agent: agent.one
      on_error: skip
    b:
      agent: agent.two
      on_error: { fallback: c }
      retry: { max: 10, backoff: 250ms, multiplier: 2, max_backoff: 1h, jitter: true }
    c:
      agent: agent.three
      on_error: { fallback: end }
  edges:
    - { from: start, to: a }
    - { from: a, to: b }
    - { from: b, to: c }
    - { from: c, to: end }
"#,
    );
}

#[test]
fn anchors_and_aliases_expand_before_spec_level_processing() {
    let document = accepts(
        "anchors.yml",
        r#"
agent.one: &shared
  model: model.smart
  prompt: Do the thing.
  output:
    verdict: { enum: [approve, revise] }

agent.two: *shared
"#,
    );
    let spec = document.as_spec().expect("a spec file");
    assert_eq!(spec.definitions.len(), 2);
}

#[test]
fn an_escaped_env_token_is_literal_text() {
    accepts(
        "escaped.yml",
        r#"
agent.one:
  model: model.smart
  prompt: "Write $${HOME} to explain the shell syntax."
  output:
    verdict: { enum: [approve, revise] }
"#,
    );
}
