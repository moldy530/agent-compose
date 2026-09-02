//! Bindings, write remaps, and the two implementation blocks (grammar 6.1,
//! 8.0, 8.2, 8.3).

use serde::Serialize;

use crate::ast::binding::HttpMethod;
use crate::ast::common::{Cel, Ident, Interpolated};
use crate::diag::{Span, Spanned};

use super::schema::FieldMap;

/// A node's input bindings (grammar 8.0).
///
/// The scalar form supplies one unnamed value and is legal only where such a
/// value has a destination — a string-in agent, an inline `exec:` node's stdin,
/// and a `map` dispatch (Decision D88).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "form", rename_all = "snake_case")]
pub enum NodeInput {
    /// `input: "input.goal"` — one unnamed value.
    Scalar {
        /// The expression supplying it.
        value: Spanned<Cel>,
    },
    /// `input: { goal: "input.goal" }` — per-field bindings.
    Fields {
        /// The bindings.
        bindings: Bindings,
    },
}

/// A mapping from a name to a CEL expression: node input bindings, an `http`
/// block's `query`/`body`, a store op's `value`/`filter`/`metadata`, a
/// trigger's `input`.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Bindings {
    /// The bindings, in declaration order.
    pub entries: Vec<Binding>,
    /// The mapping's own span.
    pub span: Span,
}

/// One entry of a [`Bindings`] map.
///
/// Both halves keep a span, because both are checked and against different
/// things: the name against the target's declared input fields, the expression
/// against the roots its surface exposes (grammar 4.1, 8.0).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Binding {
    /// The bound name. Identifier positions hold an identifier; header and
    /// query parameter names, which are not identifiers, keep their raw text.
    pub name: Spanned<String>,
    /// The expression bound to it.
    pub value: Spanned<Cel>,
}

/// A `writes:` remap: output field name to state channel name (Decision D16).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Writes {
    /// The remaps, in declaration order.
    pub entries: Vec<WriteEntry>,
    /// The mapping's own span.
    pub span: Span,
}

/// One entry of a [`Writes`] map.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct WriteEntry {
    /// An output field of the node.
    pub field: Spanned<Ident>,
    /// The channel it is written to, which `state:` must declare.
    pub channel: Spanned<Ident>,
}

/// A name/value pair whose value may embed `${ENV}` references: an `exec`
/// block's `env:`, an `http` block's `headers:` (grammar 4.3 class 2).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct InterpolatedEntry {
    /// The name, kept raw: environment variable and header names are not
    /// identifiers.
    pub name: Spanned<String>,
    /// The value.
    pub value: Spanned<Interpolated>,
}

/// An `exec:` block: a `tool.*` implementation binding (grammar 6.1) or an
/// inline node's subprocess step (grammar 8.2).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Exec {
    /// `command:` — argv[0], never shell-interpreted.
    pub command: Spanned<Interpolated>,
    /// `args:` — literal argv entries, in order, carrying no CEL (D24).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<Spanned<Interpolated>>,
    /// `cwd:`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<Spanned<Interpolated>>,
    /// `env:` — added to the child environment.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub env: Vec<InterpolatedEntry>,
    /// `expect_exit:` — the accepted exit statuses, non-empty and distinct.
    /// Absent means the default, `[0]` (grammar 6.1, Decisions D84, D100).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub expect_exit: Vec<i64>,
    /// `output:` — inline nodes only; a tool declares its result schema on the
    /// definition instead. Absent means the kind default,
    /// `{ exit_code, stdout }` (grammar 8.2).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<FieldMap>,
    /// The block's own span.
    pub span: Span,
}

/// An `http:` block: a `tool.*` implementation binding (grammar 6.1) or an
/// inline node's request (grammar 8.3).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Http {
    /// `method:` — effects are never defaulted.
    pub method: Spanned<HttpMethod>,
    /// `url:`.
    pub url: Spanned<Interpolated>,
    /// `headers:`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub headers: Vec<InterpolatedEntry>,
    /// `query:` — parameter name to CEL.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query: Option<Bindings>,
    /// `body:` — field name to CEL; illegal for `GET`/`HEAD`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<Bindings>,
    /// `expect_status:` — the accepted response statuses, non-empty and
    /// distinct. Absent means the default, any 2xx (Decisions D84, D100).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub expect_status: Vec<i64>,
    /// `output:` — inline nodes only. Absent means the kind default,
    /// `{ status, body }` (grammar 8.3).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<FieldMap>,
    /// The block's own span.
    pub span: Span,
}

/// A `function:` implementation binding: a host-registered function looked up
/// by name at build time (grammar 6.1).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct FunctionBinding {
    /// `name:` — the registry key.
    pub name: Spanned<Ident>,
    /// The block's own span.
    pub span: Span,
}

/// A `module:` implementation binding: a hand-authored TypeScript file inside
/// the project (grammar 6.1, PRD resolved q48, q49).
///
/// The path is the **normalized** project-relative spelling, which is the name
/// the artifact's file list, the manifest and every diagnostic downstream use —
/// one portable spelling, as grammar 1.4 asks of every path in this DSL.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Module {
    /// `path:` — the authored file, project-relative and `/`-separated.
    pub path: Spanned<String>,
    /// `env:` — the environment this module's code may read. Declared rather
    /// than walked: `References::of` cannot follow a `process.env` read inside
    /// arbitrary TypeScript, so the YAML is where the partition learns of it
    /// (PRD resolved q49, `docs/distributed.md` §9.1).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub env: Vec<InterpolatedEntry>,
    /// `dependencies:` — the npm packages the module imports, each at an exact
    /// version, in declaration order.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub dependencies: Vec<Dependency>,
    /// The block's own span.
    pub span: Span,
}

/// A `builtin:` implementation binding: one of the built-in tools, configured
/// (grammar 6.1, Decision D135, PRD resolved q54).
///
/// Every field but the name is a **bound**, and every bound is optional: a
/// built-in written with nothing beside its name is the same tool the `tools:`
/// shorthand attaches, which is why the shorthand needs no definition of its
/// own.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Builtin {
    /// `builtin:` — which built-in this tool is.
    pub builtin: Spanned<crate::ast::definition::Builtin>,
    /// `workspace:` — the directory this tool works inside. Absent means the
    /// execution's own built-in workspace, made fresh per execution and shared
    /// by every built-in that took the default.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace: Option<Spanned<Interpolated>>,
    /// `timeout:` — how long one command may run. `builtin.bash` only; absent
    /// means the default bound (grammar 6.1).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<Spanned<crate::ast::common::Duration>>,
    /// `env:` — the environment this tool's children run with, exactly the
    /// `exec:` shape (grammar 4.3 class 2). `builtin.bash` only.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub env: Vec<InterpolatedEntry>,
    /// `inherit_env:` — whether those children also see this process's own
    /// environment. Absent means `false`, which is the scrubbed default PRD
    /// resolved q54 ruling b fixes. `builtin.bash` only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inherit_env: Option<Spanned<bool>>,
    /// The definition's own span, which is the region the binding occupies.
    pub span: Span,
}

/// One `dependencies:` entry of a [`Module`].
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Dependency {
    /// The package name.
    pub package: Spanned<String>,
    /// The exact version it is pinned to.
    pub version: Spanned<String>,
}
