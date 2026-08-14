# CLAUDE.md

## Project

`agent-compose` is a declarative YAML DSL for agent graphs, compiled to LangGraph **TypeScript** by a **Rust** CLI/compiler. `prd.md` is the single source of truth for design decisions.

**PRD discipline**: new design questions land in the PRD's Open Questions section and must be resolved (moved to the Resolved Questions log, with rationale in the relevant section) before implementing the affected area. Never implement against an unresolved question.

## Stack

- **Compiler/CLI**: Rust, shipped as a single static binary (`agent-compose`). Parse → resolve → validate → codegen, no language-runtime dependency. `validate` must stay in the millisecond budget.
- **Codegen target**: TypeScript + LangGraph JS, pinned version per compiler release. Node.js is needed only to run generated output.
- **CEL**: Rust `cel` crate at validate time; JS CEL evaluator in generated routers; a shared conformance fixture corpus runs against both in CI (PRD §8).

## Git & PR conventions

- **Conventional Commits** for every commit message *and* every PR title (`feat:`, `fix:`, `docs:`, `chore:`, `refactor:`, `test:`, `ci:`, `build:`). PR titles matter: squash merges use the PR title as the commit subject.
- **Merging**: squash-and-merge by default; rebase-and-merge for stacked PRs. Never create merge commits.
- **Stacks**: each PR in a stack targets the PR below it; land bottom-up with rebase-and-merge.

## Operating model

For any work of **medium-to-high complexity or size**, the main session acts as **lead**: it orchestrates and reviews, it does not hand-implement. Delegate to **Opus 5 agents at `xhigh` reasoning effort**:

1. **Implement** — an implementer agent iterates on the task until it believes it is done (tests written and passing).
2. **Clean-room adversarial review** — a separate agent with *no implementer context* reviews the result and actively tries to break it: correctness, spec conformance, missed cases, weak tests.
3. **Fix** — findings go to a fix agent.
4. Repeat 2–3 until the review converges (no material findings).
5. **Lead final review** — the lead reviews the converged result itself.
   - Big problems → send the work through the full loop again (steps 1–4).
   - Small fixes → delegate a single Opus fix agent, then re-verify.

Small or mechanical tasks (doc edits, config tweaks, trivial fixes) may be done directly by the lead.

## Validation philosophy

Unit tests are necessary but not a catch-all. The full strategy lives in PRD §8; the essentials:

- Every static check gets positive/negative fixture specs; error messages are asserted exactly (error UX is a product feature).
- Codegen is verified by golden-file tests (byte-identical output) plus type-checking and constructing the generated graphs.
- A CEL conformance corpus keeps the Rust and JS interpreters in lockstep.
- End-to-end (from M1): a **mock provider server** — scripted structured-output responses per model via a control endpoint — so compiled graphs execute for real in CI with no API keys.
- Every milestone's acceptance criteria must be machine-checkable (CI green ⇒ done) so work can proceed autonomously over long stretches. Define the acceptance test suite before starting a milestone's implementation.
