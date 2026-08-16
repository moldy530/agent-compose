# CLAUDE.md

## Project

`agent-compose` is a declarative YAML DSL for agent graphs, compiled to LangGraph **TypeScript** by a **Rust** CLI/compiler. `prd.md` is the single source of truth for design decisions.

**PRD discipline**: new design questions land in the PRD's Open Questions section and must be resolved (moved to the Resolved Questions log, with rationale in the relevant section) before implementing the affected area. Never implement against an unresolved question.

## Stack

- **Compiler/CLI**: Rust, shipped as a single static binary (`agent-compose`). Parse → resolve → validate → codegen, no language-runtime dependency. `validate` must stay in the millisecond budget.
- **Codegen target**: TypeScript + LangGraph JS, pinned version per compiler release. Node.js is needed only to run generated output.
- **CEL**: Rust `cel` crate at validate time; JS CEL evaluator in generated routers; a shared conformance fixture corpus runs against both in CI (see Validation strategy).

## Git & PR conventions

- **Conventional Commits** for every commit message *and* every PR title (`feat:`, `fix:`, `docs:`, `chore:`, `refactor:`, `test:`, `ci:`, `build:`). PR titles matter: squash merges use the PR title as the commit subject.
- **Merging**: squash-and-merge by default; rebase-and-merge for stacked PRs. Never create merge commits.
- **Stacks**: each PR in a stack targets the PR below it; land bottom-up with rebase-and-merge.

## Operating model

For any work of **medium-to-high complexity or size**, the main session acts as **lead**: it authors the spec, supervises, reviews, and owns the PR — it does not hand-implement. This is a standing directive: multi-agent orchestration is the expected mode for such work. The loop:

1. **Implement** — an Opus 5 agent at `xhigh` effort builds the goal in an isolated worktree from a self-contained spec.
2. **Clean-room adversarial review** — a fresh Opus-xhigh agent with *no implementer context* reviews the full diff and actively tries to break it (correctness, PRD conformance, weak tests).
3. **Fix** — an Opus-xhigh fixer applies verified findings.
4. Steps 2–3 repeat until a review round converges (no actionable findings). Three rounds is the default budget, not a ceiling: when a round still surfaces new substantive findings, keep looping — convergence is the exit condition, not the round count. Stop extending only when a round comes back clean, findings have decayed to nits, or rounds stop making progress (the same findings recurring, or fix-churn without improvement — that means the goal needs restructuring, not more rounds).
5. **Lead final review** — the lead reviews the converged result itself and re-runs the gates.
   - Big problems → send the work through the full loop again.
   - Small fixes → delegate a single Opus fix agent, then re-verify.

**Mechanism**: plain `Agent`-tool subagents inherit the session's model/effort — per-call effort cannot be raised that way, and prose instructions to a subagent do not change its effort. Run the loop as a `Workflow` instead, authored ad hoc per goal, with every `agent()` call setting `{model: 'opus', effort: 'xhigh'}` explicitly (plus `isolation: 'worktree'` for stages that mutate files).

Details that make the loop actually work:

- **The spec is self-contained.** The implementer and reviewers are clean-room: every file path, PRD section, decision, and gate command they need goes *in* the spec text embedded in the workflow script. Reviewers get the diff + the spec, nothing else.
- **Structured findings.** Review stages return a findings schema (`file`, `line`, `severity: must-fix | should-fix | nit`, `summary`, `failure_scenario`); a round with zero actionable (non-nit) findings is convergence.
- **Verbatim gates.** The spec names exact verification commands, and the final report must quote test names/counts verbatim so invented results are spottable. Audit fixer claims that dismiss findings ("stale", "not present") with a direct grep before accepting them.

Small or mechanical tasks (doc edits, config tweaks, trivial fixes) may be done directly by the lead.

## Validation strategy

Unit tests are necessary but not a catch-all. Validation must be strong enough that milestone work is machine-checkable end-to-end: this project is built to be worked on autonomously over long stretches, so "CI green" has to mean "actually done" — no manual verification gates.

- **Compiler unit + snapshot tests (M0)**: every static check gets positive and negative fixture specs; parser/resolver output is locked with IR snapshot tests. The invalid-spec corpus asserts on exact error messages — error UX is a product feature (PRD G3), and a regression in an error message is a test failure.
- **Golden-file codegen tests (M1)**: same DSL in → byte-identical TypeScript out; goldens are reviewed in PRs like any other code.
- **Generated-code checks (M1)**: every golden fixture must type-check (`tsc`) and construct its graph under the pinned LangGraph version.
- **CEL conformance corpus**: shared fixtures (expression + input + expected result) executed against both the Rust validator's interpreter and the JS evaluator embedded in generated code. Divergence fails CI.
- **Mock provider server (M1)**: e2e without API keys. A local server implementing the provider wire surface, with a control endpoint to enqueue scripted responses per model ("next call to `model.smart` returns this structured output"). Compiled graphs then run for real — triggers, routing, cycles, fan-out joins, store ops, and model failover (scripted rate-limit responses) — deterministically in CI. This is the primary acceptance harness from M1 onward.
- **Milestone acceptance is a test suite**: each milestone defines its acceptance criteria as runnable tests before implementation of that milestone begins.
