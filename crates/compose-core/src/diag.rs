//! Diagnostics: the compiler's error contract.
//!
//! Every failure the compiler can report — from an unreadable file to a
//! cycle-termination proof that does not close — is a [`Diagnostic`]. The type
//! is deliberately inert data: it carries a stable machine-readable
//! [`DiagnosticCode`], a severity, a primary message anchored to a [`Span`],
//! optional labelled secondary spans, and optional help text. Rendering (ANSI
//! snippets, JSON, LSP payloads) belongs to the layer that reports them, not
//! here, so nothing in this module knows about terminals or serialization
//! formats.
//!
//! # The code scheme
//!
//! Codes are lowercase kebab-case strings drawn from the closed
//! [`DiagnosticCode`] enum: `unknown-key`, `invalid-duration`,
//! `undefined-reference`.
//! They are the stable identity of a failure class — test fixtures, editors,
//! and downstream tooling match on them, so a code is never renamed or reused
//! for a different meaning once it ships; a new failure class gets a new
//! variant. Messages, by contrast, are free to improve: the fixture corpus in
//! `tests/fixtures/invalid-parse/` pins the exact text so an improvement is a
//! reviewed diff rather than a silent regression (PRD G3 — error UX is a
//! product feature).
//!
//! Codes are kebab strings rather than `E0001`-style numbers because the
//! compiler is read by coding agents as much as by humans: `unknown-key` is
//! self-describing in a terminal, in a test fixture header, and in a grep.
//!
//! # Serialization
//!
//! [`Diagnostic`] is [`Serialize`], which is what
//! `agent-compose validate --format json` emits. That is *not* rendering: the
//! shape written is this module's own type, field for field, so the JSON holds
//! the same data a Rust caller reads rather than a second, prose-shaped account
//! of it, and every key is always present so a consumer reads one fixed record.
//! A code writes as its kebab spelling and a severity as its lowercase name —
//! both stable identities (above) — and a span writes as the one string form
//! [`ir::leaf`](crate::ir::leaf) already fixes for the artifact,
//! `<file>:<line>:<col>..<line>:<col>`, so a span means the same thing wherever
//! this compiler writes one. Turning any of it into a *snippet* belongs to the
//! CLI, which is where the renderer lives.

use std::fmt;
use std::ops::Range;
use std::sync::Arc;

use serde::{Serialize, Serializer};

/// The name of a source file, as spans and diagnostics refer to it.
///
/// This is the path exactly as the caller supplied it (typically relative to
/// the project root), interned behind an [`Arc`] so that the thousands of
/// spans one file produces share one allocation.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct SourceName(Arc<str>);

impl SourceName {
    /// Intern a source name.
    pub fn new(name: impl AsRef<str>) -> Self {
        Self(Arc::from(name.as_ref()))
    }

    /// The name as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for SourceName {
    fn from(name: &str) -> Self {
        Self::new(name)
    }
}

impl From<String> for SourceName {
    fn from(name: String) -> Self {
        Self::new(name)
    }
}

impl fmt::Display for SourceName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Debug for SourceName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.0, f)
    }
}

/// A 1-based line/column position.
///
/// Columns count characters, not bytes: they are what an editor shows.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Position {
    /// 1-based line number.
    pub line: u32,
    /// 1-based column number, in characters.
    pub column: u32,
}

impl Position {
    /// A position at `line`:`column`, both 1-based.
    #[must_use]
    pub const fn new(line: u32, column: u32) -> Self {
        Self { line, column }
    }
}

impl fmt::Display for Position {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.line, self.column)
    }
}

impl fmt::Debug for Position {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

/// A region of one source file.
///
/// Carries both a byte range (for slicing the source) and 1-based line/column
/// endpoints (for reporting), because recomputing one from the other at report
/// time would mean keeping every source string alive.
#[derive(Clone, PartialEq, Eq)]
pub struct Span {
    /// The file this region belongs to.
    pub source: SourceName,
    /// Byte offsets into that file, `start..end`, end-exclusive.
    pub bytes: Range<usize>,
    /// Position of the first character.
    pub start: Position,
    /// Position just past the last character.
    pub end: Position,
}

impl Span {
    /// Build a span from its parts.
    #[must_use]
    pub fn new(source: SourceName, bytes: Range<usize>, start: Position, end: Position) -> Self {
        Self {
            source,
            bytes,
            start,
            end,
        }
    }

    /// An empty span at the very beginning of a file, used when a diagnostic is
    /// about the file as a whole (an empty document, a byte-order mark).
    #[must_use]
    pub fn file_start(source: SourceName) -> Self {
        Self::new(source, 0..0, Position::new(1, 1), Position::new(1, 1))
    }

    /// A zero-width span collapsed onto this one's start.
    #[must_use]
    pub fn collapsed(&self) -> Self {
        Self::new(
            self.source.clone(),
            self.bytes.start..self.bytes.start,
            self.start,
            self.start,
        )
    }

    /// The smallest region covering both spans.
    ///
    /// Used where a construct is written as a key and a value — a definition,
    /// `agent.reviewer:` and its body — and the thing that has to be underlined
    /// is the whole entry. Both spans are assumed to come from the same file, as
    /// they do everywhere the parser joins two: it walks one file at a time.
    #[must_use]
    pub fn joined(&self, other: &Self) -> Self {
        Self::new(
            self.source.clone(),
            self.bytes.start.min(other.bytes.start)..self.bytes.end.max(other.bytes.end),
            self.start.min(other.start),
            self.end.max(other.end),
        )
    }
}

/// `Debug` renders `line:col..line:col` — position only, no byte offsets and no
/// file name. AST snapshots are per file and byte offsets shift with every
/// unrelated edit above them, so the compact form keeps snapshot diffs about
/// the change under review.
impl fmt::Debug for Span {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}..{}", self.start, self.end)
    }
}

impl fmt::Display for Span {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.source, self.start)
    }
}

/// A value paired with the source region it was written in.
#[derive(Clone, PartialEq)]
pub struct Spanned<T> {
    /// The value.
    pub value: T,
    /// Where it was written.
    pub span: Span,
}

impl<T> Spanned<T> {
    /// Pair a value with a span.
    pub const fn new(value: T, span: Span) -> Self {
        Self { value, span }
    }

    /// Apply `f` to the value, keeping the span.
    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Spanned<U> {
        Spanned {
            value: f(self.value),
            span: self.span,
        }
    }

    /// Borrow the value.
    pub const fn as_ref(&self) -> &T {
        &self.value
    }
}

/// `Debug` renders `value @ line:col..line:col`, and honours the alternate
/// flag so that a spanned struct still pretty-prints inside `{:#?}` output.
impl<T: fmt::Debug> fmt::Debug for Spanned<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if f.alternate() {
            write!(f, "{:#?} @ {:?}", self.value, self.span)
        } else {
            write!(f, "{:?} @ {:?}", self.value, self.span)
        }
    }
}

/// How bad a diagnostic is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// The composition is rejected.
    Error,
    /// The composition is accepted, with a caveat worth surfacing.
    Warning,
}

impl Severity {
    /// The lowercase name of this severity.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warning => "warning",
        }
    }
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The stable identity of a failure class.
///
/// See the module documentation for the scheme. Variants are grouped by the
/// layer that raises them: the parser raises every code down to
/// [`ReservedName`](Self::ReservedName), the resolver raises the four after
/// it — plus [`IoError`](Self::IoError), [`InvalidEncoding`](Self::InvalidEncoding)
/// and [`InvalidImportPath`](Self::InvalidImportPath), which it shares with the
/// parser because an unreadable or out-of-tree import is the same failure class
/// wherever it is noticed — and the validator raises the rest.
///
/// A validator code names a failure class, never a rule: `type-mismatch` is
/// raised by every check that compares two declared types, and which rule was
/// broken is what the message says. Several validator checks reuse a code the
/// parser already owns where the failure really is the same class — a store-op
/// `value:` that omits a required field is a [`MissingKey`](Self::MissingKey),
/// a settings key no provider plugin publishes is an
/// [`UnknownKey`](Self::UnknownKey) — because a second spelling of one class
/// would make the corpus assert on which pass happened to notice it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DiagnosticCode {
    // --- file level -------------------------------------------------------
    /// The file could not be read.
    IoError,
    /// The file is not valid UTF-8, or starts with a byte-order mark.
    InvalidEncoding,
    /// The YAML itself does not parse.
    YamlSyntax,
    /// The file contains no YAML document.
    EmptyDocument,
    /// The file contains more than one YAML document.
    MultipleDocuments,
    /// The document's root node is not a mapping.
    RootNotMapping,

    // --- YAML profile (grammar 1.1) ---------------------------------------
    /// A mapping key is not a string.
    NonStringKey,
    /// A mapping declares the same key twice.
    DuplicateKey,
    /// A mapping uses the YAML 1.1 merge key `<<:`.
    MergeKey,
    /// A node carries a YAML tag.
    YamlTag,

    // --- document model ---------------------------------------------------
    /// A section appears in a document kind that may not carry it.
    MisplacedSection,
    /// The `version:` value is outside this build's supported set.
    UnsupportedVersion,
    /// An `imports:` entry is not a relative path to a spec file.
    InvalidImportPath,
    /// A `module:` binding's path is not a project-relative path to a
    /// TypeScript file inside the project, or names a file `build` emits
    /// (grammar 6.1, PRD resolved q48). Its own class rather than an
    /// [`InvalidImportPath`](Self::InvalidImportPath): the two paths point at
    /// different kinds of file, take different extensions, and are repaired by
    /// different edits, so a corpus asserting on one would be asserting about
    /// the other's rule.
    InvalidModulePath,
    /// A `module:` binding's `dependencies:` entry is not a package name at an
    /// exact version, or is one two declarations disagree about (grammar 6.1,
    /// PRD resolved q49).
    InvalidDependency,

    // --- construct shape --------------------------------------------------
    /// A mapping carries a key the construct does not define.
    UnknownKey,
    /// A required key is absent.
    MissingKey,
    /// A value has the wrong YAML kind.
    WrongType,
    /// A value is well-typed but not legal here.
    InvalidValue,
    /// A numeric value is outside its declared range.
    ValueOutOfRange,
    /// A value is not one of a closed set of variants.
    UnknownVariant,
    /// Two keys that may not appear together both appear.
    ConflictingKeys,
    /// A provider that would reach a vendor's own endpoint declares neither
    /// `api_key:` nor the `base_url:` of a gateway that supplies one — the one
    /// *conditional* required key (grammar 12.1, Decision D120). Its own class
    /// rather than a [`MissingKey`](Self::MissingKey), for the reason
    /// [`MissingSessionKey`](Self::MissingSessionKey) is: what is absent is
    /// decided by a sibling value, and the repair is a choice of two.
    MissingCredential,

    // --- lexical forms ----------------------------------------------------
    /// A string is not a legal identifier (grammar 2.1).
    InvalidIdentifier,
    /// A string is not a legal typed address, or names the wrong namespace.
    InvalidReference,
    /// A string is not a legal duration (grammar 4.4).
    InvalidDuration,
    /// A string is not a legal path expression (grammar 4.2).
    InvalidPathExpression,
    /// An environment reference is malformed, or a secret-bearing field holds a
    /// literal instead of one.
    InvalidEnvRef,
    /// An environment reference appears on a surface that never interpolates.
    UnexpectedEnvRef,
    /// An identifier is reserved for another purpose.
    ReservedName,

    // --- resolution (grammar 1, 2, 14) ------------------------------------
    /// One typed address is defined by two definitions.
    DuplicateDefinition,
    /// A section that may appear in at most one file of a composition appears
    /// in two.
    DuplicateSection,
    /// A reference names nothing the composition defines: a typed address, a
    /// flow-local node id, a storage backend alias, or an event source.
    UndefinedReference,
    /// A file's `version:` differs from the entrypoint's.
    VersionMismatch,

    // --- expressions (grammar 4.1) ----------------------------------------
    /// A CEL expression does not parse, or reads something its surface cannot
    /// supply.
    InvalidExpression,
    /// An expression's root identifier is not in scope on its surface.
    UnknownRoot,
    /// A path, a binding, or a write remap names a field the declared schema
    /// does not have.
    UnknownField,
    /// Two declared types meet and the source cannot land in the target.
    TypeMismatch,

    // --- state (grammar 10) -----------------------------------------------
    /// A `state.*` read, a `writes:` destination, or a flow `outputs:` field
    /// names a channel `state:` does not declare.
    UndefinedChannel,
    /// A concurrent writer targets a channel with no `reduce:` policy.
    UnreducedWrite,
    /// Two output fields of one node land on one channel (Decision D93).
    ConflictingWrites,

    // --- bindings (grammar 8.0, 13.1) -------------------------------------
    /// An input field has no binding, no name-based source, and no `default:`.
    MissingBinding,

    // --- fan-out (grammar 8.6) --------------------------------------------
    /// A `map.over` path resolves to an array with no `max_items`.
    UnboundedFanOut,
    /// A variant of the item union has neither a route nor a `default:`.
    NonExhaustive,
    /// A detached dispatch writes state (Decisions D31, D94).
    DetachedWrite,

    // --- stores (grammar 11) ----------------------------------------------
    /// A `vector`/`blob` write inside a fan-out has a key that is not
    /// item-derived (Decision D67).
    UnkeyedMapWrite,
    /// A tool an attached store synthesizes has the name of a tool the same
    /// agent attaches (grammar 11.5).
    ToolNameCollision,
    /// A trigger whose flow reaches a `session`-scoped store declares no
    /// `session_key:`.
    MissingSessionKey,

    // --- providers and models (grammar 12) --------------------------------
    /// A provider cannot serve what a model, an agent, or a store asks of it.
    MissingCapability,
    /// A provider declares `server_tools:` on a kind whose wire this compiler
    /// release does not carry them on (grammar 12.1, Decision D122).
    UnsupportedServerTools,
    /// **Warning.** A `server_tools:` entry names a `type:` the kind's curated
    /// table does not have, so nothing about its config could be verified — it
    /// travels to the wire verbatim (grammar 12.1, Decision D122).
    UnknownServerTool,
    /// **Warning.** A `server_tools:` entry names a tool the kind's curated
    /// table has, and gives it a field that table does not name — a vendor
    /// parameter newer than this release, or a misspelling; nothing here can
    /// tell which. The value is unchecked and travels to the wire verbatim
    /// (grammar 12.1, Decision D122).
    UnknownServerToolField,
    /// **Warning.** The members of a failover route declare different
    /// `server_tools:` suites, so which tools the model is offered depends on
    /// which member served the call (grammar 12.2, Decision D122).
    MismatchedServerTools,

    // --- coder nodes (grammar 8.9) ----------------------------------------
    /// A `coder:` node binds a harness this release **reserves** rather than
    /// lowers: `deepagents` or `native` (grammar 8.9, PRD resolved q57). Its own
    /// class rather than an [`UnknownVariant`](Self::UnknownVariant), which is
    /// what a misspelling gets: the name is one the grammar has, and what is
    /// absent is a driver rather than a spelling, so the repair is a choice
    /// between the two harnesses v1 ships rather than a correction.
    UnsupportedHarness,
    /// A `coder:` node states a `permission_mode:` and the harness it binds has
    /// **no approval axis at all** (grammar 8.9, Decision D146, PRD resolved q60
    /// ruling a).
    ///
    /// `codex`'s containment primitive is a sandbox preset, and its per-call
    /// approval tier lives in an app server this release does not adopt
    /// (resolved q57 ruling c) — so there is nothing on that harness for a mode
    /// to select, and inventing one would be the faked slot
    /// [`UnsupportedConnectionFact`](Self::UnsupportedConnectionFact) is refused
    /// to avoid. Its own class rather than an
    /// [`UnknownVariant`](Self::UnknownVariant), for
    /// [`UnsupportedHarness`](Self::UnsupportedHarness)'s reason: the mode is
    /// spelled correctly and what is absent is the axis.
    UnsupportedPermissionMode,
    /// A `coder:` node's `permission_mode:` is outside what its `access:` level
    /// admits (grammar 8.9, Decision D146, PRD resolved q60 ruling a).
    ///
    /// **The widening bound**: a mode may never grant an operation the level's
    /// own derived mode would refuse. Both values are legal on their own and
    /// only the pair is wrong — the same shape as
    /// [`UnsupportedProviderKind`](Self::UnsupportedProviderKind) — and what the
    /// pair decides is how far a run may reach, so it is refused at `validate`
    /// rather than discovered from what a run did (PRD G3).
    WideningPermissionMode,
    /// **Warning.** A `coder:` node's `settings:` names a key the harness's
    /// curated table does not have, so nothing about its value could be verified
    /// — it travels to the SDK verbatim (grammar 8.9, Decision D140). The same
    /// two-tier posture, and the same warning shape, as
    /// [`UnknownServerTool`](Self::UnknownServerTool).
    ///
    /// The **reserved** keys are not this: they are
    /// [`ReservedHarnessSetting`](Self::ReservedHarnessSetting), an error, since
    /// PRD resolved q60 ruling b.
    UnknownHarnessSetting,
    /// A `coder:` node's `settings:` names an option the generated adapter owns
    /// — a bound this node already states, or one that contains it (grammar 8.9,
    /// Decision D146, PRD resolved q60 ruling b).
    ///
    /// An **error**, where PRD resolved q57 shipped a warning and a silent
    /// run-time drop. The hardening is resolved q58 ruling b's, one surface
    /// along: where a dropped key would change what a run may do, failing at run
    /// time — or not failing at all — instead of at compile time is the class
    /// resolved q25 refused (PRD G3). The message names the first-class key that
    /// answers the need where one does, which is what
    /// [`crate::harness::Answered`] carries.
    ReservedHarnessSetting,
    /// Two harness runs that can be in flight at once are contained by one
    /// directory (grammar 8.9, Decision D147, PRD resolved q61 ruling b).
    ///
    /// **One rule at two sites, and only one of them is decidable.** A `coder:`
    /// node a `map` dispatches whose `workspace:` expression does not read the
    /// per-dispatch scope names one directory for every item of the fan-out, so
    /// `max_concurrency` above 1 is a data race on a checkout — an **error**,
    /// with the two repairs named. Two coder nodes on statically-concurrent
    /// branches whose `workspace:` values are statically equal are the same
    /// collision one construct along, and a **warning**: what decides it is
    /// whether the two resolve to one directory, and a value bearing an
    /// `${ENV}` reference resolves at launch, so an error there would be this
    /// compiler claiming a fact it does not have.
    SharedWorkspace,
    /// A `coder:` node's model resolves to a provider declaring a connection
    /// fact the bound harness has **no slot for** (grammar 8.9, Decision D143,
    /// PRD resolved q58 ruling b).
    ///
    /// An **error** rather than the warn-and-drop resolved q30 gives a server
    /// tool, and the difference is what the fact decides: where the traffic goes
    /// and whether it authenticates. A `headers:` silently dropped on the way
    /// into a harness run is a gateway token that never reaches the wire, found
    /// on the first live call rather than at `validate` — which is the class
    /// resolved q25 refused (PRD G3).
    UnsupportedConnectionFact,
    /// A `coder:` node's model resolves through a `provider.*` whose `kind:`
    /// speaks a wire the bound harness's connection surface does not (grammar
    /// 8.9, Decision D143, PRD resolved q58 ruling b).
    ///
    /// A slot is an endpoint and a credential **on one wire**: `cc` carries a
    /// connection into `ANTHROPIC_BASE_URL` and `ANTHROPIC_API_KEY`, `codex`
    /// into its client's OpenAI base URL and key. Mapping an `openai` provider
    /// through the first, or an `anthropic` one through the second, writes one
    /// vendor's endpoint and key where the other's are read — the same class as
    /// [`UnsupportedConnectionFact`](Self::UnsupportedConnectionFact), one level
    /// up, and refused for the same reason: it decides where the traffic goes.
    UnsupportedProviderKind,
    /// A `coder:` node's `env:` names a variable the connection map would set
    /// from its model's provider (grammar 8.9, Decision D143, PRD resolved q58
    /// ruling c).
    ///
    /// **One spelling per fact**: the node and the provider would each be
    /// writing one name, and neither a silent shadowing nor a precedence rule to
    /// memorize is an answer. An author who wants one node on a different
    /// endpoint defines another provider, which is the cheap move resolved q30
    /// already leans on.
    ConflictingConnectionVariable,

    // --- graph analyses (grammar 7.4–7.8, 8.6, 13.3) ----------------------
    /// A node has a pass on which its branch takes no outgoing edge: it has
    /// none at all, it declares `on_error: skip` with every edge guarded, or a
    /// `max_iterations` budget can run out with no escape (grammar 7.6.3, 7.4).
    DeadEnd,
    /// A strongly connected component carries no bounded edge (grammar 7.4).
    UnboundedCycle,
    /// Two edges of one co-takeable pair reach a node at different step
    /// distances (grammar 7.6.2, Decision D112).
    UnbalancedConvergence,
    /// A node is not reachable from its flow's `start` (grammar 7.8).
    UnreachableNode,
    /// A flow reaches itself (grammar 7.5, Decision D26).
    RecursiveFlow,
    /// A `respond: sync` trigger's flow reaches a `human` node (grammar 13.3).
    SyncTriggerInterrupt,
    /// A `map.over` path reads a node that does not dominate the map node
    /// (grammar 8.6 rule 11, Decision D76).
    NonDominatingSource,
    /// `detach: true` under a durably checkpointed target (grammar 8.6 rule 7,
    /// Decision D59).
    UnsupportedDetach,
    /// A **detached** `map` dispatch reaches a `human` node (grammar 8.6 rule 7,
    /// 8.7, Decision D118).
    DetachedInterrupt,

    // --- triggers (grammar 13.3) ------------------------------------------
    /// Two `http` triggers declare one route: the same effective `path:` at the
    /// same `method:` (grammar 13.3).
    DuplicateRoute,
    /// Two `manual` triggers name one flow and declare different `session_key:`
    /// expressions, which are two answers for one CLI entry (grammar 13.2).
    ConflictingSessionKey,
    /// An `http` trigger declares `callback_auth:` and no `callback_allow:`
    /// (grammar 13.3, PRD resolved q33). Its own class rather than a
    /// [`MissingKey`](Self::MissingKey), for the reason
    /// [`MissingCredential`](Self::MissingCredential) is: what is absent is
    /// decided by a sibling value, and the repair is a choice of two.
    MissingCallbackAllowlist,

    // --- placements (grammar 14.1, 14.2) ----------------------------------
    /// A `placements:` entry names a member this release cannot place: a
    /// `flow.*`, whose placement is deferred (grammar 14.1, PRD resolved q44).
    UnsupportedPlacement,
    /// Two placements claim one component, or a placed `tool.*` is attached to
    /// an agent placed somewhere else — either way, two answers to which worker
    /// runs one piece of work (grammar 14.1, Decision D129).
    ConflictingPlacement,
    /// A target declares `placements:` and no `hub.join_token:` (grammar 14.2,
    /// PRD resolved q38). Its own class rather than a
    /// [`MissingKey`](Self::MissingKey), for the reason
    /// [`MissingCallbackAllowlist`](Self::MissingCallbackAllowlist) is: what is
    /// absent is decided by a sibling section, and the repair is a choice of
    /// two.
    MissingJoinToken,
    /// A store on a backend the reaching process opens for itself is bound by
    /// something a placement executes, so a mesh would run it in more than one
    /// process and each would hold its own copy (grammar 14.1 rule 5, PRD
    /// resolved q45, Decision D131). Its own class rather than a
    /// [`ConflictingPlacement`](Self::ConflictingPlacement): nothing here holds
    /// two answers to which worker runs one thing — one answer is enough, and
    /// what it forks is the store.
    ProcessLocalStore,

    // --- package registry (grammar 14.6) ----------------------------------
    /// Two `package_registry` entries authenticate to one derived `.npmrc`
    /// address with two different variables (grammar 14.6 rule 5, PRD resolved
    /// q59, Decision D145). Its own class rather than an
    /// [`InvalidValue`](Self::InvalidValue), for the reason
    /// [`ConflictingConnectionVariable`](Self::ConflictingConnectionVariable)
    /// is: neither value is wrong on its own, and what is refused is the pair —
    /// one emitted `_authToken` line written twice, which an ini parser
    /// resolves last-one-wins while Bun's per-scope table keeps both.
    ConflictingRegistryCredential,
    /// A `package_registry` entry declares no `token:` at an address at or
    /// under a tokened entry's, so npm's walk-up spends the other's there and
    /// Bun sends nothing (grammar 14.6 rule 5, PRD resolved q59, Decision
    /// D145). Its own class rather than a [`MissingKey`](Self::MissingKey), for
    /// the reason [`MissingJoinToken`](Self::MissingJoinToken) is: whether the
    /// key is required is decided by a **sibling entry's** contents, and the
    /// repair is a choice of two.
    MissingRegistryToken,

    // --- journal (grammar 14.7) -------------------------------------------
    /// A `journal:` block binds a provider that **dials out** — `postgres`,
    /// `mysql` — and declares no `url:` (grammar 14.7, PRD resolved q62,
    /// Decision D148). Its own class rather than a
    /// [`MissingKey`](Self::MissingKey), for the reason
    /// [`MissingJoinToken`](Self::MissingJoinToken) is: whether the key is
    /// required is decided by a **sibling value** — the `provider:` on the line
    /// above — and the repair is a choice of two.
    MissingJournalUrl,
    /// A `journal:` block declares a key the provider it binds does not take:
    /// `url:` under `provider: sqlite`, which opens a file beside the project
    /// and dials nothing (grammar 14.7, PRD resolved q62, Decision D148). Its
    /// own class rather than an [`UnknownKey`](Self::UnknownKey), for the reason
    /// [`UnsupportedServerTools`](Self::UnsupportedServerTools) is one: the key
    /// is the construct's own and is legal one line up, so "the construct does
    /// not define it" would be false about the block in front of the reader —
    /// what is refused is the pair.
    UnsupportedJournalKey,
}

impl DiagnosticCode {
    /// Every code, in declaration order.
    ///
    /// The list is what the tests that must be exhaustive over the enum read,
    /// so it is pinned to the enum rather than kept in step by hand: each entry
    /// is asserted to sit at its own variant's position
    /// (`codes_are_kebab_case_and_unique`), which fails on an entry that is
    /// missing, duplicated, or out of order. A code appended after the last
    /// variant below belongs at the end of this list too.
    pub const ALL: &'static [Self] = &[
        Self::IoError,
        Self::InvalidEncoding,
        Self::YamlSyntax,
        Self::EmptyDocument,
        Self::MultipleDocuments,
        Self::RootNotMapping,
        Self::NonStringKey,
        Self::DuplicateKey,
        Self::MergeKey,
        Self::YamlTag,
        Self::MisplacedSection,
        Self::UnsupportedVersion,
        Self::InvalidImportPath,
        Self::InvalidModulePath,
        Self::InvalidDependency,
        Self::UnknownKey,
        Self::MissingKey,
        Self::WrongType,
        Self::InvalidValue,
        Self::ValueOutOfRange,
        Self::UnknownVariant,
        Self::ConflictingKeys,
        Self::MissingCredential,
        Self::InvalidIdentifier,
        Self::InvalidReference,
        Self::InvalidDuration,
        Self::InvalidPathExpression,
        Self::InvalidEnvRef,
        Self::UnexpectedEnvRef,
        Self::ReservedName,
        Self::DuplicateDefinition,
        Self::DuplicateSection,
        Self::UndefinedReference,
        Self::VersionMismatch,
        Self::InvalidExpression,
        Self::UnknownRoot,
        Self::UnknownField,
        Self::TypeMismatch,
        Self::UndefinedChannel,
        Self::UnreducedWrite,
        Self::ConflictingWrites,
        Self::MissingBinding,
        Self::UnboundedFanOut,
        Self::NonExhaustive,
        Self::DetachedWrite,
        Self::UnkeyedMapWrite,
        Self::ToolNameCollision,
        Self::MissingSessionKey,
        Self::MissingCapability,
        Self::UnsupportedServerTools,
        Self::UnknownServerTool,
        Self::UnknownServerToolField,
        Self::MismatchedServerTools,
        Self::UnsupportedHarness,
        Self::UnsupportedPermissionMode,
        Self::WideningPermissionMode,
        Self::UnknownHarnessSetting,
        Self::ReservedHarnessSetting,
        Self::SharedWorkspace,
        Self::UnsupportedConnectionFact,
        Self::UnsupportedProviderKind,
        Self::ConflictingConnectionVariable,
        Self::DeadEnd,
        Self::UnboundedCycle,
        Self::UnbalancedConvergence,
        Self::UnreachableNode,
        Self::RecursiveFlow,
        Self::SyncTriggerInterrupt,
        Self::NonDominatingSource,
        Self::UnsupportedDetach,
        Self::DetachedInterrupt,
        Self::DuplicateRoute,
        Self::ConflictingSessionKey,
        Self::MissingCallbackAllowlist,
        Self::UnsupportedPlacement,
        Self::ConflictingPlacement,
        Self::MissingJoinToken,
        Self::ProcessLocalStore,
        Self::ConflictingRegistryCredential,
        Self::MissingRegistryToken,
        Self::MissingJournalUrl,
        Self::UnsupportedJournalKey,
    ];

    /// The stable kebab-case spelling of this code.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::IoError => "io-error",
            Self::InvalidEncoding => "invalid-encoding",
            Self::YamlSyntax => "yaml-syntax",
            Self::EmptyDocument => "empty-document",
            Self::MultipleDocuments => "multiple-documents",
            Self::RootNotMapping => "root-not-mapping",
            Self::NonStringKey => "non-string-key",
            Self::DuplicateKey => "duplicate-key",
            Self::MergeKey => "merge-key",
            Self::YamlTag => "yaml-tag",
            Self::MisplacedSection => "misplaced-section",
            Self::UnsupportedVersion => "unsupported-version",
            Self::InvalidImportPath => "invalid-import-path",
            Self::InvalidModulePath => "invalid-module-path",
            Self::InvalidDependency => "invalid-dependency",
            Self::UnknownKey => "unknown-key",
            Self::MissingKey => "missing-key",
            Self::WrongType => "wrong-type",
            Self::InvalidValue => "invalid-value",
            Self::ValueOutOfRange => "value-out-of-range",
            Self::UnknownVariant => "unknown-variant",
            Self::ConflictingKeys => "conflicting-keys",
            Self::MissingCredential => "missing-credential",
            Self::InvalidIdentifier => "invalid-identifier",
            Self::InvalidReference => "invalid-reference",
            Self::InvalidDuration => "invalid-duration",
            Self::InvalidPathExpression => "invalid-path-expression",
            Self::InvalidEnvRef => "invalid-env-ref",
            Self::UnexpectedEnvRef => "unexpected-env-ref",
            Self::ReservedName => "reserved-name",
            Self::DuplicateDefinition => "duplicate-definition",
            Self::DuplicateSection => "duplicate-section",
            Self::UndefinedReference => "undefined-reference",
            Self::VersionMismatch => "version-mismatch",
            Self::InvalidExpression => "invalid-expression",
            Self::UnknownRoot => "unknown-root",
            Self::UnknownField => "unknown-field",
            Self::TypeMismatch => "type-mismatch",
            Self::UndefinedChannel => "undefined-channel",
            Self::UnreducedWrite => "unreduced-write",
            Self::ConflictingWrites => "conflicting-writes",
            Self::MissingBinding => "missing-binding",
            Self::UnboundedFanOut => "unbounded-fan-out",
            Self::NonExhaustive => "non-exhaustive",
            Self::DetachedWrite => "detached-write",
            Self::UnkeyedMapWrite => "unkeyed-map-write",
            Self::ToolNameCollision => "tool-name-collision",
            Self::MissingSessionKey => "missing-session-key",
            Self::MissingCapability => "missing-capability",
            Self::UnsupportedServerTools => "unsupported-server-tools",
            Self::UnknownServerTool => "unknown-server-tool",
            Self::UnknownServerToolField => "unknown-server-tool-field",
            Self::MismatchedServerTools => "mismatched-server-tools",
            Self::UnsupportedHarness => "unsupported-harness",
            Self::UnsupportedPermissionMode => "unsupported-permission-mode",
            Self::WideningPermissionMode => "widening-permission-mode",
            Self::UnknownHarnessSetting => "unknown-harness-setting",
            Self::ReservedHarnessSetting => "reserved-harness-setting",
            Self::SharedWorkspace => "shared-workspace",
            Self::UnsupportedConnectionFact => "unsupported-connection-fact",
            Self::UnsupportedProviderKind => "unsupported-provider-kind",
            Self::ConflictingConnectionVariable => "conflicting-connection-variable",
            Self::DeadEnd => "dead-end",
            Self::UnboundedCycle => "unbounded-cycle",
            Self::UnbalancedConvergence => "unbalanced-convergence",
            Self::UnreachableNode => "unreachable-node",
            Self::RecursiveFlow => "recursive-flow",
            Self::SyncTriggerInterrupt => "sync-trigger-interrupt",
            Self::NonDominatingSource => "non-dominating-source",
            Self::UnsupportedDetach => "unsupported-detach",
            Self::DetachedInterrupt => "detached-interrupt",
            Self::DuplicateRoute => "duplicate-route",
            Self::ConflictingSessionKey => "conflicting-session-key",
            Self::MissingCallbackAllowlist => "missing-callback-allowlist",
            Self::UnsupportedPlacement => "unsupported-placement",
            Self::ConflictingPlacement => "conflicting-placement",
            Self::MissingJoinToken => "missing-join-token",
            Self::ProcessLocalStore => "process-local-store",
            Self::ConflictingRegistryCredential => "conflicting-registry-credential",
            Self::MissingRegistryToken => "missing-registry-token",
            Self::MissingJournalUrl => "missing-journal-url",
            Self::UnsupportedJournalKey => "unsupported-journal-key",
        }
    }
}

impl Serialize for DiagnosticCode {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl fmt::Display for DiagnosticCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A secondary span attached to a diagnostic, with the role it plays.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Label {
    /// Where the related source is.
    pub span: Span,
    /// What it contributes ("first defined here").
    pub message: String,
}

/// One compiler diagnostic.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Diagnostic {
    /// Stable machine-readable identity of the failure class.
    pub code: DiagnosticCode,
    /// How bad it is.
    pub severity: Severity,
    /// One-line statement of what is wrong, in lowercase, without a trailing
    /// period.
    pub message: String,
    /// The source region the message is about.
    pub span: Span,
    /// Related regions, each with its own note.
    pub labels: Vec<Label>,
    /// Optional actionable follow-up ("did you mean `retry`?").
    pub help: Option<String>,
}

impl Diagnostic {
    /// Start an error diagnostic.
    pub fn error(code: DiagnosticCode, span: Span, message: impl Into<String>) -> Self {
        Self {
            code,
            severity: Severity::Error,
            message: message.into(),
            span,
            labels: Vec::new(),
            help: None,
        }
    }

    /// Start a warning diagnostic.
    pub fn warning(code: DiagnosticCode, span: Span, message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Warning,
            ..Self::error(code, span, message)
        }
    }

    /// Attach a labelled secondary span.
    #[must_use]
    pub fn with_label(mut self, span: Span, message: impl Into<String>) -> Self {
        self.labels.push(Label {
            span,
            message: message.into(),
        });
        self
    }

    /// Attach help text.
    #[must_use]
    pub fn with_help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }

    /// Attach help text when there is any.
    #[must_use]
    pub fn with_optional_help(mut self, help: Option<String>) -> Self {
        self.help = help;
        self
    }

    /// Whether this diagnostic rejects the composition.
    #[must_use]
    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }
}

/// An ordered collection of diagnostics.
///
/// Passes push into one of these as they go; [`Diagnostics::sort`] puts them in
/// source order at the end so that a file's diagnostics read top to bottom
/// regardless of the order the passes happened to visit constructs in.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Diagnostics {
    items: Vec<Diagnostic>,
}

impl Diagnostics {
    /// An empty collection.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a diagnostic.
    pub fn push(&mut self, diagnostic: Diagnostic) {
        self.items.push(diagnostic);
    }

    /// Record an error diagnostic, returning nothing — the common call shape.
    pub fn error(&mut self, code: DiagnosticCode, span: Span, message: impl Into<String>) {
        self.push(Diagnostic::error(code, span, message));
    }

    /// Whether anything has been recorded.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// How many diagnostics have been recorded.
    #[must_use]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Whether any recorded diagnostic is an error.
    #[must_use]
    pub fn has_errors(&self) -> bool {
        self.items.iter().any(Diagnostic::is_error)
    }

    /// The diagnostics, in the order they will be reported.
    #[must_use]
    pub fn as_slice(&self) -> &[Diagnostic] {
        &self.items
    }

    /// Iterate over the diagnostics.
    pub fn iter(&self) -> std::slice::Iter<'_, Diagnostic> {
        self.items.iter()
    }

    /// Put the diagnostics in source order: by file, then by start offset, then
    /// by code, so that output is deterministic for a given input (PRD 5.12).
    pub fn sort(&mut self) {
        self.items.sort_by(|a, b| {
            a.span
                .source
                .as_str()
                .cmp(b.span.source.as_str())
                .then(a.span.bytes.start.cmp(&b.span.bytes.start))
                .then(a.span.bytes.end.cmp(&b.span.bytes.end))
                .then(a.code.cmp(&b.code))
                .then(a.message.cmp(&b.message))
        });
    }

    /// Consume the collection, yielding its diagnostics.
    #[must_use]
    pub fn into_vec(self) -> Vec<Diagnostic> {
        self.items
    }
}

impl IntoIterator for Diagnostics {
    type Item = Diagnostic;
    type IntoIter = std::vec::IntoIter<Diagnostic>;

    fn into_iter(self) -> Self::IntoIter {
        self.items.into_iter()
    }
}

impl<'a> IntoIterator for &'a Diagnostics {
    type Item = &'a Diagnostic;
    type IntoIter = std::slice::Iter<'a, Diagnostic>;

    fn into_iter(self) -> Self::IntoIter {
        self.items.iter()
    }
}

impl Extend<Diagnostic> for Diagnostics {
    fn extend<T: IntoIterator<Item = Diagnostic>>(&mut self, iter: T) {
        self.items.extend(iter);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(source: &str, bytes: Range<usize>, line: u32, column: u32) -> Span {
        Span::new(
            SourceName::new(source),
            bytes,
            Position::new(line, column),
            Position::new(line, column + 1),
        )
    }

    /// The uniqueness rule is only worth as much as the list it runs over, so
    /// the list is checked against the enum first: every entry of
    /// [`DiagnosticCode::ALL`] must sit at its own variant's position, which an
    /// entry that is missing, duplicated, or out of order fails.
    #[test]
    fn codes_are_kebab_case_and_unique() {
        let mut seen = std::collections::BTreeSet::new();
        for (position, code) in DiagnosticCode::ALL.iter().enumerate() {
            assert_eq!(
                *code as usize, position,
                "`{code}` is not the {position}th variant: `DiagnosticCode::ALL` is not the enum"
            );
            let text = code.as_str();
            assert!(
                text.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
                "{text} is not kebab-case"
            );
            assert!(seen.insert(text), "{text} is used by two variants");
        }
        assert_eq!(
            DiagnosticCode::ALL.len(),
            DiagnosticCode::MissingRegistryToken as usize + 1,
            "`DiagnosticCode::ALL` stops short of the last declared variant"
        );
    }

    #[test]
    fn diagnostics_sort_into_source_order() {
        let mut diagnostics = Diagnostics::new();
        diagnostics.error(
            DiagnosticCode::UnknownKey,
            span("b.yml", 0..1, 1, 1),
            "second file",
        );
        diagnostics.error(
            DiagnosticCode::UnknownKey,
            span("a.yml", 20..21, 3, 5),
            "later in the first file",
        );
        diagnostics.error(
            DiagnosticCode::MissingKey,
            span("a.yml", 4..5, 1, 5),
            "earlier in the first file",
        );
        diagnostics.sort();
        let messages: Vec<&str> = diagnostics.iter().map(|d| d.message.as_str()).collect();
        assert_eq!(
            messages,
            [
                "earlier in the first file",
                "later in the first file",
                "second file"
            ]
        );
    }

    #[test]
    fn spans_debug_as_positions_only() {
        let span = Span::new(
            SourceName::new("main.yml"),
            10..14,
            Position::new(3, 5),
            Position::new(3, 9),
        );
        assert_eq!(format!("{span:?}"), "3:5..3:9");
        assert_eq!(span.to_string(), "main.yml:3:5");
        assert_eq!(
            format!("{:?}", Spanned::new("draft", span)),
            "\"draft\" @ 3:5..3:9"
        );
    }

    #[test]
    fn joining_spans_covers_both_ends() {
        let key = Span::new(
            SourceName::new("main.yml"),
            10..24,
            Position::new(3, 1),
            Position::new(3, 15),
        );
        let body = Span::new(
            SourceName::new("main.yml"),
            28..90,
            Position::new(4, 3),
            Position::new(9, 24),
        );
        let joined = key.joined(&body);
        assert_eq!(format!("{joined:?}"), "3:1..9:24");
        assert_eq!(joined.bytes, 10..90);
        // Order does not matter, and a span joined with itself is itself.
        assert_eq!(body.joined(&key), joined);
        assert_eq!(key.joined(&key), key);
    }
}
