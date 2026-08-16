//! The CEL front-end: one expression in, its type, what it reads, and what is
//! wrong with it (grammar 4.1, 4.2).
//!
//! CEL is the only expression language in the spec, and every occurrence of it
//! is a YAML string the parser carried through untouched: which roots are in
//! scope depends on the *surface* the string was written at, and what a path
//! through a root means depends on schemas that routinely live in another file
//! (grammar 4.1). Both facts are known only here, which is why this module is
//! the first pass that looks inside an expression at all.
//!
//! # Parsing
//!
//! Parsing is the [`cel`](https://docs.rs/cel) crate's, pinned to an exact
//! version in `Cargo.toml`. It carries the ANTLR grammar the CEL specification
//! publishes, so what parses here is what CEL defines rather than a subset this
//! project would then have to keep in step with the JS evaluator M1 embeds in
//! generated routers. The crate expands the comprehension macros (`all`,
//! `exists`, `exists_one`, `filter`, `map`) and `has()` at parse time, exactly
//! as the specification says a conforming implementation must, so the walk
//! below sees `Comprehension` and `Select { test: true }` nodes rather than
//! calls.
//!
//! # What is checked here, and what is not
//!
//! Checked, per grammar 4.1:
//!
//! * the expression parses;
//! * every **root identifier** is one the surface exposes ([`Scope`]);
//! * every **path** from a typed root resolves against the schema it was
//!   declared with — an unknown member is reported with the offending sub-path
//!   named, and an unknown member of the `state` root is reported as an
//!   undefined channel (grammar 10.3);
//! * every **construct** is one the surface admits: the operators, `size()`,
//!   `has()`, the four string predicates, and the comprehension macros grammar
//!   4.1 enumerates. Anything else — a conversion function, a message literal,
//!   a name no plugin registers — is refused rather than accepted for one
//!   interpreter to implement and the other not (PRD 5.5's two-interpreter
//!   discipline, and PRD's own CEL-drift risk row);
//! * the **types** an operator relates: comparisons and arithmetic over
//!   compatible operands, a string predicate over strings, and — the case
//!   grammar 4.1 names outright — a literal compared against an `enum`-typed
//!   field being one of that field's variants, so
//!   `review.output.verdict == 'aprove'` is a compile error and not a silent
//!   false;
//! * the expression's **result type**, which is what the caller compares
//!   against the destination it was bound to (grammar 8.0).
//!
//! Deferred to run time, and deliberately:
//!
//! * **presence**. Reading a value that is legally absent fails the execution
//!   (Decision D110), and every absence in this grammar is a runtime fact — an
//!   unset channel (D78), an unsupplied `merge` property (D101), an `optional:`
//!   property, a `get` that missed, and `execution.item_index` outside a `map`
//!   dispatch (D115). Type-checking is against **declared** schemas throughout,
//!   never against runtime values, so `load_prefs.output.value.theme` is a
//!   `string` wherever it is legal to write.
//! * **value ranges and formats**. `min_length`, `pattern`, `max_items` and the
//!   rest constrain a declaration, and an expression is typed by its result
//!   (grammar 8.0), which carries no declaration of its own. Where two
//!   *declarations* meet with nothing between them, the constraint-aware
//!   comparison is `check::model`'s `satisfies`.
//! * **evaluation**. Nothing here executes an expression. What the two
//!   interpreters must agree on is pinned by the conformance corpus in
//!   `tests/fixtures/cel-conformance/`, which the JS evaluator M1 embeds has to
//!   pass unchanged.
//!
//! # Diagnostics
//!
//! A [`Problem`] carries a message and a code but no span: a CEL expression is
//! one YAML scalar, and its span is the string's. Sub-expression positions are
//! not mapped back into the file — a quoted scalar, a block scalar, and an
//! aliased node all put the expression's characters somewhere different — so
//! the offending sub-path is named *in the message* instead, which is what
//! grammar 4.1's "with the offending sub-path named" asks for and is stable
//! across all three spellings.

pub mod ty;

use std::sync::Arc;

use ::cel::Program;
use ::cel::common::ast::{Expr, IdedExpr, LiteralValue};

use crate::diag::DiagnosticCode;
use crate::parse::reader::{list, suggest};

pub use ty::{ObjectShape, Origin, Property, Type, UnionShape, UnionVariant};

/// The roots one surface exposes, and what they are (grammar 4.1).
#[derive(Clone, Debug, Default)]
pub struct Scope {
    /// How the surface is named in a diagnostic: "an edge guard", "a node
    /// `input:` binding".
    pub surface: String,
    /// The roots, in the order grammar 4.1's table lists them.
    pub roots: Vec<(String, Type)>,
}

impl Scope {
    /// A scope over these roots.
    #[must_use]
    pub fn new(surface: impl Into<String>, roots: Vec<(String, Type)>) -> Self {
        Self {
            surface: surface.into(),
            roots,
        }
    }

    /// The type of a root, if this surface exposes it.
    #[must_use]
    pub fn root(&self, name: &str) -> Option<&Type> {
        self.roots
            .iter()
            .find(|(root, _)| root == name)
            .map(|(_, ty)| ty)
    }

    /// Every root name, in table order.
    #[must_use]
    pub fn names(&self) -> Vec<&str> {
        self.roots.iter().map(|(name, _)| name.as_str()).collect()
    }
}

/// A root-anchored path an expression reads: `input.goal`, `payload.body`.
///
/// Two rules read these rather than the expression: item-derivation of a store
/// key, which asks whether a `key:` references the item binding or an
/// item-derived `input.<field>` (grammar 11.4, Decision D83), and a `GET`
/// trigger reading *through* `payload.body` (grammar 13.3, Decision D117).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Read {
    /// The root identifier the path starts at.
    pub root: String,
    /// The member selections applied to it, outermost first.
    pub path: Vec<String>,
    /// Whether the path was then indexed (`payload.body['goal']`). An index is
    /// a read *through* the path even though it adds no member name.
    pub indexed: bool,
}

impl Read {
    /// The path as written, for a diagnostic.
    #[must_use]
    pub fn spelling(&self) -> String {
        let mut text = self.root.clone();
        for segment in &self.path {
            text.push('.');
            text.push_str(segment);
        }
        if self.indexed {
            text.push_str("[…]");
        }
        text
    }

    /// Whether this read goes *through* `root.first` — selecting a member of
    /// it or indexing it, rather than taking it whole.
    #[must_use]
    pub fn reads_through(&self, root: &str, first: &str) -> bool {
        self.root == root
            && self.path.first().is_some_and(|segment| segment == first)
            && (self.path.len() > 1 || self.indexed)
    }
}

/// One thing wrong with an expression.
///
/// Carries no span: see the module documentation.
#[derive(Clone, Debug, PartialEq)]
pub struct Problem {
    /// The failure class.
    pub code: DiagnosticCode,
    /// The one-line message.
    pub message: String,
    /// Optional follow-up.
    pub help: Option<String>,
}

impl Problem {
    fn new(code: DiagnosticCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            help: None,
        }
    }

    fn with_help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }
}

/// What the front-end made of one expression.
#[derive(Clone, Debug)]
pub struct Analysis {
    /// The expression's result type, or [`Type::Dyn`] where the walk could not
    /// decide one — including everywhere a problem was reported, so a caller
    /// comparing the result against a destination never reports the same
    /// mistake twice.
    pub ty: Type,
    /// Every root-anchored path the expression reads.
    pub reads: Vec<Read>,
    /// Everything wrong with it.
    pub problems: Vec<Problem>,
}

/// The functions a spec author writes, for the help on an unsupported call.
const AUTHORED_SURFACE: &[&str] = &[
    "size",
    "has",
    "startsWith",
    "endsWith",
    "contains",
    "matches",
    "all",
    "exists",
    "exists_one",
    "filter",
    "map",
];

/// The parser's name for `container[key]`, which [`Walk::expr`] reads itself.
const INDEX: &str = "_[_]";

/// How deeply an expression may nest brackets before the front-end refuses to
/// parse it at all.
///
/// The pinned parser is recursive descent over the CEL grammar, so every
/// nested group costs a full pass down the precedence chain — and the chain is
/// deep enough that nesting exhausts a thread's stack long before anything
/// else about the expression matters. Measured against `cel` 0.14.3 in a debug
/// build on the 2 MiB stack a test thread gets, `Program::compile` survives 10
/// nested brackets and aborts the *process* at 11; the shipped binary has more
/// room, but a limit that only holds on the roomiest stack is not a limit.
///
/// Eight is what is left after that margin, and it is far past anything the
/// grammar's own surface produces: the deepest expression in the example
/// corpus nests two. This is the compiler's bound rather than one grammar 4.1
/// states — hence a diagnostic that says so, rather than a rule an author is
/// expected to have read.
const MAX_NESTING: usize = 8;

/// How many operations — selections, indexes, calls, and operators — an
/// expression may apply before the front-end refuses to read it at all.
///
/// Brackets are not the only way an expression recurses, and they are not even
/// the cheapest: `a.b.c.…` and `1 + 1 + …` nest one AST level per token while
/// costing nothing in [`MAX_NESTING`], and every pass over the result — the
/// parser's own tree walk, the [`Walk`] below, and the drop of what they build
/// — recurses once per level. Measured against `cel` 0.14.3 in a debug build on
/// the 2 MiB stack a test thread gets: a chain of 500 `+` reads cleanly and one
/// of 600 aborts the *process*, a chain of 1000 selections reads and 1200
/// aborts, 400 indexes read and 800 abort.
///
/// A hundred is well inside the tightest of those and far past anything a spec
/// writes: it counts *tokens* rather than levels, and the widest expression the
/// example corpus writes spends five of it —
/// `state.tasks.exists(t, t.startsWith('a')) && has(state.totals.fixed)`, the
/// widest the test corpus writes, spends ten. Because every level of nesting
/// costs at least one counted character and no character is counted for two
/// levels, the bound can be decided before the parser is handed the source —
/// where a bound behind the parser would only ever refuse what the parser had
/// already survived.
const MAX_OPERATIONS: usize = 100;

/// How much of an expression a diagnostic reprints before eliding the rest: a
/// message that quotes 30 KB of source is not a message.
const MAX_SPELLING: usize = 96;

/// Parse and check one expression against the roots its surface exposes.
#[must_use]
pub fn analyze(source: &str, scope: &Scope) -> Analysis {
    let refused = |problem: Problem| Analysis {
        ty: Type::Dyn,
        reads: Vec::new(),
        problems: vec![problem],
    };
    let shape = shape(source);
    if shape.nesting > MAX_NESTING {
        return refused(
            Problem::new(
                DiagnosticCode::InvalidExpression,
                format!(
                    "`{}` nests brackets {} deep, and this compiler reads at most {MAX_NESTING}",
                    spelling(source),
                    shape.nesting
                ),
            )
            .with_help(format!(
                "the bound is the compiler's, not the grammar's: parsing is recursive, so a deeply nested expression is refused with a message rather than exhausting the stack — split {} into shallower parts",
                scope.surface
            )),
        );
    }
    if shape.operations > MAX_OPERATIONS {
        return refused(
            Problem::new(
                DiagnosticCode::InvalidExpression,
                format!(
                    "`{}` applies {} operations, and this compiler reads at most {MAX_OPERATIONS}",
                    spelling(source),
                    shape.operations
                ),
            )
            .with_help(format!(
                "the bound is the compiler's, not the grammar's: reading an expression recurses once per selection, index, call, and operator, so a very long chain of them is refused with a message rather than exhausting the stack — split {} into smaller parts",
                scope.surface
            )),
        );
    }

    let program = match Program::compile(source) {
        Ok(program) => program,
        Err(errors) => {
            let detail = errors
                .errors
                .first()
                .map_or_else(|| "syntax error".to_string(), |error| error.msg.clone());
            return refused(
                Problem::new(
                    DiagnosticCode::InvalidExpression,
                    format!(
                        "`{}` is not a valid CEL expression: {detail}",
                        spelling(source)
                    ),
                )
                .with_help(format!(
                    "{} takes a CEL expression (grammar 4.1)",
                    scope.surface
                )),
            );
        }
    };

    let mut walk = Walk {
        scope,
        reads: Vec::new(),
        problems: Vec::new(),
        bindings: Vec::new(),
    };
    let (ty, path) = walk.expr(program.expression());
    walk.record(path.as_ref());
    Analysis {
        ty,
        reads: walk.reads,
        problems: walk.problems,
    }
}

/// What the source says about its own size, read without parsing it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Shape {
    /// The deepest nesting of `(`, `[` and `{` it reaches.
    nesting: usize,
    /// How many operations it applies: every selection, index, call, grouping,
    /// literal aggregate, and operator character outside a string.
    ///
    /// The count is an over-estimate of the AST's depth and deliberately so —
    /// every level of nesting costs at least one of these characters, and no
    /// character is counted for two levels, so an expression within
    /// [`MAX_OPERATIONS`] cannot nest past it whatever shape it takes.
    operations: usize,
}

/// Read a source's [`Shape`], skipping what a string literal holds — so a
/// `matches('^\\((a|b)\\)$')` pattern is neither a group nor an operator.
///
/// The three bracket kinds share one depth counter, because the parser re-enters
/// the same expression rule for each of them and a mixture nests exactly as
/// deeply as it looks.
fn shape(source: &str) -> Shape {
    let characters: Vec<char> = source.chars().collect();
    let mut depth = 0usize;
    let mut shape = Shape {
        nesting: 0,
        operations: 0,
    };
    let mut index = 0usize;
    while index < characters.len() {
        if let Some(after) = string_literal(&characters, index) {
            index = after;
            continue;
        }
        match characters[index] {
            '(' | '[' | '{' => {
                depth += 1;
                shape.nesting = shape.nesting.max(depth);
                shape.operations += 1;
            }
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            // One character per operation is enough: a two-character operator
            // is counted twice, which over-estimates in the safe direction.
            '.' | '?' | '+' | '-' | '*' | '/' | '%' | '!' | '<' | '>' | '=' | '&' | '|' => {
                shape.operations += 1;
            }
            _ => {}
        }
        index += 1;
    }
    shape
}

/// Where the string literal starting at `index` ends, if one starts there.
///
/// Grammar 4.1's expressions are CEL's, so the literals are CEL's eight: single
/// and triple quotes, either quote character, each of them optionally `r`-raw
/// and optionally `b`-bytes. A *terminated* literal is what this skips —
/// anything else is read as code instead, which over-counts an expression the
/// parser is going to reject anyway and, unlike guessing, cannot be spelled so
/// as to hide a deep chain from the bounds above.
fn string_literal(characters: &[char], index: usize) -> Option<usize> {
    let mut start = index;
    if matches!(characters.get(start), Some('b' | 'B'))
        && matches!(characters.get(start + 1), Some('r' | 'R' | '\'' | '"'))
    {
        start += 1;
    }
    let raw = matches!(characters.get(start), Some('r' | 'R'))
        && matches!(characters.get(start + 1), Some('\'' | '"'));
    if raw {
        start += 1;
    }
    let quote = match characters.get(start) {
        Some(character @ ('\'' | '"')) => *character,
        _ => return None,
    };
    let triple =
        characters.get(start + 1) == Some(&quote) && characters.get(start + 2) == Some(&quote);
    let mut at = start + if triple { 3 } else { 1 };
    while at < characters.len() {
        let character = characters[at];
        if !raw && character == '\\' {
            at += 2;
            continue;
        }
        // A single-quoted literal does not survive a line break, so what
        // follows one is code (grammar 4.1's `STRING`).
        if !triple && (character == '\n' || character == '\r') {
            return None;
        }
        if character == quote {
            if !triple {
                return Some(at + 1);
            }
            if characters.get(at + 1) == Some(&quote) && characters.get(at + 2) == Some(&quote) {
                return Some(at + 3);
            }
        }
        at += 1;
    }
    None
}

/// The expression as a diagnostic spells it: whole where it fits, elided where
/// it does not (see [`MAX_SPELLING`]).
fn spelling(source: &str) -> String {
    if source.chars().count() <= MAX_SPELLING {
        return source.to_string();
    }
    let mut text: String = source.chars().take(MAX_SPELLING).collect();
    text.push('…');
    text
}

/// A path under construction: a root and what has been applied to it so far.
struct Path {
    root: String,
    segments: Vec<Segment>,
}

/// One step along a path.
enum Segment {
    /// A member selection, `.field`.
    Member(String),
    /// An index, `[…]` — or `["field"]`, where the key is a literal.
    Index(Option<String>),
}

impl Path {
    /// The path as written, which is what a diagnostic names the offending
    /// sub-path with (grammar 4.1).
    fn spelling(&self) -> String {
        let mut text = self.root.clone();
        for segment in &self.segments {
            match segment {
                Segment::Member(name) => {
                    text.push('.');
                    text.push_str(name);
                }
                Segment::Index(None) => text.push_str("[…]"),
                Segment::Index(Some(key)) => text.push_str(&format!("[{key:?}]")),
            }
        }
        text
    }

    /// The [`Read`] this path names.
    ///
    /// A read is a *root-anchored* path, so it stops at the first index: what
    /// lies beyond one is a member of a value the composition cannot name
    /// statically, and the two rules that consume reads (Decisions D83, D117)
    /// ask only whether the path was read *through*.
    fn read(&self) -> Read {
        Read {
            root: self.root.clone(),
            path: self
                .segments
                .iter()
                .map_while(|segment| match segment {
                    Segment::Member(name) => Some(name.clone()),
                    Segment::Index(_) => None,
                })
                .collect(),
            indexed: self
                .segments
                .iter()
                .any(|segment| matches!(segment, Segment::Index(_))),
        }
    }
}

struct Walk<'a> {
    scope: &'a Scope,
    reads: Vec<Read>,
    problems: Vec<Problem>,
    /// Comprehension bindings, innermost last.
    bindings: Vec<(String, Type)>,
}

impl Walk<'_> {
    fn problem(&mut self, problem: Problem) {
        if !self.problems.contains(&problem) {
            self.problems.push(problem);
        }
    }

    /// Record a finished path, if the sub-expression produced one.
    fn record(&mut self, path: Option<&Path>) {
        if let Some(path) = path {
            let read = path.read();
            if !self.reads.contains(&read) {
                self.reads.push(read);
            }
        }
    }

    /// The type of a sub-expression whose path, if any, ends here.
    fn value(&mut self, expr: &IdedExpr) -> Type {
        let (ty, path) = self.expr(expr);
        self.record(path.as_ref());
        ty
    }

    fn binding(&self, name: &str) -> Option<&Type> {
        self.bindings
            .iter()
            .rev()
            .find(|(bound, _)| bound == name)
            .map(|(_, ty)| ty)
    }

    fn expr(&mut self, expr: &IdedExpr) -> (Type, Option<Path>) {
        match &expr.expr {
            Expr::Unspecified => (Type::Dyn, None),
            Expr::Literal(value) => (literal(value), None),
            Expr::Ident(name) => (self.ident(name), self.path_root(name)),
            Expr::Select(select) => self.select(select),
            Expr::List(elements) => {
                let mut items = Type::Dyn;
                let mut first = true;
                for element in &elements.elements {
                    let ty = self.value(element);
                    items = if first { ty } else { join(&items, &ty) };
                    first = false;
                }
                (Type::List(Arc::new(items)), None)
            }
            Expr::Map(entries) => {
                let mut values = Type::Dyn;
                let mut first = true;
                for entry in &entries.entries {
                    if let ::cel::common::ast::EntryExpr::MapEntry(entry) = &entry.expr {
                        self.value(&entry.key);
                        let ty = self.value(&entry.value);
                        values = if first { ty } else { join(&values, &ty) };
                        first = false;
                    }
                }
                (Type::Map(Arc::new(values)), None)
            }
            Expr::Struct(structure) => {
                self.problem(
                    Problem::new(
                        DiagnosticCode::InvalidExpression,
                        format!(
                            "`{}{{…}}` constructs a message, which is not part of the expression surface",
                            structure.type_name
                        ),
                    )
                    .with_help(
                        "the spec carries no message types: build values with the schema language instead (grammar 3, 4.1)",
                    ),
                );
                (Type::Dyn, None)
            }
            // An index is the one call that continues a path, so it is read
            // here rather than in `call` — everything else ends one.
            Expr::Call(call) if call.func_name.as_str() == INDEX => self.index(call),
            Expr::Call(call) => (self.call(call), None),
            Expr::Comprehension(comprehension) => (self.comprehension(comprehension), None),
        }
    }

    /// The path a root identifier starts, unless it is a comprehension binding
    /// (which is not a root and names nothing outside the expression).
    fn path_root(&self, name: &str) -> Option<Path> {
        if self.binding(name).is_some() {
            return None;
        }
        Some(Path {
            root: name.to_string(),
            segments: Vec::new(),
        })
    }

    fn ident(&mut self, name: &str) -> Type {
        if let Some(ty) = self.binding(name) {
            return ty.clone();
        }
        if let Some(ty) = self.scope.root(name) {
            return ty.clone();
        }
        // Macro-generated names (`@result`) are always bound above; anything
        // else starting with `@` is not something an author can write.
        if name.starts_with('@') {
            return Type::Dyn;
        }
        let roots = self.scope.names();
        let help = suggest(name, &roots).map_or_else(
            || {
                format!(
                    "{} exposes {} (grammar 4.1)",
                    self.scope.surface,
                    if roots.is_empty() {
                        "no roots".to_string()
                    } else {
                        list(&roots)
                    }
                )
            },
            |root| format!("did you mean `{root}`?"),
        );
        self.problem(
            Problem::new(
                DiagnosticCode::UnknownRoot,
                format!("`{name}` is not a root in scope here"),
            )
            .with_help(help),
        );
        Type::Dyn
    }

    fn select(&mut self, select: &::cel::common::ast::SelectExpr) -> (Type, Option<Path>) {
        let (operand, path) = self.expr(&select.operand);
        let field = select.field.as_str();
        let path = path.map(|mut path| {
            path.segments.push(Segment::Member(field.to_string()));
            path
        });
        let spelling = path
            .as_ref()
            .map_or_else(|| format!(".{field}"), Path::spelling);
        let member = self.member(&operand, field, &spelling);
        if select.test {
            // `has(x.y)` — the parser's expansion of the presence macro. The
            // member still has to resolve, because a field no schema declares
            // can never be present (grammar 4.1); what the expression yields is
            // the answer to the question, not the member.
            self.record(path.as_ref());
            return (Type::Bool, None);
        }
        (member, path)
    }

    /// The type of `<operand>.<field>`, reporting an unknown member.
    fn member(&mut self, operand: &Type, field: &str, spelling: &str) -> Type {
        match operand {
            Type::Dyn => Type::Dyn,
            Type::Map(values) => (**values).clone(),
            Type::Object(shape) => {
                if let Some(property) = shape.property(field) {
                    return property.ty.clone();
                }
                let known = shape.names();
                let (code, message, help) = match &shape.origin {
                    Origin::State => (
                        DiagnosticCode::UndefinedChannel,
                        format!("`{spelling}` names no declared state channel"),
                        suggest(field, &known).map_or_else(
                            || {
                                if known.is_empty() {
                                    "the composition declares no `state:` channels".to_string()
                                } else {
                                    format!("the declared channels are {}", list(&known))
                                }
                            },
                            |channel| format!("did you mean `state.{channel}`?"),
                        ),
                    ),
                    Origin::Node { id, has_output } if !*has_output => (
                        DiagnosticCode::UnknownField,
                        format!("`{spelling}` reads a node that has no output"),
                        format!(
                            "`{id}` is a `map` node, which has no output of its own: its dispatched instances write state instead (grammar 8.6 rule 9)"
                        ),
                    ),
                    Origin::Node { id, .. } if field != "output" => (
                        DiagnosticCode::UnknownField,
                        format!("`{spelling}` is not how a node's result is read"),
                        format!(
                            "a node's output object is `{id}.output.<field>`, and it is readable only from an edge guard and from `map.over` (grammar 4.1, Decision D42)"
                        ),
                    ),
                    origin => (
                        DiagnosticCode::UnknownField,
                        format!("`{spelling}` names no field of {}", describe(origin)),
                        suggest(field, &known).map_or_else(
                            || {
                                if known.is_empty() {
                                    format!("{} declares no fields", describe(origin))
                                } else {
                                    format!("the declared fields are {}", list(&known))
                                }
                            },
                            |name| format!("did you mean `{name}`?"),
                        ),
                    ),
                };
                self.problem(Problem::new(code, message).with_help(help));
                Type::Dyn
            }
            Type::Union(shape) => {
                if field == shape.discriminator {
                    return Type::Enum(Arc::new(
                        shape.variants.iter().map(|v| v.tag.clone()).collect(),
                    ));
                }
                self.problem(
                    Problem::new(
                        DiagnosticCode::UnknownField,
                        format!(
                            "`{spelling}` names no field of a `{}` union",
                            shape.discriminator
                        ),
                    )
                    .with_help(format!(
                        "only the discriminator `{}` is common to every variant; a union's payload is read after `route_by:` narrows it to one variant (grammar 8.6 rule 4)",
                        shape.discriminator
                    )),
                );
                Type::Dyn
            }
            other => {
                self.problem(Problem::new(
                    DiagnosticCode::TypeMismatch,
                    format!("`{spelling}` selects a field of {other}, which has no fields"),
                ));
                Type::Dyn
            }
        }
    }

    fn call(&mut self, call: &::cel::common::ast::CallExpr) -> Type {
        let name = call.func_name.as_str();
        match name {
            "_&&_" | "_||_" => {
                let types = self.operands(call);
                for ty in &types {
                    self.expect(ty, &Type::Bool, "a boolean operand of `&&`/`||`");
                }
                Type::Bool
            }
            "!_" | "@not_strictly_false" => {
                let types = self.operands(call);
                if let Some(ty) = types.first() {
                    self.expect(ty, &Type::Bool, "the operand of `!`");
                }
                Type::Bool
            }
            "-_" => {
                let types = self.operands(call);
                match types.first() {
                    Some(ty) if ty.is_numeric() => ty.clone(),
                    Some(Type::Dyn) | None => Type::Dyn,
                    Some(other) => {
                        self.mismatch("a number", other, "the operand of unary `-`");
                        Type::Dyn
                    }
                }
            }
            "_?_:_" => {
                let types = self.operands(call);
                if let Some(condition) = types.first() {
                    self.expect(condition, &Type::Bool, "the condition of `?:`");
                }
                match (types.get(1), types.get(2)) {
                    (Some(left), Some(right)) => join(left, right),
                    _ => Type::Dyn,
                }
            }
            "_+_" => self.arithmetic(call, true),
            "_-_" | "_*_" | "_/_" | "_%_" => self.arithmetic(call, false),
            "_==_" | "_!=_" => {
                self.equality(call);
                Type::Bool
            }
            "_<_" | "_<=_" | "_>_" | "_>=_" => {
                let types = self.operands(call);
                if let (Some(left), Some(right)) = (types.first(), types.get(1))
                    && !comparable(left, right)
                {
                    self.mismatch(
                        &format!("a value comparable with {left}"),
                        right,
                        "an operand of an ordering comparison",
                    );
                }
                Type::Bool
            }
            "@in" => self.membership(call),
            "size" => {
                let types = self.operands(call);
                if let Some(ty) = types.first() {
                    match ty {
                        Type::Dyn
                        | Type::String
                        | Type::StringLiteral(_)
                        | Type::Enum(_)
                        | Type::Bytes
                        | Type::List(_)
                        | Type::Map(_)
                        | Type::Object(_) => {}
                        other => {
                            self.mismatch(
                                "a string, a list, or a map",
                                other,
                                "the argument of `size()`",
                            );
                        }
                    }
                }
                Type::Int
            }
            "startsWith" | "endsWith" | "contains" | "matches" => {
                let types = self.operands(call);
                for ty in &types {
                    if !ty.is_stringy() {
                        self.mismatch("a string", ty, &format!("an operand of `{name}()`"));
                    }
                }
                Type::Bool
            }
            other => {
                for argument in &call.args {
                    self.value(argument);
                }
                if let Some(target) = &call.target {
                    self.value(target);
                }
                self.problem(
                    Problem::new(
                        DiagnosticCode::InvalidExpression,
                        format!("`{other}` is not a function this expression surface supports"),
                    )
                    .with_help(format!(
                        "the supported set is {} plus the standard operators; there are no custom extension functions in v0 (grammar 4.1)",
                        list(AUTHORED_SURFACE)
                    )),
                );
                Type::Dyn
            }
        }
    }

    /// The types of a call's operands: its receiver, if it has one, then its
    /// arguments.
    fn operands(&mut self, call: &::cel::common::ast::CallExpr) -> Vec<Type> {
        let mut types = Vec::new();
        if let Some(target) = &call.target {
            types.push(self.value(target));
        }
        for argument in &call.args {
            types.push(self.value(argument));
        }
        types
    }

    fn arithmetic(&mut self, call: &::cel::common::ast::CallExpr, additive: bool) -> Type {
        let types = self.operands(call);
        let (Some(left), Some(right)) = (types.first(), types.get(1)) else {
            return Type::Dyn;
        };
        if matches!(left, Type::Dyn) || matches!(right, Type::Dyn) {
            return Type::Dyn;
        }
        if left.is_numeric() && right.is_numeric() {
            // CEL has no implicit numeric conversion: `1 + 2.5` is an error in
            // the specification and in both interpreters.
            if std::mem::discriminant(left) != std::mem::discriminant(right) {
                self.mismatch(
                    &format!("another {left}"),
                    right,
                    "the right operand of an arithmetic operator",
                );
                return Type::Dyn;
            }
            return left.clone();
        }
        if additive {
            if left.is_stringy() && right.is_stringy() {
                return Type::String;
            }
            if let (Type::List(left), Type::List(right)) = (left, right) {
                return Type::List(Arc::new(join(left, right)));
            }
            if matches!(left, Type::Bytes) && matches!(right, Type::Bytes) {
                return Type::Bytes;
            }
        }
        self.mismatch(
            &format!("a value {} can be combined with", left),
            right,
            "the right operand of an arithmetic operator",
        );
        Type::Dyn
    }

    /// `==` and `!=`, where an `enum`-typed operand makes a literal on the
    /// other side a variant check (grammar 4.1, 7.3.1).
    fn equality(&mut self, call: &::cel::common::ast::CallExpr) {
        let types = self.operands(call);
        let (Some(left), Some(right)) = (types.first(), types.get(1)) else {
            return;
        };
        match (left, right) {
            (Type::Enum(variants), Type::StringLiteral(value))
            | (Type::StringLiteral(value), Type::Enum(variants)) => {
                self.variant(variants, value);
            }
            (left, right) => {
                if !left.assignable_to(right) && !right.assignable_to(left) {
                    self.mismatch(
                        &format!("a value comparable with {left}"),
                        right,
                        "the right operand of `==`/`!=`",
                    );
                }
            }
        }
    }

    fn membership(&mut self, call: &::cel::common::ast::CallExpr) -> Type {
        let types = self.operands(call);
        let (Some(element), Some(container)) = (types.first(), types.get(1)) else {
            return Type::Bool;
        };
        // `f in [L₁, …, Lₖ]` is one of the guard shapes exhaustiveness reads
        // (grammar 7.3.1), so every literal in the list has to be a variant of
        // the enum it is compared against.
        if let Type::Enum(variants) = element
            && let Some(argument) = call.args.get(1)
            && let Expr::List(elements) = &argument.expr
        {
            for element in &elements.elements {
                if let Expr::Literal(LiteralValue::String(value)) = &element.expr {
                    self.variant(variants, value.inner());
                }
            }
            return Type::Bool;
        }
        match container {
            Type::Dyn | Type::Object(_) => {}
            Type::List(items) => {
                if !element.assignable_to(items) && !items.assignable_to(element) {
                    self.mismatch(
                        &format!("a value comparable with {items}"),
                        element,
                        "the left operand of `in`",
                    );
                }
            }
            Type::Map(_) => {
                if !element.is_stringy() {
                    self.mismatch("a string", element, "a map key");
                }
            }
            other => {
                self.mismatch("a list or a map", other, "the right operand of `in`");
            }
        }
        Type::Bool
    }

    /// `container[key]`.
    ///
    /// The path survives the index — `matches[0].id` is a read of `matches`,
    /// and the member that follows is looked up in an item of it — so what a
    /// diagnostic beyond an index names is still the whole sub-path the author
    /// wrote (grammar 4.1).
    fn index(&mut self, call: &::cel::common::ast::CallExpr) -> (Type, Option<Path>) {
        let Some(container) = call.args.first() else {
            return (Type::Dyn, None);
        };
        let (container_ty, path) = self.expr(container);
        let mut path = path.map(|mut path| {
            path.segments.push(Segment::Index(None));
            path
        });
        // Recorded before the key is walked, so the reads stay in the order
        // the expression puts them in.
        self.record(path.as_ref());
        let key = call.args.get(1);
        let key_ty = key.map_or(Type::Dyn, |key| self.value(key));
        let ty = match &container_ty {
            Type::Dyn => Type::Dyn,
            Type::List(items) => {
                if !matches!(key_ty, Type::Dyn | Type::Int | Type::Uint) {
                    self.mismatch("an integer", &key_ty, "a list index");
                }
                (**items).clone()
            }
            Type::Map(values) => {
                if !key_ty.is_stringy() {
                    self.mismatch("a string", &key_ty, "a map key");
                }
                (**values).clone()
            }
            // An object indexed by a literal is a member selection written the
            // other way round, and reads as one.
            Type::Object(_) => match &key_ty {
                Type::StringLiteral(field) => {
                    if let Some(path) = &mut path {
                        path.segments.pop();
                        path.segments.push(Segment::Index(Some(field.to_string())));
                    }
                    let spelling = path
                        .as_ref()
                        .map_or_else(|| format!("[{field:?}]"), Path::spelling);
                    self.member(&container_ty, field, &spelling)
                }
                _ => Type::Dyn,
            },
            other => {
                self.mismatch("a list or a map", other, "the operand of an index");
                Type::Dyn
            }
        };
        (ty, path)
    }

    fn comprehension(&mut self, comprehension: &::cel::common::ast::ComprehensionExpr) -> Type {
        let range = self.value(&comprehension.iter_range);
        let element = match &range {
            Type::List(items) => (**items).clone(),
            Type::Map(_) => Type::String,
            Type::Dyn => Type::Dyn,
            other => {
                self.mismatch("a list", other, "the range of a comprehension");
                Type::Dyn
            }
        };
        let accumulator = self.value(&comprehension.accu_init);

        let depth = self.bindings.len();
        self.bindings
            .push((comprehension.iter_var.clone(), element.clone()));
        if let Some(second) = &comprehension.iter_var2 {
            self.bindings.push((second.clone(), element));
        }
        self.bindings
            .push((comprehension.accu_var.clone(), accumulator.clone()));
        self.value(&comprehension.loop_cond);
        let stepped = self.value(&comprehension.loop_step);
        // The accumulator's type after a step is what the result reads, which
        // is how `filter` and `map` come back as lists of the element type
        // rather than as the empty list they start from.
        self.bindings.truncate(
            depth
                + if comprehension.iter_var2.is_some() {
                    2
                } else {
                    1
                },
        );
        self.bindings
            .push((comprehension.accu_var.clone(), join(&accumulator, &stepped)));
        let result = self.value(&comprehension.result);
        self.bindings.truncate(depth);
        result
    }

    fn variant(&mut self, variants: &[String], value: &str) {
        if variants.iter().any(|variant| variant == value) {
            return;
        }
        let known: Vec<&str> = variants.iter().map(String::as_str).collect();
        self.problem(
            Problem::new(
                DiagnosticCode::TypeMismatch,
                format!("`{value}` is not a variant of the enum this compares against"),
            )
            .with_help(suggest(value, &known).map_or_else(
                || format!("the variants are {}", list(&known)),
                |variant| format!("did you mean `{variant}`?"),
            )),
        );
    }

    fn expect(&mut self, actual: &Type, expected: &Type, position: &str) {
        if !actual.assignable_to(expected) {
            self.mismatch(&expected.to_string(), actual, position);
        }
    }

    fn mismatch(&mut self, expected: &str, actual: &Type, position: &str) {
        self.problem(Problem::new(
            DiagnosticCode::TypeMismatch,
            format!("expected {expected} as {position}, found {actual}"),
        ));
    }
}

fn describe(origin: &Origin) -> String {
    match origin {
        Origin::Declared(what) => what.clone(),
        Origin::State => "the state object".to_string(),
        Origin::Execution => "the execution object".to_string(),
        Origin::Node { id, .. } => format!("the node `{id}`"),
        Origin::Payload(kind) => format!("the `{kind}` trigger payload"),
    }
}

fn literal(value: &LiteralValue) -> Type {
    match value {
        LiteralValue::Boolean(_) => Type::Bool,
        LiteralValue::Bytes(_) => Type::Bytes,
        LiteralValue::Double(_) => Type::Double,
        LiteralValue::Int(_) => Type::Int,
        LiteralValue::Null => Type::Null,
        LiteralValue::String(text) => Type::StringLiteral(Arc::from(text.inner())),
        LiteralValue::UInt(_) => Type::Uint,
    }
}

/// The type both branches of a choice can hold.
fn join(left: &Type, right: &Type) -> Type {
    if left == right {
        return left.clone();
    }
    match (left, right) {
        (Type::Dyn, other) | (other, Type::Dyn) => other.clone(),
        // Two known strings are a two-variant enum, which is what makes
        // `flag ? 'low' : 'high'` bind to an `enum` field that declares both.
        (Type::StringLiteral(left), Type::StringLiteral(right)) => {
            Type::Enum(Arc::new(vec![left.to_string(), right.to_string()]))
        }
        (Type::Enum(variants), Type::StringLiteral(value))
        | (Type::StringLiteral(value), Type::Enum(variants)) => {
            let mut merged = variants.as_ref().clone();
            if !merged.iter().any(|variant| variant.as_str() == &**value) {
                merged.push(value.to_string());
            }
            Type::Enum(Arc::new(merged))
        }
        (Type::Enum(left), Type::Enum(right)) => {
            let mut merged = left.as_ref().clone();
            for variant in right.iter() {
                if !merged.contains(variant) {
                    merged.push(variant.clone());
                }
            }
            Type::Enum(Arc::new(merged))
        }
        (left, right) if left.is_stringy() && right.is_stringy() => Type::String,
        (Type::List(left), Type::List(right)) => Type::List(Arc::new(join(left, right))),
        (Type::Map(left), Type::Map(right)) => Type::Map(Arc::new(join(left, right))),
        _ => Type::Dyn,
    }
}

/// Whether two types may be ordered against each other. CEL orders the numeric
/// types across their boundaries — `2.5 > 1` is well-defined — and strings
/// against strings.
fn comparable(left: &Type, right: &Type) -> bool {
    matches!(left, Type::Dyn)
        || matches!(right, Type::Dyn)
        || (left.is_numeric() && right.is_numeric())
        || (left.is_stringy() && right.is_stringy())
        || (matches!(left, Type::Bytes) && matches!(right, Type::Bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cel::ty::{Origin, Property};

    fn property(name: &str, ty: Type) -> Property {
        Property {
            name: name.to_string(),
            ty,
            optional: false,
            defaulted: false,
        }
    }

    /// A guard's scope: a node handle, the state object, and the flow input —
    /// grammar 4.1's first row.
    fn guard_scope() -> Scope {
        let output = Type::object(
            Origin::Declared("`review`'s output".to_string()),
            vec![
                property(
                    "verdict",
                    Type::Enum(Arc::new(vec!["approve".to_string(), "revise".to_string()])),
                ),
                property("feedback", Type::String),
            ],
        );
        Scope::new(
            "an edge guard",
            vec![
                (
                    "review".to_string(),
                    Type::object(
                        Origin::Node {
                            id: "review".to_string(),
                            has_output: true,
                        },
                        vec![property("output", output)],
                    ),
                ),
                (
                    "state".to_string(),
                    Type::object(
                        Origin::State,
                        vec![
                            property("draft", Type::String),
                            property("patches", Type::list(Type::String)),
                        ],
                    ),
                ),
                (
                    "input".to_string(),
                    Type::object(
                        Origin::Declared("`flow.f`'s inputs".to_string()),
                        vec![property("goal", Type::String)],
                    ),
                ),
            ],
        )
    }

    #[track_caller]
    fn ok(source: &str) -> Analysis {
        let analysis = analyze(source, &guard_scope());
        assert!(
            analysis.problems.is_empty(),
            "`{source}` should check cleanly, got {:?}",
            analysis.problems
        );
        analysis
    }

    #[track_caller]
    fn problem(source: &str) -> Problem {
        let analysis = analyze(source, &guard_scope());
        assert_eq!(
            analysis.problems.len(),
            1,
            "`{source}` should draw exactly one problem, got {:?}",
            analysis.problems
        );
        analysis.problems.into_iter().next().expect("one problem")
    }

    #[test]
    fn a_guard_over_declared_schemas_is_a_boolean() {
        assert_eq!(
            ok("review.output.verdict == 'revise' && size(state.patches) > 0").ty,
            Type::Bool
        );
    }

    #[test]
    fn an_unknown_root_names_the_roots_in_scope() {
        let problem = problem("reviw.output.verdict == 'revise'");
        assert_eq!(problem.code, DiagnosticCode::UnknownRoot);
        assert_eq!(problem.message, "`reviw` is not a root in scope here");
        assert_eq!(problem.help.as_deref(), Some("did you mean `review`?"));
    }

    /// Grammar 4.1 asks for the offending sub-path by name, not for the whole
    /// expression back.
    #[test]
    fn an_unknown_member_names_the_sub_path() {
        let problem = problem("review.output.verdikt == 'revise'");
        assert_eq!(problem.code, DiagnosticCode::UnknownField);
        assert_eq!(
            problem.message,
            "`review.output.verdikt` names no field of `review`'s output"
        );
    }

    /// An unknown member of `state` is an undefined channel, which is the
    /// vocabulary grammar 10.3 uses for it.
    #[test]
    fn an_unknown_channel_is_reported_as_one() {
        let problem = problem("size(state.drafts) > 0");
        assert_eq!(problem.code, DiagnosticCode::UndefinedChannel);
        assert_eq!(
            problem.message,
            "`state.drafts` names no declared state channel"
        );
    }

    /// The case grammar 4.1 states outright: a comparison against a non-variant
    /// is a compile error, not a silent false.
    #[test]
    fn a_literal_compared_against_an_enum_must_be_a_variant() {
        let equality = problem("review.output.verdict == 'aprove'");
        assert_eq!(equality.code, DiagnosticCode::TypeMismatch);
        assert_eq!(equality.help.as_deref(), Some("did you mean `approve`?"));
        // …and the same through `in`, which is one of the guard shapes
        // exhaustiveness reads (grammar 7.3.1).
        let membership = problem("review.output.verdict in ['approve', 'escalate']");
        assert_eq!(membership.code, DiagnosticCode::TypeMismatch);
        ok("review.output.verdict in ['approve', 'revise']");
    }

    #[test]
    fn the_supported_surface_is_closed() {
        let problem = problem("string(state.draft) == 'x'");
        assert_eq!(problem.code, DiagnosticCode::InvalidExpression);
        assert!(problem.message.contains("`string` is not a function"));
    }

    #[test]
    fn a_message_literal_is_not_part_of_the_surface() {
        let problem = problem("Finding{kind: 'a'} == 1");
        assert_eq!(problem.code, DiagnosticCode::InvalidExpression);
        assert!(problem.message.contains("constructs a message"));
    }

    #[test]
    fn an_expression_that_does_not_parse_is_reported_once() {
        let problem = problem("state.draft +");
        assert_eq!(problem.code, DiagnosticCode::InvalidExpression);
        assert!(
            problem
                .message
                .starts_with("`state.draft +` is not a valid CEL expression")
        );
    }

    /// The macros are expanded by the parser, so what the walk types is a
    /// comprehension: `filter` comes back as a list of the element type and
    /// `exists` as a boolean.
    #[test]
    fn the_comprehension_macros_are_typed_by_their_shape() {
        assert_eq!(ok("state.patches.exists(p, p != '')").ty, Type::Bool);
        assert_eq!(
            ok("state.patches.filter(p, p != '')").ty,
            Type::list(Type::String)
        );
        assert_eq!(
            ok("state.patches.map(p, size(p))").ty,
            Type::list(Type::Int)
        );
    }

    /// A comprehension binding is not a root: it is in scope inside the macro
    /// and nowhere else.
    #[test]
    fn a_comprehension_binding_is_not_a_root() {
        ok("state.patches.all(p, p.startsWith('x'))");
        let problem = problem("p.startsWith('x')");
        assert_eq!(problem.code, DiagnosticCode::UnknownRoot);
    }

    /// `has()` answers a question about a member; it does not read it.
    #[test]
    fn has_is_a_boolean_over_a_member_that_must_exist() {
        assert_eq!(ok("has(review.output.feedback)").ty, Type::Bool);
        let problem = problem("has(review.output.nope)");
        assert_eq!(problem.code, DiagnosticCode::UnknownField);
    }

    #[test]
    fn every_expression_carries_the_type_of_what_it_reads() {
        assert_eq!(ok("review.output.feedback").ty, Type::String);
        assert_eq!(ok("size(state.patches)").ty, Type::Int);
        assert_eq!(ok("state.patches[0]").ty, Type::String);
    }

    /// CEL has no implicit numeric conversion, and neither interpreter invents
    /// one.
    #[test]
    fn arithmetic_across_the_numeric_types_is_a_mismatch() {
        let problem = problem("size(state.patches) + 1.5 > 2.0");
        assert_eq!(problem.code, DiagnosticCode::TypeMismatch);
    }

    /// What the two read-shaped rules consume: the root-anchored paths, with an
    /// index recorded as a read *through* the path it indexes.
    #[test]
    fn reads_are_collected_as_root_anchored_paths() {
        let analysis = ok("review.output.feedback == state.draft");
        assert_eq!(
            analysis
                .reads
                .iter()
                .map(Read::spelling)
                .collect::<Vec<_>>(),
            ["review.output.feedback", "state.draft"]
        );
        let analysis = ok("state.patches[0] == input.goal");
        assert_eq!(
            analysis
                .reads
                .iter()
                .map(Read::spelling)
                .collect::<Vec<_>>(),
            ["state.patches[…]", "input.goal"]
        );
        assert!(analysis.reads[0].reads_through("state", "patches"));
        assert!(!analysis.reads[1].reads_through("input", "goal"));
    }

    /// A scope with no roots at all — what the item-derivation trace uses — is
    /// still walked for its reads.
    #[test]
    fn an_empty_scope_still_collects_reads() {
        let analysis = analyze("input.doc_id", &Scope::default());
        assert_eq!(analysis.reads.len(), 1);
        assert_eq!(analysis.reads[0].spelling(), "input.doc_id");
    }

    /// Nesting is counted over the three bracket kinds together, and never
    /// inside a string: a regex with parentheses in it nests no deeper than the
    /// call that carries it.
    #[test]
    fn nesting_counts_groups_and_not_string_contents() {
        assert_eq!(shape("state.draft == 'x'").nesting, 0);
        assert_eq!(shape("size(state.patches) > 0").nesting, 1);
        assert_eq!(shape("state.patches.exists(p, size(p[0]) > 1)").nesting, 3);
        assert_eq!(shape("state.draft.matches('^\\\\((a|b)\\\\)$')").nesting, 1);
        assert_eq!(shape("[[{'a': 1}]]").nesting, 3);
    }

    /// Operations are counted the same way, and over everything the walk
    /// recurses through — a selection and an operator cost what a bracket does.
    #[test]
    fn operations_count_every_step_and_not_string_contents() {
        assert_eq!(shape("state.draft").operations, 1);
        // `.draft`, `==`, and nothing for the literal.
        assert_eq!(shape("state.draft == 'x'").operations, 3);
        // `size(`, `.patches`, `>`.
        assert_eq!(shape("size(state.patches) > 0").operations, 3);
        // A pattern full of operator characters costs only the call and the
        // selections that reach it.
        assert_eq!(
            shape("state.draft.matches('^\\\\((a|b)\\\\)$')").operations,
            3
        );
        assert_eq!(shape("state.patches[0]").operations, 2);
    }

    /// The literal forms grammar 4.1 admits are skipped whole, and an
    /// *unterminated* one is read as code — an expression that hides a chain
    /// behind a quote the lexer would never close counts every step of it.
    #[test]
    fn every_string_form_is_skipped_and_an_unclosed_one_is_not() {
        assert_eq!(shape("state.draft == r'a.b.c'").operations, 3);
        assert_eq!(shape("state.draft == r\"\\\"").operations, 3);
        assert_eq!(shape("state.draft == '''a.b.c'''").operations, 3);
        assert_eq!(shape("state.draft == b'a.b'").operations, 3);
        assert_eq!(shape("state.draft == br'a.b'").operations, 3);
        // `r"\"` closes at the second quote — the backslash escapes nothing in
        // a raw literal — so what follows it is code and is counted.
        assert_eq!(shape("r\"\\\".a.b.c").operations, 3);
        // …and so is what follows a quote that never closes at all.
        assert_eq!(shape("'a.b.c").operations, 2);
    }

    /// Both bounds guard the recursion that reads an expression, so both are
    /// decided before the parser sees it — an expression past either comes back
    /// as a diagnostic, where without the guard the process would abort with no
    /// message at all.
    #[test]
    fn nesting_past_the_bound_is_a_diagnostic_rather_than_a_crash() {
        let deep = format!(
            "{}state.draft{} == 'x'",
            "(".repeat(MAX_NESTING + 1),
            ")".repeat(MAX_NESTING + 1)
        );
        let problem = problem(&deep);
        assert_eq!(problem.code, DiagnosticCode::InvalidExpression);
        assert!(
            problem
                .message
                .ends_with("nests brackets 9 deep, and this compiler reads at most 8"),
            "{}",
            problem.message
        );
        // …and the depth the bound admits still parses and still types.
        let deepest = format!(
            "{}state.draft{} == 'x'",
            "(".repeat(MAX_NESTING),
            ")".repeat(MAX_NESTING)
        );
        assert_eq!(ok(&deepest).ty, Type::Bool);
    }

    /// A chain that nests no brackets at all recurses just as deeply, which is
    /// what the operation bound is for: this test aborts the whole run rather
    /// than failing if the bound stops counting one of the shapes it covers.
    #[test]
    fn a_long_chain_is_a_diagnostic_rather_than_a_crash() {
        for long in [
            format!("state{}", ".draft".repeat(MAX_OPERATIONS + 1)),
            format!("size(state.patches){} > 0", " + 1".repeat(MAX_OPERATIONS)),
            format!("state.patches{}", "[0]".repeat(MAX_OPERATIONS)),
        ] {
            let problem = problem(&long);
            assert_eq!(problem.code, DiagnosticCode::InvalidExpression);
            assert!(
                problem
                    .message
                    .contains(", and this compiler reads at most 100"),
                "{}",
                problem.message
            );
            // A message that reprints the offender whole is no message: what it
            // carries is the head of it.
            assert!(problem.message.len() < long.len(), "{}", problem.message);
        }
        // …and a chain at the bound still parses, still types, and still fits
        // on the 2 MiB stack a test thread runs on: `size(`, `.patches`, the
        // additions, and `>` are the hundred.
        let longest = format!(
            "size(state.patches){} > 0",
            " + 1".repeat(MAX_OPERATIONS - 3)
        );
        assert_eq!(shape(&longest).operations, MAX_OPERATIONS);
        assert_eq!(ok(&longest).ty, Type::Bool);
    }

    /// The one construct that nests deeper than it is written: the parser
    /// expands each macro into a comprehension whose walk costs several levels
    /// the source never spells. What holds that down is [`MAX_NESTING`] — every
    /// macro is a call, and a call is a bracket — so the two bounds together
    /// cover what neither covers alone, and the deepest legal stack of macros
    /// still reads on a 2 MiB thread.
    #[test]
    fn the_deepest_stack_of_macros_is_read_rather_than_crashed_into() {
        let mut expression = "h == 'x'".to_string();
        for binding in "abcdefg".chars().rev() {
            expression = format!("{binding}.all({}, {expression})", next(binding));
        }
        let expression = format!("state.patches.all(a, {expression})");
        let shape = shape(&expression);
        assert_eq!(shape.nesting, MAX_NESTING);
        assert!(shape.operations <= MAX_OPERATIONS, "{}", shape.operations);
        // The bindings shadow nothing this scope declares, so what the walk
        // types is the macro stack itself.
        assert_eq!(analyze(&expression, &guard_scope()).ty, Type::Bool);
    }

    fn next(binding: char) -> char {
        char::from(binding as u8 + 1)
    }

    /// Grammar 4.1 asks for the offending sub-path by name, and an index does
    /// not end a path: what follows one is still spelled from its root.
    #[test]
    fn a_sub_path_beyond_an_index_is_still_named_whole() {
        let scope = Scope::new(
            "an edge guard",
            vec![(
                "find".to_string(),
                Type::object(
                    Origin::Declared("`find`'s output".to_string()),
                    vec![property(
                        "matches",
                        Type::list(Type::object(
                            Origin::Declared("an item of `matches`".to_string()),
                            vec![property("id", Type::String)],
                        )),
                    )],
                ),
            )],
        );
        let analysis = analyze("find.matches[0].metadata == 'x'", &scope);
        let problem = analysis.problems.first().expect("one problem");
        assert_eq!(problem.code, DiagnosticCode::UnknownField);
        assert_eq!(
            problem.message,
            "`find.matches[…].metadata` names no field of an item of `matches`"
        );
        assert_eq!(
            problem.help.as_deref(),
            Some("the declared fields are `id`")
        );
        // The read is unchanged by that: it stops at the index, which is what
        // the two rules that consume reads ask about (Decisions D83, D117).
        assert_eq!(
            analysis
                .reads
                .iter()
                .map(Read::spelling)
                .collect::<Vec<_>>(),
            ["find.matches[…]"]
        );
    }
}
