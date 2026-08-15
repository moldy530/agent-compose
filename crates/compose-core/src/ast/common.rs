//! Lexical leaves shared by every construct: identifiers, typed addresses, and
//! the four string forms the grammar distinguishes (grammar 2, 4).
//!
//! Each of the expression-ish forms keeps the author's raw text. CEL is never
//! parsed here — it stays a raw string until the validator type-checks it
//! against the surface's roots (grammar 4.1) — while durations, path
//! expressions, and environment references have lexical grammars the parser
//! decides on the spot, so they carry both the raw text and its decomposition.

use std::fmt;

/// An identifier: `[a-z][a-z0-9_]{0,63}` (grammar 2.1).
///
/// One class covers definition names, node ids, channel names, schema field
/// names, variant tags, trigger names, backend aliases, and `map`'s `as:`
/// binding — one grammar means one error message (Decision D5).
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Ident(String);

impl Ident {
    /// Wrap an already-validated identifier.
    ///
    /// Parsing goes through `parse::lexical::identifier`, which is what
    /// enforces the grammar; this constructor exists for the parser and for
    /// tests.
    #[must_use]
    pub fn new(text: impl Into<String>) -> Self {
        Self(text.into())
    }

    /// The identifier's text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Ident {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Debug for Ident {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.0, f)
    }
}

/// The six definition namespaces (grammar 2.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Namespace {
    /// `agent.*` — an LLM call with structured output.
    Agent,
    /// `tool.*` — a callable implementation.
    Tool,
    /// `flow.*` — a subgraph module.
    Flow,
    /// `store.*` — durable attachable storage.
    Store,
    /// `provider.*` — an inference connection.
    Provider,
    /// `model.*` — a model binding or route.
    Model,
}

impl Namespace {
    /// Every namespace, in the order grammar 2.2 lists them.
    pub const ALL: &'static [Self] = &[
        Self::Agent,
        Self::Tool,
        Self::Flow,
        Self::Store,
        Self::Provider,
        Self::Model,
    ];

    /// The namespace's prefix, without the dot.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Agent => "agent",
            Self::Tool => "tool",
            Self::Flow => "flow",
            Self::Store => "store",
            Self::Provider => "provider",
            Self::Model => "model",
        }
    }

    /// The namespace named by this prefix, if it is one.
    #[must_use]
    pub fn from_prefix(prefix: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|namespace| namespace.as_str() == prefix)
    }
}

impl fmt::Display for Namespace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A typed address: `<namespace>.<identifier>` (grammar 2.2, 2.3).
///
/// The same type serves both roles the grammar gives the form — a definition
/// key (`agent.reviewer:` at the top level of a file) and a reference to one
/// (`model: model.smart`) — because they are the same string with the same
/// grammar; only the position differs.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Address {
    /// Which namespace the address names.
    pub namespace: Namespace,
    /// The local name inside that namespace.
    pub name: Ident,
}

impl Address {
    /// Build an address from its parts.
    #[must_use]
    pub const fn new(namespace: Namespace, name: Ident) -> Self {
        Self { namespace, name }
    }
}

impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.namespace, self.name)
    }
}

impl fmt::Debug for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self}")
    }
}

/// A CEL expression, kept exactly as written (grammar 4.1).
///
/// The parser never looks inside: which roots are in scope depends on the
/// surface, and type-checking an expression needs the resolved schemas of
/// whatever it reads, so both belong to the validator.
#[derive(Clone, PartialEq, Eq)]
pub struct Cel(String);

impl Cel {
    /// Wrap raw expression text.
    #[must_use]
    pub fn new(text: impl Into<String>) -> Self {
        Self(text.into())
    }

    /// The expression as written.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Cel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "cel({:?})", self.0)
    }
}

/// One step of a path expression.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PathStep {
    /// `.field`
    Field(Ident),
    /// `[3]`
    Index(u64),
}

/// A path expression: a root identifier followed by field selections and
/// integer indexes, with no calls (grammar 4.2, Decision D43).
///
/// `map.over` takes one of these so the validator can statically resolve the
/// array's schema; the parser decides the lexical form and hands the
/// decomposition on.
#[derive(Clone, PartialEq, Eq)]
pub struct PathExpr {
    raw: String,
    /// The leading root identifier (`plan` in `plan.output.tasks`).
    pub root: Ident,
    /// Selections applied to the root, in order.
    pub steps: Vec<PathStep>,
}

impl PathExpr {
    /// Build a path expression from its decomposition.
    #[must_use]
    pub fn new(raw: impl Into<String>, root: Ident, steps: Vec<PathStep>) -> Self {
        Self {
            raw: raw.into(),
            root,
            steps,
        }
    }

    /// The expression as written.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.raw
    }
}

impl fmt::Debug for PathExpr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "path({:?})", self.raw)
    }
}

/// The unit of a duration literal.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DurationUnit {
    /// `ms`
    Milliseconds,
    /// `s`
    Seconds,
    /// `m`
    Minutes,
    /// `h`
    Hours,
}

impl DurationUnit {
    /// The unit's suffix.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Milliseconds => "ms",
            Self::Seconds => "s",
            Self::Minutes => "m",
            Self::Hours => "h",
        }
    }

    /// How many milliseconds one unit is.
    #[must_use]
    pub const fn in_millis(self) -> u64 {
        match self {
            Self::Milliseconds => 1,
            Self::Seconds => 1_000,
            Self::Minutes => 60_000,
            Self::Hours => 3_600_000,
        }
    }
}

/// A duration: `<positive integer><ms|s|m|h>`, single segment (grammar 4.4,
/// Decision D22).
#[derive(Clone, PartialEq, Eq)]
pub struct Duration {
    raw: String,
    /// The integer part, saturating at [`u64::MAX`]: grammar 4.4 states a form
    /// and no ceiling, so a magnitude past 64 bits is still a duration, and
    /// [`Duration::as_millis`] saturates anyway. [`Duration::as_str`] keeps the
    /// text exactly as written.
    pub amount: u64,
    /// The unit.
    pub unit: DurationUnit,
}

impl Duration {
    /// Build a duration from its parts.
    #[must_use]
    pub fn new(raw: impl Into<String>, amount: u64, unit: DurationUnit) -> Self {
        Self {
            raw: raw.into(),
            amount,
            unit,
        }
    }

    /// The duration as written.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.raw
    }

    /// The duration in milliseconds, saturating.
    #[must_use]
    pub const fn as_millis(&self) -> u64 {
        self.amount.saturating_mul(self.unit.in_millis())
    }
}

impl fmt::Debug for Duration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "duration({:?})", self.raw)
    }
}

/// An environment reference in value form: the whole string is one `${NAME}`
/// (grammar 4.3, Decision D41).
///
/// Secret-bearing and connection fields take this form and nothing else; the
/// reference survives unresolved into the IR and is checked for presence at
/// build/serve/run time, never here.
#[derive(Clone, PartialEq, Eq)]
pub struct EnvRef {
    raw: String,
    /// The variable name between the braces.
    pub name: String,
}

impl EnvRef {
    /// Build a reference from its parts.
    #[must_use]
    pub fn new(raw: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            raw: raw.into(),
            name: name.into(),
        }
    }

    /// The reference as written.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.raw
    }
}

impl fmt::Debug for EnvRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "env({:?})", self.raw)
    }
}

/// A string that may embed `${NAME}` references, kept raw (grammar 4.3).
///
/// Escaped `$${` sequences stay escaped: unescaping is a lowering concern, and
/// keeping the author's text means a diagnostic can quote it back verbatim.
#[derive(Clone, PartialEq, Eq)]
pub struct Interpolated {
    raw: String,
    /// The variable names referenced, in order of appearance.
    pub references: Vec<String>,
}

impl Interpolated {
    /// Build an interpolated string from its parts.
    #[must_use]
    pub fn new(raw: impl Into<String>, references: Vec<String>) -> Self {
        Self {
            raw: raw.into(),
            references,
        }
    }

    /// The string as written.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.raw
    }
}

impl fmt::Debug for Interpolated {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.references.is_empty() {
            write!(f, "{:?}", self.raw)
        } else {
            write!(f, "interpolated({:?})", self.raw)
        }
    }
}

/// A control-transfer target: a flow-local node id, or `end` (grammar 2.4,
/// 9.2). `on_error.fallback` and `human.on_timeout` accept exactly these.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ControlTarget {
    /// Another node of the same flow.
    Node(Ident),
    /// The `end` pseudo-node: finish this flow instance now.
    End,
}

/// Where an edge starts (grammar 7.2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EdgeSource {
    /// The `start` pseudo-node.
    Start,
    /// A node of the flow.
    Node(Ident),
}

/// Where an edge goes (grammar 7.2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EdgeTarget {
    /// The `end` pseudo-node.
    End,
    /// A node of the flow.
    Node(Ident),
}

/// The seven **reserved root names** of grammar 2.5 (Decisions D33, D74).
///
/// Five are CEL roots (`input`, `state`, `execution`, `payload`, and `item`, a
/// `map`'s default per-item binding); `messages` is the implicit
/// conversation-history channel (grammar 10.4); and `output` is the fixed
/// selector half of `<node>.output`, on the list to keep that token to one
/// meaning rather than because it is a root.
///
/// None of them may name a **state channel**, a **flow-local node id** — which
/// is why they are refused in the two edge endpoints and the two
/// control-transfer positions that name one — or a `map`'s **`as:`** binding,
/// where `item` is the one exception, being that binding's own default name
/// (grammar 2.5).
pub const RESERVED_ROOT_NAMES: &[&str] = &[
    "input",
    "state",
    "execution",
    "item",
    "messages",
    "output",
    "payload",
];

/// The two pseudo-nodes, which may not be used as node ids (grammar 2.4).
pub const PSEUDO_NODES: &[&str] = &["start", "end"];

/// A YAML literal, kept as written, for the surfaces that carry data rather
/// than schema: `default:` values and a model's open `settings:` object
/// (Decision D50).
///
/// Both are grammar 4.3 class 3, which is what lets the value stay opaque —
/// nothing in it is a reference to anything. The other open plugin-config
/// objects, a backend config and an event source, carry class-2 values instead
/// and so are read as
/// [`PluginValue`](super::deploy::PluginValue), which records the environment
/// references they may embed.
#[derive(Clone, Debug, PartialEq)]
pub enum Literal {
    /// `~` / `null`.
    Null,
    /// A boolean.
    Bool(bool),
    /// An integer.
    Int(i64),
    /// A float.
    Float(f64),
    /// A string.
    String(String),
    /// A sequence of literals.
    Sequence(Vec<crate::diag::Spanned<Literal>>),
    /// A mapping of literals, in declaration order.
    Mapping(Vec<LiteralEntry>),
}

impl Literal {
    /// How this literal is named in a diagnostic, with its article.
    #[must_use]
    pub const fn description(&self) -> &'static str {
        match self {
            Self::Null => "null",
            Self::Bool(_) => "a boolean",
            Self::Int(_) => "an integer",
            Self::Float(_) => "a number",
            Self::String(_) => "a string",
            Self::Sequence(_) => "a sequence",
            Self::Mapping(_) => "a mapping",
        }
    }
}

/// One entry of a [`Literal::Mapping`].
#[derive(Clone, Debug, PartialEq)]
pub struct LiteralEntry {
    /// The key, spanned.
    pub key: crate::diag::Spanned<String>,
    /// The value, spanned.
    pub value: crate::diag::Spanned<Literal>,
}
