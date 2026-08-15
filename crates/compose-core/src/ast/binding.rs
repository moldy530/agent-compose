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
#[derive(Clone, Debug, PartialEq)]
pub struct ExecBlock {
    /// `command:` — argv[0], never shell-interpreted.
    pub command: Option<Spanned<String>>,
    /// `args:` — literal strings, no CEL (Decision D24).
    pub args: Vec<Spanned<String>>,
    /// `cwd:`.
    pub cwd: Option<Spanned<Interpolated>>,
    /// `env:` — added to the child environment.
    pub env: Vec<InterpolatedEntry>,
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
    /// `expect_status:` — defaults to any 2xx.
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
