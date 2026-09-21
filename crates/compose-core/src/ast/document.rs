//! The two document kinds (grammar 1.2) and the sections they carry.

use crate::diag::{SourceName, Span, Spanned};

use super::common::Ident;
use super::definition::Definition;
use super::deploy::{
    EventSourcesSection, HubSection, JournalSection, PackageRegistrySection, PlacementsSection,
    StorageBackendsSection, TraceSinkSection,
};
use super::policy::PolicyBlock;
use super::schema::TypeNode;
use super::trigger::TriggersSection;

/// Which of the two disjoint document kinds a file is (grammar 1.2,
/// Decision D3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DocumentKind {
    /// A spec file: definitions plus `version`, `imports`, `defaults`,
    /// `state`, `triggers`.
    Spec,
    /// A deploy file: `version` plus `hub`, `placements`, `storage_backends`,
    /// `journal`, `package_registry`, `trace_sink`, `event_sources`.
    Deploy,
}

impl DocumentKind {
    /// How this kind is named in a diagnostic.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Spec => "spec file",
            Self::Deploy => "deploy file",
        }
    }
}

/// One parsed file: a spec file or a deploy file.
///
/// The variants differ in size because the two document kinds do; each is built
/// once per file and matched by reference from then on, so boxing would add
/// indirection to every consumer and buy nothing.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq)]
pub enum Document {
    /// A spec file.
    Spec(SpecFile),
    /// A deploy file.
    Deploy(DeployFile),
}

impl Document {
    /// Which kind this document is.
    #[must_use]
    pub const fn kind(&self) -> DocumentKind {
        match self {
            Self::Spec(_) => DocumentKind::Spec,
            Self::Deploy(_) => DocumentKind::Deploy,
        }
    }

    /// The file this document was parsed from.
    #[must_use]
    pub const fn source(&self) -> &SourceName {
        match self {
            Self::Spec(file) => &file.source,
            Self::Deploy(file) => &file.source,
        }
    }

    /// The `version:` value, if the file declared one.
    #[must_use]
    pub const fn version(&self) -> Option<&Spanned<String>> {
        match self {
            Self::Spec(file) => file.version.as_ref(),
            Self::Deploy(file) => file.version.as_ref(),
        }
    }

    /// This document as a spec file, if it is one.
    #[must_use]
    pub const fn as_spec(&self) -> Option<&SpecFile> {
        match self {
            Self::Spec(file) => Some(file),
            Self::Deploy(_) => None,
        }
    }

    /// This document as a deploy file, if it is one.
    #[must_use]
    pub const fn as_deploy(&self) -> Option<&DeployFile> {
        match self {
            Self::Deploy(file) => Some(file),
            Self::Spec(_) => None,
        }
    }
}

/// A spec file (grammar 1.2).
#[derive(Clone, Debug, PartialEq)]
pub struct SpecFile {
    /// The file this was parsed from.
    pub source: SourceName,
    /// `version:` — required in the entrypoint, optional elsewhere
    /// (Decision D4). Whether *this* file is the entrypoint is a
    /// composition-level fact, so the resolver enforces the requirement; the
    /// parser enforces it only where the file itself proves it, i.e. when
    /// `imports:` is present.
    pub version: Option<Spanned<String>>,
    /// `imports:` — entrypoint only (Decision D1).
    pub imports: Option<ImportsSection>,
    /// `defaults:` — level 3 of the policy resolution chain.
    pub defaults: Option<Spanned<PolicyBlock>>,
    /// `state:` — the graph's channels.
    pub state: Option<StateSection>,
    /// `triggers:` — what causes an execution to exist.
    pub triggers: Option<TriggersSection>,
    /// The typed-address definitions this file declares, in declaration order.
    pub definitions: Vec<Definition>,
    /// The document root's span.
    pub span: Span,
}

/// A deploy file (grammar 14).
#[derive(Clone, Debug, PartialEq)]
pub struct DeployFile {
    /// The file this was parsed from.
    pub source: SourceName,
    /// `version:` — required in every deploy file.
    pub version: Option<Spanned<String>>,
    /// `placements:` — the named claims a worker asserts at join (grammar 14.1).
    pub placements: Option<PlacementsSection>,
    /// `hub:` — the process that owns the graph (grammar 14.2).
    pub hub: Option<HubSection>,
    /// `storage_backends:`
    pub storage_backends: Option<StorageBackendsSection>,
    /// `journal:` — which backend this target's execution journal binds
    /// (grammar 14.7).
    pub journal: Option<JournalSection>,
    /// `package_registry:` — where this target's installer resolves packages
    /// from (grammar 14.6).
    pub package_registry: Option<PackageRegistrySection>,
    /// `trace_sink:` — where every settled execution's trace ships
    /// (grammar 14.5).
    pub trace_sink: Option<TraceSinkSection>,
    /// `event_sources:` — reserved grammar.
    pub event_sources: Option<EventSourcesSection>,
    /// The document root's span.
    pub span: Span,
}

/// The `imports:` section (grammar 1.4).
#[derive(Clone, Debug, PartialEq)]
pub struct ImportsSection {
    /// The imported paths, in declaration order (which is not semantic, but is
    /// preserved for reporting).
    pub paths: Vec<Spanned<ImportPath>>,
    /// How many entries the parser refused outright — a value that is not a
    /// string, or a path its lexical rules reject: absolute, a URL, a glob, an
    /// environment reference, the wrong extension, or outside the portable
    /// charset. Each one named a file that is now missing from the composition
    /// along with every definition it declares, and [`Self::paths`] cannot say
    /// so because a refused entry never reaches it.
    ///
    /// An entry the parser refused for being a *verbatim repeat* is not counted:
    /// the file is in the composition under the first spelling, so nothing is
    /// missing. The resolver reads this to decide whether the composition's name
    /// table is all there, and withholds the reference pass when it is not.
    pub dropped: usize,
    /// The section's own span.
    pub span: Span,
}

/// One `imports:` entry: a relative path to a spec file.
///
/// The parser decides the lexical rules grammar 1.4 calls parse errors — no
/// absolute paths, URLs, or globs, and a `.yml`/`.yaml` extension — and rejects
/// a path repeated verbatim. Whether the path resolves inside the project root,
/// collides with another *after normalization*, or names the entrypoint itself
/// needs the composition, so the resolver decides those.
#[derive(Clone, PartialEq, Eq)]
pub struct ImportPath(String);

impl ImportPath {
    /// Wrap a path as written.
    #[must_use]
    pub fn new(path: impl Into<String>) -> Self {
        Self(path.into())
    }

    /// The path as written.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for ImportPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(&self.0, f)
    }
}

/// The `state:` section (grammar 10).
#[derive(Clone, Debug, PartialEq)]
pub struct StateSection {
    /// The declared channels, in declaration order.
    pub channels: Vec<Channel>,
    /// The section's own span.
    pub span: Span,
}

/// One state channel: a type node plus the channel-only `reduce` key
/// (grammar 10.1). The channel's `default:` lives on its type node.
#[derive(Clone, Debug, PartialEq)]
pub struct Channel {
    /// The channel name; never a reserved identifier (grammar 2.5).
    pub name: Spanned<Ident>,
    /// The channel's type.
    pub ty: TypeNode,
    /// `reduce:` — absent means unreduced: single-writer, sequential.
    pub reduce: Option<Spanned<Reduce>>,
    /// The whole entry's span, name and body together. [`Self::name`] carries
    /// the name alone, for the diagnostics that are about the name.
    pub span: Span,
}

/// A channel's concurrency policy (grammar 10.2, Decision D32).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reduce {
    /// Each write contributes one element; requires an array channel.
    Append,
    /// Shallow key-wise merge; requires an object channel.
    Merge,
    /// Last write in the superstep wins; legal on any channel.
    LastWins,
}

impl Reduce {
    /// Every policy, in the order grammar 10.2 lists them.
    pub const ALL: &'static [Self] = &[Self::Append, Self::Merge, Self::LastWins];

    /// The keyword that names this policy.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Append => "append",
            Self::Merge => "merge",
            Self::LastWins => "last_wins",
        }
    }
}

/// The top-level keys a spec file may carry, besides definition keys
/// (grammar 1.5).
pub const SPEC_SECTIONS: &[&str] = &["version", "imports", "defaults", "state", "triggers"];

/// The top-level keys a deploy file may carry (grammar 1.5).
pub const DEPLOY_SECTIONS: &[&str] = &[
    "version",
    "hub",
    "placements",
    "storage_backends",
    "journal",
    "package_registry",
    "trace_sink",
    "event_sources",
];
