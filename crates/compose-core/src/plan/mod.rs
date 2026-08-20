//! `agent-compose plan`: what changed between two specs, as data.
//!
//! PRD §2 states the problem this answers — "reviewing what changed in the
//! topology requires reading router functions" — and PRD §7 M2 names the
//! answer: a topology and validation diff between two specs. What that means
//! concretely is one report with four sections, produced by [`plan`] and
//! specified, normatively, by `docs/plan.md`:
//!
//! | Section | The question it answers |
//! |---|---|
//! | components | what does this composition *have* that the other does not, and what is different about the ones both have |
//! | topology | what moved inside a flow's graph, in the state channels, and in the policy defaults |
//! | interfaces | what does a **caller** feel: a flow's declared I/O, a trigger's route and response mode |
//! | validation | what is the compiler now saying that it was not, and what has it stopped saying |
//!
//! # The diff is over the artifact, not the files
//!
//! Both sides are resolved to the flat IR first (PRD 5.1), and the comparison
//! is over that. Multi-file composition is authoring UX, so a definition moved
//! between files, an `imports:` list reordered, a comment, a reflowed mapping,
//! and two definitions swapped in one file are all changes to *files* and to
//! nothing the composition does — and a plan that reported them would be a
//! `git diff` with extra steps. `diff`'s module docs are where the one
//! consequence of that is worked out: source regions come out of the comparison
//! before anything is compared.
//!
//! What the IR does not flatten away, this reports. Two specs whose artifacts
//! agree in everything but their source coordinates produce an empty plan,
//! byte for byte, every time.
//!
//! **That cuts both ways, and the first place it is worth knowing about is a
//! leaf the artifact keeps as source text.** A duration, a CEL expression and
//! the integer-or-float spelling of a number reach the IR as the author wrote
//! them, so `timeout: 120s` against `timeout: 2m` is a reported change on a pair
//! that builds to a byte-identical project. It is the narrow reading of "the
//! diff is over the artifact" rather than an oversight — `diff`'s module docs
//! work out why the alternative is worse, `docs/plan.md` §11 states it for a
//! reader of the format, and `tests/projects/plan/respelled` pins all three.
//!
//! **The second is a default written out.** The IR materializes none of them
//! (`crate::ir`), so a key written with the value the grammar already supplies
//! is a key the artifact holds where the absent one is not: writing
//! `max_tool_iterations: 8` out reports, on a pair that again builds
//! byte-identically. Normalizing it away would need a table of keys and their
//! defaults consulted by key, over `settings:` and `default:` as well — the rule
//! `diff`'s module docs refuse, for the reason they give. `docs/plan.md` §11
//! states this residual too, and `tests/projects/plan/restated-defaults` pins
//! four keys and both builds.
//!
//! # Check diagnostics are content, not a refusal
//!
//! [`plan`] runs the **whole** check phase over both sides, and the difference
//! between the two reports is the fourth section. A composition the validator
//! rejects still has an artifact — resolution is what decides whether there is
//! one — so "the after spec introduces three errors" is exactly the sentence a
//! plan exists to say, and the command exits `0` having said it.
//!
//! Resolution is the other half of that sentence: a spec that does not parse or
//! does not resolve has no artifact at all, and there is nothing to compare.
//! That is what [`Refusal`] carries, and it is the one outcome where the
//! command has no plan to write.

mod diff;
mod document;
mod sections;

use crate::diag::Diagnostic;
use crate::ir::Ir;

pub use document::{
    ChangeKind, ComponentChange, ComponentKind, FieldChange, Finding, InterfaceChange,
    InterfaceKind, PLAN_VERSION, Plan, Refusal, Refused, Spec, SpecSide, TopologyChange,
    TopologyKind, Validation,
};

/// One side of a plan: a resolved composition, and how it was named.
///
/// `resolution` is what resolving it reported *alongside* the artifact, which is
/// warnings — a composition with an error has no artifact
/// (`crate::resolve`). It joins the check phase's own report in the
/// validation section, so a warning that appeared or went away is a change like
/// any other rather than something only `validate` can see.
#[derive(Clone, Copy, Debug)]
pub struct Composition<'a> {
    /// The entrypoint the command resolved — the file it was given, or the
    /// `main.yml` inside the directory it was given — which is what the plan
    /// reports this side by. The IR's own `entrypoint` is relative to the
    /// project root and is therefore `main.yml` on both sides of most
    /// comparisons.
    pub entrypoint: &'a str,
    /// The resolved artifact.
    pub ir: &'a Ir,
    /// What resolving it reported.
    pub resolution: &'a [Diagnostic],
}

/// Diff two resolved compositions.
///
/// Deterministic: the same pair produces the same document, byte for byte. Each
/// section is sorted by the address of what it is about, which is a property of
/// the artifact rather than of the order anything was visited in — see
/// `docs/plan.md` §Ordering.
#[must_use]
pub fn plan(before: Composition<'_>, after: Composition<'_>) -> Plan {
    Plan {
        plan_version: PLAN_VERSION,
        before: side(&before),
        after: side(&after),
        components: sections::components(before.ir, after.ir),
        topology: sections::topology(before.ir, after.ir),
        interfaces: sections::interfaces(before.ir, after.ir),
        validation: sections::validation(&before, &after),
    }
}

fn side(composition: &Composition<'_>) -> Spec {
    Spec {
        entrypoint: composition.entrypoint.to_string(),
        target: composition.ir.target.clone(),
        spec_version: composition.ir.spec_version.clone(),
    }
}
