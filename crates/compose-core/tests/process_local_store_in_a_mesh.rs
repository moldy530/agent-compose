//! Which stores a mesh may reach, in both directions (grammar 14.1 rule 5,
//! Decision D131, PRD resolved q45, `docs/distributed.md` §1, §9.1).
//!
//! The rule has a table, and a negative corpus can only pin the rows that
//! refuse. The rows that **accept** are the ones a one-token slip turns into an
//! over-rejection nothing else notices — a networked backend refused, or a store
//! only the hub opens refused, either of which makes the ordinary mesh
//! unwritable. The rows that **refuse** are the ones a slip turns into silence,
//! which is worse: two processes, two copies of one store, a read that comes
//! back empty, and a flow that carries on.
//!
//! Three axes cross here, and each gets its own section below:
//!
//! * **the backend** — which of the eleven storage providers §14.3 names opens
//!   in-process. The list is asserted provider by provider rather than as a
//!   set, because the criterion is "opened in-process" and a provider added
//!   later has to be classified rather than inherited;
//! * **the binder** — an agent's `stores:` (§11.5) and a flow's `store:` node
//!   (§11.4), each reached directly and through an attachment;
//! * **the reach** — the closure §9.1 fixes, which is what makes an *unplaced*
//!   agent's store refusable when a placed agent attaches the flow that reaches
//!   it, and what leaves a store only the hub opens alone.
//!
//! `tests/fixtures/invalid-check/process-local-store-*` pins the exact rendering
//! of three refusals. What is here is the table itself.

use std::fs;
use std::path::PathBuf;

use compose_core::codegen::env::{Partition, Process};
use compose_core::{Diagnostic, resolve_with_target};

/// The provider and model every case needs, and nothing more.
const BACKEND: &str = r#"version: "0.1"
provider.p:
  kind: anthropic
  api_key: ${K}
provider.embed:
  kind: openai
  api_key: ${EMBED_KEY}
model.m:
  provider: provider.p
  id: some-model
"#;

/// One `kv` store bound to the alias every deploy file below defines.
const KV_STORE: &str = r#"store.notes:
  kind: kv
  scope: global
  description: What has been filed.
  backend: notes_db
  value_schema:
    last: { type: string }
"#;

/// An agent that binds `store.notes`, attaching whatever a case wants attached.
fn filer(name: &str, attaches: &str) -> String {
    let tools = if attaches.is_empty() {
        String::new()
    } else {
        format!("  tools: [{attaches}]\n")
    };
    format!(
        r#"agent.{name}:
  model: model.m
  prompt: File what you are given.
  stores: [store.notes]
{tools}  input:
    text: {{ type: string }}
  output:
    filed: {{ type: string }}
"#
    )
}

/// An agent that binds nothing, attaching whatever a case wants attached.
fn plain(name: &str, attaches: &str) -> String {
    let tools = if attaches.is_empty() {
        String::new()
    } else {
        format!("  tools: [{attaches}]\n")
    };
    format!(
        r#"agent.{name}:
  model: model.m
  prompt: Answer the question.
{tools}  input:
    text: {{ type: string }}
  output:
    filed: {{ type: string }}
"#
    )
}

/// A flow with one `agent:` node, usable as a `tools:` entry and as an entry
/// point of its own.
fn calling(flow: &str, agent: &str) -> String {
    format!(
        r#"flow.{flow}:
  description: Hand one note to the filer.
  inputs:
    text: {{ type: string }}
  outputs: {{}}
  nodes:
    file:
      agent: agent.{agent}
      input:
        text: "input.text"
  edges:
    - {{ from: start, to: file }}
    - {{ from: file, to: end }}
"#
    )
}

/// A flow whose own `store:` node writes `store.notes` — the other binder.
const STORING_FLOW: &str = r#"flow.keep:
  description: Keep one note.
  inputs:
    text: { type: string }
  outputs: {}
  nodes:
    keep:
      store: store.notes
      op: set
      key: "input.text"
      value:
        last: "input.text"
  edges:
    - { from: start, to: keep }
    - { from: keep, to: end }
"#;

/// A deploy file with a hub, the placements a case declares, and one alias.
fn mesh(placements: &str, provider: &str) -> String {
    format!(
        r#"version: "0.1"

hub:
  join_token: ${{MESH_JOIN_TOKEN}}

placements:
{placements}
storage_backends:
  aliases:
    notes_db: {{ provider: {provider} }}
"#
    )
}

/// A scratch project of this test's own, cleaned out before use.
fn project(name: &str, spec: &str, deploy: Option<&str>) -> PathBuf {
    let directory = std::env::temp_dir()
        .join("agent-compose-process-local-store")
        .join(format!("{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&directory);
    fs::create_dir_all(directory.join("deploy")).expect("can create the project");
    fs::write(directory.join("main.yml"), spec).expect("can write the spec");
    if let Some(body) = deploy {
        fs::write(directory.join("deploy/mesh.yml"), body).expect("can write the deploy file");
    }
    directory
}

/// Resolve and check one case, under the target it names.
///
/// A resolver diagnostic is this harness failing rather than the rule firing, so
/// it is asserted away first: every case here is a composition that resolves.
#[track_caller]
fn diagnose(name: &str, spec: &str, deploy: Option<&str>, target: &str) -> Vec<Diagnostic> {
    let directory = project(name, spec, deploy);
    let resolution = resolve_with_target(directory.join("main.yml"), target);
    assert!(
        resolution.diagnostics.is_empty(),
        "`{name}` is meant to resolve; the harness is measuring the wrong pass:\n{}",
        render(&resolution.diagnostics)
    );
    let ir = resolution
        .ir
        .expect("a clean resolution produces an artifact");
    compose_core::check(&ir)
}

#[track_caller]
fn accepts(name: &str, spec: &str, deploy: &str) {
    let diagnostics = diagnose(name, spec, Some(deploy), "mesh");
    assert!(
        diagnostics.is_empty(),
        "`{name}` is a legal mesh and the checks refused it:\n{}",
        render(&diagnostics)
    );
}

/// Refuse, and refuse for this reason: exactly one diagnostic, under this code,
/// naming this store and this placement.
#[track_caller]
fn refuses(name: &str, spec: &str, deploy: &str, store: &str, placement: &str) {
    let diagnostics = diagnose(name, spec, Some(deploy), "mesh");
    refused(name, &diagnostics, store, placement);
}

#[track_caller]
fn refused(name: &str, diagnostics: &[Diagnostic], store: &str, placement: &str) {
    assert_eq!(
        diagnostics.len(),
        1,
        "`{name}` is meant to produce exactly the store refusal:\n{}",
        render(diagnostics)
    );
    assert_eq!(
        diagnostics[0].code.to_string(),
        "process-local-store",
        "`{name}` was refused under the wrong code: {}",
        diagnostics[0].message
    );
    assert!(
        diagnostics[0].message.contains(&format!("`{store}`")),
        "`{name}` does not name the store that forks: {}",
        diagnostics[0].message
    );
    assert!(
        diagnostics[0]
            .message
            .contains(&format!("placement `{placement}`")),
        "`{name}` does not name the placement that opens it: {}",
        diagnostics[0].message
    );
}

fn render(diagnostics: &[Diagnostic]) -> String {
    diagnostics
        .iter()
        .map(|diagnostic| {
            format!(
                "  {} at {}: {}",
                diagnostic.code, diagnostic.span, diagnostic.message
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

// ---------------------------------------------------------------------------
// The backend axis: which of §14.3's providers open in-process
// ---------------------------------------------------------------------------

/// The four backends that have no server in the middle, each refused.
///
/// Asserted provider by provider rather than over a set, because the criterion
/// is *opened in-process* and the vocabulary is open to additions: a provider
/// added later is classified by the same question, and this test is where
/// classifying it wrongly shows up.
#[test]
fn every_backend_the_reaching_process_opens_for_itself_is_refused() {
    for (provider, kind, extra) in [
        ("memory", "kv", ""),
        ("sqlite", "kv", ""),
        ("sqlite_vec", "vector", VECTOR_STORE),
        ("local_fs", "blob", BLOB_STORE),
    ] {
        let store = if extra.is_empty() { KV_STORE } else { extra };
        refuses(
            &format!("local-backend-{provider}"),
            &format!("{BACKEND}{store}{}", filer("archivist", "")),
            &mesh("  vault:\n    members: [agent.archivist]\n", provider),
            "store.notes",
            "vault",
        );
        assert_eq!(
            kind_of(provider),
            kind,
            "`{provider}` serves a different store kind than this case declares"
        );
    }
}

/// …and the seven that are a connection to something outside the process, each
/// accepted.
///
/// This is the over-rejection guard, and it is the half a negative corpus can
/// say nothing about: a rule that classified `redis` as local would make the
/// repair it recommends the next diagnostic.
#[test]
fn every_networked_backend_is_accepted_from_a_placement() {
    for (provider, extra) in [
        ("redis", ""),
        ("postgres", ""),
        ("chroma", VECTOR_STORE),
        ("pgvector", VECTOR_STORE),
        ("qdrant", VECTOR_STORE),
        ("s3", BLOB_STORE),
        ("gcs", BLOB_STORE),
    ] {
        let store = if extra.is_empty() { KV_STORE } else { extra };
        accepts(
            &format!("networked-backend-{provider}"),
            &format!("{BACKEND}{store}{}", filer("archivist", "")),
            &mesh("  vault:\n    members: [agent.archivist]\n", provider),
        );
    }
}

/// A `vector` store, for the providers that serve one.
const VECTOR_STORE: &str = r#"store.notes:
  kind: vector
  scope: global
  description: What has been filed.
  backend: notes_db
  embed:
    model: text-embedding-3-small
    provider: provider.embed
"#;

/// A `blob` store, for the providers that serve one.
const BLOB_STORE: &str = r#"store.notes:
  kind: blob
  scope: global
  description: What has been filed.
  backend: notes_db
"#;

/// Which store kind a provider serves, read off the compiler's own vocabulary
/// so the cases above cannot declare a pairing §14.3 refuses.
fn kind_of(provider: &str) -> &'static str {
    compose_core::ast::deploy::BackendProvider::ALL
        .iter()
        .find(|held| held.as_str() == provider)
        .expect("the case names a provider grammar 14.3 has")
        .kind()
        .as_str()
}

// ---------------------------------------------------------------------------
// The reach axis: the closure, not `members:`
// ---------------------------------------------------------------------------

/// A store only the **hub** ever opens is untouched, placements or not.
///
/// The over-rejection guard for the whole rule: a mesh may keep every local
/// store it likes, as long as nothing a worker runs reaches one. A rule written
/// over "the target declares placements" rather than over the closure would
/// refuse this and make a local store unusable the moment a project grew a
/// placement anywhere.
#[test]
fn a_store_only_the_hub_opens_is_untouched_by_a_placement_elsewhere() {
    accepts(
        "hub-only",
        &format!(
            "{BACKEND}{KV_STORE}{}{}{}",
            filer("archivist", ""),
            plain("signer", ""),
            calling("archive", "archivist")
        ),
        &mesh("  mac:\n    members: [agent.signer]\n", "sqlite"),
    );
}

/// A placement's own member binding the store is the direct row.
#[test]
fn a_placed_agent_binding_a_local_store_is_refused() {
    refuses(
        "direct",
        &format!("{BACKEND}{KV_STORE}{}", filer("archivist", "")),
        &mesh("  vault:\n    members: [agent.archivist]\n", "sqlite"),
        "store.notes",
        "vault",
    );
}

/// An **unplaced** agent binding the store is refused when a placed agent
/// attaches the flow that reaches it.
///
/// The row that makes this a rule about execution rather than about `members:`.
/// `agent.archivist` is in no placement and the deploy file never names it; what
/// puts it in the worker's process is the tool loop of the agent that does
/// (grammar 14.1 rule 4).
#[test]
fn a_store_an_attached_flow_reaches_is_refused() {
    refuses(
        "through-a-flow",
        &format!(
            "{BACKEND}{KV_STORE}{}{}{}",
            filer("archivist", ""),
            calling("archive", "archivist"),
            plain("signer", "flow.archive")
        ),
        &mesh("  mac:\n    members: [agent.signer]\n", "sqlite"),
        "store.notes",
        "mac",
    );
}

/// The same flow reached by a `flow:` **node** instead is untouched.
///
/// The other direction of grammar 14.1 rule 4, and the reason the closure is
/// the right thing to read: the hub schedules the nodes of an instance a
/// `flow:` node starts, so `agent.archivist` is dispatched by the hub and the
/// store has one process after all.
#[test]
fn a_store_reached_through_a_flow_node_is_untouched() {
    let spec = format!(
        r#"{BACKEND}{KV_STORE}{}{}{}flow.release:
  inputs:
    text: {{ type: string }}
  outputs: {{}}
  nodes:
    archive:
      flow: flow.archive
      input:
        text: "input.text"
    sign:
      agent: agent.signer
      input:
        text: "input.text"
  edges:
    - {{ from: start, to: archive }}
    - {{ from: archive, to: sign }}
    - {{ from: sign, to: end }}
"#,
        filer("archivist", ""),
        calling("archive", "archivist"),
        plain("signer", "")
    );
    accepts(
        "flow-node",
        &spec,
        &mesh("  mac:\n    members: [agent.signer]\n", "sqlite"),
    );
}

/// A flow's own `store:` node is the second binder, and it is refused when the
/// flow runs in a worker's process.
#[test]
fn a_store_node_of_an_attached_flow_is_refused() {
    refuses(
        "store-node",
        &format!(
            "{BACKEND}{KV_STORE}{STORING_FLOW}{}",
            plain("signer", "flow.keep")
        ),
        &mesh("  mac:\n    members: [agent.signer]\n", "sqlite"),
        "store.notes",
        "mac",
    );
}

/// One store reached from two placements is one fault with one repair, so it is
/// reported once.
///
/// A report per placement would put two diagnostics with one fix in front of an
/// author, and fixing either would clear both.
#[test]
fn a_store_two_placements_reach_is_one_diagnostic() {
    let diagnostics = diagnose(
        "two-placements",
        &format!(
            "{BACKEND}{KV_STORE}{}{}",
            filer("left", ""),
            filer("right", "")
        ),
        Some(&mesh(
            "  one:\n    members: [agent.left]\n  two:\n    members: [agent.right]\n",
            "sqlite",
        )),
        "mesh",
    );
    refused("two-placements", &diagnostics, "store.notes", "one");
}

/// Two **stores** on local backends are two faults, and get two diagnostics.
///
/// The dedupe above is on the store, not on the pass: an author holding two
/// genuinely separate problems is told about both.
#[test]
fn two_local_stores_a_placement_reaches_are_two_diagnostics() {
    let spec = format!(
        r#"{BACKEND}{KV_STORE}store.audit:
  kind: kv
  scope: global
  description: What has been audited.
  backend: notes_db
  value_schema:
    last: {{ type: string }}
agent.archivist:
  model: model.m
  prompt: File what you are given.
  stores: [store.notes, store.audit]
  input:
    text: {{ type: string }}
  output:
    filed: {{ type: string }}
"#
    );
    let diagnostics = diagnose(
        "two-stores",
        &spec,
        Some(&mesh(
            "  vault:\n    members: [agent.archivist]\n",
            "sqlite",
        )),
        "mesh",
    );
    assert_eq!(
        diagnostics.len(),
        2,
        "two stores fork and two are reported:\n{}",
        render(&diagnostics)
    );
    let named: Vec<&str> = diagnostics
        .iter()
        .map(|diagnostic| diagnostic.message.as_str())
        .collect();
    // Source order, which is how every report reads: the `stores:` entry that
    // binds `store.notes` is written before the one that binds `store.audit`.
    assert!(named[0].contains("`store.notes`"), "{named:?}");
    assert!(named[1].contains("`store.audit`"), "{named:?}");
}

// ---------------------------------------------------------------------------
// The scope axis, and the target axis
// ---------------------------------------------------------------------------

/// Every `scope:` refuses, `execution` included.
///
/// `scope: execution` is the exemption an author would expect and the rule does
/// not grant: an execution spans processes the moment one of its nodes is
/// dispatched to a worker, so the fork is the same fork one node later. A rule
/// that exempted it would be a rule that fires on the shapes least likely to
/// matter and stays quiet on the one a mesh actually writes.
#[test]
fn every_scope_refuses_including_execution() {
    for scope in ["execution", "session", "global"] {
        let store = format!(
            "store.notes:\n  kind: kv\n  scope: {scope}\n  description: What has been filed.\n  \
             backend: notes_db\n  value_schema:\n    last: {{ type: string }}\n"
        );
        refuses(
            &format!("scope-{scope}"),
            &format!("{BACKEND}{store}{}", filer("archivist", "")),
            &mesh("  vault:\n    members: [agent.archivist]\n", "sqlite"),
            "store.notes",
            "vault",
        );
    }
}

/// `--target local` refuses too, and says the repair that target admits.
///
/// `local` substitutes local storage for **every** store unconditionally, so a
/// `local` mesh — which grammar §14 admits, and which is how a mesh is developed
/// — reaches a forked store by construction. The help is target-dependent for
/// the same reason: `local` refuses a `storage_backends:` block (D87), so
/// offering one would send an author to a key the next compile rejects.
#[test]
fn the_local_target_refuses_and_names_the_repair_it_admits() {
    let deploy = "version: \"0.1\"\n\nhub:\n  join_token: ${MESH_JOIN_TOKEN}\n\n\
                  placements:\n  vault:\n    members: [agent.archivist]\n";
    let directory = project(
        "local-target",
        &format!(
            "{BACKEND}store.notes:\n  kind: kv\n  scope: global\n  description: Filed.\n  \
             value_schema:\n    last: {{ type: string }}\n{}",
            filer("archivist", "")
        ),
        None,
    );
    fs::write(directory.join("deploy/local.yml"), deploy).expect("can write the deploy file");
    let resolution = resolve_with_target(directory.join("main.yml"), "local");
    assert!(
        resolution.diagnostics.is_empty(),
        "the harness is measuring the wrong pass:\n{}",
        render(&resolution.diagnostics)
    );
    let diagnostics = compose_core::check(&resolution.ir.expect("an artifact"));
    refused("local-target", &diagnostics, "store.notes", "vault");
    let help = diagnostics[0]
        .help
        .as_deref()
        .expect("the refusal offers a repair");
    assert!(
        help.contains("admits no `storage_backends:`"),
        "under `local` the repair is a target of its own, not a key `local` refuses: {help}"
    );
    assert!(
        !help.contains("bind `redis`"),
        "the named-target repair must not be offered under `local`: {help}"
    );
}

/// Both repairs are offered, and the message says which one this release runs.
///
/// The rule is ratified and correct, and the backend half of its repair is the
/// deployment the design is heading for — but a build of *this* release opens
/// only the process-local backends: `src/stores.ts` throws on every other
/// provider, because production `storage_backends` land behind the store plugin
/// interface in M3 (PRD §7). An author who followed a message that stopped at
/// "bind `redis`" would write a deploy file that validates, builds, and throws
/// at the first `store set`, which is a worse outcome than the one they came in
/// with. So the message names the repair that runs today as well, and this is
/// the test that fails when the release grows the backends and the caveat is
/// left behind: a `redis` store that no longer throws makes the sentence below
/// wrong, and the fixtures' exact `# help:` lines come with it.
///
/// **The order the two are named in is pinned as well**, because
/// `src/docs/codes/process-local-store.md` tells a reader what it is — the
/// document leads with the repair a build runs and says so, and says the
/// diagnostic leads with the other. Nothing else holds those two texts together,
/// and a reader who checks one against the other is exactly the reader the
/// `explain` document is for.
#[test]
fn the_repair_names_the_half_a_build_of_this_release_can_run() {
    let caveat = "refuses at the first store op";
    let diagnostics = diagnose(
        "release-caveat",
        &format!("{BACKEND}{KV_STORE}{}", filer("archivist", "")),
        Some(&mesh(
            "  vault:\n    members: [agent.archivist]\n",
            "sqlite",
        )),
        "mesh",
    );
    refused("release-caveat", &diagnostics, "store.notes", "vault");
    let named = diagnostics[0]
        .help
        .clone()
        .expect("the refusal offers a repair");
    assert!(
        named.contains("bind `redis` or `postgres` instead"),
        "the design's repair is not offered under a named target: {named}"
    );
    assert!(
        named.contains(caveat) && named.contains("M3"),
        "the message offers a networked backend without saying that this release refuses one at \
         run time, so an author who takes it gets a project that compiles and throws: {named}"
    );
    assert!(
        named.contains(
            "taking the component out of `placements:` is the repair a build of this \
                        release runs"
        ),
        "the message never names the repair this release can actually run: {named}"
    );

    // **The order of the two, which the `explain` document describes.** The
    // networked backend is named first because it is the shape the deployment is
    // heading for, and the caveat is last because it is what a reader has to
    // leave with. `src/docs/codes/process-local-store.md` states that ordering
    // and contrasts it with its own, so a message reordered here without the
    // document is a document that describes another compiler's output.
    let backend = named
        .find("bind `redis` or `postgres` instead")
        .expect("the design's repair is offered");
    let composition = named
        .find("take the component that binds it out of `placements:`")
        .expect("the repair a build runs is offered");
    let last = named.find(caveat).expect("the caveat is offered");
    assert!(
        backend < composition && composition < last,
        "the two repairs are no longer named backend-first with the caveat last, which is the \
         order `agent-compose explain process-local-store` tells a reader to expect: {named}"
    );

    // …and under `local`, where the first repair is a target of its own rather
    // than a key, the same caveat: a `deploy/<target>.yml` naming `s3` is a
    // build this release refuses at the first blob op just the same.
    let directory = project(
        "release-caveat-local",
        &format!(
            "{BACKEND}store.notes:\n  kind: kv\n  scope: global\n  description: Filed.\n  \
             value_schema:\n    last: {{ type: string }}\n{}",
            filer("archivist", "")
        ),
        None,
    );
    fs::write(
        directory.join("deploy/local.yml"),
        "version: \"0.1\"\n\nhub:\n  join_token: ${MESH_JOIN_TOKEN}\n\n\
         placements:\n  vault:\n    members: [agent.archivist]\n",
    )
    .expect("can write the deploy file");
    let resolution = resolve_with_target(directory.join("main.yml"), "local");
    let diagnostics = compose_core::check(&resolution.ir.expect("an artifact"));
    refused("release-caveat-local", &diagnostics, "store.notes", "vault");
    let local = diagnostics[0]
        .help
        .as_deref()
        .expect("the refusal offers a repair");
    assert!(
        local.contains(caveat),
        "the `local` message sends an author to a target of its own without saying this release \
         would refuse the backend it names: {local}"
    );
}

/// A composition that places nothing keeps every local store it has.
///
/// The floor: this is what every project without a deploy layer is, and the
/// rule may not cost it anything.
#[test]
fn a_composition_that_places_nothing_keeps_its_local_stores() {
    let directory = project(
        "no-placements",
        &format!(
            "{BACKEND}store.notes:\n  kind: kv\n  scope: global\n  description: Filed.\n  \
             value_schema:\n    last: {{ type: string }}\n{}",
            filer("archivist", "")
        ),
        None,
    );
    let resolution = resolve_with_target(directory.join("main.yml"), "local");
    let diagnostics = compose_core::check(&resolution.ir.expect("an artifact"));
    assert!(
        diagnostics.is_empty(),
        "a single-process project may keep a local store:\n{}",
        render(&diagnostics)
    );
}

// ---------------------------------------------------------------------------
// The closure this rule and the manifest share
// ---------------------------------------------------------------------------

/// The refusal and the environment manifest read **one** answer.
///
/// `docs/distributed.md` §9.1 partitions `${ENV}` references by which process
/// can execute each surface, and this rule asks that same question of a store.
/// The property is what makes sharing the walk observable: for every store, the
/// rule refuses exactly when the partition puts that store in some placement's
/// process. A second derivation could satisfy every case above and still
/// disagree here — a store the manifest routes a credential to and the rule
/// leaves alone is a worker holding a key for a store it must not have opened.
#[test]
fn the_refusal_and_the_manifest_agree_about_every_store() {
    let spec = format!(
        "{BACKEND}{KV_STORE}{STORING_FLOW}{}{}{}",
        filer("archivist", ""),
        calling("archive", "archivist"),
        plain("signer", "flow.archive")
    );
    for (case, placements) in [
        ("none-placed", "  idle:\n    members: [agent.archivist]\n"),
        ("attacher-placed", "  mac:\n    members: [agent.signer]\n"),
    ] {
        let directory = project(
            &format!("agreement-{case}"),
            &spec,
            Some(&mesh(placements, "sqlite")),
        );
        let resolution = resolve_with_target(directory.join("main.yml"), "mesh");
        assert!(
            resolution.diagnostics.is_empty(),
            "`{case}` is meant to resolve:\n{}",
            render(&resolution.diagnostics)
        );
        let ir = resolution.ir.expect("an artifact");
        let partition = Partition::of(&ir);
        let in_a_worker = partition
            .processes_of("store.notes")
            .any(|process| matches!(process, Process::Placement(_)));
        let refused = compose_core::check(&ir)
            .iter()
            .any(|diagnostic| diagnostic.code.to_string() == "process-local-store");
        assert_eq!(
            refused, in_a_worker,
            "`{case}`: the rule and the partition disagree about `store.notes`"
        );
        // …and the case is worth having only because the partition really does
        // put the store in a worker here.
        assert!(in_a_worker, "`{case}` does not exercise the closure at all");
    }
}

/// The route the diagnostic draws is the route the partition took.
///
/// The labels are not decoration: they are the chain that answers "why is this
/// store in a worker's process at all", and an author who cannot see it is left
/// with a verdict. The chain starts at the `members:` entry that holds the
/// placement and ends at the binding — here `agent.signer` in `mac`, its
/// `tools:` entry, the `agent:` node inside the flow, and the `stores:` entry
/// that opens the store.
#[test]
fn the_refusal_draws_the_chain_from_the_members_entry_to_the_binding() {
    let diagnostics = diagnose(
        "chain",
        &format!(
            "{BACKEND}{KV_STORE}{}{}{}",
            filer("archivist", ""),
            calling("archive", "archivist"),
            plain("signer", "flow.archive")
        ),
        Some(&mesh("  mac:\n    members: [agent.signer]\n", "sqlite")),
        "mesh",
    );
    assert_eq!(diagnostics.len(), 1, "{}", render(&diagnostics));
    let labels: Vec<&str> = diagnostics[0]
        .labels
        .iter()
        .map(|label| label.message.as_str())
        .collect();
    assert_eq!(
        labels,
        [
            "`agent.signer` reaches `flow.archive` here",
            "`flow.archive` reaches `agent.archivist` here",
            "`agent.signer` is a member of placement `mac`",
        ],
        "the chain is drawn from the placement inward, and ends at the entry that placed it"
    );
    assert!(
        diagnostics[0].span.source.as_str().ends_with("main.yml"),
        "the primary span is the binding, which is in the composition"
    );
    assert!(
        diagnostics[0].labels[2]
            .span
            .source
            .as_str()
            .ends_with("mesh.yml"),
        "the placement label is in the deploy file that wrote it"
    );
}
