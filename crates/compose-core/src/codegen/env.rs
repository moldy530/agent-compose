//! `src/env.ts`: every `${ENV}` reference, and `readEnvironment()`, the presence
//! check over them (PRD 5.9, grammar 4.3).
//!
//! # Who runs the check
//!
//! **`src/index.ts` does**, at module scope — see [`super::project`]. Loading the
//! generated project resolves every reference or throws naming all the missing
//! ones, which is PRD 5.9's "resolution happens at process start in generated
//! code, never at compile" and PRD §7 M1's "env-ref presence checks at process
//! start". This module emits the check; the barrel calls it; nothing here reads
//! `process.env` at compile time.
//!
//! **`agent-compose build` does not.** Env refs survive *unresolved* into the IR
//! and into generated code, which is what keeps the artifact committable and
//! keeps a key out of every file this compiler writes. PRD 5.9 says so outright
//! — "`validate` and `build` check ref syntax only … Presence is a launch-time
//! check" — and grammar 4.3 agrees, substituting an interpolated ref "at process
//! start". An earlier draft of 5.9 had put `build` in the presence-checking
//! list; PRD §9's resolved question 15 settles it the other way, on
//! artifact portability: a build whose success depended on the building
//! machine's environment could not be reproduced on the deploying one, an
//! artifact has to build on a CI box holding no secrets, and `build --check` is
//! a CI diff (PRD §8) rather than a deployment. The least-privilege
//! distribution of PRD 5.10 needs the same thing — a deployment receives the
//! variables its own resolved surfaces name, which is computable only because
//! nothing upstream resolved them.
//!
//! Both halves are pinned by name rather than left to be inferred from an
//! absence of tests: `agent-compose`'s
//! `tests/build_cli.rs::build_emits_with_every_environment_reference_unset`
//! decides that `build` emits with every referenced variable unset, and
//! `tests/generated_code_gates.rs::the_generated_project_checks_its_environment_when_it_is_loaded`
//! decides that loading the emitted project refuses, naming each one. The
//! generated `README.md` says which command does it, so a reader of the emitted
//! project is not left to assume either.
//!
//! # What the module gives the rest of the project
//!
//! * `environmentReferences` — every name the composition references, sorted,
//!   each with the surfaces that wrote it. That list is also PRD 5.9's
//!   least-privilege input: the set of variables an isolated deployment needs is
//!   computable from it, statically, because every secret is a named ref.
//! * `readEnvironment()` — resolves all of them at once and throws naming
//!   **every** missing variable rather than the first, because a deploy that is
//!   missing three keys should learn that in one run.
//!
//! Presence is `!== undefined`: a variable that is set to the empty string is
//! set. `process.env` cannot tell an empty assignment from an intentional one,
//! and a compiler that guessed would refuse a legitimately empty value with a
//! message about it being absent.

use std::collections::{BTreeMap, BTreeSet, btree_map::Entry};

use crate::ast::common::{Interpolated, Namespace};
use crate::ast::deploy::PluginValue;
use crate::diag::Span;
use crate::ir::Ir;
use crate::ir::binding::{Exec, Http, InterpolatedEntry};
use crate::ir::definition::{DefinitionBody, Model};
use crate::ir::flow::{NodeKind, ToolImplementation};
use crate::ir::trigger::{InboundAuth, TriggerKind};

use super::names;

/// One **process** of a deployment: the hub, or a named placement
/// (`docs/distributed.md` §9.1).
///
/// The hub is not a placement and never becomes one — "a component in no
/// placement executes on the hub" is grammar §14.1's default rather than an
/// entry an author writes — so it is a variant rather than a reserved name a
/// composition could collide with.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Process {
    /// The process that owns the graph.
    Hub,
    /// The process a worker claiming this placement name runs.
    Placement(String),
}

impl Process {
    /// This process's name, as a manifest and a refusal spell it.
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Self::Hub => "hub",
            Self::Placement(name) => name.as_str(),
        }
    }
}

/// The synthetic owner every deploy-layer and trigger reference is filed under.
///
/// Not an address, and deliberately unspellable as one: grammar §2.1's addresses
/// are `<namespace>.<identifier>`, so nothing a composition declares can collide
/// with it.
const DEPLOY_OWNER: &str = "deploy:";

/// The owner key one `storage_backends:` entry's references are filed under.
///
/// A key of its own rather than [`DEPLOY_OWNER`], because a backend is the one
/// deploy-layer surface a **placement** can also reach: a store an agent
/// attaches is opened in that agent's process, so the credential that opens it
/// is spent there. §9.1's fifth clause still holds — a deploy-layer variable is
/// the hub's — and this only ever *adds* processes, never moves one off the
/// hub's list, exactly as the unplaced-agent clause does for a component.
///
/// `site` is the spec address `References::backend` records the reference under,
/// so the two cannot name different entries.
fn backend_owner(site: &str) -> String {
    format!("{DEPLOY_OWNER}{site}")
}

/// The owner key the `package_registry:` credentials are filed under
/// (grammar 14.6, PRD resolved q59).
///
/// A key of its own for [`backend_owner`]'s reason, arrived at from the other
/// end: a registry token is spent by whichever process runs `bun install`, and
/// every process does. The hub installs the project it serves, and a worker
/// installs the artifact it materialises (`docs/distributed.md` §4 step 4) — so
/// this owner runs in **every** process of the deployment, which is the widest
/// membership any surface has and the honest one. §9.1's fifth clause holds as
/// it does for a backend: the hub keeps it, and the placements are added.
const REGISTRY_OWNER: &str = "deploy:package_registry";

/// Which processes each reference-holding surface can execute in
/// (`docs/distributed.md` §9.1).
///
/// # Why the answer is a *set* rather than a place
///
/// §9.1's rule is "a process's manifest is the variables referenced by every
/// component that **can execute in it**", and the same component can execute in
/// two: an unplaced `tool.*` two placed agents attach runs on both their
/// workers, and an unplaced `agent.*` a placed agent reaches through an attached
/// `flow.*` runs on that worker **and** on the hub, because every flow a
/// composition declares is startable on the hub (grammar Decision D64). The two
/// worked examples §9.1 closes with are those two directions, and
/// [`the_keychain_password_of_a_placed_agents_attached_tool_is_that_placements_alone`]
/// and [`a_key_an_attached_flow_reaches_belongs_to_the_worker_and_to_the_hub`]
/// are them.
///
/// # What an owner is
///
/// The unit is the surface that **holds** the `${ENV}`, which is a definition
/// address for a provider, a tool and an agent, the *flow's* address for the
/// `exec:`/`http:` node bindings inside it — every node of one flow runs
/// wherever that flow's instance runs — and [`DEPLOY_OWNER`] for the trigger
/// table and the deploy layer, which are the hub's by §9.1's fifth clause.
///
/// One deploy-layer surface has an owner of its own, [`backend_owner`]: a
/// `storage_backends:` entry is the hub's like the rest of that layer, **and**
/// every placement whose components open a store bound to it, because the
/// credential that opens a store is spent in the process that opens it. The
/// fifth clause is not weakened by that — nothing moves off the hub's list —
/// exactly as being reached from a placed agent's loop adds a placement to an
/// unplaced agent's variables without moving them.
///
/// # Why the answer records *how* it got there
///
/// Two passes read this partition and each needs a different half of one walk.
/// [`References::for_process`] needs the membership — which variables a process
/// holds — and `check::placements` needs the **route**: grammar 14.1 rule 5
/// refuses a process-local store reached from a placement, and a diagnostic
/// that named only the verdict would leave an author with "this store forks"
/// and no line to look at. So every arrival records the edge that carried it,
/// and [`Partition::route`] walks those edges back to the `members:` entry that
/// started them.
///
/// The alternative — deciding the refusal here and rebuilding the chain in the
/// check — is the one shape this type exists to rule out: two derivations of one
/// closure agree on the day they are written, and a diagnostic pointing at a
/// path the partition did not take is worse than none.
#[derive(Debug, Default)]
pub struct Partition {
    /// Owner key to the processes it can execute in, each with the edge that
    /// carried it there.
    runs: BTreeMap<String, BTreeMap<Process, Carried>>,
    /// Every process this deployment has, hub first.
    processes: Vec<Process>,
}

/// How one surface came to run in one process: the edge that carried it there.
#[derive(Clone, Debug)]
struct Carried {
    /// The owner it was carried from — absent where the process is the
    /// surface's own: a `members:` entry, or a component the hub dispatches.
    from: Option<String>,
    /// Where that edge is written. Absent where no line of any file wrote it —
    /// the hub-dispatchable default of §9.1's second clause, which is a rule
    /// rather than a reference.
    site: Option<Span>,
}

/// One surface on the way from a placement's `members:` entry to a component it
/// reaches ([`Partition::route`]).
#[derive(Clone, Copy, Debug)]
pub struct Step<'a> {
    /// The surface this step reached.
    pub owner: &'a str,
    /// Where the edge that reached it is written — the `members:` entry for the
    /// first step, and the attachment, node or list entry for every one after.
    pub site: Option<&'a Span>,
}

impl Partition {
    /// Compute the partition for one artifact.
    #[must_use]
    pub fn of(ir: &Ir) -> Self {
        let claims = claims(ir);
        let mut runs: BTreeMap<String, BTreeMap<Process, Carried>> = BTreeMap::new();
        let mut processes = vec![Process::Hub];
        if let Some(section) = ir.deploy.placements.as_ref() {
            for placement in section.entries.values() {
                processes.push(Process::Placement(placement.name.value.to_string()));
            }
        }

        /// Record that `owner` runs in `process`, reached by `how`.
        ///
        /// The **first** arrival is the one kept, which is what makes a route
        /// deterministic: the fixed point below walks the definitions in address
        /// order, so the edge recorded is the first one that reached this owner
        /// and not whichever the last round happened to try.
        fn arrive(
            runs: &mut BTreeMap<String, BTreeMap<Process, Carried>>,
            owner: &str,
            process: Process,
            how: Carried,
        ) -> bool {
            match runs.entry(owner.to_string()).or_default().entry(process) {
                Entry::Vacant(slot) => {
                    slot.insert(how);
                    true
                }
                Entry::Occupied(_) => false,
            }
        }

        /// A process that is this surface's own rather than one carried into it.
        fn own(site: Option<&Span>) -> Carried {
            Carried {
                from: None,
                site: site.cloned(),
            }
        }

        // The deploy layer and the trigger table are the hub's, always
        // (§9.1's fifth clause).
        arrive(&mut runs, DEPLOY_OWNER, Process::Hub, own(None));
        // …and the installer configuration is **every** process's, because
        // every process installs: the hub installs the project it serves, and a
        // worker runs `bun install` over the artifact it just materialised
        // (`docs/distributed.md` §4 step 4, grammar 14.6). Like the backend
        // clause below, this only ever adds processes.
        if let Some(registry) = ir.deploy.package_registry.as_ref() {
            for process in &processes {
                arrive(
                    &mut runs,
                    REGISTRY_OWNER,
                    process.clone(),
                    own(Some(&registry.span)),
                );
            }
        }
        // …and each `storage_backends:` entry is the hub's too, unconditionally
        // and for the same clause. The pass below **adds** the placements whose
        // components open a store bound to it; nothing takes it off the hub's
        // list, so an entry no store binds keeps the membership it always had.
        if let Some(backends) = ir.deploy.storage_backends.as_ref() {
            for (kind, config) in &backends.defaults {
                let owner = backend_owner(&format!("deploy.storage_backends.defaults.{kind}"));
                arrive(&mut runs, &owner, Process::Hub, own(Some(&config.span)));
            }
            for (alias, config) in &backends.aliases {
                let owner = backend_owner(&format!("deploy.storage_backends.aliases.{alias}"));
                arrive(&mut runs, &owner, Process::Hub, own(Some(&config.span)));
            }
        }

        // Membership — §9.1's first clause — and unconditional hub
        // dispatchability, its second.
        for (address, claim) in &claims {
            arrive(
                &mut runs,
                address,
                Process::Placement(claim.placement.clone()),
                own(Some(&claim.at)),
            );
        }
        for (address, definition) in &ir.definitions {
            match &definition.body {
                // Every flow a composition declares is startable on the hub
                // (grammar Decision D64), so its own nodes' bindings are the
                // hub's whatever else reaches them.
                DefinitionBody::Flow(_) => {
                    arrive(&mut runs, address, Process::Hub, own(None));
                }
                // …and so is every unplaced agent, for the same reason read one
                // level in: an `agent:` node of some flow is a node the hub's
                // scheduler starts.
                DefinitionBody::Agent(_) if !claims.contains_key(address) => {
                    arrive(&mut runs, address, Process::Hub, own(None));
                }
                _ => {}
            }
        }
        // An unplaced `tool.*` the hub itself dispatches: the one a `function:`
        // node names (grammar §8.4) and the one a `map` dispatches to
        // (grammar §8.6). A **placed** one is dispatched to its placement
        // instead, which is what placing it means, so it is left alone.
        for definition in ir.definitions.values() {
            let DefinitionBody::Flow(flow) = &definition.body else {
                continue;
            };
            for node in &flow.nodes {
                let mut hub_dispatched = |address: &crate::ast::common::Address| {
                    let key = address.to_string();
                    if !claims.contains_key(&key) {
                        arrive(&mut runs, &key, Process::Hub, own(None));
                    }
                };
                match &node.kind {
                    NodeKind::Function { function } => hub_dispatched(&function.value),
                    NodeKind::Map { map } => {
                        for target in crate::check::reach::targets(&map.dispatch) {
                            if matches!(target.value.namespace, Namespace::Agent | Namespace::Tool)
                            {
                                hub_dispatched(&target.value);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        // …then the closures, to a fixed point. Two edges carry a process
        // inward: what an agent **attaches** runs in the agent's own tool loop
        // (grammar §5.4, §14.1 rule 4), and what a flow's own nodes name runs
        // in the process running that flow's instance — except where the thing
        // named is **placed**, which is the hub handing it to a worker rather
        // than running it (grammar §14.1). A provider is neither: it is reached
        // through a `model:` and its credential is spent wherever the model is
        // called, so it follows its callers unconditionally.
        let mut moved = true;
        while moved {
            moved = false;
            for (address, definition) in &ir.definitions {
                let held: Vec<Process> = runs
                    .get(address)
                    .map(|held| held.keys().cloned().collect())
                    .unwrap_or_default();
                if held.is_empty() {
                    continue;
                }
                let mut carry = |target: &str, placeable: bool, site: &Span| {
                    if placeable && claims.contains_key(target) {
                        return;
                    };
                    for process in &held {
                        moved |= arrive(
                            &mut runs,
                            target,
                            process.clone(),
                            Carried {
                                from: Some(address.clone()),
                                site: Some(site.clone()),
                            },
                        );
                    }
                };
                match &definition.body {
                    DefinitionBody::Agent(agent) => {
                        for provider in providers_of(ir, &agent.model.value.to_string()) {
                            carry(&provider, false, &agent.model.span);
                        }
                        for attached in &agent.tools {
                            carry(&attached.value.to_string(), true, &attached.span);
                        }
                        // A store an agent attaches is opened by the synthesized
                        // tool its own loop calls (grammar §11.5), so it is
                        // opened wherever the agent runs. Not placeable: a
                        // `store.*` is not a `members:` entry (grammar §14.1).
                        for attached in &agent.stores {
                            carry(&attached.value.to_string(), false, &attached.span);
                        }
                    }
                    DefinitionBody::Flow(flow) => {
                        for node in &flow.nodes {
                            match &node.kind {
                                NodeKind::Agent { agent } => {
                                    carry(&agent.value.to_string(), true, &agent.span);
                                }
                                NodeKind::Function { function } => {
                                    carry(&function.value.to_string(), true, &function.span);
                                }
                                NodeKind::Flow { flow, .. } => {
                                    carry(&flow.value.to_string(), true, &flow.span);
                                }
                                NodeKind::Map { map } => {
                                    for target in crate::check::reach::targets(&map.dispatch) {
                                        carry(&target.value.to_string(), true, &target.span);
                                    }
                                }
                                // A `store:` node is opened by whichever process
                                // runs this flow's instance (grammar §11.4).
                                NodeKind::Store { store, .. } => {
                                    carry(&store.value.to_string(), false, &store.span);
                                }
                                // A `coder:` node's harness is **pointed at** its
                                // model's provider (grammar §8.9, Decision D143,
                                // PRD resolved q58 ruling d): the endpoint, the
                                // credential and the headers cross into the run,
                                // so that connection's variables are spent
                                // wherever this flow's instance runs. Not
                                // placeable, like an agent's own model: a
                                // `provider.*` is not a `members:` entry.
                                //
                                // Without this edge a provider reached *only*
                                // from a placed component would never join the
                                // hub's list, while the hub goes on being able
                                // to start every flow (Decision D64) — so the
                                // hub would launch clean and the coder node
                                // would fail on its first run for a variable no
                                // manifest asked for, which is §9.1's own
                                // failure mode.
                                NodeKind::Coder { coder } => {
                                    for provider in providers_of(ir, &coder.model.value.to_string())
                                    {
                                        carry(&provider, false, &coder.model.span);
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        // A storage backend follows the stores it binds. Run **after** the
        // closure, because what a backend is reached by is decided by which
        // processes reached the store, and nothing is downstream of a backend
        // for the fixed point to carry further.
        //
        // A store an agent attaches runs in that agent's process, so its
        // backend's credential is spent there and a manifest without it is a
        // worker that joins clean (§9.2 cannot refuse over a variable the
        // manifest does not name) and fails at its first store op — precisely
        // the failure §9.1 exists to prevent.
        for (address, definition) in &ir.definitions {
            let DefinitionBody::Store(store) = &definition.body else {
                continue;
            };
            let Some(site) = crate::ir::deploy::backend_of(ir, store).site else {
                continue;
            };
            let held: Vec<Process> = runs
                .get(address)
                .map(|held| held.keys().cloned().collect())
                .unwrap_or_default();
            let owner = backend_owner(&site);
            for process in held {
                arrive(
                    &mut runs,
                    &owner,
                    process,
                    Carried {
                        from: Some(address.clone()),
                        site: None,
                    },
                );
            }
        }

        // A component nothing places and nothing reaches is the hub's, which is
        // where every variable of a composition with no `placements:` at all
        // lived before this partition existed. Stated as a floor rather than
        // left implicit: a variable that belonged to no process would be one no
        // launch check and no join ever asks about.
        for address in ir.definitions.keys() {
            runs.entry(address.clone()).or_default();
        }
        let orphans: Vec<String> = runs
            .iter()
            .filter(|(_, held)| held.is_empty())
            .map(|(owner, _)| owner.clone())
            .collect();
        for owner in orphans {
            arrive(&mut runs, &owner, Process::Hub, own(None));
        }

        Self { runs, processes }
    }

    /// Every process of this deployment, the hub first and the placements in
    /// the order the deploy file declares them.
    pub fn processes(&self) -> impl Iterator<Item = &Process> {
        self.processes.iter()
    }

    /// Every process the surface `owner` can execute in, the hub first.
    pub fn processes_of(&self, owner: &str) -> impl Iterator<Item = &Process> {
        self.runs.get(owner).into_iter().flat_map(BTreeMap::keys)
    }

    /// How `owner` came to run in `process`: the surfaces from the one that
    /// holds the process itself to `owner`, in that order.
    ///
    /// The first step is where the process **starts** — a placement's
    /// `members:` entry, for a [`Process::Placement`] — and each step after it
    /// is the edge that carried the process one component further in. The last
    /// step is `owner` itself.
    ///
    /// Empty where `owner` does not run in `process` at all. A one-step route is
    /// a surface holding the process in its own right, with nothing to walk.
    #[must_use]
    pub fn route(&self, owner: &str, process: &Process) -> Vec<Step<'_>> {
        let mut walked = Vec::new();
        let mut seen = BTreeSet::new();
        let mut at = owner;
        loop {
            // The **key** rather than the argument, so every step borrows from
            // this partition and a route outlives the string it was asked about.
            let Some((held, how)) = self
                .runs
                .get_key_value(at)
                .and_then(|(held, runs)| runs.get(process).map(|how| (held.as_str(), how)))
            else {
                // Only reachable for the `owner` this was called with: every
                // `from` was written by an arrival that carried this process.
                return Vec::new();
            };
            if !seen.insert(held) {
                // Unreachable by construction — an arrival is recorded once, so
                // no owner is its own ancestor — and cheap insurance against a
                // future edge that made it otherwise, since the alternative is a
                // compiler that hangs.
                break;
            }
            walked.push(Step {
                owner: held,
                site: how.site.as_ref(),
            });
            let Some(from) = how.from.as_deref() else {
                break;
            };
            at = from;
        }
        walked.reverse();
        walked
    }

    /// Whether the surface `owner` holds can execute in `process`.
    #[must_use]
    fn holds(&self, owner: &str, process: &Process) -> bool {
        self.runs
            .get(owner)
            .is_some_and(|held| held.contains_key(process))
    }
}

/// One placement's claim on a component: the name, and the `members:` entry
/// that wrote it.
struct Claim {
    /// The placement's name.
    placement: String,
    /// Where the component is named in that placement's `members:`.
    at: Span,
}

/// Which placement claims each component, by address (grammar §14.1).
///
/// The first claim wins, exactly as `check::placements` reads it: a component
/// two placements name is already a compile error, and reading it twice here
/// would make this pass's answer depend on which report the author fixes.
fn claims(ir: &Ir) -> BTreeMap<String, Claim> {
    let mut claims = BTreeMap::new();
    let Some(section) = ir.deploy.placements.as_ref() else {
        return claims;
    };
    for placement in section.entries.values() {
        for member in &placement.members {
            claims
                .entry(member.value.to_string())
                .or_insert_with(|| Claim {
                    placement: placement.name.value.to_string(),
                    at: member.span.clone(),
                });
        }
    }
    claims
}

/// The `provider.*` addresses a `model.*` reaches, following a `route:`
/// (grammar §12.2).
fn providers_of(ir: &Ir, model: &str) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    let mut seen = BTreeSet::new();
    let mut pending = vec![model.to_string()];
    while let Some(address) = pending.pop() {
        if !seen.insert(address.clone()) {
            continue;
        }
        let Some(DefinitionBody::Model(model)) =
            ir.definitions.get(&address).map(|held| &held.body)
        else {
            continue;
        };
        match model {
            Model::Direct(direct) => {
                found.insert(direct.provider.value.to_string());
            }
            Model::Route(route) => {
                for member in &route.route {
                    pending.push(member.value.to_string());
                }
            }
        }
    }
    found
}

/// Every environment reference the composition makes, sorted by variable name.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct References {
    /// Variable name to the surfaces that reference it, in discovery order.
    entries: BTreeMap<String, Vec<String>>,
}

impl References {
    /// Collect every reference in the composition.
    ///
    /// The walk is over the IR in its own canonical order — definitions by
    /// address, then the trigger table, then the deploy layer — so the sites
    /// recorded for a variable are in a deterministic order without being
    /// sorted, and read in the order a person would find them.
    ///
    /// **The trigger table is walked for the reason the rest of this module
    /// exists.** An `http` trigger's `auth:` and `callback_auth:` carry four
    /// `${ENV}` references (grammar 13.3), and the served app reads all four:
    /// the token a caller's header is compared against, the secret a signature
    /// is verified with, and the two a delivery identifies itself by. §4.3's
    /// promise is that the variables a deployment needs are computable from the
    /// artifact statically, so a runtime that read `process.env.WEBHOOK_TOKEN`
    /// without this walk would let a deployment missing the variable start
    /// clean — `readEnvironment()` reports only what `environmentReferences`
    /// lists — and then refuse every real call, or deliver every callback
    /// unsigned.
    #[must_use]
    pub fn of(ir: &Ir) -> Self {
        Self::collect(ir, &mut |_| true)
    }

    /// The references **one process** of this deployment needs
    /// (`docs/distributed.md` §9.1).
    ///
    /// The same walk as [`Self::of`], filtered by whether the surface holding
    /// each reference can execute in `process` — which is the whole of §9.1's
    /// partition, and the reason it is a filter over one walk rather than a
    /// second walk of its own: two collectors would agree on the day they were
    /// written, and a variable dropped from one of them is a deployment that
    /// starts clean and fails at its first call.
    ///
    /// The hub's answer for a composition with no `placements:` is [`Self::of`]
    /// exactly, because every surface of such a composition executes on the hub.
    #[must_use]
    pub fn for_process(ir: &Ir, partition: &Partition, process: &Process) -> Self {
        Self::collect(ir, &mut |owner| partition.holds(owner, process))
    }

    /// The walk both of the above are, with `wanted` deciding which owners
    /// contribute.
    fn collect(ir: &Ir, wanted: &mut dyn FnMut(&str) -> bool) -> Self {
        let mut references = Self::default();
        for (address, definition) in &ir.definitions {
            if !wanted(address) {
                continue;
            }
            match &definition.body {
                DefinitionBody::Provider(provider) => {
                    let config = &provider.config;
                    for (key, value) in [
                        ("api_key", config.api_key.as_ref()),
                        ("base_url", config.base_url.as_ref()),
                        ("access_key_id", config.access_key_id.as_ref()),
                        ("secret_access_key", config.secret_access_key.as_ref()),
                        ("session_token", config.session_token.as_ref()),
                        ("credentials_json", config.credentials_json.as_ref()),
                    ] {
                        if let Some(reference) = value {
                            references.record(&reference.value.name, &format!("{address}.{key}"));
                        }
                    }
                    for (key, value) in [
                        ("api_version", config.api_version.as_ref()),
                        ("organization", config.organization.as_ref()),
                        ("region", config.region.as_ref()),
                        ("location", config.location.as_ref()),
                        ("project", config.project.as_ref()),
                        ("profile", config.profile.as_ref()),
                    ] {
                        if let Some(text) = value {
                            references.text(&text.value, &format!("{address}.{key}"));
                        }
                    }
                    references.entries_of(&config.headers, &format!("{address}.headers"));
                    // A server tool's config is grammar 4.3 class 2 like the
                    // rest of a provider's non-secret keys, so a vector store id
                    // or a gateway's container name can be an `${ENV}` the run
                    // supplies (Decision D122).
                    for (index, tool) in config.server_tools.iter().enumerate() {
                        let site = format!("{address}.server_tools[{index}]");
                        for (key, value) in &tool.config {
                            references.plugin(&value.value, &format!("{site}.{key}"));
                        }
                    }
                }
                DefinitionBody::Tool(tool) => match &tool.implementation {
                    ToolImplementation::Exec { exec } => {
                        references.exec(exec, &format!("{address}.exec"));
                    }
                    ToolImplementation::Http { http } => {
                        references.http(http, &format!("{address}.http"));
                    }
                    // Declared rather than walked, which is the whole reason
                    // grammar 6.1 puts an `env:` on the binding: the references
                    // an authored module makes are `process.env` reads inside
                    // TypeScript this compiler does not parse, so the YAML is
                    // the only place the partition can learn of them (PRD
                    // resolved q49). Filed under the tool's own address like
                    // every other tool surface, so §9.1's executes-in closure
                    // carries them to exactly the processes that can run it.
                    ToolImplementation::Module { module } => {
                        references.entries_of(&module.env, &format!("{address}.module.env"));
                    }
                    // A built-in's `workspace:` is grammar 4.3 class 2 like an
                    // `exec:`'s `cwd:`, and its `env:` is that block's `env:`
                    // exactly, for the same reason: where a graph may work and
                    // what its children may read are properties of the machine
                    // running it, not of the composition (Decision D135). Filed
                    // under the tool's own address, so §9.1's executes-in
                    // closure carries them to exactly the processes that can run
                    // it — which, for a placed agent, is the worker that took
                    // its dispatch (PRD resolved q41, q45).
                    ToolImplementation::Builtin { builtin } => {
                        if let Some(workspace) = &builtin.workspace {
                            references.text(&workspace.value, &format!("{address}.workspace"));
                        }
                        references.entries_of(&builtin.env, &format!("{address}.env"));
                    }
                    ToolImplementation::Function { .. } => {}
                },
                DefinitionBody::Flow(flow) => {
                    for node in &flow.nodes {
                        let id = node.id.value.as_str();
                        match &node.kind {
                            NodeKind::Exec { exec } => {
                                references.exec(exec, &format!("{address}.node.{id}.exec"));
                            }
                            NodeKind::Http { http } => {
                                references.http(http, &format!("{address}.node.{id}.http"));
                            }
                            // A coder node's `workspace:` is grammar 4.3 class 2
                            // like an `exec:`'s `cwd:`, and its `env:` is that
                            // block's `env:` exactly — which is PRD resolved
                            // q54 ruling b applied one construct along
                            // (Decision D139). Filed under the **node**, the
                            // way an inline `exec:` node's are, because the
                            // node is where the whole component is declared:
                            // there is no definition address to file them
                            // under.
                            NodeKind::Coder { coder } => {
                                references.text(
                                    &coder.workspace.value,
                                    &format!("{address}.node.{id}.workspace"),
                                );
                                references
                                    .entries_of(&coder.env, &format!("{address}.node.{id}.env"));
                            }
                            NodeKind::Agent { .. }
                            | NodeKind::Function { .. }
                            | NodeKind::Flow { .. }
                            | NodeKind::Map { .. }
                            | NodeKind::Human { .. }
                            | NodeKind::Store { .. } => {}
                        }
                    }
                }
                // A shorthand built-in carries no bounds, so it names no
                // environment: a built-in with a `workspace:` or an `env:` is a
                // `tool.*` with a `builtin:` binding, whose references are
                // collected under that tool's own address above (grammar 5.5,
                // 6.1, Decision D135).
                DefinitionBody::Agent(_) => {}
                DefinitionBody::Store(_) | DefinitionBody::Model(_) => {}
            }
        }

        // **Before** the deploy layer's own gate, because a backend is filed
        // under an owner of its own: the credential that opens a store is spent
        // in every process that opens it, which is the hub and every placement
        // whose components reach it (§9.1, [`backend_owner`]).
        if let Some(backends) = &ir.deploy.storage_backends {
            for (kind, config) in &backends.defaults {
                let site = format!("deploy.storage_backends.defaults.{kind}");
                if wanted(&backend_owner(&site)) {
                    references.backend(config, &site);
                }
            }
            for (alias, config) in &backends.aliases {
                let site = format!("deploy.storage_backends.aliases.{alias}");
                if wanted(&backend_owner(&site)) {
                    references.backend(config, &site);
                }
            }
        }

        // **Before** the deploy layer's own gate, for the backend's reason read
        // the other way round: a registry credential is spent wherever an
        // install runs, which is the hub *and* every placement that materialises
        // the artifact (PRD resolved q41, q59, `docs/distributed.md` §4, §9.1).
        //
        // It is a credential no *running* process spends: what reads it is
        // `bun install` over the emitted `bunfig.toml`/`.npmrc`, before this
        // project's code exists. It is on the manifest all the same, and
        // deliberately — an unset variable is not an install-time error in
        // either installer, so the presence check at launch and at join is what
        // turns a `401` from a mirror into a named variable (resolved q15).
        if let Some(registry) = &ir.deploy.package_registry
            && wanted(REGISTRY_OWNER)
        {
            if let Some(token) = &registry.token {
                references.record(&token.value.name, "deploy.package_registry.token");
            }
            for (scope, entry) in &registry.scopes {
                if let Some(token) = &entry.token {
                    references.record(
                        &token.value.name,
                        &format!("deploy.package_registry.scopes.{scope}.token"),
                    );
                }
            }
        }

        if !wanted(DEPLOY_OWNER) {
            return references;
        }

        // The hub's own credential: the bearer token every join presents
        // (`docs/distributed.md` §3, grammar §14.2). Read only where the target
        // declares `placements:`, because that is where the routes verifying it
        // are mounted — a launch check over a variable no code touches would be
        // exactly the false requirement §9.1 is written against.
        if let Some(hub) = &ir.deploy.hub
            && ir.deploy.placements.is_some()
            && let Some(token) = &hub.join_token
        {
            references.record(&token.value.name, "deploy.hub.join_token");
        }

        // The trace sink's own credential, and it is the hub's: the hub owns
        // the trace and is the process that ships it, so a worker that never
        // exports one must not be asked for the token (PRD resolved q41, q50,
        // `docs/distributed.md` §9.1). Unconditional past this gate, unlike
        // `hub.join_token:` above, because there is no second key deciding
        // whether the code that reads it exists: a target that declares a sink
        // exports from every execution it settles, under `run` as under `serve`.
        if let Some(sink) = &ir.deploy.trace_sink
            && let Some(auth) = &sink.auth
        {
            if let Some(bearer) = &auth.bearer {
                references.record(
                    &bearer.token.value.name,
                    "deploy.trace_sink.auth.bearer.token",
                );
            }
            if let Some(hmac) = &auth.hmac {
                references.record(
                    &hmac.secret.value.name,
                    "deploy.trace_sink.auth.hmac.secret",
                );
            }
        }

        if let Some(triggers) = &ir.triggers {
            for (name, trigger) in &triggers.entries {
                let TriggerKind::Http(http) = &trigger.kind else {
                    continue;
                };
                let at = format!("triggers.{name}");
                match &http.auth {
                    Some(InboundAuth::Bearer(bearer)) => {
                        references
                            .record(&bearer.token.value.name, &format!("{at}.auth.bearer.token"));
                    }
                    Some(InboundAuth::Hmac(hmac)) => {
                        references
                            .record(&hmac.secret.value.name, &format!("{at}.auth.hmac.secret"));
                    }
                    None => {}
                }
                if let Some(callback) = &http.callback_auth {
                    if let Some(bearer) = &callback.bearer {
                        references.record(
                            &bearer.token.value.name,
                            &format!("{at}.callback_auth.bearer.token"),
                        );
                    }
                    if let Some(hmac) = &callback.hmac {
                        references.record(
                            &hmac.secret.value.name,
                            &format!("{at}.callback_auth.hmac.secret"),
                        );
                    }
                }
            }
        }

        if let Some(sources) = &ir.deploy.event_sources {
            for (name, source) in &sources.entries {
                let at = format!("deploy.event_sources.{name}");
                for (key, reference) in &source.connection {
                    references.record(&reference.value.name, &format!("{at}.{key}"));
                }
                for (key, value) in &source.extra {
                    references.plugin(&value.value, &format!("{at}.{key}"));
                }
            }
        }

        references
    }

    /// Whether the composition references nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The variable names, sorted.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.entries.keys().map(String::as_str)
    }

    /// Where a variable is referenced.
    #[must_use]
    pub fn sites(&self, name: &str) -> &[String] {
        self.entries.get(name).map_or(&[], Vec::as_slice)
    }

    fn record(&mut self, name: &str, site: &str) {
        let sites = self.entries.entry(name.to_string()).or_default();
        if !sites.iter().any(|existing| existing == site) {
            sites.push(site.to_string());
        }
    }

    /// An interpolable string contributes every name it really carries — which
    /// is the list the IR records, not a scan of the text: an escaped `$${NAME}`
    /// is text and names nothing (grammar 4.3).
    fn text(&mut self, text: &Interpolated, site: &str) {
        for name in &text.references {
            self.record(name, site);
        }
    }

    fn entries_of(&mut self, entries: &[InterpolatedEntry], site: &str) {
        for entry in entries {
            self.text(&entry.value.value, &format!("{site}.{}", entry.name.value));
        }
    }

    fn exec(&mut self, exec: &Exec, site: &str) {
        self.text(&exec.command.value, &format!("{site}.command"));
        for (index, argument) in exec.args.iter().enumerate() {
            self.text(&argument.value, &format!("{site}.args[{index}]"));
        }
        if let Some(cwd) = &exec.cwd {
            self.text(&cwd.value, &format!("{site}.cwd"));
        }
        self.entries_of(&exec.env, &format!("{site}.env"));
    }

    fn http(&mut self, http: &Http, site: &str) {
        self.text(&http.url.value, &format!("{site}.url"));
        self.entries_of(&http.headers, &format!("{site}.headers"));
    }

    fn backend(&mut self, config: &crate::ir::deploy::BackendConfig, site: &str) {
        for (key, reference) in &config.connection {
            self.record(&reference.value.name, &format!("{site}.{key}"));
        }
        for (key, value) in &config.extra {
            self.plugin(&value.value, &format!("{site}.{key}"));
        }
    }

    fn plugin(&mut self, value: &PluginValue, site: &str) {
        match value {
            PluginValue::Text(text) => self.text(text, site),
            PluginValue::Sequence(items) => {
                for (index, item) in items.iter().enumerate() {
                    self.plugin(&item.value, &format!("{site}[{index}]"));
                }
            }
            PluginValue::Mapping(entries) => {
                for entry in entries {
                    self.plugin(&entry.value.value, &format!("{site}.{}", entry.key.value));
                }
            }
            PluginValue::Null
            | PluginValue::Bool(_)
            | PluginValue::Int(_)
            | PluginValue::Float(_) => {}
        }
    }
}

/// `src/env.ts`.
#[must_use]
pub fn module(ir: &Ir, references: &References) -> super::GeneratedFile {
    let mut contents = super::header(ir, "// ");
    contents.push_str(MODULE_DOC);
    contents.push_str("\nimport process from \"node:process\";\n");
    contents.push_str(DECLARATIONS);

    if references.is_empty() {
        contents.push_str(
            "export const environmentReferences: readonly EnvironmentReference[] = [];\n",
        );
    } else {
        contents
            .push_str("export const environmentReferences: readonly EnvironmentReference[] = [\n");
        for name in references.names() {
            contents.push_str(&format!(
                "  {{\n    name: {},\n    sites: [\n",
                names::string(name)
            ));
            for site in references.sites(name) {
                contents.push_str(&format!("      {},\n", names::string(site)));
            }
            contents.push_str("    ],\n  },\n");
        }
        contents.push_str("];\n");
    }

    contents.push_str(CHECKER);

    super::GeneratedFile {
        path: "src/env.ts".to_string(),
        contents,
    }
}

const MODULE_DOC: &str = "\
//
// Every `${ENV}` reference the composition makes, and `readEnvironment()`, the
// presence check over them (PRD 5.9, grammar 4.3). `./index.ts` is what calls
// it, at module scope, so loading this project is what checks its environment.
// No value is baked in here: the spec never contains a credential, and neither
// does this file — `agent-compose build` resolved nothing.
";

const DECLARATIONS: &str = r#"
/** One `${ENV}` reference, and the surfaces that wrote it. */
export interface EnvironmentReference {
  /** The variable name, between the braces. */
  readonly name: string;
  /** Where it is referenced, as spec addresses. */
  readonly sites: readonly string[];
}

/**
 * Every reference this composition makes, sorted by name.
 *
 * This is also the least-privilege list of PRD 5.9: a deployment of this graph
 * needs these variables and no others.
 */
"#;

const CHECKER: &str = r#"
/** Raised when the process starts without a variable the composition needs. */
export class MissingEnvironmentError extends Error {
  /** Every variable that was missing, sorted by name. */
  readonly missing: readonly EnvironmentReference[];

  constructor(missing: readonly EnvironmentReference[]) {
    const detail = missing
      .map((reference) => `${reference.name} (referenced by ${reference.sites.join(", ")})`)
      .join("; ");
    super(`missing environment ${missing.length === 1 ? "variable" : "variables"}: ${detail}`);
    this.name = "MissingEnvironmentError";
    this.missing = missing;
  }
}

/**
 * Resolve every reference at process start, or throw naming all of them.
 *
 * Presence is `!== undefined`: a variable set to the empty string is set. All
 * missing variables are reported at once, because a deployment that is short
 * three keys should learn that in one run rather than in three.
 */
export function readEnvironment(
  source: Record<string, string | undefined> = process.env,
): ReadonlyMap<string, string> {
  const resolved = new Map<string, string>();
  const missing: EnvironmentReference[] = [];
  for (const reference of environmentReferences) {
    const value = source[reference.name];
    if (value === undefined) {
      missing.push(reference);
    } else {
      resolved.set(reference.name, value);
    }
  }
  if (missing.length > 0) {
    throw new MissingEnvironmentError(missing);
  }
  return resolved;
}
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codegen::test_support::{ir_of, ir_of_mesh};

    /// The variables one process of a deployment needs, sorted.
    fn manifest(ir: &Ir, partition: &Partition, process: &Process) -> Vec<String> {
        References::for_process(ir, partition, process)
            .names()
            .map(str::to_string)
            .collect()
    }

    /// A composition and a deploy layer, partitioned.
    fn partitioned(source: &str, deploy: &str) -> (Ir, Partition) {
        let ir = ir_of_mesh(source, deploy);
        let partition = Partition::of(&ir);
        (ir, partition)
    }

    /// **`docs/distributed.md` §9.1's first worked example**, which is the case
    /// the partition rule exists for.
    ///
    /// > `tool.sign` carries `KEYCHAIN_PASSWORD` in its `exec.env`, `agent.signer`
    /// > attaches it, and the deploy file places `agent.signer` in `mac` and
    /// > nothing else. `validate` accepts that — §14.1 rule 4's first row, the
    /// > tool claims nothing and runs where the agent runs. `KEYCHAIN_PASSWORD`
    /// > belongs to **`mac`'s** manifest and **not** to the hub's: the hub never
    /// > runs `tool.sign`, so requiring the secret there would be a false
    /// > requirement, and omitting it from `mac`'s would let a machine without a
    /// > keychain join clean (§9.2) and fail on its first dispatch.
    ///
    /// Both halves are asserted, because each is a different failure: the hub's
    /// is a deployment that refuses to start over a value it never reads, and
    /// `mac`'s is the machine §9.2's join-time check exists to catch getting in.
    #[test]
    fn the_keychain_password_of_a_placed_agents_attached_tool_is_that_placements_alone() {
        let (ir, partition) = partitioned(
            "version: \"0.1\"\n\
provider.vendor:\n  kind: openai\n  api_key: ${VENDOR_KEY}\n\
model.smart:\n  provider: provider.vendor\n  id: some-model\n\
tool.sign:\n  description: Sign one artifact.\n  input: { path: { type: string } }\n  output: { signature: { type: string } }\n  exec:\n    command: codesign\n    env:\n      KEYCHAIN_PASSWORD: ${KEYCHAIN_PASSWORD}\n\
agent.signer:\n  model: model.smart\n  prompt: Sign what you are given.\n  tools: [tool.sign]\n  input: { path: { type: string } }\n  output: { verdict: { type: string } }\n\
flow.release:\n  inputs:\n    path: { type: string }\n  outputs: {}\n  nodes:\n    sign:\n      agent: agent.signer\n      input:\n        path: \"input.path\"\n  edges:\n    - { from: start, to: sign }\n    - { from: sign, to: end }\n",
            "version: \"0.1\"\n\
hub:\n  join_token: ${MESH_TOKEN}\n\
placements:\n  mac:\n    members: [agent.signer]\n",
        );

        assert_eq!(
            manifest(&ir, &partition, &Process::Placement("mac".to_string())),
            ["KEYCHAIN_PASSWORD", "VENDOR_KEY"],
            "the worker runs the agent and the tool it attaches, so it needs both their secrets"
        );
        assert_eq!(
            manifest(&ir, &partition, &Process::Hub),
            ["MESH_TOKEN"],
            "the hub runs neither the agent nor its tool: all it holds is the mesh's own \
             credential (§9.1's fifth clause)"
        );
    }

    /// **A `coder:` node's provider connection is on the manifest of every
    /// process that runs its flow** (`docs/distributed.md` §9.1, PRD resolved
    /// q58 ruling d).
    ///
    /// Resolved q58 made the provider's `base_url:`, credential and `headers:`
    /// cross into a harness run, which turns a `model:` on a coder node into a
    /// **reference site** the partition has to follow — exactly as an agent's
    /// `model:` already is. Before it, the same provider was a connection this
    /// runtime opened and no harness ever saw, so nothing was spent on the
    /// process running the node.
    ///
    /// The composition below is built to make the missing edge visible, because
    /// the easy case hides it: a provider **nothing** reaches falls to the hub
    /// under the orphan floor at the end of [`Partition::of`], so a coder node's
    /// connection would appear to work while the walk never looked at it. Here
    /// the same `provider.gateway` also serves a **placed** agent, which gives it
    /// `mac` and takes it off the orphan list — and the hub, which can start
    /// every flow a composition declares (Decision D64) and therefore runs this
    /// coder node, would launch clean and fail on the node's first run for a
    /// variable no manifest asked for.
    #[test]
    fn a_coder_nodes_provider_is_on_the_manifest_of_every_process_running_its_flow() {
        let (ir, partition) = partitioned(
            "version: \"0.1\"\n\
provider.gateway:\n  kind: anthropic\n  base_url: ${GATEWAY_URL}\n  api_key: ${GATEWAY_KEY}\n\
model.smart:\n  provider: provider.gateway\n  id: some-model\n\
agent.signer:\n  model: model.smart\n  prompt: Sign what you are given.\n  input: { path: { type: string } }\n  output: { verdict: { type: string } }\n\
state:\n  summary: { type: string, default: \"\" }\n\
flow.release:\n  inputs:\n    path: { type: string }\n  outputs:\n    summary: { type: string }\n  nodes:\n    build:\n      coder:\n        harness: cc\n        model: model.smart\n        workspace: /srv/checkout\n        prompt: Build it.\n        output:\n          summary: { type: string }\n      input: \"input.path\"\n    sign:\n      agent: agent.signer\n      input:\n        path: \"input.path\"\n  edges:\n    - { from: start, to: build }\n    - { from: build, to: sign }\n    - { from: sign, to: end }\n",
            "version: \"0.1\"\n\
hub:\n  join_token: ${MESH_TOKEN}\n\
placements:\n  mac:\n    members: [agent.signer]\n",
        );

        assert_eq!(
            manifest(&ir, &partition, &Process::Hub),
            ["GATEWAY_KEY", "GATEWAY_URL", "MESH_TOKEN"],
            "the hub runs `flow.release` and its `coder:` node, so it holds the connection that \
             node's harness is pointed at — and the placed agent's own dispatch is the worker's"
        );
        assert_eq!(
            manifest(&ir, &partition, &Process::Placement("mac".to_string())),
            ["GATEWAY_KEY", "GATEWAY_URL"],
            "the worker runs the placed agent, which calls the same connection"
        );
    }

    /// **§9.1's second worked example**, "from the other side, because the
    /// symmetric mistake is the expensive one".
    ///
    /// > `agent.outer` is placed in `mac` and attaches `flow.review`, whose one
    /// > `agent:` node names `agent.inner`, which is unplaced and reads
    /// > `${INNER_KEY}`. Every call *through `agent.outer`* runs `agent.inner`
    /// > on the `mac` worker, so `INNER_KEY` belongs to `mac`'s manifest — and it
    /// > belongs to the **hub's** as well, because `agent-compose run main.yml
    /// > flow.review` starts that flow on the hub and the hub dispatches
    /// > `agent.inner` itself.
    ///
    /// The clause that produces the second half is the unconditional one: an
    /// unplaced `agent.*` is dispatched by the hub whatever else reaches it
    /// (grammar Decision D64). A partition that read "reached by something
    /// placed" as "therefore not the hub's" would pass `readEnvironment()` and
    /// fail at the first direct run's first model call.
    #[test]
    fn a_key_an_attached_flow_reaches_belongs_to_the_worker_and_to_the_hub() {
        let (ir, partition) = partitioned(
            "version: \"0.1\"\n\
provider.outer:\n  kind: openai\n  api_key: ${OUTER_KEY}\n\
provider.inner:\n  kind: openai\n  api_key: ${INNER_KEY}\n\
model.outer:\n  provider: provider.outer\n  id: some-model\n\
model.inner:\n  provider: provider.inner\n  id: some-model\n\
agent.inner:\n  model: model.inner\n  prompt: Review it.\n  input: { finding: { type: string } }\n  output: { note: { type: string } }\n\
flow.review:\n  description: Have the reviewer look at one finding.\n  inputs:\n    finding: { type: string }\n  outputs: {}\n  nodes:\n    look:\n      agent: agent.inner\n      input:\n        finding: \"input.finding\"\n  edges:\n    - { from: start, to: look }\n    - { from: look, to: end }\n\
agent.outer:\n  model: model.outer\n  prompt: Ask the reviewer.\n  tools: [flow.review]\n  input: { finding: { type: string } }\n  output: { verdict: { type: string } }\n\
flow.release:\n  inputs:\n    finding: { type: string }\n  outputs: {}\n  nodes:\n    ask:\n      agent: agent.outer\n      input:\n        finding: \"input.finding\"\n  edges:\n    - { from: start, to: ask }\n    - { from: ask, to: end }\n",
            "version: \"0.1\"\n\
hub:\n  join_token: ${MESH_TOKEN}\n\
placements:\n  mac:\n    members: [agent.outer]\n",
        );

        let mac = manifest(&ir, &partition, &Process::Placement("mac".to_string()));
        assert!(
            mac.contains(&"INNER_KEY".to_string()),
            "every call through `agent.outer` runs `agent.inner` in the worker's own tool loop: \
             {mac:?}"
        );
        assert!(mac.contains(&"OUTER_KEY".to_string()), "{mac:?}");

        let hub = manifest(&ir, &partition, &Process::Hub);
        assert!(
            hub.contains(&"INNER_KEY".to_string()),
            "`agent.inner` is unplaced, so the hub dispatches it whenever `flow.review` is \
             started directly — being reached from a placed agent's tool loop adds a placement, \
             it never moves the variable off the hub's list (§9.1): {hub:?}"
        );
        assert!(
            !hub.contains(&"OUTER_KEY".to_string()),
            "`agent.outer` is placed, so the hub never spends its provider's credential: {hub:?}"
        );

        // …and the route into `mac` is the chain that put it there, which is
        // what grammar 14.1 rule 5's refusal draws for an author (D131). It
        // starts at the `members:` entry and ends at the surface asked about.
        let route: Vec<&str> = partition
            .route("agent.inner", &Process::Placement("mac".to_string()))
            .into_iter()
            .map(|step| step.owner)
            .collect();
        assert_eq!(route, ["agent.outer", "flow.review", "agent.inner"]);
        let placed = partition.route("agent.outer", &Process::Placement("mac".to_string()));
        assert_eq!(
            placed.len(),
            1,
            "a surface holding the process in its own right has nothing to walk"
        );
        assert!(
            placed[0].site.is_some(),
            "the first step is the `members:` entry, so it has a line to point at"
        );
        assert!(
            partition.route("agent.outer", &Process::Hub).is_empty(),
            "a placed agent has no hub execution, so there is no route into one"
        );
    }

    /// **A `coder:` node's variables follow the flow that holds it**, which is
    /// the hub *and* every placement whose components reach that flow (§9.1).
    ///
    /// The bullet §9.1 gives coder nodes said "the hub's, and no placement's",
    /// on the reasoning that `members:` binds components and a coder node is not
    /// one. The reasoning is right about `members:` and wrong about the answer:
    /// a coder node's references are filed under its **flow's** address, so the
    /// executes-in closure carries them wherever that flow runs — and §14.1 rule
    /// 4 runs an attached flow's instance inside its agent's own loop, on the
    /// worker. An operator who read the old bullet would provision the hub only,
    /// and the worker would be refused at the join (§9.2) over a variable the
    /// manifest names and the machine does not hold.
    ///
    /// So both halves are asserted, the way the two above are: the placement's,
    /// because that is the manifest §9.2 checks; and the hub's, because a flow
    /// is startable there whatever else reaches it (Decision D64).
    ///
    /// The node's **provider** is on both lists too, since PRD resolved q58:
    /// the connection crosses into the run, so its variables are spent wherever
    /// the run happens. That edge has a case of its own, because this
    /// composition cannot see it slip —
    /// [`a_coder_nodes_provider_is_on_the_manifest_of_every_process_running_its_flow`].
    #[test]
    fn a_coder_nodes_workspace_and_env_follow_the_flow_that_holds_it() {
        let (ir, partition) = partitioned(
            "version: \"0.1\"\n\
provider.vendor:\n  kind: anthropic\n  api_key: ${VENDOR_KEY}\n\
model.smart:\n  provider: provider.vendor\n  id: some-model\n\
flow.patch:\n  description: Patch one thing.\n  inputs:\n    goal: { type: string }\n  outputs: {}\n  nodes:\n    build:\n      coder:\n        harness: cc\n        model: model.smart\n        workspace: ${CODER_REPO_ROOT}\n        prompt: Do the work.\n        output:\n          summary: { type: string }\n        env:\n          CODER_SECRET: ${CODER_SECRET}\n      input: \"input.goal\"\n  edges:\n    - { from: start, to: build }\n    - { from: build, to: end }\n\
agent.signer:\n  model: model.smart\n  prompt: Ask for a patch.\n  tools: [flow.patch]\n  input: { goal: { type: string } }\n  output: { verdict: { type: string } }\n\
flow.release:\n  inputs:\n    goal: { type: string }\n  outputs: {}\n  nodes:\n    sign:\n      agent: agent.signer\n      input:\n        goal: \"input.goal\"\n  edges:\n    - { from: start, to: sign }\n    - { from: sign, to: end }\n",
            "version: \"0.1\"\n\
hub:\n  join_token: ${MESH_TOKEN}\n\
placements:\n  mac:\n    members: [agent.signer]\n",
        );

        assert_eq!(
            manifest(&ir, &partition, &Process::Placement("mac".to_string())),
            ["CODER_REPO_ROOT", "CODER_SECRET", "VENDOR_KEY"],
            "the worker runs the agent, and grammar §14.1 rule 4 starts the flow it attaches — \
             coder node and all — inside that agent's own loop"
        );
        assert_eq!(
            manifest(&ir, &partition, &Process::Hub),
            [
                "CODER_REPO_ROOT",
                "CODER_SECRET",
                "MESH_TOKEN",
                "VENDOR_KEY"
            ],
            "…and `agent-compose run main.yml flow.patch` starts the same flow on the hub, which \
             is why the placement **adds** rather than moves (Decision D64). `VENDOR_KEY` is \
             there because the provider *connection* now crosses into a harness run: the harness \
             owns its client and this composition points it (PRD resolved q58 ruling d, \
             Decision D143)"
        );
    }

    /// The rule that is unconditional is unconditional in both directions.
    ///
    /// An unplaced `tool.*` a `function:` node names is the hub's (§9.1),
    /// **and** the placement's of every agent that attaches it — two placed
    /// agents attaching one unplaced tool is "the ordinary case, and the tool's
    /// secrets go to both placements".
    #[test]
    fn a_tool_two_placements_reach_and_the_hub_dispatches_belongs_to_all_three() {
        let (ir, partition) = partitioned(
            "version: \"0.1\"\n\
provider.vendor:\n  kind: openai\n  api_key: ${VENDOR_KEY}\n\
model.smart:\n  provider: provider.vendor\n  id: some-model\n\
tool.shared:\n  description: Look something up.\n  input: { q: { type: string } }\n  output: { hit: { type: string } }\n  exec:\n    command: lookup\n    env:\n      SHARED_TOKEN: ${SHARED_TOKEN}\n\
agent.left:\n  model: model.smart\n  prompt: Ask.\n  tools: [tool.shared]\n  input: { q: { type: string } }\n  output: { a: { type: string } }\n\
agent.right:\n  model: model.smart\n  prompt: Ask again.\n  tools: [tool.shared]\n  input: { q: { type: string } }\n  output: { a: { type: string } }\n\
flow.both:\n  inputs:\n    q: { type: string }\n  outputs: {}\n  nodes:\n    left:\n      agent: agent.left\n      input:\n        q: \"input.q\"\n    right:\n      agent: agent.right\n      input:\n        q: \"input.q\"\n    direct:\n      function: tool.shared\n      input:\n        q: \"input.q\"\n  edges:\n    - { from: start, to: left }\n    - { from: left, to: right }\n    - { from: right, to: direct }\n    - { from: direct, to: end }\n",
            "version: \"0.1\"\n\
hub:\n  join_token: ${MESH_TOKEN}\n\
placements:\n  one:\n    members: [agent.left]\n  two:\n    members: [agent.right]\n",
        );

        for placement in ["one", "two"] {
            let held = manifest(&ir, &partition, &Process::Placement(placement.to_string()));
            assert!(
                held.contains(&"SHARED_TOKEN".to_string()),
                "`{placement}` attaches `tool.shared` and does not hold its secret: {held:?}"
            );
        }
        let hub = manifest(&ir, &partition, &Process::Hub);
        assert!(
            hub.contains(&"SHARED_TOKEN".to_string()),
            "a `function:` node names `tool.shared` directly, and the hub's scheduler is what \
             starts that node: {hub:?}"
        );
        assert!(
            !hub.contains(&"VENDOR_KEY".to_string()),
            "both agents are placed, so the hub calls no model: {hub:?}"
        );
    }

    /// A store's **backend credential** follows the store, to the hub and to
    /// every placement that opens it (§9.1).
    ///
    /// The failure this prevents is the one §9.2's join-time check exists for,
    /// arrived at from the deploy layer: `agent.archivist` is placed on `vault`
    /// and attaches `store.docs`, whose alias resolves to a networked backend
    /// with a `${CHROMA_URL}`. The worker opens that store in its own tool loop,
    /// so a manifest without the variable lets it join clean — §3.1's `403`
    /// cannot fire over a name the manifest does not carry — and fail at its
    /// first `docs_search` on a machine that has no `CHROMA_URL`.
    ///
    /// Both halves are asserted, because §9.1's fifth clause is not weakened by
    /// this: a deploy-layer variable stays the **hub's** whatever else reaches
    /// it, exactly as an unplaced agent's does. The placement is added, never
    /// substituted.
    #[test]
    fn a_stores_backend_credential_belongs_to_the_hub_and_to_every_placement_that_opens_it() {
        let (ir, partition) = partitioned(
            "version: \"0.1\"\n\
provider.vendor:\n  kind: openai\n  api_key: ${VENDOR_KEY}\n\
provider.embed:\n  kind: openai\n  api_key: ${EMBED_KEY}\n\
model.smart:\n  provider: provider.vendor\n  id: some-model\n\
store.docs:\n  kind: vector\n  scope: global\n  backend: docs_db\n  embed:\n    model: text-embedding-3-small\n    provider: provider.embed\n\
agent.archivist:\n  model: model.smart\n  prompt: File it.\n  stores: [store.docs]\n  input: { q: { type: string } }\n  output: { a: { type: string } }\n\
flow.archive:\n  inputs:\n    q: { type: string }\n  outputs: {}\n  nodes:\n    file:\n      agent: agent.archivist\n      input:\n        q: \"input.q\"\n  edges:\n    - { from: start, to: file }\n    - { from: file, to: end }\n",
            "version: \"0.1\"\n\
hub:\n  join_token: ${MESH_TOKEN}\n\
placements:\n  vault:\n    members: [agent.archivist]\n\
storage_backends:\n  aliases:\n    docs_db:\n      provider: chroma\n      url: ${CHROMA_URL}\n",
        );

        let vault = manifest(&ir, &partition, &Process::Placement("vault".to_string()));
        assert!(
            vault.contains(&"CHROMA_URL".to_string()),
            "the worker opens `store.docs` in the placed agent's own tool loop and does not hold \
             the credential that opens it: {vault:?}"
        );
        let hub = manifest(&ir, &partition, &Process::Hub);
        assert!(
            hub.contains(&"CHROMA_URL".to_string()),
            "§9.1's fifth clause: a deploy-layer variable is the hub's, and a placement reaching \
             it adds a process rather than moving one: {hub:?}"
        );
        assert!(
            !hub.contains(&"VENDOR_KEY".to_string()),
            "the only agent is placed, so the hub calls no model: {hub:?}"
        );
    }

    /// A backend entry no store binds keeps the membership it always had.
    ///
    /// The over-narrowing guard for the clause above: filing a backend under an
    /// owner of its own could have dropped one that nothing resolves to off
    /// every list, and a variable belonging to no process is one no launch check
    /// and no join ever asks about.
    #[test]
    fn a_backend_no_store_binds_is_still_the_hubs() {
        let (ir, partition) = partitioned(
            "version: \"0.1\"\n\
provider.vendor:\n  kind: openai\n  api_key: ${VENDOR_KEY}\n\
model.smart:\n  provider: provider.vendor\n  id: some-model\n\
agent.plain:\n  model: model.smart\n  prompt: Answer.\n  input: { q: { type: string } }\n  output: { a: { type: string } }\n\
flow.ask:\n  inputs:\n    q: { type: string }\n  outputs: {}\n  nodes:\n    ask:\n      agent: agent.plain\n      input:\n        q: \"input.q\"\n  edges:\n    - { from: start, to: ask }\n    - { from: ask, to: end }\n",
            "version: \"0.1\"\n\
hub:\n  join_token: ${MESH_TOKEN}\n\
placements:\n  vault:\n    members: [agent.plain]\n\
storage_backends:\n  defaults:\n    kv:\n      provider: redis\n      url: ${REDIS_URL}\n",
        );
        let hub = manifest(&ir, &partition, &Process::Hub);
        assert!(hub.contains(&"REDIS_URL".to_string()), "{hub:?}");
        let vault = manifest(&ir, &partition, &Process::Placement("vault".to_string()));
        assert!(
            !vault.contains(&"REDIS_URL".to_string()),
            "no store binds that default, so no placement opens it: {vault:?}"
        );
    }

    /// A composition with no `placements:` partitions to exactly what it always
    /// had — the whole environment, on the hub.
    ///
    /// The over-narrowing guard: every clause above takes something *off* the
    /// hub's list, and a rule that took too much would leave a single-process
    /// deployment starting without a variable it needs.
    #[test]
    fn a_composition_that_places_nothing_gives_the_hub_the_whole_environment() {
        let ir = ir_of(PROJECT);
        let partition = Partition::of(&ir);
        assert_eq!(
            manifest(&ir, &partition, &Process::Hub),
            References::of(&ir).names().collect::<Vec<_>>(),
            "with nothing placed, the hub is the only process there is"
        );
        assert_eq!(partition.processes().count(), 1);
    }

    const PROJECT: &str = "version: \"0.1\"\n\
provider.p:\n  kind: openai_compatible\n  api_key: ${LLM_KEY}\n  base_url: ${LLM_URL}\n\
model.m:\n  provider: provider.p\n  id: some-model\n\
tool.search:\n  description: Search.\n  input: { q: { type: string } }\n  output: { hits: { type: integer } }\n  http:\n    method: GET\n    url: \"https://${SEARCH_HOST}/v1\"\n    headers:\n      authorization: \"Bearer ${LLM_KEY}\"\n";

    #[test]
    fn every_reference_is_collected_with_where_it_was_written() {
        let references = References::of(&ir_of(PROJECT));
        assert_eq!(
            references.names().collect::<Vec<_>>(),
            ["LLM_KEY", "LLM_URL", "SEARCH_HOST"]
        );
        assert_eq!(
            references.sites("LLM_KEY"),
            [
                "provider.p.api_key",
                "tool.search.http.headers.authorization"
            ]
        );
        assert_eq!(references.sites("SEARCH_HOST"), ["tool.search.http.url"]);
    }

    /// Grammar 4.3: `$${NAME}` is an escape, and the IR's own reference list —
    /// not a scan of the text — is what says which tokens are real.
    #[test]
    fn an_escaped_token_references_nothing() {
        let references = References::of(&ir_of(
            "version: \"0.1\"\n\
tool.t:\n  description: A tool.\n  input: {}\n  output: {}\n  exec:\n    command: echo\n    args: [\"$${NOT_A_REF}\"]\n",
        ));
        assert!(references.is_empty(), "{references:?}");
    }

    /// An authenticated trigger's four credentials are variables the deployment
    /// needs (grammar 13.3, §4.3).
    ///
    /// The served app compares a caller's header against one of them and signs
    /// its deliveries with another, so a deployment missing one starts clean and
    /// then refuses every real call or delivers every callback unsigned. Naming
    /// them here is what makes the launch check refuse first.
    #[test]
    fn an_authenticated_triggers_credentials_are_variables_the_deployment_needs() {
        let references = References::of(&ir_of(
            "version: \"0.1\"\n\
flow.support:\n  outputs: {}\n  nodes:\n    approve:\n      human:\n        input: {}\n        output:\n          decision: { enum: [approve, reject] }\n  edges:\n    - { from: start, to: approve }\n    - { from: approve, to: end }\n\
triggers:\n  intake:\n    type: http\n    flow: flow.support\n    callback: \"payload.body.callback_url\"\n    auth:\n      hmac:\n        secret: ${WEBHOOK_SECRET}\n    callback_auth:\n      bearer:\n        token: ${CALLBACK_TOKEN}\n      hmac:\n        secret: ${CALLBACK_SECRET}\n    callback_allow:\n      - \"https://hooks.example.com/*\"\n  open:\n    type: http\n    flow: flow.support\n    path: /open\n    auth:\n      bearer:\n        token: ${WEBHOOK_TOKEN}\n",
        ));
        assert_eq!(
            references.names().collect::<Vec<_>>(),
            [
                "CALLBACK_SECRET",
                "CALLBACK_TOKEN",
                "WEBHOOK_SECRET",
                "WEBHOOK_TOKEN"
            ]
        );
        assert_eq!(
            references.sites("WEBHOOK_SECRET"),
            ["triggers.intake.auth.hmac.secret"]
        );
        assert_eq!(
            references.sites("CALLBACK_TOKEN"),
            ["triggers.intake.callback_auth.bearer.token"]
        );
        assert_eq!(
            references.sites("CALLBACK_SECRET"),
            ["triggers.intake.callback_auth.hmac.secret"]
        );
        assert_eq!(
            references.sites("WEBHOOK_TOKEN"),
            ["triggers.open.auth.bearer.token"]
        );
    }

    /// The trace sink's credential is a variable the deployment needs, and it is
    /// the **hub's** (grammar 14.5, PRD resolved q41, q50).
    ///
    /// Two halves, and each is a different failure. Omitting it from the hub's
    /// list is a deployment that starts clean and then ships every trace
    /// unsigned — the sink answers 401 and the executions never notice, which is
    /// exactly the silence an at-least-once delivery is designed to keep. Adding
    /// it to a placement's is the false requirement `docs/distributed.md` §9.1
    /// is written against: the hub owns the trace and is the only process that
    /// exports one, so a worker refusing to join over a token it would never
    /// spend is the compiler inventing a dependency.
    #[test]
    fn the_trace_sinks_credential_is_the_hubs_and_no_workers() {
        let (ir, partition) = partitioned(
            "version: \"0.1\"\n\
provider.vendor:\n  kind: openai\n  api_key: ${VENDOR_KEY}\n\
model.smart:\n  provider: provider.vendor\n  id: some-model\n\
agent.signer:\n  model: model.smart\n  prompt: Sign what you are given.\n  input: { path: { type: string } }\n  output: { verdict: { type: string } }\n\
flow.release:\n  inputs:\n    path: { type: string }\n  outputs: {}\n  nodes:\n    sign:\n      agent: agent.signer\n      input:\n        path: \"input.path\"\n  edges:\n    - { from: start, to: sign }\n    - { from: sign, to: end }\n",
            "version: \"0.1\"\n\
hub:\n  join_token: ${MESH_TOKEN}\n\
placements:\n  mac:\n    members: [agent.signer]\n\
trace_sink:\n  url: \"https://collector.internal.example/v1/traces\"\n  auth:\n    bearer:\n      token: ${TRACE_SINK_TOKEN}\n    hmac:\n      secret: ${TRACE_SINK_SECRET}\n",
        );

        assert_eq!(
            manifest(&ir, &partition, &Process::Hub),
            ["MESH_TOKEN", "TRACE_SINK_SECRET", "TRACE_SINK_TOKEN"],
            "the hub owns the trace and is the process that ships it, so both sink credentials \
             are its own"
        );
        assert_eq!(
            manifest(&ir, &partition, &Process::Placement("mac".to_string())),
            ["VENDOR_KEY"],
            "a worker exports no trace, so asking it for the sink's credentials would be the \
             false requirement §9.1 exists to prevent"
        );

        let references = References::of(&ir);
        assert_eq!(
            references.sites("TRACE_SINK_TOKEN"),
            ["deploy.trace_sink.auth.bearer.token"]
        );
        assert_eq!(
            references.sites("TRACE_SINK_SECRET"),
            ["deploy.trace_sink.auth.hmac.secret"]
        );
    }

    /// …and a sink that authenticates nothing contributes nothing.
    ///
    /// The sibling of [`an_open_trigger_contributes_no_variable`]: an
    /// unauthenticated collector on a private network is a posture grammar 14.5
    /// admits, so declaring the section must not by itself put a variable on the
    /// launch check.
    #[test]
    fn an_unauthenticated_trace_sink_contributes_no_variable() {
        let references = References::of(&ir_of_mesh(
            "version: \"0.1\"\n",
            "version: \"0.1\"\n\
trace_sink:\n  url: \"http://localhost:4318/v1/traces\"\n  format: otlp\n",
        ));
        assert!(references.is_empty(), "{references:?}");
    }

    /// A trigger declaring no credential contributes none.
    #[test]
    fn an_open_trigger_contributes_no_variable() {
        let references = References::of(&ir_of(
            "version: \"0.1\"\n\
flow.support:\n  outputs: {}\n  nodes:\n    approve:\n      human:\n        input: {}\n        output:\n          decision: { enum: [approve, reject] }\n  edges:\n    - { from: start, to: approve }\n    - { from: approve, to: end }\n\
triggers:\n  intake:\n    type: http\n    flow: flow.support\n    callback: \"payload.body.callback_url\"\n",
        ));
        assert!(references.is_empty(), "{references:?}");
    }

    #[test]
    fn a_composition_referencing_nothing_still_emits_the_checker() {
        let ir = ir_of("version: \"0.1\"\n");
        let emitted = module(&ir, &References::of(&ir)).contents;
        assert!(emitted.contains("environmentReferences: readonly EnvironmentReference[] = [];"));
        assert!(emitted.contains("export function readEnvironment("));
    }

    #[test]
    fn the_emitted_list_carries_names_and_sites() {
        let ir = ir_of(PROJECT);
        let emitted = module(&ir, &References::of(&ir)).contents;
        assert!(emitted.contains("    name: \"LLM_KEY\",\n"), "{emitted}");
        assert!(
            emitted.contains("      \"tool.search.http.headers.authorization\",\n"),
            "{emitted}"
        );
        assert!(
            !emitted.contains("${LLM_KEY}"),
            "no reference is resolved or reproduced verbatim: {emitted}"
        );
    }
}
