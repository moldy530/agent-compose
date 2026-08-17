//! Grammar 3.8's mapping table, implemented once in both directions.
//!
//! ```text
//! DSL  ──►  JSON Schema 2020-12  ──►  Zod
//! ```
//!
//! Both lowerings live here on purpose. The table is one contract with two
//! columns, and two modules implementing one column each would drift the first
//! time a constraint key was added to one of them —
//! `tests/generated_code_gates.rs` exists to catch exactly that, running one
//! corpus of documents against both columns, and it can only compare the two if
//! both are reachable from one place.
//!
//! # Where the table left latitude, and what was chosen
//!
//! Grammar 3.8 fixes seven rows and the snake_case → camelCase key rule. The
//! rest of the vocabulary of grammar 3.3–3.7 is not in the table, so the choices
//! below are this module's, made to keep the two columns *observably equivalent*
//! (that is what the conformance corpus checks) rather than merely plausible:
//!
//! | DSL | JSON Schema | Zod | why |
//! |---|---|---|---|
//! | `description` | `description` | `.describe(…)` | it reaches the model in a structured-output schema (grammar 3.2) |
//! | `exclusive_minimum` | `exclusiveMinimum` | `.gt(…)` | Zod spells the exclusive bounds `gt`/`lt` |
//! | `multiple_of` | `multipleOf` | `.multipleOf(…)` | |
//! | `min_items` | `minItems` | `.min(…)` | |
//! | `unique_items` | `uniqueItems` | `.refine(uniqueItems, …)` | Zod has no built-in; the emitted helper compares JSON encodings, which is not what `uniqueItems` means but is what it *decides* here — see below |
//! | `optional: [b]` **and** `default:` on `b` | `b` omitted from `required` | `.default(v)` alone | grammar 3.6 makes a defaulted property implicitly optional, and `.default(v).optional()` would answer `undefined` for an omitted property instead of the default — the one composition where the two orderings differ |
//!
//! Two spellings are the table's own and are kept verbatim even where Zod offers
//! a newer one: a closed object is `z.object({…}).strict()` (not
//! `z.strictObject`), and an integer is `z.number().int()` (not `z.int()`). The
//! grammar is normative and both pairs denote the same schema.
//!
//! # `unique_items`, and the one row that leans on its surroundings
//!
//! JSON Schema's `uniqueItems` compares **instances**: two objects are one item
//! when they carry the same keys with equal values, whatever order those keys
//! were written in. The emitted `uniqueItems` compares `JSON.stringify`
//! encodings, and an object's encoding follows its key order — a different rule,
//! which on raw input answers differently (`[{a: 1, b: 2}, {b: 2, a: 1}]` is
//! unique to the encoding rule and a duplicate to JSON Schema's).
//!
//! It is nevertheless the same *decision* in the position it is emitted into,
//! and for a reason worth stating because nothing about the helper says it:
//! `.refine` runs over the array's already-**parsed** elements, and
//! `z.object({…}).strict()` rebuilds each object in the order its own shape
//! declares. Two equal instances therefore reach the helper with identical
//! encodings. `state.authors` in the corpus pins that decision from raw text
//! (`as_written`, so the key order a document was written in survives the
//! corpus's own round trip), which is what would fail if a pinned Zod release
//! ever stopped normalizing — the alternative, a canonicalizing helper in every
//! generated project, would be a rule no gate could tell apart from this one.
//!
//! # `format:`, and why six of the ten are written out
//!
//! Grammar 3.3 fixes a closed vocabulary of ten formats and grammar 3.8's table
//! says nothing about any of them, so the reading is this module's to choose —
//! and choosing a Zod constructor by *name* is how the two columns drift. Zod's
//! constructors are not implementations of JSON Schema's `format` keyword and do
//! not claim to be: `z.url()` is `new URL()`, which repairs its input;
//! `z.iso.datetime()` is an upper-case-only ISO 8601 profile, not RFC 3339;
//! `z.email()` is a deliberately narrow subset; `z.hostname()` admits the root
//! dot. Every one of those was a document the published JSON Schema accepted and
//! the emitted Zod refused — a model answering its own contract and failing the
//! parse.
//!
//! So the reading is named by RFC, and where Zod's constructor reads a different
//! one the check is written out in [`HELPERS`] (`rfc3339Date`, `rfc3339Time`,
//! `rfc3339DateTime`, `rfc3986Uri`, `rfc1123Hostname`, `rfc5321Email`). Four
//! formats keep their constructor because on those four the two agree:
//! `duration`, `uuid`, `ipv4`, `ipv6`. [`zod_format`] is the whole table.
//!
//! # The divergence ledger
//!
//! Agreement is a **test**, not a claim: `tests/generated_code_gates.rs` runs
//! `tests/fixtures/schema-lowering/cases.json` against both columns and pins
//! each column's verdict on each document separately, so a document the two
//! answer differently cannot pass as agreement. A corpus case that records two
//! different verdicts must name one of the divergences below, and every
//! divergence below must be exercised by a case — the two lists are asserted
//! against each other.
//!
//! | id | where | which way | why it is left |
//! |---|---|---|---|
//! | `integer-beyond-the-safe-range` | `type: integer` | JSON accepts, Zod refuses | grammar 3.8 fixes both spellings, and `z.number().int()` is JavaScript's safe-integer range. Past 2^53 the language cannot count: `JSON.parse` has already rounded `9007199254740993` to an even neighbour by the time any check sees it, so the emitted parse refuses what it cannot represent rather than accepting a number that is no longer the one that was sent |
//! | `display-name-is-not-an-addr-spec` | `format: email` | JSON accepts, Zod refuses | `Name <a@example.test>`. JSON Schema defines `email` as RFC 5321's `Mailbox` rule, which has no display-name form; the Rust column's parser offers one and accepts it. This is the one row where the emitted Zod is the **stricter and more correct** column, so it is recorded rather than widened |
//! | `leap-second-away-from-midnight` | `format: time`, `format: date-time` | Zod accepts, JSON refuses | `rfc3339Time` admits `:60` wherever the rest parses; the JSON column admits it only where the value normalizes to `23:59:60` UTC. Narrowing the regex to `[0-5]\d` would trade this corner for the opposite one, and neither is reachable from a model emitting a wall-clock time |
//! | `duration-skips-a-designator` | `format: duration` | Zod accepts, JSON refuses | `P1Y1D`, `PT1H1S`. ISO 8601 admits a skipped designator and RFC 3339's appendix-A ABNF nests them (`dur-year = Y [dur-month]`); grammar 3.3's `duration` is the ISO 8601 one, which is what `z.iso.duration()` reads |
//! | `punycode-payload-undecoded` | `format: hostname`, `format: email` | Zod accepts, JSON refuses | an `xn--` label whose payload is not decodable punycode. `rfc1123Hostname` checks the label's *shape*; decoding it would be a punycode implementation inside a generated module, for a case a model does not produce |
//!
//! # Patterns
//!
//! `pattern:` is RE2 (Decision D12), which is **not** a subset of JavaScript's
//! syntax — [`super::pattern`] is the module that decides what transfers, and
//! [`super::diagnostics`] is what refuses a `build` whose patterns do not.

use std::borrow::Cow;

use serde_json::{Map, Value, json};

use crate::ast::schema::{Number, ScalarKind, StringFormat};
use crate::check::model;
use crate::ir::definition::DefinitionBody;
use crate::ir::flow::NodeKind;
use crate::ir::schema::{ArrayType, EnumType, FieldMap, ObjectType, Scalar, TypeForm, TypeNode};
use crate::ir::{Channel, Ir};

use super::names::{self, Names};

/// One schema the emitted project declares, and where it came from.
#[derive(Clone, Debug)]
pub struct Surface<'ir> {
    /// The canonical path (see [`super::names`]), which is the key every other
    /// module reaches this schema by.
    pub path: String,
    /// What it is, for the doc comment above it.
    pub about: String,
    /// The schema itself.
    pub body: Body<'ir>,
}

/// A schema is either a declaration surface's field map or a single type node —
/// a state channel is the only one of the second kind (grammar 10.1).
///
/// The field map is a [`Cow`] because some of them are written nowhere: an
/// inline `exec:`/`http:` node that declares no `output:` has a **kind default**
/// as its result schema (grammar 8.2, 8.3), and a `store:` op node has a
/// **derived** one (grammar 11.4). Both are synthesized rather than parsed. See
/// [`surfaces`].
#[derive(Clone, Debug)]
pub enum Body<'ir> {
    /// A field map: a closed object (grammar 3.1).
    Fields(Cow<'ir, FieldMap>),
    /// One type node, which is what a channel declares.
    Type(&'ir TypeNode),
}

/// Every schema of the composition, in canonical order.
///
/// Definitions come first, in the address order [`Ir::definitions`] already
/// holds them in, each definition's surfaces in the order grammar 3.9 tabulates
/// them, and a flow's node-level schemas after its own in **node declaration
/// order**. State channels come last, in channel-name order.
///
/// This function is the single enumeration of "every schema there is": the name
/// registry, the emitted module, and the conformance corpus all walk it, so a
/// surface added to the grammar reaches all three at once or none.
///
/// "Every schema there is" includes the ones nobody writes. Three node kinds
/// have a result schema that appears nowhere in the source text:
///
/// * an inline `exec:` node with no `output:` — grammar 8.2's
///   `{exit_code, stdout}`;
/// * an inline `http:` node with no `output:` — grammar 8.3's `{status, body}`;
/// * a `store:` op node, whose result is *derived* from the op and the store it
///   names — grammar 11.4's catalogue (Decision D34).
///
/// The validator already resolves all three (`check::Context::node_output`),
/// which is why an edge guard over such a node is type-checked at all. Omitting
/// them here would leave a node with a checked result surface and no emitted
/// schema, no export name and no conformance case, and the node-fn PR would find
/// nothing to parse the node's answer with. Each shape is taken from
/// [`crate::check::model`] rather than restated, because two spellings of one
/// grammar rule is exactly the drift this function exists to prevent.
#[must_use]
pub fn surfaces(ir: &Ir) -> Vec<Surface<'_>> {
    let mut surfaces = Vec::new();
    for (address, definition) in &ir.definitions {
        match &definition.body {
            DefinitionBody::Agent(agent) => {
                if let Some(input) = &agent.input {
                    surfaces.push(Surface {
                        path: format!("{address}.input"),
                        about: format!("`{address}` — its declared input (grammar 5.3)."),
                        body: borrowed(input),
                    });
                }
                surfaces.push(Surface {
                    path: format!("{address}.output"),
                    about: format!(
                        "`{address}` — the structured output the model is constrained to, and \
                         what routing reads (PRD 5.2, 5.3)."
                    ),
                    body: borrowed(&agent.output),
                });
            }
            DefinitionBody::Tool(tool) => {
                surfaces.push(Surface {
                    path: format!("{address}.input"),
                    about: format!("`{address}` — its parameters (grammar 6)."),
                    body: borrowed(&tool.input),
                });
                surfaces.push(Surface {
                    path: format!("{address}.output"),
                    about: format!("`{address}` — its result (grammar 6)."),
                    body: borrowed(&tool.output),
                });
            }
            DefinitionBody::Flow(flow) => {
                if let Some(inputs) = &flow.inputs {
                    surfaces.push(Surface {
                        path: format!("{address}.inputs"),
                        about: format!("`{address}` — the module's parameters (grammar 7.5)."),
                        body: borrowed(inputs),
                    });
                }
                surfaces.push(Surface {
                    path: format!("{address}.outputs"),
                    about: format!(
                        "`{address}` — the module's result, materialized from the state channels \
                         of the same names at quiescence (grammar 7.5, 7.6.3)."
                    ),
                    body: borrowed(&flow.outputs),
                });
                for node in &flow.nodes {
                    let id = node.id.value.as_str();
                    match &node.kind {
                        NodeKind::Human { human } => {
                            surfaces.push(Surface {
                                path: format!("{address}.node.{id}.input"),
                                about: format!(
                                    "`{address}` node `{id}` — what the human is shown \
                                     (grammar 8.7)."
                                ),
                                body: borrowed(&human.input),
                            });
                            surfaces.push(Surface {
                                path: format!("{address}.node.{id}.output"),
                                about: format!(
                                    "`{address}` node `{id}` — what the human returns, routable \
                                     like any structured output (grammar 8.7)."
                                ),
                                body: borrowed(&human.output),
                            });
                        }
                        NodeKind::Exec { exec } => {
                            surfaces.push(Surface {
                                path: format!("{address}.node.{id}.output"),
                                about: format!(
                                    "`{address}` node `{id}` — what the subprocess produces \
                                     (grammar 8.2){}.",
                                    default_note(exec.output.is_none())
                                ),
                                body: exec.output.as_ref().map_or_else(
                                    || owned(model::exec_default_output(&exec.span)),
                                    borrowed,
                                ),
                            });
                        }
                        NodeKind::Http { http } => {
                            surfaces.push(Surface {
                                path: format!("{address}.node.{id}.output"),
                                about: format!(
                                    "`{address}` node `{id}` — what the response decodes to \
                                     (grammar 8.3){}.",
                                    default_note(http.output.is_none())
                                ),
                                body: http.output.as_ref().map_or_else(
                                    || owned(model::http_default_output(&http.span)),
                                    borrowed,
                                ),
                            });
                        }
                        NodeKind::Store { store, op, params } => {
                            // Grammar 11.4's row, derived from the op and the
                            // store it names — a shape that exists nowhere in
                            // the source text, like the two kind defaults
                            // above. A `store:` whose address did not resolve
                            // has no row to derive, and the resolver has
                            // already said so, which is the one case with no
                            // surface at all.
                            if let Some(definition) = ir.definitions.get(&store.value.to_string())
                                && let DefinitionBody::Store(definition) = &definition.body
                            {
                                surfaces.push(Surface {
                                    path: format!("{address}.node.{id}.output"),
                                    about: format!(
                                        "`{address}` node `{id}` — what the `{}` of \
                                         `{}` answers, derived from the op and the store \
                                         (grammar 11.4, Decision D34).",
                                        op.as_str(),
                                        store.value
                                    ),
                                    body: owned(model::store_output(
                                        definition.kind,
                                        *op,
                                        definition.value_schema.as_ref(),
                                        definition.metadata_schema.as_ref(),
                                        params.top_k.or(params.limit),
                                        &node.span,
                                    )),
                                });
                            }
                        }
                        NodeKind::Agent { .. }
                        | NodeKind::Function { .. }
                        | NodeKind::Flow { .. }
                        | NodeKind::Map { .. } => {}
                    }
                }
            }
            DefinitionBody::Store(store) => {
                if let Some(value) = &store.value_schema {
                    surfaces.push(Surface {
                        path: format!("{address}.value_schema"),
                        about: format!("`{address}` — the value it stores (grammar 11.1)."),
                        body: borrowed(value),
                    });
                }
                if let Some(metadata) = &store.metadata_schema {
                    surfaces.push(Surface {
                        path: format!("{address}.metadata_schema"),
                        about: format!(
                            "`{address}` — the metadata a match carries (grammar 11.1)."
                        ),
                        body: borrowed(metadata),
                    });
                }
            }
            DefinitionBody::Provider(_) | DefinitionBody::Model(_) => {}
        }
    }

    if let Some(state) = &ir.state {
        for (name, channel) in &state.entries {
            surfaces.push(Surface {
                path: format!("state.{name}"),
                about: format!("State channel `{name}` — its declared type (grammar 10.1)."),
                body: Body::Type(&channel.ty),
            });
        }
    }

    surfaces
}

/// A field map the composition wrote, as a [`Body`].
///
/// Every surface the source text spells out is one of these; the helper exists
/// so the borrow is written once instead of at each of a dozen push sites.
fn borrowed(fields: &FieldMap) -> Body<'_> {
    Body::Fields(Cow::Borrowed(fields))
}

/// A field map the *grammar* supplies where the composition wrote none — an
/// inline node's kind default (grammar 8.2, 8.3) or a store op's derived row
/// (grammar 11.4).
fn owned(fields: FieldMap) -> Body<'static> {
    Body::Fields(Cow::Owned(fields))
}

/// The clause the doc comment of an inline node's result schema carries when the
/// schema is the kind default rather than one the node declared.
fn default_note(defaulted: bool) -> &'static str {
    if defaulted {
        ", which it does not declare, so this is the kind default"
    } else {
        ""
    }
}

/// Every state channel, in channel-name order, paired with its canonical path.
///
/// [`super::state`] reads this rather than `ir.state` directly so the two
/// modules cannot disagree about which channels exist or what they are called.
#[must_use]
pub fn channels(ir: &Ir) -> Vec<(String, &Channel)> {
    ir.state
        .as_ref()
        .map(|state| {
            state
                .entries
                .iter()
                .map(|(name, channel)| (format!("state.{name}"), channel))
                .collect()
        })
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// The Zod column
// ---------------------------------------------------------------------------

/// `src/schemas.ts`: every schema of the composition, as Zod.
#[must_use]
pub fn module(ir: &Ir, names: &Names) -> super::GeneratedFile {
    let surfaces = surfaces(ir);
    let mut contents = super::header(ir, "// ");
    contents.push_str(MODULE_DOC);
    contents.push_str("\nimport { z } from \"zod\";\n");

    for helper in HELPERS {
        if surfaces
            .iter()
            .any(|surface| surface.declares(&|ty| helper.wanted_by(ty)))
        {
            contents.push_str(helper.source);
        }
    }

    for surface in &surfaces {
        let name = names.value(&surface.path);
        let ty = names.ty(&surface.path);
        contents.push('\n');
        contents.push_str(&names::doc("", std::slice::from_ref(&surface.about)));
        let expression = match &surface.body {
            Body::Fields(fields) => field_map(fields, ""),
            Body::Type(node) => type_node(node, ""),
        };
        contents.push_str(&format!("export const {name} = {expression};\n"));
        contents.push_str(&format!("export type {ty} = z.infer<typeof {name}>;\n"));
    }

    super::GeneratedFile {
        path: "src/schemas.ts".to_string(),
        contents,
    }
}

const MODULE_DOC: &str = "\
//
// Every schema the composition declares, lowered per grammar 3.8. These are the
// runtime contract: an agent's output is parsed with the schema named for it
// before any edge is evaluated, a tool's arguments are parsed with its input
// schema, and a state channel's type is the schema its channel is built from.
";

/// One module-level helper the emitted schemas can lean on.
///
/// A helper is written only where the composition reaches the form that wants it
/// — an unused `const` in a generated module is a question a reader has to
/// answer for nothing — and the table is walked in order, so a helper that calls
/// another (`rfc5321Email` calls `rfc1123Hostname`, `rfc3339DateTime` calls both
/// of its halves) is declared after it. `wants` lists those callees, so asking
/// for one asks for its dependencies too and the order of the table is the only
/// thing keeping them in scope.
struct Helper {
    /// The `format:` values whose lowering calls it directly.
    formats: &'static [StringFormat],
    /// Whether an array's `unique_items` calls it.
    unique_items: bool,
    /// Other helpers it calls, which therefore must be emitted with it.
    wants: &'static [&'static str],
    /// Its own name, for [`Helper::wants`] to name it by.
    name: &'static str,
    /// The declaration, opening with a blank line.
    source: &'static str,
}

impl Helper {
    /// Whether this type node reaches this helper, directly or through one that
    /// does.
    fn wanted_by(&self, ty: &TypeNode) -> bool {
        HELPERS.iter().any(|helper| {
            (helper.name == self.name || helper.wants.contains(&self.name)) && helper.called_by(ty)
        })
    }

    /// Whether this type node's own lowering calls this helper by name.
    fn called_by(&self, ty: &TypeNode) -> bool {
        match &ty.form {
            TypeForm::Scalar(scalar) => scalar
                .format
                .is_some_and(|format| self.formats.contains(&format)),
            TypeForm::Array(array) => self.unique_items && array.unique_items == Some(true),
            TypeForm::Enum(_) | TypeForm::Object(_) | TypeForm::Union(_) => false,
        }
    }
}

/// Every helper, in declaration order — callees before their callers.
const HELPERS: &[Helper] = &[
    Helper {
        formats: &[],
        unique_items: true,
        wants: &[],
        name: "uniqueItems",
        source: r#"
/**
 * `unique_items: true` (grammar 3.5). Zod has no built-in.
 *
 * JSON Schema's `uniqueItems` compares *instances*: two objects are one item
 * when they carry the same keys with equal values, in whatever order those keys
 * were written. `JSON.stringify` compares encodings, and an object's encoding
 * follows its key order — so the two are not the same rule in general.
 *
 * They are the same rule **here**. `.refine` runs over the array's already
 * *parsed* elements, and `z.object({…}).strict()` rebuilds every object in the
 * order its own shape declares, so two equal instances have identical encodings
 * by the time this is called. That is a fact about where this is used, which is
 * why it is not exported: applied to raw input, it would call
 * `[{a: 1, b: 2}, {b: 2, a: 1}]` unique and JSON Schema would not.
 */
const uniqueItems = (items: readonly unknown[]): boolean =>
  new Set(items.map((item) => JSON.stringify(item))).size === items.length;
"#,
    },
    Helper {
        formats: &[StringFormat::Hostname],
        unique_items: false,
        wants: &[],
        name: "rfc1123Hostname",
        source: r#"
/**
 * `format: hostname` (grammar 3.3): an RFC 1123 host name, which is what JSON
 * Schema's `hostname` means. `z.hostname()` is a looser reading — it accepts the
 * root-relative `example.test.`, which JSON Schema refuses — and a value one
 * column accepts and the other refuses is the drift grammar 3.8's table exists
 * to prevent, so the rule is written out.
 */
const rfc1123Hostname = (value: string): boolean => {
  if (value.length === 0 || value.length > 253 || value.endsWith(".")) {
    return false;
  }
  return value.split(".").every((label) => {
    if (label.length === 0 || label.length > 63) {
      return false;
    }
    if (!/^[A-Za-z0-9](?:[A-Za-z0-9-]*[A-Za-z0-9])?$/.test(label)) {
      return false;
    }
    // RFC 5891: hyphens in the third and fourth positions are reserved for the
    // `xn--` prefix of an internationalized label.
    return !(label.length >= 4 && label[2] === "-" && label[3] === "-") || label.startsWith("xn--");
  });
};
"#,
    },
    Helper {
        formats: &[StringFormat::Date, StringFormat::DateTime],
        unique_items: false,
        wants: &[],
        name: "rfc3339Date",
        source: r#"
/**
 * `format: date` (grammar 3.3): an RFC 3339 `full-date`, calendar-checked — the
 * month bounds the day, and February bounds it by the proleptic Gregorian leap
 * rule, so `2026-02-30` is not a date.
 */
const rfc3339Date = (value: string): boolean => {
  const parts = /^(\d{4})-(\d{2})-(\d{2})$/.exec(value);
  if (parts === null) {
    return false;
  }
  const [, year, month, day] = parts.map(Number);
  if (month < 1 || month > 12 || day < 1) {
    return false;
  }
  const leap = (year % 4 === 0 && year % 100 !== 0) || year % 400 === 0;
  return day <= [31, leap ? 29 : 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31][month - 1];
};
"#,
    },
    Helper {
        formats: &[StringFormat::Time, StringFormat::DateTime],
        unique_items: false,
        wants: &[],
        name: "rfc3339Time",
        source: r#"
/**
 * `format: time` (grammar 3.3), which JSON Schema reads as RFC 3339 `full-time`
 * — an offset is required. Zod's `z.iso.time()` refuses an offset outright, so
 * the check is written here rather than borrowed from a constructor that means
 * something else. The offset may be `Z`, `z`, or `±HH:MM`, and a leap second is
 * spelled `:60`.
 */
const rfc3339Time =
  /^([01]\d|2[0-3]):[0-5]\d:([0-5]\d|60)(\.\d+)?([Zz]|[+-]([01]\d|2[0-3]):[0-5]\d)$/;
"#,
    },
    Helper {
        formats: &[StringFormat::DateTime],
        unique_items: false,
        wants: &["rfc3339Date", "rfc3339Time"],
        name: "rfc3339DateTime",
        source: r#"
/**
 * `format: date-time` (grammar 3.3): an RFC 3339 `date-time`, which is a
 * `full-date`, the separator, and a `full-time`. The separator is `T` or `t` and
 * the offset may be `Z` or `z`: RFC 3339 says so in as many words, and the JSON
 * Schema column agrees, while `z.iso.datetime()` accepts only the upper-case
 * spellings.
 */
const rfc3339DateTime = (value: string): boolean => {
  const separator = value.search(/[Tt]/);
  return (
    separator > 0 &&
    rfc3339Date(value.slice(0, separator)) &&
    rfc3339Time.test(value.slice(separator + 1))
  );
};
"#,
    },
    Helper {
        formats: &[StringFormat::Uri],
        unique_items: false,
        wants: &[],
        name: "rfc3986Uri",
        source: r#"
/**
 * `format: uri` (grammar 3.3): an absolute RFC 3986 URI — a scheme, a
 * hierarchical part, and the optional query and fragment — spelled as the
 * grammar's own production. `z.url()` is not this check: it is `new URL()`,
 * which is the WHATWG parser, and that one *repairs* what it is given. It
 * accepts a space and a non-ASCII character in a path (percent-encoding them)
 * and rejects `https://`, which RFC 3986 admits as a URI with an empty
 * authority — divergences in both directions from the column beside it.
 */
const rfc3986Uri =
  /^[A-Za-z][A-Za-z0-9+\-.]*:(?:\/\/(?:(?:[A-Za-z0-9\-._~!$&'()*+,;=:]|%[0-9A-Fa-f]{2})*@)?(?:\[[A-Za-z0-9:.]+\]|(?:[A-Za-z0-9\-._~!$&'()*+,;=]|%[0-9A-Fa-f]{2})*)(?::\d*)?(?:\/(?:[A-Za-z0-9\-._~!$&'()*+,;=:@]|%[0-9A-Fa-f]{2})*)*|\/?(?:(?:[A-Za-z0-9\-._~!$&'()*+,;=:@]|%[0-9A-Fa-f]{2})+(?:\/(?:[A-Za-z0-9\-._~!$&'()*+,;=:@]|%[0-9A-Fa-f]{2})*)*)?)(?:\?(?:[A-Za-z0-9\-._~!$&'()*+,;=:@/?]|%[0-9A-Fa-f]{2})*)?(?:#(?:[A-Za-z0-9\-._~!$&'()*+,;=:@/?]|%[0-9A-Fa-f]{2})*)?$/;
"#,
    },
    Helper {
        formats: &[StringFormat::Email],
        unique_items: false,
        wants: &["rfc1123Hostname"],
        name: "rfc5321Email",
        source: r#"
/**
 * `format: email` (grammar 3.3): an RFC 5322 `addr-spec` whose domain is an RFC
 * 1123 host name or an address literal — the reading JSON Schema's `email`
 * takes. `z.email()` is a deliberately narrow subset of it and refuses three
 * things the column beside it accepts: a quoted local part, a single-label
 * domain (`a@b`), and `a@[192.0.2.1]`.
 */
const rfc5321Email = (value: string): boolean => {
  // The *last* `@` splits: an unquoted local part cannot hold one, and a quoted
  // one can hold as many as it likes.
  const at = value.lastIndexOf("@");
  if (at <= 0 || at === value.length - 1 || at > 64) {
    return false;
  }
  const local = value.slice(0, at);
  const domain = value.slice(at + 1);
  const atext = /^[A-Za-z0-9!#$%&'*+\-/=?^_`{|}~\u0080-\uffff]+$/;
  const quoted = /^(?:[\t \x21\x23-\x5b\x5d-\x7e\u0080-\uffff]|\\[\x21-\x7e])*$/;
  const localOk =
    local.length > 2 && local.startsWith('"') && local.endsWith('"')
      ? quoted.test(local.slice(1, -1))
      : local.split(".").every((atom) => atext.test(atom));
  if (!localOk) {
    return false;
  }
  if (domain.length > 2 && domain.startsWith("[") && domain.endsWith("]")) {
    const literal = domain.slice(1, -1);
    return literal.startsWith("IPv6:")
      ? z.ipv6().safeParse(literal.slice(5)).success
      : z.ipv4().safeParse(literal).success;
  }
  return rfc1123Hostname(domain);
};
"#,
    },
];

/// Every name [`module`] can declare besides the schemas themselves.
///
/// [`super::names`]'s reserved list is checked against this, so a helper added
/// to [`HELPERS`] cannot be left out of the namespace the registry
/// disambiguates against.
#[cfg(test)]
pub(super) fn helper_names() -> impl Iterator<Item = &'static str> {
    HELPERS.iter().map(|helper| helper.name)
}

impl Surface<'_> {
    /// Visit every type node inside this surface, outermost first.
    ///
    /// [`super::diagnostics`] reads it: a rule stated over a type node has to
    /// reach the nested ones too, and the only enumeration of "every type node
    /// there is" should be the one [`surfaces`] already fixes.
    pub fn walk(&self, visit: &mut dyn FnMut(&TypeNode)) {
        match &self.body {
            Body::Fields(fields) => {
                for field in &fields.fields {
                    walk(&field.ty, visit);
                }
            }
            Body::Type(ty) => walk(ty, visit),
        }
    }

    /// Whether any type node anywhere inside this surface satisfies `predicate`.
    fn declares(&self, predicate: &dyn Fn(&TypeNode) -> bool) -> bool {
        match &self.body {
            Body::Fields(fields) => fields
                .fields
                .iter()
                .any(|field| declares(&field.ty, predicate)),
            Body::Type(ty) => declares(ty, predicate),
        }
    }
}

/// Visit this type node and everything nested inside it, outermost first.
fn walk(ty: &TypeNode, visit: &mut dyn FnMut(&TypeNode)) {
    visit(ty);
    match &ty.form {
        TypeForm::Array(array) => walk(&array.items, visit),
        TypeForm::Object(object) => {
            for field in &object.properties.fields {
                walk(&field.ty, visit);
            }
        }
        TypeForm::Union(union) => {
            for variant in &union.variants {
                for field in &variant.fields.fields {
                    walk(&field.ty, visit);
                }
            }
        }
        TypeForm::Scalar(_) | TypeForm::Enum(_) => {}
    }
}

/// Whether this type node or anything nested inside it satisfies `predicate`.
fn declares(ty: &TypeNode, predicate: &dyn Fn(&TypeNode) -> bool) -> bool {
    if predicate(ty) {
        return true;
    }
    match &ty.form {
        TypeForm::Array(array) => declares(&array.items, predicate),
        TypeForm::Object(object) => object
            .properties
            .fields
            .iter()
            .any(|field| declares(&field.ty, predicate)),
        TypeForm::Union(union) => union.variants.iter().any(|variant| {
            variant
                .fields
                .fields
                .iter()
                .any(|field| declares(&field.ty, predicate))
        }),
        TypeForm::Scalar(_) | TypeForm::Enum(_) => false,
    }
}

/// A field map as `z.object({ … }).strict()` — a closed object (grammar 3.1,
/// Decision D8).
///
/// Every declared property is **required**. `optional:` is a key of an *object*
/// type node (grammar 3.4), not of a declaration surface, so the only thing that
/// makes a property of a field map optional is its own `default:` (grammar 3.6)
/// — which `.default(…)` already expresses.
///
/// `indent` is the indentation of the line the expression starts on; the
/// object's properties sit one level deeper and the closing brace comes back to
/// it.
#[must_use]
pub fn field_map(map: &FieldMap, indent: &str) -> String {
    properties(&map.fields, &[], indent)
}

/// The shared body of a field map and an object type node: `z.object({…})`
/// closed with `.strict()`, with `optional:` applied to the properties that name
/// it.
fn properties(
    fields: &[crate::ir::schema::Field],
    optional: &[crate::diag::Spanned<crate::ast::common::Ident>],
    indent: &str,
) -> String {
    if fields.is_empty() {
        return "z.object({}).strict()".to_string();
    }
    let inner = format!("{indent}  ");
    let mut text = String::from("z.object({\n");
    for field in fields {
        let name = field.name.value.as_str();
        let mut rendered = type_node(&field.ty, &inner);
        // A defaulted property is already optional at its surface (grammar 3.6),
        // and `.default(v).optional()` would answer an omitted property with
        // `undefined` instead of the default — see the module docs.
        if declared_default(&field.ty).is_none()
            && optional.iter().any(|entry| entry.value.as_str() == name)
        {
            rendered.push_str(".optional()");
        }
        text.push_str(&format!("{inner}{}: {rendered},\n", property_key(name)));
    }
    text.push_str(&format!("{indent}}}).strict()"));
    text
}

/// A property key, quoted only when it has to be.
///
/// Identifiers are grammar 2.1's `lower , { lower | digit | "_" }`, every one of
/// which is a legal JavaScript property name, so no key here is ever quoted —
/// the function exists so that stays a decision rather than an assumption.
fn property_key(name: &str) -> String {
    let bare = name
        .chars()
        .next()
        .is_some_and(|first| first.is_ascii_lowercase())
        && name.chars().all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '_'
        });
    if bare {
        name.to_string()
    } else {
        names::string(name)
    }
}

/// A type node as a Zod expression (grammar 3.2).
#[must_use]
pub fn type_node(ty: &TypeNode, indent: &str) -> String {
    let mut text = match &ty.form {
        TypeForm::Scalar(scalar) => scalar_expression(scalar),
        TypeForm::Enum(enumeration) => enum_expression(enumeration),
        TypeForm::Object(object) => object_expression(object, indent),
        TypeForm::Array(array) => array_expression(array, indent),
        TypeForm::Union(union) => union_expression(union, indent),
    };
    if let Some(description) = &ty.description {
        text.push_str(&format!(".describe({})", names::string(&description.value)));
    }
    if let Some(default) = declared_default(ty) {
        text.push_str(&format!(".default({})", names::literal(default)));
    }
    text
}

/// The `default:` a type node declares, whichever form carries it. A union never
/// does (grammar 3.6).
fn declared_default(ty: &TypeNode) -> Option<&crate::ast::common::Literal> {
    match &ty.form {
        TypeForm::Scalar(scalar) => scalar.default.as_ref(),
        TypeForm::Enum(enumeration) => enumeration.default.as_ref(),
        TypeForm::Object(object) => object.default.as_ref(),
        TypeForm::Array(array) => array.default.as_ref(),
        TypeForm::Union(_) => None,
    }
    .map(|default| &default.value)
}

fn scalar_expression(scalar: &Scalar) -> String {
    let mut text = match (scalar.kind, scalar.format) {
        (ScalarKind::String, Some(format)) => zod_format(format).to_string(),
        (ScalarKind::String, None) => "z.string()".to_string(),
        (ScalarKind::Integer, _) => "z.number().int()".to_string(),
        (ScalarKind::Number, _) => "z.number()".to_string(),
        (ScalarKind::Boolean, _) => "z.boolean()".to_string(),
    };
    if let Some(min) = scalar.min_length {
        text.push_str(&format!(".min({min})"));
    }
    if let Some(max) = scalar.max_length {
        text.push_str(&format!(".max({max})"));
    }
    if let Some(pattern) = &scalar.pattern {
        text.push_str(&format!(".regex({})", regex_expression(pattern)));
    }
    for (constraint, value) in [
        ("min", scalar.minimum.as_ref()),
        ("max", scalar.maximum.as_ref()),
        ("gt", scalar.exclusive_minimum.as_ref()),
        ("lt", scalar.exclusive_maximum.as_ref()),
        ("multipleOf", scalar.multiple_of.as_ref()),
    ] {
        if let Some(value) = value {
            text.push_str(&format!(".{constraint}({})", names::number(value)));
        }
    }
    text
}

/// The check one `format:` lowers to (grammar 3.3).
///
/// Four of the ten are Zod's own constructor, because on those four Zod's
/// reading and JSON Schema's `format` keyword agree on every document the
/// conformance corpus can find: `duration`, `uuid`, `ipv4`, `ipv6`.
///
/// The other six are written out in [`HELPERS`] instead, because their
/// constructors read a *different specification* from the one JSON Schema's
/// keyword names — and JSON Schema is what a model is handed, so a value it
/// blessed that the emitted Zod then refused is a run failing on its own
/// contract. Each is one sentence:
///
/// * `date-time`/`date` — `z.iso.datetime()` and `z.iso.date()` take the
///   upper-case-only ISO 8601 profile; RFC 3339 admits `t` and `z`, and a leap
///   second.
/// * `time` — `z.iso.time()` **refuses** an offset and RFC 3339's `full-time`
///   **requires** one, so the two are disjoint rather than merely different.
/// * `email` — `z.email()` is a narrow subset that refuses a quoted local part,
///   a single-label domain, and an address literal.
/// * `uri` — `z.url()` is the WHATWG parser, which repairs its input rather than
///   validating it.
/// * `hostname` — `z.hostname()` accepts a trailing root dot.
const fn zod_format(format: StringFormat) -> &'static str {
    match format {
        StringFormat::DateTime => {
            "z.string().refine(rfc3339DateTime, { message: \"expected an RFC 3339 date-time\" })"
        }
        StringFormat::Date => {
            "z.string().refine(rfc3339Date, { message: \"expected an RFC 3339 date\" })"
        }
        StringFormat::Time => "z.string().regex(rfc3339Time)",
        StringFormat::Duration => "z.iso.duration()",
        StringFormat::Email => {
            "z.string().refine(rfc5321Email, { message: \"expected an email address\" })"
        }
        StringFormat::Uri => "z.string().regex(rfc3986Uri)",
        StringFormat::Uuid => "z.uuid()",
        StringFormat::Hostname => {
            "z.string().refine(rfc1123Hostname, { message: \"expected a hostname\" })"
        }
        StringFormat::Ipv4 => "z.ipv4()",
        StringFormat::Ipv6 => "z.ipv6()",
    }
}

fn enum_expression(enumeration: &EnumType) -> String {
    let variants: Vec<String> = enumeration
        .variants
        .iter()
        .map(|variant| names::string(&variant.value))
        .collect();
    format!("z.enum([{}])", variants.join(", "))
}

fn object_expression(object: &ObjectType, indent: &str) -> String {
    properties(&object.properties.fields, &object.optional, indent)
}

fn array_expression(array: &ArrayType, indent: &str) -> String {
    let mut text = format!("z.array({})", type_node(&array.items, indent));
    if let Some(min) = array.min_items {
        text.push_str(&format!(".min({min})"));
    }
    if let Some(max) = array.max_items {
        text.push_str(&format!(".max({max})"));
    }
    if array.unique_items == Some(true) {
        text.push_str(".refine(uniqueItems, { message: \"expected unique items\" })");
    }
    text
}

fn union_expression(union: &crate::ir::schema::UnionType, indent: &str) -> String {
    let inner = format!("{indent}  ");
    let discriminator = union.discriminator.value.as_str();
    let mut text = format!("z.discriminatedUnion({}, [\n", names::string(discriminator));
    for variant in &union.variants {
        let deeper = format!("{inner}  ");
        let mut object = String::from("z.object({\n");
        object.push_str(&format!(
            "{deeper}{}: z.literal({}),\n",
            property_key(discriminator),
            names::string(variant.tag.value.as_str())
        ));
        for field in &variant.fields.fields {
            object.push_str(&format!(
                "{deeper}{}: {},\n",
                property_key(field.name.value.as_str()),
                type_node(&field.ty, &deeper)
            ));
        }
        object.push_str(&format!("{inner}}}).strict()"));
        text.push_str(&format!("{inner}{object},\n"));
    }
    text.push_str(&format!("{indent}])"));
    text
}

/// A `pattern:` as a JavaScript regular expression.
///
/// The decision — whether RE2's source text *has* a JavaScript spelling — is
/// [`super::pattern`]'s, and [`super::diagnostics`] is what stops `build` before
/// a pattern with no spelling reaches here. Emission stays total for the
/// library caller who skipped that step: the refused text is written out as it
/// would have been, which is a module `tsc` and the first `import` both report.
fn regex_expression(pattern: &str) -> String {
    super::pattern::javascript(pattern).unwrap_or_else(|_| super::pattern::verbatim(pattern))
}

// ---------------------------------------------------------------------------
// The JSON Schema column
// ---------------------------------------------------------------------------

/// A field map as JSON Schema draft 2020-12: a closed object.
#[must_use]
pub fn json_field_map(map: &FieldMap) -> Value {
    let mut properties = Map::new();
    let mut required = Vec::new();
    for field in &map.fields {
        let name = field.name.value.as_str();
        properties.insert(name.to_string(), json_type_node(&field.ty));
        if declared_default(&field.ty).is_none() {
            required.push(Value::String(name.to_string()));
        }
    }
    object_schema(properties, required)
}

fn object_schema(properties: Map<String, Value>, required: Vec<Value>) -> Value {
    let mut schema = Map::new();
    schema.insert("type".to_string(), json!("object"));
    schema.insert("properties".to_string(), Value::Object(properties));
    schema.insert("required".to_string(), Value::Array(required));
    schema.insert("additionalProperties".to_string(), json!(false));
    Value::Object(schema)
}

/// A type node as JSON Schema draft 2020-12.
#[must_use]
pub fn json_type_node(ty: &TypeNode) -> Value {
    let mut schema = match &ty.form {
        TypeForm::Scalar(scalar) => json_scalar(scalar),
        TypeForm::Enum(enumeration) => {
            let variants: Vec<Value> = enumeration
                .variants
                .iter()
                .map(|variant| Value::String(variant.value.clone()))
                .collect();
            json!({ "type": "string", "enum": variants })
        }
        TypeForm::Object(object) => {
            let mut properties = Map::new();
            let mut required = Vec::new();
            for field in &object.properties.fields {
                let name = field.name.value.as_str();
                properties.insert(name.to_string(), json_type_node(&field.ty));
                let optional = declared_default(&field.ty).is_some()
                    || object
                        .optional
                        .iter()
                        .any(|entry| entry.value.as_str() == name);
                if !optional {
                    required.push(Value::String(name.to_string()));
                }
            }
            object_schema(properties, required)
        }
        TypeForm::Array(array) => {
            let mut schema = Map::new();
            schema.insert("type".to_string(), json!("array"));
            schema.insert("items".to_string(), json_type_node(&array.items));
            if let Some(max) = array.max_items {
                schema.insert("maxItems".to_string(), json!(max));
            }
            if let Some(min) = array.min_items {
                schema.insert("minItems".to_string(), json!(min));
            }
            if array.unique_items == Some(true) {
                schema.insert("uniqueItems".to_string(), json!(true));
            }
            Value::Object(schema)
        }
        TypeForm::Union(union) => {
            let discriminator = union.discriminator.value.as_str();
            let variants: Vec<Value> = union
                .variants
                .iter()
                .map(|variant| {
                    let mut properties = Map::new();
                    let mut required = vec![Value::String(discriminator.to_string())];
                    properties.insert(
                        discriminator.to_string(),
                        json!({ "const": variant.tag.value.as_str() }),
                    );
                    for field in &variant.fields.fields {
                        let name = field.name.value.as_str();
                        properties.insert(name.to_string(), json_type_node(&field.ty));
                        if declared_default(&field.ty).is_none() {
                            required.push(Value::String(name.to_string()));
                        }
                    }
                    object_schema(properties, required)
                })
                .collect();
            json!({ "oneOf": variants })
        }
    };

    let object = schema
        .as_object_mut()
        .expect("every lowering above answers an object");
    if let Some(description) = &ty.description {
        object.insert("description".to_string(), json!(description.value));
    }
    if let Some(default) = declared_default(ty) {
        object.insert("default".to_string(), json_literal(default));
    }
    schema
}

fn json_scalar(scalar: &Scalar) -> Value {
    let mut schema = Map::new();
    schema.insert("type".to_string(), json!(scalar.kind.as_str()));
    if let Some(min) = scalar.min_length {
        schema.insert("minLength".to_string(), json!(min));
    }
    if let Some(max) = scalar.max_length {
        schema.insert("maxLength".to_string(), json!(max));
    }
    if let Some(pattern) = &scalar.pattern {
        schema.insert("pattern".to_string(), json!(pattern));
    }
    if let Some(format) = scalar.format {
        schema.insert("format".to_string(), json!(format.as_str()));
    }
    for (key, value) in [
        ("minimum", scalar.minimum.as_ref()),
        ("maximum", scalar.maximum.as_ref()),
        ("exclusiveMinimum", scalar.exclusive_minimum.as_ref()),
        ("exclusiveMaximum", scalar.exclusive_maximum.as_ref()),
        ("multipleOf", scalar.multiple_of.as_ref()),
    ] {
        if let Some(number) = value {
            schema.insert(key.to_string(), json_number(number));
        }
    }
    Value::Object(schema)
}

fn json_number(number: &Number) -> Value {
    match number {
        Number::Int(int) => json!(int),
        Number::Float(float) => json!(float),
    }
}

/// A literal as JSON data — the same shape [`crate::ir::leaf`] writes it in.
#[must_use]
pub fn json_literal(literal: &crate::ast::common::Literal) -> Value {
    use crate::ast::common::Literal;
    match literal {
        Literal::Null => Value::Null,
        Literal::Bool(boolean) => json!(boolean),
        Literal::Int(int) => json!(int),
        Literal::Float(float) => json!(float),
        Literal::String(text) => json!(text),
        Literal::Sequence(items) => {
            Value::Array(items.iter().map(|item| json_literal(&item.value)).collect())
        }
        Literal::Mapping(entries) => {
            let mut map = Map::new();
            for entry in entries {
                map.insert(entry.key.value.clone(), json_literal(&entry.value.value));
            }
            Value::Object(map)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codegen::test_support::ir_of;

    fn schema_of(source: &str, path: &str) -> String {
        let ir = ir_of(source);
        let surfaces = surfaces(&ir);
        let surface = surfaces
            .iter()
            .find(|surface| surface.path == path)
            .unwrap_or_else(|| {
                panic!(
                    "`{path}` is not a surface; the composition declares {:?}",
                    surfaces.iter().map(|s| s.path.as_str()).collect::<Vec<_>>()
                )
            });
        match &surface.body {
            Body::Fields(fields) => field_map(fields, ""),
            Body::Type(ty) => type_node(ty, ""),
        }
    }

    const CHANNEL: &str = "version: \"0.1\"\nstate:\n";

    #[test]
    fn scalars_carry_their_constraint_vocabulary() {
        assert_eq!(
            schema_of(
                &format!("{CHANNEL}  a: {{ type: string, min_length: 1, max_length: 200 }}\n"),
                "state.a"
            ),
            "z.string().min(1).max(200)"
        );
        assert_eq!(
            schema_of(
                &format!("{CHANNEL}  a: {{ type: integer, minimum: 0, multiple_of: 2 }}\n"),
                "state.a"
            ),
            "z.number().int().min(0).multipleOf(2)"
        );
        assert_eq!(
            schema_of(
                &format!(
                    "{CHANNEL}  a: {{ type: number, exclusive_minimum: 0, exclusive_maximum: 1 }}\n"
                ),
                "state.a"
            ),
            "z.number().gt(0).lt(1)"
        );
        assert_eq!(
            schema_of(&format!("{CHANNEL}  a: {{ type: boolean }}\n"), "state.a"),
            "z.boolean()"
        );
    }

    /// Six of the ten formats are written out rather than borrowed from a Zod
    /// constructor that reads a different specification (see the module docs).
    #[test]
    fn a_format_lowers_to_the_check_the_rfc_names() {
        assert_eq!(
            schema_of(
                &format!("{CHANNEL}  a: {{ type: string, format: date-time }}\n"),
                "state.a"
            ),
            "z.string().refine(rfc3339DateTime, { message: \"expected an RFC 3339 date-time\" })",
            "`z.iso.datetime()` is an upper-case-only ISO 8601 profile, not RFC 3339"
        );
        assert_eq!(
            schema_of(
                &format!("{CHANNEL}  a: {{ type: string, format: time }}\n"),
                "state.a"
            ),
            "z.string().regex(rfc3339Time)",
            "`z.iso.time()` refuses the offset RFC 3339 requires"
        );
        assert_eq!(
            schema_of(
                &format!("{CHANNEL}  a: {{ type: string, format: email, max_length: 320 }}\n"),
                "state.a"
            ),
            "z.string().refine(rfc5321Email, { message: \"expected an email address\" })\
             .max(320)",
            "a constraint still chains onto the refined string"
        );
        assert_eq!(
            schema_of(
                &format!("{CHANNEL}  a: {{ type: string, format: uri }}\n"),
                "state.a"
            ),
            "z.string().regex(rfc3986Uri)"
        );
        assert_eq!(
            schema_of(
                &format!("{CHANNEL}  a: {{ type: string, format: uuid }}\n"),
                "state.a"
            ),
            "z.uuid()",
            "the four that agree with JSON Schema keep their constructor"
        );
    }

    /// A helper is written only where the composition reaches it, and a helper
    /// that calls another brings it along.
    #[test]
    fn the_helper_prelude_holds_what_the_composition_reaches_and_no_more() {
        let module = |source: &str| {
            let ir = ir_of(source);
            crate::codegen::schema::module(&ir, &Names::of(&ir)).contents
        };

        let bare = module(&format!("{CHANNEL}  a: {{ type: string }}\n"));
        for helper in HELPERS {
            assert!(
                !bare.contains(&format!("const {} ", helper.name)),
                "`{}` is declared in a module that never calls it",
                helper.name
            );
        }

        // `date-time` calls both halves of itself, and neither is declared after
        // the caller that needs it.
        let stamped = module(&format!(
            "{CHANNEL}  a: {{ type: string, format: date-time }}\n"
        ));
        let at = |name: &str| {
            stamped
                .find(&format!("const {name} "))
                .map(|at| at as isize)
        };
        assert!(at("rfc3339Date").is_some() && at("rfc3339Time").is_some());
        assert!(at("rfc3339DateTime") > at("rfc3339Date"));
        assert!(at("rfc3339DateTime") > at("rfc3339Time"));
        assert!(at("rfc5321Email").is_none(), "and nothing it does not call");

        // `email` calls `rfc1123Hostname`, which `hostname` also selects.
        let addressed = module(&format!(
            "{CHANNEL}  a: {{ type: string, format: email }}\n"
        ));
        assert!(addressed.contains("const rfc1123Hostname "));
        assert!(
            addressed.find("const rfc1123Hostname ") < addressed.find("const rfc5321Email "),
            "a callee is declared before its caller"
        );
    }

    #[test]
    fn an_enum_is_a_zod_enum_in_declaration_order() {
        assert_eq!(
            schema_of(
                &format!("{CHANNEL}  a: {{ enum: [revise, approve] }}\n"),
                "state.a"
            ),
            "z.enum([\"revise\", \"approve\"])"
        );
    }

    #[test]
    fn a_default_becomes_a_zod_default() {
        assert_eq!(
            schema_of(
                &format!("{CHANNEL}  a: {{ type: string, default: \"\" }}\n"),
                "state.a"
            ),
            "z.string().default(\"\")"
        );
        assert_eq!(
            schema_of(
                &format!("{CHANNEL}  a: {{ type: integer, default: 1 }}\n"),
                "state.a"
            ),
            "z.number().int().default(1)"
        );
    }

    #[test]
    fn a_description_becomes_a_zod_describe() {
        assert_eq!(
            schema_of(
                &format!("{CHANNEL}  a: {{ type: string, description: The draft. }}\n"),
                "state.a"
            ),
            "z.string().describe(\"The draft.\")"
        );
    }

    #[test]
    fn an_array_carries_its_bounds_and_its_uniqueness_check() {
        assert_eq!(
            schema_of(
                &format!(
                    "{CHANNEL}  a:\n    type: array\n    max_items: 20\n    min_items: 1\n    items: {{ type: string }}\n"
                ),
                "state.a"
            ),
            "z.array(z.string()).min(1).max(20)"
        );
        assert_eq!(
            schema_of(
                &format!(
                    "{CHANNEL}  a:\n    type: array\n    unique_items: true\n    items: {{ type: string }}\n"
                ),
                "state.a"
            ),
            "z.array(z.string()).refine(uniqueItems, { message: \"expected unique items\" })"
        );
    }

    #[test]
    fn an_object_is_closed_and_lists_its_optional_properties() {
        assert_eq!(
            schema_of(
                &format!(
                    "{CHANNEL}  a:\n    type: object\n    properties:\n      name: {{ type: string }}\n      email: {{ type: string, format: email }}\n    optional: [email]\n"
                ),
                "state.a"
            ),
            "z.object({\n  name: z.string(),\n  email: z.string().refine(rfc5321Email, \
             { message: \"expected an email address\" }).optional(),\n}).strict()"
        );
    }

    /// Grammar 3.6 makes a defaulted property implicitly optional; writing
    /// `.optional()` on top of `.default()` would answer an omitted property
    /// with `undefined` instead of the default.
    #[test]
    fn a_defaulted_property_is_not_also_written_optional() {
        assert_eq!(
            schema_of(
                &format!(
                    "{CHANNEL}  a:\n    type: object\n    properties:\n      n: {{ type: integer, default: 0 }}\n    optional: [n]\n"
                ),
                "state.a"
            ),
            "z.object({\n  n: z.number().int().default(0),\n}).strict()"
        );
    }

    #[test]
    fn an_empty_field_map_is_a_closed_object_with_no_properties() {
        let source = "version: \"0.1\"\n\
tool.noop:\n  description: Does nothing.\n  input: {}\n  output: {}\n  exec:\n    command: \"true\"\n";
        assert_eq!(
            schema_of(source, "tool.noop.input"),
            "z.object({}).strict()"
        );
    }

    #[test]
    fn a_union_synthesizes_its_discriminator_as_a_literal() {
        let source = "version: \"0.1\"\n\
agent.triage:\n  model: model.m\n  prompt: p\n  output:\n    findings:\n      type: array\n      max_items: 3\n      items:\n        discriminator: kind\n        variants:\n          auto_fixable: { file: { type: string } }\n          needs_human: { summary: { type: string } }\n\
provider.p:\n  kind: anthropic\n  api_key: ${K}\n\
model.m:\n  provider: provider.p\n  id: some-model\n";
        assert_eq!(
            schema_of(source, "agent.triage.output"),
            "z.object({\n  \
               findings: z.array(z.discriminatedUnion(\"kind\", [\n    \
                 z.object({\n      kind: z.literal(\"auto_fixable\"),\n      file: z.string(),\n    }).strict(),\n    \
                 z.object({\n      kind: z.literal(\"needs_human\"),\n      summary: z.string(),\n    }).strict(),\n  \
               ])).max(3),\n\
             }).strict()"
        );
    }

    #[test]
    fn a_pattern_is_a_regex_literal_with_its_slashes_escaped() {
        assert_eq!(regex_expression("^a.*z$"), "/^a.*z$/");
        assert_eq!(regex_expression("a/b"), "/a\\/b/");
        assert_eq!(regex_expression("a\\/b"), "/a\\/b/");
        assert_eq!(regex_expression("a\\\\"), "/a\\\\/");
        assert_eq!(regex_expression("a\nb"), "new RegExp(\"a\\nb\")");
    }

    #[test]
    fn the_json_column_lowers_the_same_table() {
        let ir = ir_of(&format!(
            "{CHANNEL}  a:\n    type: object\n    properties:\n      name: {{ type: string, min_length: 1 }}\n      email: {{ type: string, format: email }}\n    optional: [email]\n"
        ));
        let surfaces = surfaces(&ir);
        let Body::Type(ty) = &surfaces[0].body else {
            panic!("a channel is a type node");
        };
        assert_eq!(
            json_type_node(ty),
            json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "minLength": 1 },
                    "email": { "type": "string", "format": "email" },
                },
                "required": ["name"],
                "additionalProperties": false,
            })
        );
    }

    #[test]
    fn a_union_lowers_to_one_of_with_a_const_tag() {
        let source = "version: \"0.1\"\n\
agent.triage:\n  model: model.m\n  prompt: p\n  output:\n    finding:\n      discriminator: kind\n      variants:\n        a: { x: { type: string } }\n        b: { y: { type: integer } }\n\
provider.p:\n  kind: anthropic\n  api_key: ${K}\n\
model.m:\n  provider: provider.p\n  id: some-model\n";
        let ir = ir_of(source);
        let surfaces = surfaces(&ir);
        let surface = surfaces
            .iter()
            .find(|surface| surface.path == "agent.triage.output")
            .expect("the agent declares an output");
        let Body::Fields(fields) = &surface.body else {
            panic!("an output is a field map");
        };
        assert_eq!(
            json_field_map(fields),
            json!({
                "type": "object",
                "properties": {
                    "finding": {
                        "oneOf": [
                            {
                                "type": "object",
                                "properties": { "kind": { "const": "a" }, "x": { "type": "string" } },
                                "required": ["kind", "x"],
                                "additionalProperties": false,
                            },
                            {
                                "type": "object",
                                "properties": { "kind": { "const": "b" }, "y": { "type": "integer" } },
                                "required": ["kind", "y"],
                                "additionalProperties": false,
                            },
                        ]
                    }
                },
                "required": ["finding"],
                "additionalProperties": false,
            })
        );
    }

    #[test]
    fn every_surface_of_a_composition_is_enumerated_once_in_canonical_order() {
        let source = "version: \"0.1\"\n\
state:\n  draft: { type: string, default: \"\" }\n\
provider.p:\n  kind: anthropic\n  api_key: ${K}\n\
model.m:\n  provider: provider.p\n  id: some-model\n\
agent.a:\n  model: model.m\n  prompt: p\n  input: { goal: { type: string } }\n  output: { verdict: { enum: [ok] } }\n\
tool.t:\n  description: A tool.\n  input: {}\n  output: {}\n  exec: { command: \"true\" }\n\
store.s:\n  kind: kv\n  scope: execution\n  value_schema: { v: { type: string } }\n\
flow.f:\n  outputs: { draft: { type: string } }\n  nodes:\n    a: { agent: agent.a, input: { goal: \"'x'\" } }\n    ask:\n      human:\n        input: { q: { type: string } }\n        output: { answer: { type: string } }\n    probe: { exec: { command: \"true\" } }\n    ping: { http: { method: GET, url: \"https://example.test\" } }\n    keep: { store: store.s, op: set, key: \"'k'\", value: { v: \"'x'\" } }\n  edges:\n    - { from: start, to: a }\n    - { from: a, to: ask }\n    - { from: ask, to: probe }\n    - { from: probe, to: ping }\n    - { from: ping, to: keep }\n    - { from: keep, to: end }\n";
        let ir = ir_of(source);
        assert_eq!(
            surfaces(&ir)
                .iter()
                .map(|surface| surface.path.as_str())
                .collect::<Vec<_>>(),
            [
                "agent.a.input",
                "agent.a.output",
                "flow.f.outputs",
                "flow.f.node.ask.input",
                "flow.f.node.ask.output",
                // None of the three writes an `output:`, and all three are
                // here anyway: a kind default and a derived store row are the
                // node's result schema, not the absence of one.
                "flow.f.node.probe.output",
                "flow.f.node.ping.output",
                "flow.f.node.keep.output",
                "store.s.value_schema",
                "tool.t.input",
                "tool.t.output",
                "state.draft",
            ]
        );
    }

    /// An inline node with no `output:` has the **kind default** as its result
    /// schema (grammar 8.2, 8.3), and the emitter writes it.
    ///
    /// The validator already resolves the same default (`check::model`), so a
    /// composition can guard an edge on `probe.output.exit_code` while the
    /// emitted module holds no schema and no export name for it. The two
    /// defaults are asserted field by field here rather than only through the
    /// goldens, because a change to either one is a change to what a node's
    /// answer is parsed with.
    #[test]
    fn an_inline_node_with_no_output_gets_its_kind_default_schema() {
        let source = "version: \"0.1\"\n\
flow.f:\n  outputs: {}\n  nodes:\n    probe: { exec: { command: \"true\" } }\n    ping: { http: { method: GET, url: \"https://example.test\" } }\n  edges:\n    - { from: start, to: probe }\n    - { from: probe, to: ping }\n    - { from: ping, to: end }\n";
        assert_eq!(
            schema_of(source, "flow.f.node.probe.output"),
            "z.object({\n  exit_code: z.number().int(),\n  stdout: z.string(),\n}).strict()"
        );
        assert_eq!(
            schema_of(source, "flow.f.node.ping.output"),
            "z.object({\n  status: z.number().int(),\n  body: z.string(),\n}).strict()"
        );

        // And the doc comment says the schema is a default rather than
        // something the node was read as declaring.
        let ir = ir_of(source);
        let module = crate::codegen::schema::module(&ir, &Names::of(&ir)).contents;
        assert!(
            module.contains("which it does not declare, so this is the kind default"),
            "{module}"
        );
    }

    /// A node that writes the default out by hand is the same schema, and is
    /// **not** described as a default — the control for the test above.
    #[test]
    fn a_declared_output_is_emitted_as_declared() {
        let source = "version: \"0.1\"\n\
flow.f:\n  outputs: {}\n  nodes:\n    probe:\n      exec:\n        command: \"true\"\n        output:\n          exit_code: { type: integer }\n          stdout: { type: string }\n  edges:\n    - { from: start, to: probe }\n    - { from: probe, to: end }\n";
        assert_eq!(
            schema_of(source, "flow.f.node.probe.output"),
            "z.object({\n  exit_code: z.number().int(),\n  stdout: z.string(),\n}).strict()"
        );
        let ir = ir_of(source);
        let module = crate::codegen::schema::module(&ir, &Names::of(&ir)).contents;
        assert!(!module.contains("kind default"), "{module}");
    }
}
