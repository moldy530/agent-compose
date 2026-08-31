//! Bindings and implementation blocks: the shapes a tool definition and an
//! inline node share (grammar 6.1, 8.0, 8.2, 8.3).

use crate::diag::{Span, Spanned};

use super::common::{Cel, Ident, Interpolated};
use super::schema::FieldMap;

/// A node's input bindings (grammar 8.0).
#[derive(Clone, Debug, PartialEq)]
pub enum NodeInput {
    /// The scalar form, `input: "input.goal"`, binding a string-in agent's
    /// single unnamed input (Decision D14).
    Scalar(Spanned<Cel>),
    /// Per-field bindings.
    Fields(Bindings),
}

/// A mapping from a name to a CEL expression: node input bindings, an `http`
/// block's `query`/`body`, a store op's `value`/`filter`/`metadata`, a
/// trigger's `input`.
#[derive(Clone, Debug, PartialEq)]
pub struct Bindings {
    /// The bindings, in declaration order.
    pub entries: Vec<Binding>,
    /// The mapping's own span.
    pub span: Span,
}

impl Bindings {
    /// Whether nothing is bound (`{}`).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// One entry of a [`Bindings`] map.
#[derive(Clone, Debug, PartialEq)]
pub struct Binding {
    /// The bound name. Identifier positions hold an [`Ident`]; header and query
    /// parameter names, which are not identifiers, keep their raw text.
    pub name: Spanned<String>,
    /// The expression bound to it.
    pub value: Spanned<Cel>,
}

/// A `writes:` remap: output field name to state channel name (grammar 8.0,
/// Decision D16).
#[derive(Clone, Debug, PartialEq)]
pub struct Writes {
    /// The remaps, in declaration order.
    pub entries: Vec<WriteEntry>,
    /// The mapping's own span.
    pub span: Span,
}

/// One entry of a [`Writes`] map.
#[derive(Clone, Debug, PartialEq)]
pub struct WriteEntry {
    /// An output field of the node.
    pub field: Spanned<Ident>,
    /// The declared channel it is written to.
    pub channel: Spanned<Ident>,
}

/// A name/value pair whose value may embed `${ENV}` references: an `exec`
/// block's `env:`, an `http` block's `headers:`.
#[derive(Clone, Debug, PartialEq)]
pub struct InterpolatedEntry {
    /// The name, kept raw: environment variable and header names are not
    /// identifiers.
    pub name: Spanned<String>,
    /// The value.
    pub value: Spanned<Interpolated>,
}

/// An `exec:` block: a `tool.*` implementation binding (grammar 6.1) or an
/// inline node's subprocess step (grammar 8.2).
///
/// The whole surface is **interpolable**: `command`, every entry of `args`,
/// `cwd`, and `env` values may embed `${ENV}` references, substituted at
/// process start (grammar 4.3 class 2, Decision D92). That is orthogonal to
/// Decision D24's rule that `args` carries no *CEL* — CEL reads graph data at
/// run time, an env ref reads the process environment at start.
#[derive(Clone, Debug, PartialEq)]
pub struct ExecBlock {
    /// `command:` — argv[0], never shell-interpreted.
    pub command: Option<Spanned<Interpolated>>,
    /// `args:` — literal argv entries, no CEL (Decision D24).
    pub args: Vec<Spanned<Interpolated>>,
    /// `cwd:`.
    pub cwd: Option<Spanned<Interpolated>>,
    /// `env:` — added to the child environment.
    pub env: Vec<InterpolatedEntry>,
    /// `expect_exit:` — the accepted exit statuses, non-empty and distinct.
    /// Empty here means the key was not declared, which is the default `[0]`
    /// (grammar 6.1, Decisions D84, D100).
    pub expect_exit: Vec<Spanned<i64>>,
    /// `output:` — inline nodes only; a tool declares its result schema on the
    /// definition instead. Defaults to `{ exit_code, stdout }` (grammar 8.2).
    pub output: Option<FieldMap>,
    /// The block's own span.
    pub span: Span,
}

/// The HTTP methods an implementation may use (grammar 6.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HttpMethod {
    /// `GET`
    Get,
    /// `POST`
    Post,
    /// `PUT`
    Put,
    /// `PATCH`
    Patch,
    /// `DELETE`
    Delete,
    /// `HEAD`
    Head,
    /// `OPTIONS`
    Options,
}

impl HttpMethod {
    /// Every method, in the order grammar 6.1 lists them.
    pub const ALL: &'static [Self] = &[
        Self::Get,
        Self::Post,
        Self::Put,
        Self::Patch,
        Self::Delete,
        Self::Head,
        Self::Options,
    ];

    /// The method's spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
            Self::Put => "PUT",
            Self::Patch => "PATCH",
            Self::Delete => "DELETE",
            Self::Head => "HEAD",
            Self::Options => "OPTIONS",
        }
    }

    /// Whether this method carries a request body. `GET`/`HEAD` do not, which
    /// is what makes `body:` illegal on them and `query:` the slot a bound
    /// input object claims (grammar 6.1, 8.3).
    #[must_use]
    pub const fn carries_body(self) -> bool {
        !matches!(self, Self::Get | Self::Head)
    }
}

/// An `http:` block: a `tool.*` implementation binding (grammar 6.1) or an
/// inline node's request (grammar 8.3).
#[derive(Clone, Debug, PartialEq)]
pub struct HttpBlock {
    /// `method:` — required; effects are never defaulted.
    pub method: Option<Spanned<HttpMethod>>,
    /// `url:`.
    pub url: Option<Spanned<Interpolated>>,
    /// `headers:`.
    pub headers: Vec<InterpolatedEntry>,
    /// `query:` — parameter name to CEL.
    pub query: Option<Bindings>,
    /// `body:` — field name to CEL; illegal for `GET`/`HEAD`.
    pub body: Option<Bindings>,
    /// `expect_status:` — the accepted response statuses, non-empty and
    /// distinct. Empty here means the key was not declared, which is the
    /// default "any 2xx" (grammar 6.1, Decisions D84, D100).
    pub expect_status: Vec<Spanned<i64>>,
    /// `output:` — inline nodes only. Defaults to `{ status, body }`
    /// (grammar 8.3).
    pub output: Option<FieldMap>,
    /// The block's own span.
    pub span: Span,
}

/// A `function:` implementation binding: a host-registered function looked up
/// by name at build time (grammar 6.1).
#[derive(Clone, Debug, PartialEq)]
pub struct FunctionBinding {
    /// `name:` — the registry key.
    pub name: Option<Spanned<Ident>>,
    /// The block's own span.
    pub span: Span,
}

/// A `module:` implementation binding: a hand-authored TypeScript file inside
/// the project, under the contract the tool's own `input:`/`output:` fixes
/// (grammar 6.1, PRD resolved q48, q49).
///
/// The scalar form — `module: ./src/tools/sign.ts` — is the same block with
/// nothing but its `path:`, so a tool that reads no environment and pulls in no
/// package writes one line.
#[derive(Clone, Debug, PartialEq)]
pub struct ModuleBlock {
    /// `path:` — the project-relative path of the authored file, `/`-separated
    /// and ending in `.ts`.
    pub path: Option<Spanned<String>>,
    /// `env:` — the environment this module's code may read. Declared rather
    /// than walked, because `References::of` cannot follow a `process.env` read
    /// inside arbitrary TypeScript (PRD resolved q49); interpolable, exactly as
    /// an `exec:` block's `env:` is (grammar 4.3 class 2).
    pub env: Vec<InterpolatedEntry>,
    /// `dependencies:` — the npm packages the module imports, each pinned to an
    /// exact version (PRD resolved q49).
    pub dependencies: Vec<DependencyEntry>,
    /// The block's own span. For the scalar form this is the path's own span,
    /// which is the only region the binding occupies.
    pub span: Span,
}

/// One `dependencies:` entry of a [`ModuleBlock`]: a package name and the exact
/// version it is pinned to.
#[derive(Clone, Debug, PartialEq)]
pub struct DependencyEntry {
    /// The package name, kept raw: npm names are not identifiers.
    pub package: Spanned<String>,
    /// The exact version. Never a range — the artifact ships no lockfile, so a
    /// pin is what keeps the hub's install and every worker's resolving one
    /// tree (PRD resolved q49).
    pub version: Spanned<String>,
}
