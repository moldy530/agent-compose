//! The plan document: the record types `agent-compose plan --format json`
//! writes.
//!
//! `docs/plan.md` is the normative account of every one of them, and
//! `crates/compose-core/tests/plan_format_inventory.rs` is that account made
//! executable — a field declared here and not written down there is a test
//! failure, because the document is what a consumer pins `plan_version` on.
//!
//! Every record is a plain struct of named fields, and every vocabulary a
//! unit-variant enum. That is a deliberate constraint rather than an accident of
//! what the sections needed: the shape a reader of this format meets is one
//! fixed record per section, which is the same promise
//! [`diag`](crate::diag)'s serialization makes, and it is what lets the
//! inventory read these declarations at all.

use serde::Serialize;
use serde_json::Value;

use crate::diag::{Diagnostic, DiagnosticCode, Severity, Span};

/// The shape of the document this module emits.
///
/// Independent of [`IR_VERSION`](crate::ir::IR_VERSION) and of the `spec_version`
/// a composition declares: a plan is a report *about* two artifacts, and it
/// changes for reasons of its own. It is the first key of the document, so a
/// consumer can dispatch on it before reading anything else, and a consumer that
/// does not know a version must refuse the document rather than guess.
pub const PLAN_VERSION: u32 = 4;

/// One plan: what changed between two resolved compositions.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Plan {
    /// The shape of this document — always [`PLAN_VERSION`] when this compiler
    /// wrote it.
    pub plan_version: u32,
    /// The composition compared **from**.
    pub before: Spec,
    /// The composition compared **to**.
    pub after: Spec,
    /// Definitions and triggers added, removed, or changed.
    pub components: Vec<ComponentChange>,
    /// What changed inside a flow's graph, in its channels, and in the
    /// composition's policy defaults.
    pub topology: Vec<TopologyChange>,
    /// What a caller of the composition feels: a flow's declared I/O, and a
    /// trigger's surface.
    pub interfaces: Vec<InterfaceChange>,
    /// What the validator says about each side that it does not say about the
    /// other.
    pub validation: Validation,
}

impl Plan {
    /// The plan as pretty-printed JSON, with a trailing newline.
    ///
    /// Serialized from the struct rather than through a
    /// [`Value`](serde_json::Value), so the document's own keys read in
    /// declaration order rather than sorted. The field values a change carries
    /// are `Value`s and *are* sorted, which is what makes them reproducible.
    ///
    /// # Errors
    ///
    /// Returns the serializer's error. Nothing here can produce one — every
    /// value a [`FieldChange`] holds came out of the IR, which
    /// [`Ir::to_json`](crate::ir::Ir::to_json) already establishes is
    /// representable — and it is surfaced rather than swallowed all the same.
    pub fn to_json(&self) -> serde_json::Result<String> {
        let mut json = serde_json::to_string_pretty(self)?;
        json.push('\n');
        Ok(json)
    }

    /// Whether the two compositions differ in anything this plan reports.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.components.is_empty()
            && self.topology.is_empty()
            && self.interfaces.is_empty()
            && self.validation.introduced.is_empty()
            && self.validation.resolved.is_empty()
    }

    /// How many changes the plan reports, over all four sections.
    #[must_use]
    pub fn len(&self) -> usize {
        self.components.len()
            + self.topology.len()
            + self.interfaces.len()
            + self.validation.introduced.len()
            + self.validation.resolved.len()
    }
}

/// One side of a plan, identified the way the command was given it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Spec {
    /// The entrypoint the command resolved: the file it was given, or the
    /// `main.yml` inside the directory it was given. Every span in this side of
    /// the plan is relative to **this file's own directory**, which is that
    /// composition's project root (grammar 1.4).
    pub entrypoint: String,
    /// The deploy target the composition was resolved for.
    pub target: String,
    /// The DSL version the composition declares.
    pub spec_version: String,
}

/// What happened to one subject.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    /// The after spec declares it and the before spec does not.
    Added,
    /// The before spec declares it and the after spec does not.
    Removed,
    /// Both declare it, and something about it differs.
    Changed,
}

/// Which kind of component a [`ComponentChange`] is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ComponentKind {
    /// An `agent.*` definition.
    Agent,
    /// A `tool.*` definition.
    Tool,
    /// A `flow.*` definition.
    Flow,
    /// A `store.*` definition.
    Store,
    /// A `provider.*` definition.
    Provider,
    /// A `model.*` definition.
    Model,
    /// An entry of `triggers:`.
    Trigger,
    /// An entry of the active target's `placements:`.
    Placement,
    /// The active target's `hub:` block.
    Hub,
    /// The active target's `trace_sink:` block.
    TraceSink,
    /// The active target's `package_registry:` block.
    PackageRegistry,
    /// The active target's `journal:` block.
    Journal,
    /// An entry of the active target's `event_sources:`.
    EventSource,
}

/// Which kind of site a [`TopologyChange`] is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TopologyKind {
    /// One node of a flow.
    Node,
    /// One edge of a flow.
    Edge,
    /// One `state:` channel.
    Channel,
    /// The composition's `defaults:` block.
    Defaults,
}

/// Which surface an [`InterfaceChange`] is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InterfaceKind {
    /// A flow's declared `inputs:`/`outputs:`.
    Flow,
    /// A trigger's delivery surface.
    Trigger,
}

/// Which of the two specs something belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SpecSide {
    /// The composition compared from.
    Before,
    /// The composition compared to.
    After,
}

impl SpecSide {
    /// The lowercase name of this side — the one vocabulary of this format the
    /// human report has to spell out, because a refusal names which spec failed
    /// (`crates/agent-compose/src/main.rs`). The other four are never printed:
    /// the mark and the address carry them, so none of them has one of these,
    /// and a second spelling of a member is a second thing to keep in step with
    /// `serde`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Before => "before",
            Self::After => "after",
        }
    }
}

/// One declared thing added, removed, or changed.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ComponentChange {
    /// What happened to it.
    pub change: ChangeKind,
    /// Which kind of component it is.
    pub component: ComponentKind,
    /// Its canonical address.
    pub address: String,
    /// Which of its resolved fields differ. Empty on an addition or a removal:
    /// the component itself is the change.
    pub fields: Vec<FieldChange>,
    /// Where it is written — in the **after** spec, except on a removal, which
    /// carries its before-spec location.
    pub span: Span,
}

/// One node, edge, channel, or policy default added, removed, or changed.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TopologyChange {
    /// What happened to it.
    pub change: ChangeKind,
    /// Which kind of site it is.
    pub site: TopologyKind,
    /// Its canonical address: `<flow>.<node id>` for a node,
    /// `<flow>.<from>-><to>` for an edge, `state.<name>` for a channel, and
    /// `defaults` for the policy block.
    pub address: String,
    /// The flow the site belongs to. Absent on a channel and on `defaults`,
    /// which are the composition's rather than any one flow's.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flow: Option<String>,
    /// Which of its resolved fields differ. Empty on an addition or a removal.
    pub fields: Vec<FieldChange>,
    /// Where it is written — in the **after** spec, except on a removal.
    pub span: Span,
}

/// One caller-visible surface changed.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct InterfaceChange {
    /// What happened to it. Always `changed`: a surface that arrived or left
    /// arrived or left with its component, which is [`ComponentChange`]'s to
    /// report.
    pub change: ChangeKind,
    /// Which surface it is.
    pub surface: InterfaceKind,
    /// The address of the flow or trigger whose surface this is.
    pub address: String,
    /// Which of its resolved fields differ.
    pub fields: Vec<FieldChange>,
    /// Where it is written, in the **after** spec.
    pub span: Span,
}

/// One field of one subject, and what it holds on each side.
///
/// An absent `before` means the field is **not declared** in the before spec,
/// which is not the same as one declared `null`; the same holds for `after`.
/// Both being present is an ordinary change of value.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct FieldChange {
    /// Where the field sits inside the resolved subject: keys joined with `.`,
    /// array elements as `[i]`, relative to the subject itself.
    pub path: String,
    /// What the before spec declares there.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<Value>,
    /// What the after spec declares there.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<Value>,
}

/// The validator's verdict on each side, as the difference between them.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Validation {
    /// What the after spec is told and the before spec is not.
    pub introduced: Vec<Finding>,
    /// What the before spec is told and the after spec is not.
    pub resolved: Vec<Finding>,
}

/// One diagnostic, as a plan reports it.
///
/// The identifying half of a [`Diagnostic`](crate::diag::Diagnostic) and no
/// more: a plan says *which* diagnostics moved, and `agent-compose validate` is
/// where one is read in full, with its labels, its help, and its snippet.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Finding {
    /// The stable machine-readable identity of the failure class.
    pub code: DiagnosticCode,
    /// How bad it is.
    pub severity: Severity,
    /// The one-line statement of what is wrong.
    pub message: String,
    /// Where it is reported — in the **after** spec for an introduced finding,
    /// in the before spec for a resolved one.
    pub span: Span,
}

/// The document a plan that could not be produced writes instead.
///
/// A plan is a diff of two **resolved** compositions, so a spec that does not
/// parse or does not resolve leaves nothing to compare: what the command has to
/// report is that spec's own diagnostics, which is what this carries.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Refusal {
    /// The shape of this document — [`PLAN_VERSION`], the same key a plan
    /// opens with, so one dispatch reads either.
    pub plan_version: u32,
    /// Every spec that could not be resolved, in `before`, `after` order. Never
    /// empty: a refusal exists because at least one of them failed.
    pub failed: Vec<Refused>,
}

impl Refusal {
    /// The refusal as pretty-printed JSON, with a trailing newline.
    ///
    /// # Errors
    ///
    /// Returns the serializer's error, which nothing here can produce.
    pub fn to_json(&self) -> serde_json::Result<String> {
        let mut json = serde_json::to_string_pretty(self)?;
        json.push('\n');
        Ok(json)
    }
}

/// One spec that could not be resolved.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Refused {
    /// Which side of the comparison it is.
    pub spec: SpecSide,
    /// The entrypoint the command resolved, spelled the way [`Spec`]'s is.
    pub entrypoint: String,
    /// Everything the parser and the resolver reported, in source order and in
    /// the shape `agent-compose validate --format json` writes.
    pub diagnostics: Vec<Diagnostic>,
}
