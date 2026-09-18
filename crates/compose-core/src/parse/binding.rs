//! Bindings and implementation blocks (grammar 6.1, 8.0, 8.2, 8.3).

use crate::ast::binding::{
    Binding, Bindings, DependencyEntry, ExecBlock, FunctionBinding, HttpBlock, HttpMethod,
    InterpolatedEntry, ModuleBlock, NodeInput, WriteEntry, Writes,
};
use crate::ast::schema::{ScalarKind, Surface, TypeForm};
use crate::diag::{Diagnostic, DiagnosticCode, Spanned};
use crate::yaml::{Mapping, Node, Yaml};

use super::lexical;
use super::reader::{Cx, Fields, expect_mapping, expect_sequence, expect_string, in_range};
use super::schema;

/// What a binding map's keys must look like.
#[derive(Clone, Copy)]
pub(crate) enum NameForm {
    /// An identifier (grammar 2.1): schema field names, body fields.
    Identifier,
    /// `[A-Za-z0-9_-]+`: HTTP header and query parameter names.
    HeaderLike,
    /// `[A-Za-z_][A-Za-z0-9_]*`: environment variable names.
    EnvVar,
}

impl NameForm {
    /// Whether one written name takes this form. Public within the crate
    /// because `section::header_shape` holds a trigger's `header:` to
    /// [`Self::HeaderLike`] — the same form, read from the same place, so the
    /// two surfaces cannot drift apart.
    pub(crate) fn accepts(self, text: &str) -> bool {
        match self {
            Self::Identifier => lexical::is_identifier(text),
            Self::HeaderLike => {
                !text.is_empty()
                    && text
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
            }
            Self::EnvVar => {
                let mut bytes = text.bytes();
                bytes
                    .next()
                    .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
                    && bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_')
            }
        }
    }

    const fn rule(self) -> &'static str {
        match self {
            Self::Identifier => {
                "identifiers are 1-64 characters of lowercase letters, digits, and `_`, starting with a letter"
            }
            Self::HeaderLike => "names here are letters, digits, `_`, and `-`",
            Self::EnvVar => {
                "environment variable names are letters, digits, and `_`, starting with a letter or `_`"
            }
        }
    }

    const fn noun(self) -> &'static str {
        match self {
            Self::Identifier => "identifier",
            Self::HeaderLike => "name",
            Self::EnvVar => "environment variable name",
        }
    }
}

fn check_name(
    key: &Spanned<String>,
    form: NameForm,
    subject: &str,
    cx: &mut Cx,
) -> Option<Spanned<String>> {
    if form.accepts(&key.value) {
        return Some(key.clone());
    }
    cx.push(
        Diagnostic::error(
            DiagnosticCode::InvalidIdentifier,
            key.span.clone(),
            format!(
                "`{}` is not a valid {} in {subject}",
                key.value,
                form.noun()
            ),
        )
        .with_help(form.rule()),
    );
    None
}

/// Read a node's `input:` — either the scalar form or per-field bindings
/// (grammar 8.0).
pub(crate) fn node_input(node: &Node, subject: &str, cx: &mut Cx) -> Option<NodeInput> {
    match &node.value {
        Yaml::String(_) => lexical::cel(node, subject, cx).map(NodeInput::Scalar),
        Yaml::Mapping(_) => {
            bindings(node, subject, NameForm::Identifier, cx).map(NodeInput::Fields)
        }
        _ => {
            cx.wrong_type(
                node,
                subject,
                "a mapping of bindings, or a single CEL expression",
            );
            None
        }
    }
}

/// Read a node's `input:` where only per-field bindings are legal.
///
/// The scalar form of grammar 8.0 binds a string-in agent's single unnamed
/// input (§5.3, Decision D14). A node whose target declares *named* input
/// fields has nothing for a scalar to bind, so accepting one would drop the
/// author's expression on the floor — the silently-ignored-key class that
/// Decisions D61 and D66 exist to reject. `why` names the declaration the
/// bindings have to match, which differs per node kind.
pub(crate) fn field_input(node: &Node, subject: &str, why: &str, cx: &mut Cx) -> Option<Bindings> {
    let Yaml::Mapping(mapping) = &node.value else {
        cx.push(
            Diagnostic::error(
                DiagnosticCode::WrongType,
                node.span.clone(),
                format!(
                    "expected a mapping of bindings for {subject}, found {}",
                    node.description()
                ),
            )
            .with_help(format!(
                "{why}; the scalar form binds a string-in agent's single unnamed input instead (grammar 5.3, 8.0, Decision D14)"
            )),
        );
        return None;
    };
    Some(bindings_from(
        mapping,
        node,
        subject,
        NameForm::Identifier,
        cx,
    ))
}

/// Read a mapping of name to CEL expression.
pub(crate) fn bindings(
    node: &Node,
    subject: &str,
    form: NameForm,
    cx: &mut Cx,
) -> Option<Bindings> {
    let mapping = expect_mapping(node, subject, cx)?;
    Some(bindings_from(mapping, node, subject, form, cx))
}

fn bindings_from(
    mapping: &Mapping,
    node: &Node,
    subject: &str,
    form: NameForm,
    cx: &mut Cx,
) -> Bindings {
    let mut entries = Vec::new();
    for entry in mapping.entries() {
        let Some(name) = check_name(&entry.key, form, subject, cx) else {
            continue;
        };
        let Some(value) = lexical::cel(&entry.value, &format!("`{}` in {subject}", name.value), cx)
        else {
            continue;
        };
        entries.push(Binding { name, value });
    }
    Bindings {
        entries,
        span: node.span.clone(),
    }
}

/// Read a `writes:` remap (grammar 8.0).
pub(crate) fn writes(node: &Node, subject: &str, cx: &mut Cx) -> Option<Writes> {
    let mapping = expect_mapping(node, subject, cx)?;
    let mut entries = Vec::new();
    for entry in mapping.entries() {
        let Some(field) = lexical::key_identifier(&entry.key, "output field name", cx) else {
            continue;
        };
        let Some(text) = expect_string(
            &entry.value,
            &format!("the channel `{}` is written to", field.value),
            cx,
        ) else {
            continue;
        };
        let Some(channel) = lexical::channel_name(&text, "state channel name", cx) else {
            continue;
        };
        entries.push(WriteEntry { field, channel });
    }
    Some(Writes {
        entries,
        span: node.span.clone(),
    })
}

/// Read an `exec:` block. `output` is legal on an inline node only: a tool
/// declares its result schema on the definition (grammar 6.1, 8.2).
pub(crate) fn exec_block(
    node: &Node,
    subject: &str,
    allow_output: bool,
    cx: &mut Cx,
) -> Option<ExecBlock> {
    let mapping = expect_mapping(node, subject, cx)?;
    let mut fields = Fields::new(mapping, node.span.clone(), subject);

    // `command` and `args` are class-2 surfaces like `cwd` and `env`: the whole
    // `exec:` block is interpolable, and nothing in it is shell-interpreted, so
    // a substituted value is one argv element rather than a re-parsed command
    // line (grammar 4.3, Decision D92).
    let command = fields
        .require("command", cx)
        .and_then(|node| lexical::interpolated(node, "`command`", cx))
        .filter(|command| {
            if command.value.as_str().is_empty() {
                cx.error(
                    DiagnosticCode::InvalidValue,
                    &command.span,
                    "`command` must not be empty",
                );
                return false;
            }
            true
        });

    let mut args = Vec::new();
    if let Some(node) = fields.take("args")
        && let Some(items) = expect_sequence(node, "`args`", cx)
    {
        for item in items {
            if let Some(arg) = lexical::interpolated(item, "each entry of `args`", cx) {
                args.push(arg);
            }
        }
    }

    let cwd = fields
        .take("cwd")
        .and_then(|node| lexical::interpolated(node, "`cwd`", cx));
    let env = fields
        .take("env")
        .map(|node| interpolated_map(node, "`env`", NameForm::EnvVar, cx))
        .unwrap_or_default();
    let expect_exit = accepted_outcomes(&mut fields, "expect_exit", "exit status", 0..=255, cx);
    let output = output_schema(&mut fields, subject, allow_output, EXEC_ENVELOPE, cx);
    fields.finish(cx);

    Some(ExecBlock {
        command,
        args,
        cwd,
        env,
        expect_exit,
        output,
        span: node.span.clone(),
    })
}

/// Read an accepted-outcome list: `expect_exit` or `expect_status`
/// (grammar 6.1, Decisions D84, D100).
///
/// One construct on two surfaces, so one reader: a **non-empty** list of
/// **distinct** members in the key's own range. An empty list accepts no
/// outcome at all, so every run would be an error; membership is a set test, so
/// a repeated member changes nothing about which outcomes are accepted. Both
/// halves are the inert key Decision D61 refuses, in the two spellings this
/// shape admits.
fn accepted_outcomes(
    fields: &mut Fields<'_>,
    key: &'static str,
    noun: &str,
    range: std::ops::RangeInclusive<i64>,
    cx: &mut Cx,
) -> Vec<Spanned<i64>> {
    let mut accepted: Vec<Spanned<i64>> = Vec::new();
    let Some(node) = fields.take(key) else {
        return accepted;
    };
    let Some(items) = expect_sequence(node, &format!("`{key}`"), cx) else {
        return accepted;
    };
    if items.is_empty() {
        cx.push(
            Diagnostic::error(
                DiagnosticCode::InvalidValue,
                node.span.clone(),
                format!("`{key}` must declare at least one {noun}"),
            )
            .with_help(format!(
                "an empty `{key}` accepts no outcome at all, so every call would be a node error"
            )),
        );
    }
    for item in items {
        let Some(value) = super::reader::expect_integer(item, &format!("each `{key}` entry"), cx)
        else {
            continue;
        };
        if !in_range(&value, &format!("an `{key}` entry"), range.clone(), cx) {
            continue;
        }
        if let Some(first) = accepted.iter().find(|other| other.value == value.value) {
            cx.push(
                Diagnostic::error(
                    DiagnosticCode::InvalidValue,
                    value.span.clone(),
                    format!("`{key}` lists {} twice", value.value),
                )
                .with_label(first.span.clone(), "first listed here")
                .with_help(format!(
                    "membership in `{key}` is a set test, so the second entry changes nothing"
                )),
            );
            continue;
        }
        accepted.push(value);
    }
    accepted
}

const HTTP_METHODS: &[(&str, HttpMethod)] = &[
    ("GET", HttpMethod::Get),
    ("POST", HttpMethod::Post),
    ("PUT", HttpMethod::Put),
    ("PATCH", HttpMethod::Patch),
    ("DELETE", HttpMethod::Delete),
    ("HEAD", HttpMethod::Head),
    ("OPTIONS", HttpMethod::Options),
];

/// Read an `http:` block (grammar 6.1, 8.3).
pub(crate) fn http_block(
    node: &Node,
    subject: &str,
    allow_output: bool,
    cx: &mut Cx,
) -> Option<HttpBlock> {
    let mapping = expect_mapping(node, subject, cx)?;
    let mut fields = Fields::new(mapping, node.span.clone(), subject);

    let method = fields
        .require("method", cx)
        .and_then(|node| lexical::keyword(node, "`method`", HTTP_METHODS, cx));
    let url = fields
        .require("url", cx)
        .and_then(|node| lexical::interpolated(node, "`url`", cx));
    let headers = fields
        .take("headers")
        .map(|node| header_map(node, "`headers`", cx))
        .unwrap_or_default();
    let query = fields
        .take("query")
        .and_then(|node| bindings(node, "`query`", NameForm::HeaderLike, cx));
    let body = fields.take_entry("body").and_then(|entry| {
        if let Some(method) = method.as_ref()
            && !method.value.carries_body()
        {
            cx.push(
                Diagnostic::error(
                    DiagnosticCode::ConflictingKeys,
                    entry.key.span.clone(),
                    format!(
                        "`body` is not legal on a `{}` request",
                        method.value.as_str()
                    ),
                )
                .with_label(method.span.clone(), "the method is declared here")
                .with_help("`GET` and `HEAD` carry no request body; use `query:` instead"),
            );
            return None;
        }
        bindings(&entry.value, "`body`", NameForm::Identifier, cx)
    });

    let expect_status =
        accepted_outcomes(&mut fields, "expect_status", "status code", 100..=599, cx);

    let output = output_schema(&mut fields, subject, allow_output, HTTP_ENVELOPE, cx);
    fields.finish(cx);

    Some(HttpBlock {
        method,
        url,
        headers,
        query,
        body,
        expect_status,
        output,
        span: node.span.clone(),
    })
}

/// Read a `function:` implementation binding (grammar 6.1).
pub(crate) fn function_binding(node: &Node, subject: &str, cx: &mut Cx) -> Option<FunctionBinding> {
    let mapping = expect_mapping(node, subject, cx)?;
    let mut fields = Fields::new(mapping, node.span.clone(), subject);
    let name = fields
        .require("name", cx)
        .and_then(|node| expect_string(node, "`name`", cx))
        .and_then(|text| lexical::identifier(&text, "host function name", cx));
    fields.finish(cx);
    Some(FunctionBinding {
        name,
        span: node.span.clone(),
    })
}

/// The extension a `module:` binding's path takes.
///
/// One, and it is `.ts`: the generated project is TypeScript run from source
/// with no build step (see `codegen`), so a `.js` beside it would be a second
/// authoring language nothing type-checks.
///
/// It is not the whole extension rule, because it cannot be: a `.d.ts` ends in
/// `.ts` and passes this test while naming no implementation at all, which is
/// [`MODULE_DECLARATION_ENDING`]'s to refuse.
const MODULE_EXTENSION: &str = ".ts";

/// The ending that makes a `.ts` file a **declaration** file rather than an
/// implementation.
///
/// [`MODULE_EXTENSION`] alone cannot tell the two apart, because a declaration
/// file's name ends in `.ts` too — and `./src/tools/sign.d.ts` is a binding
/// whose project could never type-check whatever it held: `src/modules.ts`
/// imports a bound implementation for its **value**, and `tsc` refuses a value
/// import of a declaration file outright (TS2846). A build would scaffold
/// executable code into a file that may hold none, `--check` would be clean
/// forever, and the type gate PRD resolved q48 makes the merge tool would be
/// unreachable for that composition — so the refusal is here, at the path, with
/// a span on what was written.
///
/// Matched **case-insensitively** for the reason the emitted-name comparison
/// below is: the string is a file name first, and macOS and Windows do not tell
/// `sign.D.ts` and `sign.d.ts` apart.
const MODULE_DECLARATION_ENDING: &str = ".d.ts";

const MODULE_PATH_RULE: &str = "a `module:` path is `/`-separated, each segment `.`, `..`, or a name matching `[A-Za-z0-9_][A-Za-z0-9_.-]*`, and the last segment ends in `.ts`: no leading `/`, no backslashes, no whitespace, no URLs (grammar 6.1)";

/// Read a `module:` implementation binding (grammar 6.1, PRD resolved q48).
///
/// Two spellings of one block. The **scalar** form,
/// `module: ./src/tools/sign.ts`, is the block with nothing but its `path:`,
/// which is what a tool reading no environment and importing no package writes;
/// the **mapping** form adds `env:` and `dependencies:`. They are one shape
/// rather than two because everything downstream — the artifact's file list, the
/// environment partition, the generated `package.json` — reads the same three
/// values whichever way they were written.
pub(crate) fn module_block(node: &Node, subject: &str, cx: &mut Cx) -> Option<ModuleBlock> {
    if let Yaml::String(_) = &node.value {
        // `lexical::text` rather than `expect_string`, so the scalar form is the
        // same class-3 surface the block form's `path:` is: a path is compile-
        // time identity, and an `${ENV}` in one is refused rather than carried
        // (grammar 4.3, Decision D92).
        let path = lexical::text(node, "`module`", cx)
            .and_then(|written| module_path(&written, subject, cx));
        return Some(ModuleBlock {
            path,
            env: Vec::new(),
            dependencies: Vec::new(),
            span: node.span.clone(),
        });
    }

    let mapping = expect_mapping(node, subject, cx)?;
    let mut fields = Fields::new(mapping, node.span.clone(), subject);
    let path = fields
        .require("path", cx)
        .and_then(|node| lexical::text(node, "`path`", cx))
        .and_then(|written| module_path(&written, subject, cx));
    // Interpolable, exactly as an `exec:` block's `env:` is (grammar 4.3
    // class 2): what a machine calls its keychain is a property of the machine.
    // There is no collision rule against the tool's `input:` here — the D66 one
    // an `exec:` obeys exists because an `exec:` tool's input *arrives* as
    // environment variables, and a module's arrives as a typed argument.
    let env = fields
        .take("env")
        .map(|node| interpolated_map(node, "`env`", NameForm::EnvVar, cx))
        .unwrap_or_default();
    let dependencies = fields
        .take("dependencies")
        .map(|node| dependency_map(node, subject, cx))
        .unwrap_or_default();
    fields.finish(cx);

    Some(ModuleBlock {
        path,
        env,
        dependencies,
        span: node.span.clone(),
    })
}

/// The longest a `module:` path may be, in bytes.
///
/// A ustar header holds a name in 100 bytes, and the artifact is **served as a
/// tar**: `docs/distributed.md` §3.5's `/workers/artifact/{hash}` packs every
/// entry of `ARTIFACT_FILES`, which PRD resolved q49 widened to carry the
/// authored files a composition references. Every emitted name is a compiler
/// constant well under the limit, so before the module binding the bound could
/// not be reached; an authored path is the composition's to write, and one byte
/// over it would be a spec the validator accepted and no worker could ever fetch
/// the artifact of. Refused here, with a span, rather than thrown inside a hub's
/// route.
const MODULE_PATH_BYTES: usize = 100;

/// Check a `module:` path against grammar 6.1's rules, and hand back the
/// **normalized** spelling — which is the name the artifact, the manifest and
/// every diagnostic downstream use.
fn module_path(written: &Spanned<String>, subject: &str, cx: &mut Cx) -> Option<Spanned<String>> {
    let text = written.value.as_str();
    if !lexical::is_relative_path(text, &[MODULE_EXTENSION]) {
        cx.push(
            Diagnostic::error(
                DiagnosticCode::InvalidModulePath,
                written.span.clone(),
                format!("`{text}` is not a project-relative path to a TypeScript file"),
            )
            .with_help(MODULE_PATH_RULE),
        );
        return None;
    }
    if text
        .rsplit('/')
        .next()
        .unwrap_or(text)
        .to_ascii_lowercase()
        .ends_with(MODULE_DECLARATION_ENDING)
    {
        cx.push(
            Diagnostic::error(
                DiagnosticCode::InvalidModulePath,
                written.span.clone(),
                format!("`{text}` is a TypeScript declaration file, which holds no implementation"),
            )
            .with_help(format!(
                "a `.d.ts` states types and never code, and `src/modules.ts` imports a bound implementation for its value — which `tsc` refuses of a declaration file — so the project this emits could not type-check whatever the file held: name the implementation itself, `{}/<name>.ts` being the conventional place (grammar 6.1, PRD resolved q48)",
                crate::codegen::AUTHORED_ZONE
            )),
        );
        return None;
    }
    let Some(normalized) = lexical::normalize_relative(text) else {
        cx.push(
            Diagnostic::error(
                DiagnosticCode::InvalidModulePath,
                written.span.clone(),
                format!("`{text}` climbs out of the project root"),
            )
            .with_help(
                "a module implementation is part of the composition and ships inside the artifact every worker fetches, so every prefix of its path stays inside the project root — the entrypoint's own directory (grammar 6.1, `docs/distributed.md` §4)",
            ),
        );
        return None;
    };
    if normalized.len() > MODULE_PATH_BYTES {
        cx.push(
            Diagnostic::error(
                DiagnosticCode::InvalidModulePath,
                written.span.clone(),
                format!(
                    "`{normalized}` is {} bytes long, and a path inside the artifact holds {MODULE_PATH_BYTES}",
                    normalized.len()
                ),
            )
            .with_help(format!(
                "the artifact is served to every worker as a tar and a ustar header holds a name in {MODULE_PATH_BYTES} bytes (`docs/distributed.md` §3.5), so a longer path is an artifact no worker could fetch: shorten it — `{}/<name>.ts` is the conventional place (grammar 6.1, PRD resolved q49)",
                crate::codegen::AUTHORED_ZONE
            )),
        );
        return None;
    }
    // Case-insensitively, because the string is a **file name** before it is
    // anything else and macOS and Windows hold `src/Graph.ts` and `src/graph.ts`
    // in one place. A comparison that distinguished them would accept a
    // composition here and then, on such a host, write the emitted file over the
    // author's implementation — a build that destroys code and a `--check` that
    // can never converge, on the machines a validator cannot see. One portable
    // spelling is the rule grammar 1.4 already applies to a path (D80).
    //
    // **Naming one is not the only way to collide with one.** A path *under* an
    // emitted name — `src/graph.ts/impl.ts` — is a directory where the emitter
    // writes a file, so one tree cannot hold both: the build scaffolds into the
    // author's checkout, writes most of the emission set, and then fails on a
    // `create_dir_all` with an IO error rather than a diagnostic. Equality alone
    // would let it through, so the comparison is over the path *and its
    // directories* (`lexical::path_conflict`).
    if let Some((emitted, conflict)) = crate::codegen::EMITTED_PATHS
        .iter()
        .find_map(|path| lexical::path_conflict(path, &normalized).map(|kind| (*path, kind)))
    {
        let zone = crate::codegen::AUTHORED_ZONE;
        let (message, help) = match conflict {
            lexical::PathConflict::Same => (
                format!("`{normalized}` is a file `agent-compose build` generates"),
                format!(
                    "the compiler writes that file itself, so the implementation {subject} names would be overwritten by every build: put it somewhere `build` does not emit — `{zone}/<name>.ts` is the conventional place (grammar 6.1, PRD resolved q47)"
                ),
            ),
            lexical::PathConflict::Cased => (
                format!(
                    "`{normalized}` differs only in case from `{emitted}`, a file `agent-compose build` generates"
                ),
                format!(
                    "macOS and Windows hold those two spellings in one file, so on such a host the build would write `{emitted}` over the implementation {subject} names — and a composition that works on one machine and destroys code on another is not one this compiler emits: put it somewhere `build` does not emit — `{zone}/<name>.ts` is the conventional place (grammar 6.1, PRD resolved q47)"
                ),
            ),
            lexical::PathConflict::Nested => (
                format!(
                    "`{normalized}` is inside `{emitted}`, a file `agent-compose build` generates"
                ),
                format!(
                    "the compiler writes `{emitted}` as a file, so no directory of that name can sit beside it: one tree cannot hold both, and the build would refuse partway through the emission rather than at the spec: put the implementation {subject} names somewhere `build` does not emit — `{zone}/<name>.ts` is the conventional place (grammar 6.1, PRD resolved q47)"
                ),
            ),
        };
        cx.push(
            Diagnostic::error(
                DiagnosticCode::InvalidModulePath,
                written.span.clone(),
                message,
            )
            .with_help(help),
        );
        return None;
    }
    Some(Spanned::new(normalized, written.span.clone()))
}

/// Read a `module:` binding's `dependencies:` map (grammar 6.1, PRD resolved
/// q49).
fn dependency_map(node: &Node, subject: &str, cx: &mut Cx) -> Vec<DependencyEntry> {
    let Some(mapping) = expect_mapping(node, "`dependencies`", cx) else {
        return Vec::new();
    };
    let mut entries = Vec::new();
    for entry in mapping.entries() {
        let package = entry.key.clone();
        if !is_package_name(&package.value) {
            cx.push(
                Diagnostic::error(
                    DiagnosticCode::InvalidDependency,
                    package.span.clone(),
                    format!("`{}` is not an npm package name", package.value),
                )
                .with_help(
                    "a package name is at most 214 characters of lowercase letters, digits, `-`, `_` and `.`, optionally scoped as `@scope/name`, and starts with none of `.`, `_` or `-`",
                ),
            );
            continue;
        }
        let Some(version) = lexical::text(&entry.value, "a dependency version", cx) else {
            continue;
        };
        if !is_exact_version(&version.value) {
            cx.push(
                Diagnostic::error(
                    DiagnosticCode::InvalidDependency,
                    version.span.clone(),
                    format!(
                        "{subject} pins `{}` to `{}`, which is not an exact version",
                        package.value, version.value
                    ),
                )
                .with_help(
                    "the artifact carries no lockfile, so a pin is what keeps the hub's install and every worker's resolving one tree: write an exact `MAJOR.MINOR.PATCH` — no `^`, `~`, `>`, `<`, `*` or `x`, and no `git`, `file`, `npm` or `workspace` specifier (PRD resolved q49)",
                ),
            );
            continue;
        }
        // One `package.json` holds one version of a package, and the generated
        // half of it is fixed per compiler release (`codegen::project::PINS`),
        // so a module pinning a runtime dependency at another version is a
        // conflict decidable right here — in one file, against a constant.
        if let Some((_, pinned)) = crate::codegen::project::PINS
            .iter()
            .chain(crate::codegen::project::DEV_PINS)
            .find(|(name, _)| *name == package.value)
            && *pinned != version.value
        {
            cx.push(
                Diagnostic::error(
                    DiagnosticCode::InvalidDependency,
                    version.span.clone(),
                    format!(
                        "`{}` is pinned to `{}` here and to `{pinned}` by the generated project",
                        package.value, version.value
                    ),
                )
                .with_help(format!(
                    "a generated project's own dependency set is fixed by the compiler release that wrote it, and one `package.json` holds one version of a package: pin `{}` at `{pinned}`, or reach for a package the generated runtime does not (grammar 6.1)",
                    package.value
                )),
            );
            continue;
        }
        entries.push(DependencyEntry { package, version });
    }
    entries
}

/// Whether a string is an npm package name.
///
/// The published rule, minus the parts that are advice: at most 214 characters,
/// lowercase, made of letters, digits, `-`, `_` and `.`, optionally scoped as
/// `@scope/name`, and starting with none of `.`, `_` or `-`.
fn is_package_name(text: &str) -> bool {
    if text.is_empty() || text.len() > 214 {
        return false;
    }
    let bare = match text.strip_prefix('@') {
        Some(scoped) => {
            let Some((scope, name)) = scoped.split_once('/') else {
                return false;
            };
            if !is_package_segment(scope) {
                return false;
            }
            name
        }
        None => text,
    };
    is_package_segment(bare)
}

fn is_package_segment(text: &str) -> bool {
    !text.is_empty()
        && !text.starts_with(['.', '_', '-'])
        && text.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_' | b'.')
        })
}

/// Whether a version string is an exact semantic version.
///
/// `MAJOR.MINOR.PATCH`, each a run of digits with no leading zero past `0`,
/// with an optional `-prerelease` and `+build`. Everything npm accepts *besides*
/// one of these — a range operator, a wildcard, a dist-tag, a `git`/`file`/`npm`/
/// `workspace` specifier, a URL — resolves to a different tree on a different
/// day or on a different machine, which is the one thing a lockfile-less
/// artifact cannot have (PRD resolved q49).
///
/// The tails are read the way semver.org §9 and §10 write them rather than as a
/// bag of admitted characters: **build first**, at the first `+`, then the
/// prerelease at the first `-` of what is left, each a `.`-separated list of
/// non-empty identifiers. A looser reading admits degenerate spellings — `1.2.3-+`
/// has an empty prerelease and `1.2.3-01` a numeric identifier with a leading
/// zero — that `npm install` rejects on the emitted `package.json`, which is the
/// failure the exactness rule exists to move to `validate`.
fn is_exact_version(text: &str) -> bool {
    // Build metadata is everything after the *first* `+`; a second one is not a
    // separator but an illegal character, and `identifiers` refuses it.
    let (rest, build) = match text.split_once('+') {
        Some((rest, build)) => (rest, Some(build)),
        None => (text, None),
    };
    if let Some(build) = build
        && !identifiers(build, false)
    {
        return false;
    }
    // The core holds no `-`, so the prerelease starts at the first one.
    let (core, prerelease) = match rest.split_once('-') {
        Some((core, prerelease)) => (core, Some(prerelease)),
        None => (rest, None),
    };
    if let Some(prerelease) = prerelease
        && !identifiers(prerelease, true)
    {
        return false;
    }
    let parts: Vec<&str> = core.split('.').collect();
    parts.len() == 3 && parts.iter().all(|part| is_numeric_identifier(part))
}

/// Whether a semver tail is a `.`-separated list of identifiers.
///
/// `numeric` says whether an all-digit identifier is held to semver's
/// no-leading-zero rule, which applies to a prerelease and not to build
/// metadata.
fn identifiers(text: &str, numeric: bool) -> bool {
    !text.is_empty()
        && text.split('.').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
                && (!numeric
                    || !part.bytes().all(|byte| byte.is_ascii_digit())
                    || is_numeric_identifier(part))
        })
}

/// A run of digits with no leading zero past `0` — semver's numeric identifier,
/// which each of `MAJOR`, `MINOR` and `PATCH` is.
fn is_numeric_identifier(part: &str) -> bool {
    !part.is_empty()
        && part.bytes().all(|byte| byte.is_ascii_digit())
        && (part.len() == 1 || !part.starts_with('0'))
}

/// The fields an inline `exec:` node binds from the child process itself, with
/// the type each one is fixed to (grammar 8.2, Decision D56).
const EXEC_ENVELOPE: &[(&str, ScalarKind)] = &[
    ("exit_code", ScalarKind::Integer),
    ("stdout", ScalarKind::String),
    ("stderr", ScalarKind::String),
];

/// The fields an inline `http:` node binds from the response itself
/// (grammar 8.3, Decision D56).
const HTTP_ENVELOPE: &[(&str, ScalarKind)] = &[
    ("status", ScalarKind::Integer),
    ("body", ScalarKind::String),
];

/// An inline node's `output:` field map, rejected where the block is a tool's
/// implementation binding.
///
/// `envelope` names the fields this kind of node synthesizes rather than
/// decodes; declaring one with any other type is an error, because its value
/// comes from the process or the response and cannot be anything else.
fn output_schema(
    fields: &mut Fields<'_>,
    subject: &str,
    allow_output: bool,
    envelope: &[(&str, ScalarKind)],
    cx: &mut Cx,
) -> Option<crate::ast::schema::FieldMap> {
    if !allow_output {
        // Lifting a §8.2 inline node block into a §6.1 tool binding is the
        // likeliest copy-paste at this surface, and the one `suggest` cannot
        // rescue: no key of the binding is a near miss for `output`. So the
        // reason is said here rather than left as a bare unknown key (PRD G3).
        if let Some(entry) = fields.take_entry("output") {
            cx.push(
                Diagnostic::error(
                    DiagnosticCode::UnknownKey,
                    entry.key.span.clone(),
                    format!("unknown key `output` in {subject}"),
                )
                .with_help(
                    "a tool declares its result schema once, as `output:` on the definition; only an inline node's block carries one of its own (grammar 6.1, 8.2)",
                ),
            );
        }
        return None;
    }
    let map = fields.take("output").and_then(|node| {
        schema::field_map(node, &format!("`output` of {subject}"), Surface::Result, cx)
    })?;

    for (name, required) in envelope {
        let Some(field) = map.field(name) else {
            continue;
        };
        let declared = match &field.ty.form {
            TypeForm::Scalar(scalar) => Some(scalar.kind.value),
            TypeForm::Invalid => continue,
            _ => None,
        };
        if declared == Some(*required) {
            continue;
        }
        cx.push(
            Diagnostic::error(
                DiagnosticCode::InvalidValue,
                field.ty.span.clone(),
                format!(
                    "`{name}` is an envelope field and must be declared as `{{ type: {} }}`",
                    required.as_str()
                ),
            )
            .with_help(
                "envelope fields are bound from the process or the response itself and are never decoded from its payload, so their types are fixed (grammar 8.2, 8.3)",
            ),
        );
    }
    Some(map)
}

/// A `headers:` map: [`interpolated_map`] under [`NameForm::HeaderLike`], with
/// every value held to what one header field can carry (grammar 12.1, 8.3,
/// Decision D144).
///
/// The name half has been checked here since headers existed, because a name is
/// written onto a request verbatim and a space or a colon in one forges a second
/// field rather than naming this one awkwardly. The **value** half is the same
/// sentence read one column along, and it became a compile-time question when
/// PRD resolved q58 let a provider's `headers:` cross into a `coder:` node's
/// run: on `cc` they are encoded into `ANTHROPIC_CUSTOM_HEADERS`, which the
/// Agent SDK's bundled runtime splits on newlines and then on each line's first
/// colon, so a value carrying a line break declares one header and sends two —
/// the second of them spelled by whoever wrote the value, `x-api-key` included.
/// On the agent path the same value goes to `fetch`, which refuses it at the
/// call; q58 ruling b's own principle is that a fact deciding what a connection
/// sends must fail at `validate` rather than on the first live request (PRD G3),
/// and that is the same answer for both.
///
/// **Both `headers:` positions, one rule** — a provider's (grammar 12.1) and an
/// `http:` block's (grammar 8.3) — because this function is the only thing
/// either writes its values through and the sentence is true at both: the
/// `http:` value is handed to `fetch` as it stands. q58 names the provider half
/// because that is the half that newly crosses a boundary; the `http:` half is
/// the same byte reaching the same wire one construct over, and leaving it to
/// fail at the first live request is G3's failure mode, not a narrower scope.
/// Decision D144 records the pair and the corpus pins each position's own
/// message.
///
/// The refused set is **every control character but a tab**, which is RFC 9110
/// §5.5's own `field-content`: a field value is visible characters with `SP` and
/// `HTAB` admitted between them, so a tab is a byte a header value does carry
/// and neither the wire nor `ANTHROPIC_CUSTOM_HEADERS` — split on newlines, then
/// on each line's first colon — is forged by one. Refusing the rest rather than
/// only `\r` and `\n` is the wire's line, not a widening of it: `\r` alone is no
/// more carriable than `\r\n`, and `\0` truncates the environment variable the
/// `cc` crossing encodes into. This is narrower than `section::prefix_shape`,
/// which refuses a tab too and should: a `prefix:` is a fixed token written
/// ahead of a credential (`Bearer `, `sha256=`), where a tab is a typo rather
/// than content.
///
/// The rule is on the **written** text; the `cc` driver's `forgesAHeaderField`
/// re-asks it of the resolved value over the same set, because a `${ENV}` a
/// gateway deployment sets is text this compiler never sees.
pub(crate) fn header_map(node: &Node, subject: &str, cx: &mut Cx) -> Vec<InterpolatedEntry> {
    let mut held = Vec::new();
    for entry in interpolated_map(node, subject, NameForm::HeaderLike, cx) {
        if header_value_shape(&entry, subject, cx) {
            held.push(entry);
        }
    }
    held
}

/// Whether one `headers:` value is a value one header field can carry.
///
/// See [`header_map`] for why the question is asked at all, and why a tab is the
/// one control character that passes.
fn header_value_shape(entry: &InterpolatedEntry, subject: &str, cx: &mut Cx) -> bool {
    if !entry
        .value
        .value
        .as_str()
        .chars()
        .any(|c| c.is_control() && c != '\t')
    {
        return true;
    }
    cx.push(
        Diagnostic::error(
            DiagnosticCode::InvalidValue,
            entry.value.span.clone(),
            format!(
                "`{}` in {subject} must not contain control characters other than a tab, found {:?}",
                entry.name.value,
                entry.value.value.as_str()
            ),
        )
        .with_help(
            "a header value is written onto the request as it stands — and, where a `coder:` node's `model:` carries it into a `cc` run, into the newline-delimited `ANTHROPIC_CUSTOM_HEADERS` — so a carriage return or a newline in it ends this field and begins another: the composition would declare one header and the run would send two, the second one spelled by the value. A space and a tab are the whitespace a field value does carry (RFC 9110 5.5); nothing else is",
        ),
    );
    false
}

pub(crate) fn interpolated_map(
    node: &Node,
    subject: &str,
    form: NameForm,
    cx: &mut Cx,
) -> Vec<InterpolatedEntry> {
    let Some(mapping) = expect_mapping(node, subject, cx) else {
        return Vec::new();
    };
    let mut entries = Vec::new();
    for entry in mapping.entries() {
        let Some(name) = check_name(&entry.key, form, subject, cx) else {
            continue;
        };
        let Some(value) =
            lexical::interpolated(&entry.value, &format!("`{}` in {subject}", name.value), cx)
        else {
            continue;
        };
        entries.push(InterpolatedEntry { name, value });
    }
    entries
}

/// The `env:` keys of an `exec` block that collide with the upper-snake-cased
/// name of an input field (grammar 6.1, 8.2, Decision D66).
///
/// The bound input object arrives as environment variables, so a colliding
/// `env:` key is one whose value would be silently discarded.
pub(crate) fn reject_env_collisions<'a>(
    block: &ExecBlock,
    input_names: impl IntoIterator<Item = &'a Spanned<String>>,
    cx: &mut Cx,
) {
    for name in input_names {
        let upper = name.value.to_ascii_uppercase();
        let Some(entry) = block.env.iter().find(|entry| entry.name.value == upper) else {
            continue;
        };
        cx.push(
            Diagnostic::error(
                DiagnosticCode::ConflictingKeys,
                entry.name.span.clone(),
                format!(
                    "`env` declares `{upper}`, which is also where the input field `{}` arrives",
                    name.value
                ),
            )
            .with_label(name.span.clone(), "this input field takes the same slot")
            .with_help("an object input is passed to the child as upper-snake-cased environment variables, so one of the two values would be silently discarded"),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A dependency pin is exactly what `npm install` would accept as one, so
    /// the refusal lands on the spec rather than on the emitted `package.json`
    /// (PRD resolved q49, D133).
    #[test]
    fn a_pin_is_a_version_and_not_a_shape_that_resembles_one() {
        assert!(is_exact_version("1.4.0"));
        assert!(is_exact_version("0.0.0"));
        assert!(is_exact_version("10.20.30"));
        assert!(is_exact_version("1.0.0-alpha"));
        assert!(is_exact_version("1.0.0-alpha.1"));
        assert!(is_exact_version("1.0.0-0.3.7"));
        assert!(is_exact_version("1.0.0-x-y-z.--"));
        assert!(is_exact_version("1.0.0+20130313144700"));
        assert!(is_exact_version("1.0.0-beta+exp.sha.5114f85"));

        // A range, a wildcard, a tag, a specifier: what the rule is written for.
        assert!(!is_exact_version("^1.4.0"));
        assert!(!is_exact_version("~1.4.0"));
        assert!(!is_exact_version(">=1.4.0"));
        assert!(!is_exact_version("1.x"));
        assert!(!is_exact_version("*"));
        assert!(!is_exact_version("latest"));
        assert!(!is_exact_version("npm:other@1.4.0"));

        // The core is three numeric identifiers, no more and no fewer.
        assert!(!is_exact_version("1.4"));
        assert!(!is_exact_version("1.4.0.1"));
        assert!(!is_exact_version("01.4.0"));
        assert!(!is_exact_version("1.04.0"));
        assert!(!is_exact_version(""));

        // The degenerate tails: each is a string a looser reading admits and
        // `npm` does not, so admitting one moves the failure from `validate` to
        // an install of an emitted manifest.
        assert!(!is_exact_version("1.2.3-"));
        assert!(!is_exact_version("1.2.3+"));
        assert!(!is_exact_version("1.2.3-+"));
        assert!(!is_exact_version("1.2.3-alpha..1"));
        assert!(!is_exact_version("1.2.3+build..1"));
        assert!(!is_exact_version("1.2.3-01"));
        assert!(!is_exact_version("1.2.3-alpha+"));
        assert!(!is_exact_version("1.2.3-al pha"));
        assert!(!is_exact_version("1.2.3+build+more"));

        // Build metadata keeps no leading-zero rule — semver §10 says so, and a
        // build number is not an ordering — and `-` is an identifier there, so
        // `1.2.3+-` is a version rather than a degenerate tail. Asserted because
        // it *looks* like one: the rule is semver's alphabet and not a guess at
        // which spellings seem meaningful.
        assert!(is_exact_version("1.2.3+001"));
        assert!(is_exact_version("1.2.3+-"));
    }
}
