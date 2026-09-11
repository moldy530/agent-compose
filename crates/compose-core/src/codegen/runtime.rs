//! `src/runtime.ts`: what every compiled node does when it runs.
//!
//! [`super::graph`] emits the *composition* — one descriptor per node, the
//! guards, the budgets, the wiring — and this module emits everything those
//! descriptors share: the activity loop of grammar 9, the two provider
//! surfaces, the subprocess and HTTP wrappers, the intra-agent tool loop, and
//! the router of grammar 7.3.
//!
//! It is a **constant**: byte-identical in every project this compiler release
//! builds. Two things follow, and both are the reason it is a whole module
//! rather than text interleaved with the per-composition emission:
//!
//! * a golden diff is about the composition, not about the runtime beside it —
//!   the runtime moves only when the compiler version in the header does;
//! * the file is real TypeScript in the compiler's own tree
//!   (`src/codegen/js/runtime.ts`), so it is edited, read and reviewed as
//!   TypeScript rather than as a Rust string literal.
//!
//! The decisions it carries — why LangGraph's own `retryPolicy`/`timeout` are
//! not used, why the model is called with `fetch` rather than through an SDK,
//! and what an agent node contributes to the conversation channel — are argued
//! in the emitted file's own header, where a reader of a generated project
//! finds them.
//!
//! # The divergence ledger
//!
//! What the *wire* cannot be asked to do, in the ledger style [`super::cel`],
//! [`super::schema`] and [`super::graph`] use: a difference between what the
//! compiler promises and what a provider surface will accept, signed off on
//! rather than re-derived.
//!
//! | id | where | which way | why it is left |
//! |---|---|---|---|
//! | `strict-is-refused-by-an-optional-property` | an agent's `output:` on the Chat Completions surface, when any object in it declares `optional:` | the request carries `strict: false`, so the decoder is **not** constrained by the schema its answer is then parsed with | OpenAI's structured-output decoder closes a schema only when every object in it lists every property in `required` and sets `additionalProperties: false`, all the way down (`crates/mock-provider/WIRE-NOTES.md` (13), which is also what the mock enforces); `optional:` (grammar 3.4) is exactly the construct that breaks the first, and a result schema refuses `default:` (grammar 3.9) so `optional:` is the only way in. The three ways out are all worse. Sending `strict: true` anyway is a 400 — the composition would not run at all. Rewriting the property to the `["string", "null"]`-and-required shape OpenAI documents would make the schema the model is constrained by different from the schema its answer is parsed with, which is the one property PRD 9.16 says must hold and the whole reason `withStructuredOutput` was refused. Refusing the composition at `build` would make a legal grammar unbuildable on three of grammar 12.1's six kinds over a construct the Messages API has no trouble with — `tool_choice` pins the tool and the schema goes on the wire whole. So the parse stays the contract: the emitted node function answers `<agent>Output.parse(…)` over [`super::schema`]'s Zod, which is the same schema at either `strict`, and an unconstrained model that misses it is a node error rather than a bad write. `a_nested_optional_property_costs_the_strict_decoder_and_not_the_parse` pins both halves |
//! | `a-constraint-keyword-the-decoder-cannot-compile` | an agent's `output:` on **every** wire's native rung and on the forced-tool rung of the two OpenAI ones — **and a tool's `parameters`** on the Responses wire, the one surface that declares `strict` on each function it is handed (`loweredStrictTool`; a `tool.*`'s `input:`, a `flow.*` attached as a tool, a store's or a builtin's synthesized arguments alike) | the constraint is **stripped from the request** and folded into that schema node's `description`, so the decoder is not constrained by it and the model is only told about it | PRD §9 resolved q55, and the same doctrine one step out from the row above. A structured-output parameter is a schema *compiler* over a subset of JSON Schema: the Messages wire's `output_config` format compiles no array-length, numeric or string-length constraint, and OpenAI's `strict` decoder compiles none of those and no `uniqueItems`. Grammar D10 makes `max_items` REQUIRED on every result-schema array and §3.5 sends it as `maxItems`, so **every** real composition was a 400 on the rung resolved q53 prefers — and correctly not a laddering one, since the schema was never going to compile on the other rung either. The alternatives the ruling rejects: demoting every array-bearing schema to the forced tool abandons native-first for virtually every composition, and relaxing D10 gives up the bound `over:` dispatch is built on. So the wire is handed the lowering's image (`LOWERED_AWAY` in `js/runtime.ts`, one row per (wire, mechanism), edited when a vendor's subset moves — resolved q30's treadmill terms), each stripped keyword folded into the node's `description` so the model is still aimed at it, and the emitted Zod parse is unchanged and still the contract: an answer that overruns a stripped bound fails `<agent>Output.parse(…)` exactly as any schema-violating answer does, and is never truncated to fit. The delta between the two columns is exactly the table, proven document by document by `the_wire_schema_is_the_lowering_of_the_schema_the_parse_checks` in `tests/generated_code_gates.rs`; the mock provider refuses an under-lowered schema on every enforced row, so an acceptance run is the other half of the proof. A **tool's** `parameters` takes the same projection, on the one wire that promises `strict` over it, and with one consequence the pinned schema does not have: the bound is enforced by `parseToolArguments` rather than by `<agent>Output.parse(…)`, so an argument that overruns it comes back to the model as a Decision D119 refusal and the tool loop turns again — a spent turn of `max_tool_iterations:` rather than a node error. `a_client_tools_schema_is_lowered_where_its_wire_declares_strict` pins which document went out |
//!
//! Both rows are reachable from ordinary grammar, which is why they are pinned
//! rather than left to be discovered: `agent-openai`'s `flow.triage` is the
//! first's shape, and the acceptance test named above asserts the `strict:
//! false` on the recorded request *and* that the run still refuses an answer the
//! schema does not admit. The second's shape is any `output:` with an array in
//! it — `agent-anthropic`'s `agent.tallier`, `agent-openai`'s and
//! `server-tools`' twins of it — and
//! `an_array_bearing_output_rides_each_wires_native_rung_lowered` pins what
//! reaches the wire, `an_answer_over_a_stripped_bound_fails_the_parse` pins that
//! the bound still binds. Its **tool** half is any bounded `input:` on a tool an
//! agent reaches over the Responses wire — `server-tools`' `tool.lookup` carries
//! a `min_length:` — and
//! `a_client_tools_schema_is_lowered_where_its_wire_declares_strict` pins it on
//! both sides: lowered where that wire declares `strict`, and sent whole on a
//! wire that declares none over it. That second side is also what the refusal's
//! own sentence has to say on those two wires — a row named there would be a
//! repair that changes nothing about the document that went out, which
//! `a_schema_keyword_refusal_on_a_wire_that_sends_a_tool_whole_names_no_row`
//! holds it to on a call that pinned nothing and
//! `a_schema_keyword_refusal_on_a_pinned_call_names_the_tool_that_declares_it`
//! on the call that carries **both** documents at once, where naming the pinned
//! schema's row alone would depend on which call of the agent a gateway
//! happened to refuse.

use crate::ir::Ir;

/// The runtime's source, carried in the compiler and emitted verbatim.
///
/// Public because one rule about it is decided outside this crate: the
/// per-(wire, mechanism) lowering tables of PRD §9 resolved q55 are stated here
/// and again in `mock-provider`'s `lowering` module, and
/// `crates/agent-compose/tests/wire_lowering_agreement.rs` — the one test target
/// that can see both crates — asserts the two are equal. Everything else about
/// the file is read through [`module`].
pub const SOURCE: &str = include_str!("js/runtime.ts");

/// `src/runtime.ts`.
#[must_use]
pub fn module(ir: &Ir) -> super::GeneratedFile {
    let mut contents = super::header(ir, "// ");
    contents.push_str(SOURCE);
    super::GeneratedFile {
        path: "src/runtime.ts".to_string(),
        contents,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codegen::test_support::ir_of;

    #[test]
    fn the_module_is_emitted_verbatim_under_the_header() {
        let emitted = module(&ir_of("version: \"0.1\"\n")).contents;
        assert!(emitted.starts_with("// This file was generated by agent-compose"));
        assert!(emitted.contains("export async function runNode("));
        assert!(emitted.ends_with(SOURCE), "the runtime is emitted verbatim");
    }

    /// The compiler's endpoint table and the runtime's fallback are one fact
    /// written twice, in two languages, and this is the seam between them.
    ///
    /// `ProviderKind::default_endpoint` is what makes the credential rule
    /// decidable — "no `base_url:`" means "reaching the vendor" — and the
    /// diagnostic quotes the host. If the emitted `baseUrl` ever fell back
    /// somewhere else, `validate` would be requiring a key for an endpoint the
    /// run does not reach, which is a rule about nothing. Held from the Rust
    /// side because that is the side with the table (grammar 12.1,
    /// Decision D120).
    #[test]
    fn the_default_endpoints_are_the_ones_the_emitted_runtime_falls_back_to() {
        for kind in crate::ast::definition::ProviderKind::ALL {
            let Some(endpoint) = kind.default_endpoint() else {
                continue;
            };
            assert!(
                SOURCE.contains(&format!(
                    "if (provider.kind === \"{}\") return \"{endpoint}\";",
                    kind.as_str()
                )),
                "the emitted runtime does not fall back to `{endpoint}` for `{}`",
                kind.as_str()
            );
        }
    }

    /// A key the composition never declared reaches the wire as no header, not
    /// as an empty one (grammar 12.1, Decision D120).
    ///
    /// Stated over the emitted source because the runtime is a constant this
    /// crate ships: the acceptance suite proves it on a live request, and this
    /// is what fails first if the `?? ""` spelling comes back on either wire.
    #[test]
    fn neither_wire_defaults_an_absent_key_to_an_empty_header() {
        assert!(
            !SOURCE.contains("apiKey ?? \"\""),
            "an absent `api_key:` is a dropped header, never an empty one"
        );
        assert_eq!(
            SOURCE.matches(".apiKey").count(),
            1,
            "every wire reads the key through `credential`, which is the one place that \
             decides whether a header is sent at all"
        );
        assert!(
            SOURCE.contains("...credential(model.provider, \"x-api-key\")"),
            "the Messages wire authenticates through `credential`"
        );
        assert!(
            SOURCE.contains("credential(provider, azure ? \"api-key\" : \"authorization\")"),
            "both OpenAI-shaped wires authenticate through `credential`"
        );
    }

    /// A provider's declared `headers:` are composed **after** the credential
    /// they may replace — the layering the keyless repair rides on.
    ///
    /// D120 drops the vendor credential and puts the gateway's own token in
    /// `headers:` (grammar 12.1, `docs/topics/models.md`), so that map has to
    /// reach the wire beside whatever `credential` composed and win wherever
    /// the two spell the same name: a declared header is the author's word about
    /// the connection, and the credential is this runtime's default for it.
    /// `headerSet` decides that by position alone — later layers win — so the
    /// argument order at the one call site in `send` *is* the rule.
    ///
    /// Pinned here because reordering it is silent everywhere else: a keyless
    /// request's outputs do not change, and the acceptance suite's
    /// `a_provider_with_no_key_sends_no_authentication_header_on_either_wire`
    /// sees only the collision-free case, where an absent credential contributes
    /// no entry for a later layer to overwrite.
    #[test]
    fn a_declared_header_is_composed_after_the_credential_it_may_replace() {
        assert!(
            SOURCE.contains(
                "headerSet({ \"content-type\": \"application/json\" }, headers, model.provider.headers)"
            ),
            "`send` must compose a provider's `headers:` as the last layer, after the credential"
        );
    }

    /// A placed pause's budget is the composition's own `timeout:`, spent from
    /// the moment the wait goes on the hub's board — the same number and the
    /// same instant an unplaced pause's is (`docs/distributed.md` §3.4, PRD
    /// resolved q46).
    ///
    /// The wire's `expires_at` is stamped by the **worker's** clock, which is
    /// not this one's. A timer armed off it would give a `timeout: 5m` node no
    /// time at all on a worker ten minutes behind and a quarter of an hour on
    /// one ten minutes ahead, while the same node unplaced always gets five
    /// minutes. So the descriptor's budget is what is armed.
    ///
    /// **And it is armed whole, every time the wait is planted**, which is the
    /// half a restart decides: a hub that re-derives an unanswered pause plants
    /// it again, exactly as a resumed generation re-parks a local wait nobody
    /// answered, and spending a predecessor's elapsed time out of the budget
    /// would make "how long do I have" an answer a deploy file decides — which
    /// q46's parity bar does not allow. Pinned as a grep because the alternative
    /// is a test that has to move two machines' clocks apart;
    /// `tests/toolchain/human-waits.mjs` drives the behaviour, and
    /// `distributed_hub_wire.rs` drives the restart over real processes.
    ///
    /// **And what it publishes is what it armed.** The `expiresAt` on the board
    /// entry and on the record is derived here, off the instant this planting
    /// spends the budget from — [`runHuman`]'s own line, which is what the
    /// assertion holds it to. Republishing the wire's instant would show a
    /// question as expired while the resume surface still takes its answer, on
    /// the first planting behind a slow worker's clock as surely as on a
    /// re-derivation after a long outage.
    #[test]
    fn a_remote_pauses_budget_is_the_compositions_and_is_armed_whole_whenever_it_is_planted() {
        let held = function_body("export async function holdRemotePause(");
        let local = function_body("export async function runHuman(");
        let armed =
            "timer = setTimeout(() => settle(\"expired\", undefined), descriptor.timeoutMs);";
        assert!(
            local.contains(armed),
            "the unplaced arming this one is held to has moved, so the two are no longer the same \
             line: {local}"
        );
        assert!(
            held.contains(armed),
            "a pause a worker opened is armed with something other than the node's own \
             `timeout:`, whole, from the moment the wait is planted: {held}"
        );
        assert!(
            !held.contains("descriptor.timeoutMs -"),
            "a re-derived pause is armed with what is left of a predecessor's budget rather than \
             with the node's own, so a hub restart costs a person time the same wait unplaced \
             would have given them (PRD resolved q46): {held}"
        );
        let dated = ": new Date(began + descriptor.timeoutMs).toISOString();";
        assert!(
            local.contains(dated),
            "the unplaced derivation this one is held to has moved, so the two are no longer the \
             same line: {local}"
        );
        assert!(
            held.contains(dated),
            "a pause a worker opened publishes a deadline derived from something other than the \
             instant its own timer is armed from, so a reader is shown an expiry that is not the \
             one that will fire: {held}"
        );
        assert!(
            !held.contains("remote.expiresAt"),
            "the wire's `expires_at` is read where the wait is planted — armed off, or republished \
             onto the board or the record — so another machine's clock decides how long a \
             `timeout:` lasts or what a status route says about it (docs/distributed.md §3.4): \
             {held}"
        );
    }

    /// A planted pause is dated by the planting, off the same reading its
    /// deadline is armed from (`docs/distributed.md` §3.4, PRD resolved q46).
    ///
    /// The other half of the rule above, and the one PRD resolved q46 names
    /// "status visibility": a wait a process opens is dated when that process
    /// opens it, so the pair a status route publishes is the node's `timeout:`
    /// apart. A hub that kept the wire's `paused_at` beside a deadline of its own
    /// would publish two machines' readings — an interval of nothing, negative
    /// once a worker's lead passes the budget — and would journal, on a worker
    /// running ahead of it, the entry `docs/durability.md` §3.4 refuses by name:
    /// one whose answer arrives before its question.
    ///
    /// Pinned as a grep for the reason above it, and against [`runHuman`]'s own
    /// line so that the two datings cannot drift apart;
    /// `tests/toolchain/human-waits.mjs`'s `remote_planting` block and
    /// `distributed_hub_wire.rs` drive the behaviour itself.
    #[test]
    fn a_remote_pause_is_dated_by_the_planting_that_arms_it() {
        let held = function_body("export async function holdRemotePause(");
        let local = function_body("export async function runHuman(");
        let dated = "const pausedAt = new Date(began).toISOString();";
        assert!(
            local.contains(dated),
            "the unplaced dating this one is held to has moved, so the two are no longer the same \
             line: {local}"
        );
        assert!(
            held.contains(dated),
            "a pause a worker opened is dated by something other than the instant this hub plants \
             it at, so the pair a status route publishes is not the node's `timeout:` apart the \
             way the same node unplaced publishes it: {held}"
        );
        assert!(
            !held.contains("remote.pausedAt"),
            "the wire's `paused_at` is republished onto the board or the record, so a reader is \
             shown a pair read off two machines' clocks and a fast worker's pause journals an \
             answer that arrives before its question (docs/distributed.md §3.4): {held}"
        );
    }

    /// The answer to a pause a worker opened is journaled **inside** the
    /// settlement, where a local pause's is (`docs/durability.md` §3.4).
    ///
    /// [`runHuman`]'s `slot.keep` runs before its promise resolves, and the
    /// reason is stated at length there: the resume route answers `202` off the
    /// settlement, so a record written in a later turn of the event loop is one
    /// a process killed in between never wrote — leaving a wait this board has
    /// settled with nothing in the journal, which the next start re-derives off
    /// the settled dispatch row and asks a second time. The hub's writer is
    /// handed in as `keep` for exactly that ordering, and a write that throws
    /// fails the node rather than hanging it.
    #[test]
    fn a_remote_pauses_answer_is_journaled_before_its_promise_resolves() {
        let held = function_body("export async function holdRemotePause(");
        let kept = held
            .find("keep(settled);")
            .expect("`holdRemotePause` writes the record through the caller's `keep`");
        let resolved = held
            .find("resolve(settled);")
            .expect("`holdRemotePause` resolves with the record it wrote");
        assert!(
            kept < resolved,
            "the record is written after the promise resolves, so the `202` the resume route \
             answers can precede the write it acknowledges: {held}"
        );
    }

    /// A `builtin.bash` command's stop sweep is armed **before** the shell is
    /// forked (grammar 5.5, Decision D124).
    ///
    /// A stop signal has a *disposition* before it has a listener: until the
    /// first `process.on` for it, the platform installs no handler and the
    /// kernel's default ends this process where it stands. So a shell forked
    /// before the arming is a shell whose group a `Ctrl-C` in the window between
    /// the two would leave running, with the graph that started it gone — the
    /// orphan D124 exists to close, reached at the one moment it is easiest to
    /// reach. Armed first, the window cannot reopen: a listener runs between
    /// turns of the event loop, and the spawn and the `holdCommand` beside it
    /// are one turn.
    ///
    /// Pinned as an **order** rather than as a behaviour because the behaviour
    /// is a race: the acceptance suite's
    /// `a_run_asked_to_stop_takes_its_command_with_it` sends its signal as soon
    /// as the command's own marker appears, and on an idle machine the runtime
    /// wins that race whichever way round these two lines are — it lost it under
    /// load, which is how the window was found. What is decidable here is the
    /// ordering that makes the race unreachable.
    #[test]
    fn a_commands_stop_sweep_is_armed_before_the_shell_is_forked() {
        let forked = function_body("function forkBoundShell(");
        let armed = forked
            .find("armCommandSweep();")
            .expect("`forkBoundShell` arms the stop sweep");
        let spawned = forked
            .find("return spawn(executable, SHELL_ARGUMENTS, {")
            .expect("`forkBoundShell` forks the shell");
        assert!(
            armed < spawned,
            "the sweep is armed after the shell is forked, so a stop signal arriving in between \
             is met by the kernel's default disposition: the graph ends and the shell's process \
             group is left running inside the workspace (grammar 5.5, Decision D124): {forked}"
        );
        // …and the fork has **one** call site, which is what makes the ordering
        // above a property of the runtime rather than of one function: a second
        // `spawn` of the shell would carry its own ordering, and the window this
        // closes is reopened by whichever one forgets.
        assert_eq!(
            SOURCE.matches("SHELL_ARGUMENTS, {").count(),
            1,
            "a `builtin.bash` shell is forked somewhere other than `forkBoundShell`, which is the \
             one call site that arms the stop sweep first"
        );
    }

    /// A typed command reads **its own** standard input, never the shell's
    /// (grammar 5.5, PRD resolved q54).
    ///
    /// The one place this runtime's protocol and the model's program share a
    /// channel. `builtin.bash` is a session, so the shell reads its script from
    /// standard input (`-s`) and each command is *typed* into that pipe followed
    /// by the marker lines that close it — which means a command that reads
    /// standard input reads them: `read -r line` consumes the `…_status=$?` line
    /// and the call settles with no status, and `cat` consumes both `printf`s,
    /// spends the whole `timeout:` and answers the model with this runtime's own
    /// marker text as the command's output. Neither is a failure a test of the
    /// tool's *behaviour* would notice, because both look exactly like a command
    /// that did what it was asked.
    ///
    /// So it is pinned as text: the command runs inside a brace group whose
    /// standard input is `/dev/null`. A group rather than a subshell, because
    /// `cd build` has to still be true for the next call, which is the whole of
    /// what a session is.
    #[test]
    fn a_typed_command_is_redirected_away_from_the_shells_own_input() {
        let typed = function_body("async function typeIntoShell(");
        assert!(
            typed.contains(r"`{ ${command}\n") && typed.contains(r"\n} < /dev/null\n"),
            "the command is typed into the shell without a standard input of its own, so a \
             command that reads one reads this runtime's marker protocol instead (grammar 5.5, \
             PRD resolved q54): {typed}"
        );
    }

    /// Every `builtin.files` write goes through the hard-link check
    /// (PRD resolved q54, Decision D119).
    ///
    /// `targetWithinWorkspace` answers "is this path inside the workspace" by
    /// resolving it, which is the whole answer for a symbolic link and no answer
    /// at all for a hard one: a second name for an inode has no target, so
    /// `realpath` reports the path inside the workspace and a write through it
    /// changes what a name outside reads. The refusal therefore lives at the
    /// write rather than at the resolution — and a second `writeFile` reached
    /// from anywhere else would be a write with no such refusal in front of it,
    /// which is what this pins.
    #[test]
    fn every_file_write_is_preceded_by_the_second_name_refusal() {
        assert_eq!(
            SOURCE.matches("fs.promises.writeFile(").count(),
            1,
            "`builtin.files` writes a file somewhere other than `writeFileText`, which is the \
             one call site the hard-link refusal stands in front of"
        );
        let written = function_body("async function writeFileText(");
        let refused = written
            .find("await refuseSecondName(operation, requested, target);")
            .expect("`writeFileText` refuses a target with a second name");
        let wrote = written
            .find("await fs.promises.writeFile(target, contents, \"utf8\");")
            .expect("`writeFileText` writes the file");
        assert!(
            refused < wrote,
            "the hard-link refusal runs after the write it is meant to prevent: {written}"
        );
    }

    /// **Every** composer puts the *lowered* schema on the wire, and none of
    /// them reaches the emitted one (PRD §9 resolved q55, ruling a).
    ///
    /// The projection is only a contract if it has no way around it. A composer
    /// that read `request.pinned.schema` directly would send the schema whole —
    /// on the Messages wire that is the 400 the ruling was written from, and on
    /// the OpenAI wires it is a `strict` decoder refusing a keyword it does not
    /// compile — and nothing else in this crate would notice: the goldens carry
    /// the full schema in `deployment.ts` either way, because lowering happens
    /// at request composition and not at codegen.
    ///
    /// So the rule is pinned as *both* halves, per composer: the lowered pin is
    /// derived, and the raw one is not reachable inside the function. The
    /// acceptance suite proves the behaviour against a mock that enforces each
    /// subset; this is what fails first, with no toolchain, when a composer
    /// stops projecting.
    #[test]
    fn every_wire_composes_its_structured_output_from_the_lowered_schema() {
        for (composer, wire) in [
            ("async function callMessages(", "messages"),
            ("async function callChatCompletions(", "chat_completions"),
            ("async function callResponses(", "responses"),
        ] {
            let body = function_body(composer);
            assert!(
                body.contains(&format!(
                    "loweredPin(request.pinned, \"{wire}\", mechanism)"
                )),
                "`{composer}…` does not project its pinned schema through the `{wire}` lowering \
                 table (PRD resolved q55): {body}"
            );
            assert!(
                !body.contains("request.pinned.schema") && !body.contains("request.pinned!.schema"),
                "`{composer}…` reaches the emitted schema directly, so a request can carry a \
                 keyword this wire's decoder cannot compile: {body}"
            );
        }
    }

    /// The lowering tables carry the keyword grammar D10 makes unavoidable, and
    /// the Messages forced-tool row is empty (PRD §9 resolved q55).
    ///
    /// Stated from the Rust side because the table is the whole of the ruling's
    /// mechanics and the two things asserted here are the two that make it
    /// *matter*: `maxItems` is on every result-schema array by grammar D10, and
    /// the empty row is a claim about a rung that takes the schema whole rather
    /// than an absence of one. `crates/agent-compose/tests/wire_lowering_agreement.rs`
    /// holds the tables to the mock provider's enforced subsets, keyword by
    /// keyword; this is what fails first if the table stops being a table.
    #[test]
    fn the_lowering_tables_are_where_the_ruling_puts_them() {
        assert!(
            SOURCE.contains("const ANTHROPIC_NATIVE_UNCOMPILED: readonly string[] = ["),
            "the Messages wire's lowering table is gone"
        );
        assert!(
            SOURCE.contains("const OPENAI_STRICT_UNCOMPILED: readonly string[] = ["),
            "the OpenAI wires' lowering table is gone"
        );
        assert!(
            SOURCE.contains("messages: { native: ANTHROPIC_NATIVE_UNCOMPILED, forced_tool: [] },"),
            "the Messages forced-tool row is no longer the empty one: a schema rides that rung as \
             a tool's `input_schema`, which the API takes whole (PRD resolved q55)"
        );
    }

    /// The body of one top-level declaration of the runtime.
    ///
    /// The same reader `codegen::mesh` and `codegen::serve` use, for the same
    /// reason: a rule about what one function does is only a rule if it is read
    /// off that function rather than off the file around it. The module is
    /// formatted, so a top-level declaration opens at column zero and closes on
    /// a line that is exactly `}`.
    fn function_body(header: &str) -> String {
        let mut lines = SOURCE.lines().skip_while(|line| !line.starts_with(header));
        let opened = lines
            .next()
            .unwrap_or_else(|| panic!("`src/runtime.ts` declares `{header}…`"));
        let mut held = String::from(opened);
        for line in lines {
            held.push('\n');
            held.push_str(line);
            if line == "}" {
                return held;
            }
        }
        panic!("`{header}…` has no closing brace in the first column")
    }

    /// The runtime is the same bytes for every composition: a project that
    /// declares nothing and one that declares everything differ in `graph.ts`,
    /// and this is what makes that true.
    #[test]
    fn every_composition_gets_the_same_runtime() {
        let empty = module(&ir_of("version: \"0.1\"\n")).contents;
        let full = module(&ir_of(crate::codegen::test_support::EVERY_FORM)).contents;
        assert_eq!(
            empty.replace("main.yml", ""),
            full.replace("main.yml", ""),
            "the runtime differs between two compositions"
        );
    }
}
