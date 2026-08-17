//! Grammar 9.3's resolution chain, applied.
//!
//! The IR is deliberately **faithful rather than convenient** (see [`crate::ir`]):
//! an agent that declares no `timeout:` has none in the artifact, because
//! resolving the chain there would leave nothing able to tell an author's
//! `timeout: 60s` from an invented one. Codegen is the pass that applies it, and
//! this module is that application — one [`Resolved`] per node, with the level
//! each field came from carried alongside so the emitted code can say so in a
//! comment.
//!
//! # The four levels (grammar 9.3)
//!
//! | level | source | where it is applied |
//! |---|---|---|
//! | 1 | the instantiating `flow:` node's `policy:` | **at run time**, by `runtime.effectivePolicy` over the instance's own `$run.policy` |
//! | 2 | the node's own `retry`/`timeout`/`on_error` | here |
//! | 3 | the composition's `defaults:` | here |
//! | 4 | built-in: no retry, no timeout, `on_error: fail` | here |
//!
//! **Level 1 is not this module's**, and that is a property of the level rather
//! than a gap. It belongs to the *instantiation site*, and one flow may be
//! instantiated from several: `flow.review_loop` under `policy: { timeout: 30s }`
//! at one `flow:` node and under nothing at another compiles to one set of node
//! descriptors either way. So [`super::graph`] emits the override as data on the
//! instantiating node, the instance carries it in `$run`, and the runtime lays it
//! over the [`Resolved`] levels below — winning per field, and withheld from a
//! `human` node's `timeout` and `retry` exactly as level 3 is (Decision D102).
//! When two instantiation sites in a nesting chain set one field, the
//! **outermost** wins (`runtime.instancePolicy`, Decision D79), and a `map`
//! contributes no level 1 at all: it has no `policy:` key, so a dispatched
//! subflow's nodes see level 2 downward (grammar 8.6 rule 10).
//!
//! # The `human` exemption (Decision D102)
//!
//! A `human` node resolves **no** `timeout` and **no** `retry`, at any level —
//! grammar 8.7 makes the two keys illegal on the node itself, and levels 1 and 3
//! are skipped for them too, because a wait is not an activity timeout and
//! re-prompting a human is not a retry. `on_error` is not exempt and resolves
//! through the whole chain, since it covers delivery failures.

use crate::ast::common::{ControlTarget, Duration};
use crate::ir::Ir;
use crate::ir::flow::{Node, NodeKind};
use crate::ir::policy::{OnError, Policy, Retry};

/// Which level of grammar 9.3's chain a resolved field came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    /// Level 2: the node's own key.
    Node,
    /// Level 3: the composition's `defaults:`.
    Defaults,
    /// Level 4: the built-in.
    BuiltIn,
    /// The field is exempt from the chain entirely (Decision D102).
    Exempt,
}

impl Level {
    /// How a generated comment names it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Node => "the node",
            Self::Defaults => "`defaults:`",
            Self::BuiltIn => "the built-in",
            Self::Exempt => "exempt (Decision D102)",
        }
    }
}

/// One node's policy, with the level each field was resolved at.
#[derive(Clone, Debug, PartialEq)]
pub struct Resolved<'ir> {
    /// The retry block, and where it came from.
    pub retry: (Option<&'ir Retry>, Level),
    /// The timeout, and where it came from.
    pub timeout: (Option<&'ir Duration>, Level),
    /// The error strategy, and where it came from.
    pub on_error: (Strategy<'ir>, Level),
}

/// The `on_error` strategy a node resolves to (grammar 9.2).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Strategy<'ir> {
    /// Abort the execution with the node's error.
    Fail,
    /// Produce no output and write nothing; route through grammar 7.3 with one
    /// substitution (Decision D97).
    Skip,
    /// Schedule this target instead of evaluating the node's own edges.
    Fallback(&'ir ControlTarget),
}

/// Resolve one node's policy against the composition's `defaults:`.
#[must_use]
pub fn resolve<'ir>(ir: &'ir Ir, node: &'ir Node) -> Resolved<'ir> {
    let defaults: Option<&Policy> = ir.defaults.as_ref().map(|declared| &declared.value);
    let exempt = matches!(node.kind, NodeKind::Human { .. });

    let retry = if exempt {
        (None, Level::Exempt)
    } else if let Some(retry) = node.policy.retry.as_ref() {
        (Some(retry), Level::Node)
    } else if let Some(retry) = defaults.and_then(|policy| policy.retry.as_ref()) {
        (Some(retry), Level::Defaults)
    } else {
        (None, Level::BuiltIn)
    };

    let timeout = if exempt {
        (None, Level::Exempt)
    } else if let Some(timeout) = node.policy.timeout.as_ref() {
        (Some(&timeout.value), Level::Node)
    } else if let Some(timeout) = defaults.and_then(|policy| policy.timeout.as_ref()) {
        (Some(&timeout.value), Level::Defaults)
    } else {
        (None, Level::BuiltIn)
    };

    let on_error = match node.policy.on_error.as_ref() {
        Some(on_error) => (strategy(on_error), Level::Node),
        None => match defaults.and_then(|policy| policy.on_error.as_ref()) {
            Some(on_error) => (strategy(on_error), Level::Defaults),
            None => (Strategy::Fail, Level::BuiltIn),
        },
    };

    Resolved {
        retry,
        timeout,
        on_error,
    }
}

fn strategy(on_error: &OnError) -> Strategy<'_> {
    match on_error {
        OnError::Fail { .. } => Strategy::Fail,
        OnError::Skip { .. } => Strategy::Skip,
        OnError::Fallback { target, .. } => Strategy::Fallback(&target.value),
    }
}

/// A duration in milliseconds, which is what every JavaScript timer takes.
///
/// Saturating at [`u64::MAX`] is [`Duration::as_millis`]'s, and this narrows to
/// the range a `number` holds exactly: past 2^53 a JavaScript timer cannot
/// count the milliseconds anyway, and grammar 4.4 puts no ceiling on the
/// integer, so a duration written past it is emitted as the largest budget the
/// runtime can hold rather than as a rounded one.
#[must_use]
pub fn milliseconds(duration: &Duration) -> u64 {
    duration.as_millis().min(1_u64 << 53)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codegen::test_support::ir_of;
    use crate::ir::definition::DefinitionBody;

    fn node_of<'ir>(ir: &'ir Ir, flow: &str, id: &str) -> &'ir Node {
        let definition = ir.definition(flow).expect("the flow is declared");
        let DefinitionBody::Flow(flow) = &definition.body else {
            panic!("not a flow");
        };
        flow.nodes
            .iter()
            .find(|node| node.id.value.as_str() == id)
            .expect("the node is declared")
    }

    const PROJECT: &str = r#"version: "0.1"

defaults:
  timeout: 30s
  retry: { max: 1, backoff: 2s }
  on_error: fail

provider.p:
  kind: anthropic
  api_key: ${K}

model.m:
  provider: provider.p
  id: some-model

agent.a:
  model: model.m
  prompt: p
  output: { verdict: { enum: [ok] } }

flow.f:
  outputs: {}
  nodes:
    plain: { agent: agent.a, input: "'x'" }
    tuned:
      agent: agent.a
      input: "'x'"
      retry: { max: 3, backoff: 5s }
      timeout: 90s
      on_error: { fallback: rescue }
    rescue: { agent: agent.a, input: "'x'", on_error: skip }
    ask:
      human:
        input: { q: { type: string } }
        output: { decision: { enum: [ok] } }
        timeout: 24h
        on_timeout: end
  edges:
    - { from: start, to: plain }
    - { from: plain, to: tuned }
    - { from: tuned, to: ask }
    - { from: ask, to: rescue }
    - { from: rescue, to: end }
"#;

    #[test]
    fn a_node_that_declares_nothing_takes_the_defaults() {
        let ir = ir_of(PROJECT);
        let resolved = resolve(&ir, node_of(&ir, "flow.f", "plain"));
        assert_eq!(resolved.timeout.1, Level::Defaults);
        assert_eq!(milliseconds(resolved.timeout.0.expect("a timeout")), 30_000);
        assert_eq!(resolved.retry.1, Level::Defaults);
        assert_eq!(resolved.retry.0.expect("a retry").max, 1);
        assert_eq!(resolved.on_error, (Strategy::Fail, Level::Defaults));
    }

    #[test]
    fn a_nodes_own_keys_outrank_the_defaults() {
        let ir = ir_of(PROJECT);
        let resolved = resolve(&ir, node_of(&ir, "flow.f", "tuned"));
        assert_eq!(resolved.timeout.1, Level::Node);
        assert_eq!(milliseconds(resolved.timeout.0.expect("a timeout")), 90_000);
        assert_eq!(resolved.retry.0.expect("a retry").max, 3);
        assert!(matches!(
            resolved.on_error,
            (Strategy::Fallback(_), Level::Node)
        ));
    }

    /// Decision D102: the two keys are withheld from a `human` node at every
    /// level, so a composition-wide budget cannot cut a wait short.
    #[test]
    fn a_human_node_resolves_no_timeout_and_no_retry() {
        let ir = ir_of(PROJECT);
        let resolved = resolve(&ir, node_of(&ir, "flow.f", "ask"));
        assert_eq!(resolved.timeout, (None, Level::Exempt));
        assert_eq!(resolved.retry, (None, Level::Exempt));
        // …and `on_error` is not exempt: it covers delivery failures.
        assert_eq!(resolved.on_error, (Strategy::Fail, Level::Defaults));
    }

    #[test]
    fn a_composition_with_no_defaults_falls_through_to_the_built_in() {
        let ir = ir_of(
            r#"version: "0.1"

provider.p:
  kind: anthropic
  api_key: ${K}

model.m:
  provider: provider.p
  id: some-model

agent.a:
  model: model.m
  prompt: p
  output: { verdict: { enum: [ok] } }

flow.f:
  outputs: {}
  nodes:
    only: { agent: agent.a, input: "'x'" }
  edges:
    - { from: start, to: only }
    - { from: only, to: end }
"#,
        );
        let resolved = resolve(&ir, node_of(&ir, "flow.f", "only"));
        assert_eq!(resolved.retry, (None, Level::BuiltIn));
        assert_eq!(resolved.timeout, (None, Level::BuiltIn));
        assert_eq!(resolved.on_error, (Strategy::Fail, Level::BuiltIn));
    }
}
