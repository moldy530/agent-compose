//! The curated table of server tools, per provider kind (grammar 12.1,
//! resolved q30).
//!
//! A **server tool** executes on the provider's side, inside the model call:
//! the runtime dispatches nothing, and the results arrive woven into the
//! assistant's turn. A provider declares its own suite in that provider's wire
//! vocabulary, and the runtime appends the array to the `tools` of every request
//! that provider serves.
//!
//! # Why the table is small on purpose
//!
//! The governing constraint is **no manual support treadmill**: a server tool a
//! vendor ships tomorrow must be usable the day it ships, without waiting for a
//! compiler release. So the checking is two-tier (q30):
//!
//! * a config naming a tool **in this table** is validated **strictly** against
//!   the fields the table models — a mistyped value, a value outside a closed
//!   set or a stated range, a missing required field and a constraint violation
//!   are errors with diagnostics naming the repair (PRD G3);
//! * anything the table cannot speak for is a **warning** that names exactly
//!   what could not be verified, and then travels to the wire verbatim.
//!
//! The second tier is reached at **two** granularities, and for one reason. A
//! `type:` outside the table is the obvious one. The other is a *field* outside
//! a tabled tool's row: a vendor adds parameters to a tool it already ships, and
//! a compiler that refused every field its own table predates would be the
//! treadmill again — narrower, and just as blocking, since a row is keyed on
//! `type:` alone and there is no way to opt one entry out of the strict tier.
//! Nothing here can tell "a field this release is older than" from "a
//! misspelling" — so both are warned about, the near miss is named in the help
//! when there is one, and the key travels.
//!
//! The table is therefore a convenience that buys diagnostics, never a gate —
//! at either granularity. A kind with **no** row — `openai_compatible`, whose
//! gateway may honour any vocabulary at all — is served by the second tier
//! alone, which is why [`known`] answers an empty slice for it rather than being
//! unimplemented.
//!
//! # One source
//!
//! Everything that has an opinion about a known tool reads this module: the
//! validator ([`crate::check::providers`]), the tests that hold the published
//! JSON Schema to the same shapes
//! (`crates/compose-core/tests/schema_conformance.rs`), and the topic document
//! that teaches the key (`docs/topics/models.md`). Adding a tool is an edit
//! here and a mirrored `if`/`then` in `schemas/agent-compose.schema.json`, which
//! `the_published_schema_knows_every_server_tool_the_table_does` holds together.

use super::definition::ProviderKind;

/// The shape of one field of a server-tool config.
///
/// A deliberately small vocabulary: these are wire config objects, not the
/// schema language, and every shape here exists because a documented field of a
/// launch-scope tool has it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FieldShape {
    /// An integer, inclusive bounds.
    Integer(i64, i64),
    /// A number, inclusive bounds.
    Number(f64, f64),
    /// A boolean.
    Boolean,
    /// Any string. Interpolable, like every other non-secret provider value
    /// (grammar 4.3 class 2).
    Text,
    /// One of a closed set of strings.
    Choice(&'static [&'static str]),
    /// An array of strings.
    Strings,
    /// A nested object with its own fields.
    Object(&'static ObjectShape),
    /// A string **or** a nested object — the one field in the launch scope that
    /// is written both ways (`code_interpreter`'s `container:`, which is either
    /// a container id or a request to make one).
    TextOrObject(&'static ObjectShape),
    /// A field the vendor documents whose **interior** this vocabulary cannot
    /// state: a recursive shape, or a union of several. Any value at all is
    /// accepted and travels verbatim.
    ///
    /// It is here so that such a field is a *known* field — the row says the
    /// tool has it, so no warning — rather than one the table is silent about.
    /// The alternative is worse in both directions: leaving it out warns on a
    /// documented parameter, and modelling it badly refuses one.
    /// `file_search`'s `filters:` is the launch scope's only case (a comparison
    /// filter, or a compound one holding more filters).
    Opaque,
}

impl FieldShape {
    /// How a diagnostic names this shape, with its article.
    #[must_use]
    pub const fn description(self) -> &'static str {
        match self {
            Self::Integer(..) => "an integer",
            Self::Number(..) => "a number",
            Self::Boolean => "a boolean",
            Self::Text | Self::Choice(_) => "a string",
            Self::Strings => "an array of strings",
            Self::Object(_) => "a mapping",
            Self::TextOrObject(_) => "a string or a mapping",
            Self::Opaque => "any value",
        }
    }
}

/// One field of a config object.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Field {
    /// The key, as the wire spells it.
    pub name: &'static str,
    /// What it holds.
    pub shape: FieldShape,
}

/// The fields one config object admits, and which of them it requires.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ObjectShape {
    /// Every field, in the order the vendor's own reference lists them.
    pub fields: &'static [Field],
    /// The fields that must be present.
    pub required: &'static [&'static str],
    /// Pairs of fields that may not both appear.
    pub exclusive: &'static [[&'static str; 2]],
}

impl ObjectShape {
    /// The shape of one field, if the object has it.
    #[must_use]
    pub fn field(&self, name: &str) -> Option<FieldShape> {
        self.fields
            .iter()
            .find(|field| field.name == name)
            .map(|field| field.shape)
    }

    /// Every field name, in declaration order.
    #[must_use]
    pub fn names(&self) -> Vec<&'static str> {
        self.fields.iter().map(|field| field.name).collect()
    }
}

/// One server tool a provider kind is known to serve.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ServerTool {
    /// The `type:` string, exactly as the wire takes it. Versioned where the
    /// vendor versions it — `web_search_20250305` is a different tool from a
    /// later dated revision, and a config written for one is not a config for
    /// the other.
    pub type_name: &'static str,
    /// One line naming what it does, for the diagnostics and the docs.
    pub summary: &'static str,
    /// Everything beside `type:`.
    pub shape: &'static ObjectShape,
}

// ---------------------------------------------------------------------------
// Anthropic — the Messages wire
// ---------------------------------------------------------------------------

/// `user_location:`, as both vendors spell it: an approximate location the
/// search is biased towards.
const USER_LOCATION: ObjectShape = ObjectShape {
    fields: &[
        Field {
            name: "type",
            shape: FieldShape::Choice(&["approximate"]),
        },
        Field {
            name: "city",
            shape: FieldShape::Text,
        },
        Field {
            name: "region",
            shape: FieldShape::Text,
        },
        Field {
            name: "country",
            shape: FieldShape::Text,
        },
        Field {
            name: "timezone",
            shape: FieldShape::Text,
        },
    ],
    required: &["type"],
    exclusive: &[],
};

/// `citations:` on `web_fetch`.
const CITATIONS: ObjectShape = ObjectShape {
    fields: &[Field {
        name: "enabled",
        shape: FieldShape::Boolean,
    }],
    required: &["enabled"],
    exclusive: &[],
};

/// Every Messages-wire server tool's `name:`, which the API requires beside the
/// `type:` and which **must be the tool's canonical name**.
///
/// A closed choice of one rather than free text, because the constraint is the
/// vendor's: the Messages API pairs each dated `type:` with one fixed `name:` —
/// `web_search_20250305` is `web_search`, `web_fetch_20250910` is `web_fetch`,
/// either `code_execution_*` is `code_execution` — and answers a request whose
/// two disagree with a 400. That is precisely the failure the strict tier
/// exists to move to compile time, and a shape that only asked for *a* string
/// would let a copied-and-edited config through to it.
///
/// One [`Field`] per canonical name rather than a shared constant, since the
/// name is what makes the row's own `type:` legal on the wire.
const WEB_SEARCH_NAME: Field = Field {
    name: "name",
    shape: FieldShape::Choice(&["web_search"]),
};

/// See [`WEB_SEARCH_NAME`].
const WEB_FETCH_NAME: Field = Field {
    name: "name",
    shape: FieldShape::Choice(&["web_fetch"]),
};

/// See [`WEB_SEARCH_NAME`]. Both dated code-execution revisions carry it.
const CODE_EXECUTION_NAME: Field = Field {
    name: "name",
    shape: FieldShape::Choice(&["code_execution"]),
};

const CACHE_CONTROL: ObjectShape = ObjectShape {
    fields: &[Field {
        name: "type",
        shape: FieldShape::Choice(&["ephemeral"]),
    }],
    required: &["type"],
    exclusive: &[],
};

const WEB_SEARCH_SHAPE: ObjectShape = ObjectShape {
    fields: &[
        WEB_SEARCH_NAME,
        Field {
            name: "max_uses",
            shape: FieldShape::Integer(1, i64::MAX),
        },
        Field {
            name: "allowed_domains",
            shape: FieldShape::Strings,
        },
        Field {
            name: "blocked_domains",
            shape: FieldShape::Strings,
        },
        Field {
            name: "user_location",
            shape: FieldShape::Object(&USER_LOCATION),
        },
        Field {
            name: "cache_control",
            shape: FieldShape::Object(&CACHE_CONTROL),
        },
    ],
    required: &["name"],
    // The service refuses a request that both allows and blocks: one list is an
    // allow-list and the other is a deny-list, and a tool carrying both has no
    // answer to "which".
    exclusive: &[["allowed_domains", "blocked_domains"]],
};

const WEB_FETCH_SHAPE: ObjectShape = ObjectShape {
    fields: &[
        WEB_FETCH_NAME,
        Field {
            name: "max_uses",
            shape: FieldShape::Integer(1, i64::MAX),
        },
        Field {
            name: "allowed_domains",
            shape: FieldShape::Strings,
        },
        Field {
            name: "blocked_domains",
            shape: FieldShape::Strings,
        },
        Field {
            name: "citations",
            shape: FieldShape::Object(&CITATIONS),
        },
        Field {
            name: "max_content_tokens",
            shape: FieldShape::Integer(1, i64::MAX),
        },
        Field {
            name: "cache_control",
            shape: FieldShape::Object(&CACHE_CONTROL),
        },
    ],
    required: &["name"],
    exclusive: &[["allowed_domains", "blocked_domains"]],
};

const CODE_EXECUTION_SHAPE: ObjectShape = ObjectShape {
    fields: &[
        CODE_EXECUTION_NAME,
        // `cache_control:` is a property of a **tool definition** on this wire
        // rather than of any one tool, so it is here for the same reason it is
        // on the two above: the request carries it, and a row that left it out
        // would refuse a config the service takes.
        Field {
            name: "cache_control",
            shape: FieldShape::Object(&CACHE_CONTROL),
        },
    ],
    required: &["name"],
    exclusive: &[],
};

/// The Messages wire's server tools (resolved q30's launch scope).
const ANTHROPIC: &[ServerTool] = &[
    ServerTool {
        type_name: "web_search_20250305",
        summary: "search the web and cite what it found",
        shape: &WEB_SEARCH_SHAPE,
    },
    ServerTool {
        type_name: "web_fetch_20250910",
        summary: "fetch one URL's contents",
        shape: &WEB_FETCH_SHAPE,
    },
    ServerTool {
        type_name: "code_execution_20250522",
        summary: "run Python in a sandbox",
        shape: &CODE_EXECUTION_SHAPE,
    },
    ServerTool {
        type_name: "code_execution_20250825",
        summary: "run Python in a sandbox, with the bash and file tools",
        shape: &CODE_EXECUTION_SHAPE,
    },
];

// ---------------------------------------------------------------------------
// OpenAI — the Responses wire
// ---------------------------------------------------------------------------

/// `filters:` on the Responses `web_search` tool.
const WEB_SEARCH_FILTERS: ObjectShape = ObjectShape {
    fields: &[Field {
        name: "allowed_domains",
        shape: FieldShape::Strings,
    }],
    required: &[],
    exclusive: &[],
};

const OPENAI_WEB_SEARCH_SHAPE: ObjectShape = ObjectShape {
    fields: &[
        Field {
            name: "filters",
            shape: FieldShape::Object(&WEB_SEARCH_FILTERS),
        },
        Field {
            name: "search_context_size",
            shape: FieldShape::Choice(&["low", "medium", "high"]),
        },
        Field {
            name: "user_location",
            shape: FieldShape::Object(&USER_LOCATION),
        },
    ],
    required: &[],
    exclusive: &[],
};

const RANKING_OPTIONS: ObjectShape = ObjectShape {
    fields: &[
        Field {
            name: "ranker",
            shape: FieldShape::Text,
        },
        Field {
            name: "score_threshold",
            shape: FieldShape::Number(0.0, 1.0),
        },
    ],
    required: &[],
    exclusive: &[],
};

const FILE_SEARCH_SHAPE: ObjectShape = ObjectShape {
    fields: &[
        Field {
            name: "vector_store_ids",
            shape: FieldShape::Strings,
        },
        // A comparison filter — `{ type: eq, key: kind, value: handbook }` — or
        // a compound one, `{ type: and, filters: [ … ] }`, which holds filters
        // of either sort to any depth. [`FieldShape`] states neither unions nor
        // recursion, and a field is better known-and-uninspected than absent:
        // see [`FieldShape::Opaque`].
        Field {
            name: "filters",
            shape: FieldShape::Opaque,
        },
        Field {
            name: "max_num_results",
            shape: FieldShape::Integer(1, 50),
        },
        Field {
            name: "ranking_options",
            shape: FieldShape::Object(&RANKING_OPTIONS),
        },
    ],
    // The one required field in the launch scope that is not a `name:`: a file
    // search with no store to search is a request the service refuses.
    required: &["vector_store_ids"],
    exclusive: &[],
};

/// `container:` in its object form — a container the service makes for this
/// call, optionally seeded with files.
const CONTAINER: ObjectShape = ObjectShape {
    fields: &[
        Field {
            name: "type",
            shape: FieldShape::Choice(&["auto"]),
        },
        Field {
            name: "file_ids",
            shape: FieldShape::Strings,
        },
    ],
    required: &["type"],
    exclusive: &[],
};

const CODE_INTERPRETER_SHAPE: ObjectShape = ObjectShape {
    fields: &[Field {
        name: "container",
        shape: FieldShape::TextOrObject(&CONTAINER),
    }],
    required: &["container"],
    exclusive: &[],
};

const IMAGE_GENERATION_SHAPE: ObjectShape = ObjectShape {
    fields: &[
        Field {
            name: "background",
            shape: FieldShape::Choice(&["transparent", "opaque", "auto"]),
        },
        // How much of an input image the edit preserves.
        Field {
            name: "input_fidelity",
            shape: FieldShape::Choice(&["high", "low"]),
        },
        Field {
            name: "model",
            shape: FieldShape::Text,
        },
        Field {
            name: "moderation",
            shape: FieldShape::Choice(&["auto", "low"]),
        },
        Field {
            name: "output_compression",
            shape: FieldShape::Integer(0, 100),
        },
        Field {
            name: "output_format",
            shape: FieldShape::Choice(&["png", "webp", "jpeg"]),
        },
        Field {
            name: "partial_images",
            shape: FieldShape::Integer(0, 3),
        },
        Field {
            name: "quality",
            shape: FieldShape::Choice(&["low", "medium", "high", "auto"]),
        },
        Field {
            name: "size",
            shape: FieldShape::Text,
        },
    ],
    required: &[],
    exclusive: &[],
};

/// The Responses wire's built-in tool suite (resolved q30's launch scope).
///
/// Chat Completions carries none of these, which is why an `openai` provider
/// that declares `server_tools:` speaks the Responses API for **all** of its
/// calls (Decision D122).
const OPENAI: &[ServerTool] = &[
    ServerTool {
        type_name: "web_search",
        summary: "search the web and cite what it found",
        shape: &OPENAI_WEB_SEARCH_SHAPE,
    },
    ServerTool {
        type_name: "web_search_preview",
        summary: "the preview spelling of `web_search`",
        shape: &OPENAI_WEB_SEARCH_SHAPE,
    },
    ServerTool {
        type_name: "file_search",
        summary: "search the vector stores this config names",
        shape: &FILE_SEARCH_SHAPE,
    },
    ServerTool {
        type_name: "code_interpreter",
        summary: "run Python in a container",
        shape: &CODE_INTERPRETER_SHAPE,
    },
    ServerTool {
        type_name: "image_generation",
        summary: "generate an image inside the turn",
        shape: &IMAGE_GENERATION_SHAPE,
    },
];

/// The server tools this compiler release knows one kind serves.
///
/// An **empty** slice is a real answer rather than a gap, and it means two
/// different things:
///
/// * `openai_compatible` — a gateway may honour any vocabulary at all, so
///   nothing here could be authoritative and every entry is second-tier
///   (warned, passed through verbatim);
/// * `azure_openai`, `bedrock`, `vertex` — the key is refused outright on these
///   kinds until their wires are taught the shape, so the table is never
///   consulted (see [`ProviderKind::serves_server_tools`]).
#[must_use]
pub const fn known(kind: ProviderKind) -> &'static [ServerTool] {
    match kind {
        ProviderKind::Anthropic => ANTHROPIC,
        ProviderKind::OpenAi => OPENAI,
        ProviderKind::OpenAiCompatible
        | ProviderKind::AzureOpenAi
        | ProviderKind::Bedrock
        | ProviderKind::Vertex => &[],
    }
}

/// The row for one `type:`, if the kind is known to serve it.
#[must_use]
pub fn lookup(kind: ProviderKind, type_name: &str) -> Option<&'static ServerTool> {
    known(kind).iter().find(|tool| tool.type_name == type_name)
}

/// Every `type:` the kind's table names, in table order.
#[must_use]
pub fn type_names(kind: ProviderKind) -> Vec<&'static str> {
    known(kind).iter().map(|tool| tool.type_name).collect()
}

/// The one `name:` the wire pairs with this `type:`, where the row pins one.
///
/// The pinning is [`FieldShape::Choice`] of exactly one string — see
/// [`WEB_SEARCH_NAME`] — so a row that has a `name:` at all *decides* it, and
/// the name a Messages-wire entry reaches the request under is knowable without
/// reading the config the author wrote. That is what lets the compiler see two
/// entries landing in one slot of the `tools` array: `code_execution_20250522`
/// and `code_execution_20250825` are two types with one name, and the API
/// refuses a `tools` array carrying a name twice.
///
/// `None` for a row whose wire addresses its tools some other way — the
/// Responses built-ins carry no `name:` at all — and for a `type:` the table has
/// no row for.
#[must_use]
pub fn canonical_name(kind: ProviderKind, type_name: &str) -> Option<&'static str> {
    match lookup(kind, type_name)?.shape.field("name")? {
        FieldShape::Choice([only]) => Some(only),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The table is only usable if a row's own claims hold: every required
    /// field is a field, every exclusive pair names two of them, and no `type:`
    /// is duplicated — including across the two kinds' tables, since a reader
    /// looking a type up has one name in hand and no kind.
    #[test]
    fn every_row_is_internally_consistent() {
        for kind in ProviderKind::ALL {
            let mut seen = std::collections::BTreeSet::new();
            for tool in known(*kind) {
                assert!(
                    seen.insert(tool.type_name),
                    "`{}` lists `{}` twice",
                    kind.as_str(),
                    tool.type_name
                );
                assert!(
                    !tool.summary.is_empty(),
                    "`{}` says what it does",
                    tool.type_name
                );
                check_object(tool.type_name, tool.shape);
            }
        }
    }

    fn check_object(subject: &str, shape: &ObjectShape) {
        let names = shape.names();
        for required in shape.required {
            assert!(
                names.contains(required),
                "`{subject}` requires `{required}`, which is not one of its fields"
            );
        }
        for [left, right] in shape.exclusive {
            assert!(
                names.contains(left) && names.contains(right),
                "`{subject}`'s exclusive pair names a field it does not have"
            );
            assert!(
                !shape.required.contains(left) && !shape.required.contains(right),
                "`{subject}` requires a field it also excludes"
            );
        }
        for field in shape.fields {
            assert!(
                !field.name.is_empty(),
                "`{subject}` has a field with no name"
            );
            assert_ne!(
                field.name, "type",
                "`{subject}`'s `type:` is the row's own identity, not one of its fields"
            );
            match field.shape {
                FieldShape::Object(nested) | FieldShape::TextOrObject(nested) => {
                    check_nested(&format!("{subject}.{}", field.name), nested);
                }
                FieldShape::Choice(choices) => {
                    assert!(!choices.is_empty(), "`{subject}`'s choice has variants");
                }
                _ => {}
            }
        }
    }

    /// A nested object's own consistency. `type:` **is** an ordinary field
    /// inside one — `user_location: { type: approximate }` — which is why this
    /// is not [`check_object`].
    fn check_nested(subject: &str, shape: &ObjectShape) {
        let names = shape.names();
        for required in shape.required {
            assert!(
                names.contains(required),
                "`{subject}` requires `{required}`, which is not one of its fields"
            );
        }
    }

    /// The kinds the table has rows for are exactly the kinds whose wire this
    /// release teaches *and* whose vendor publishes a suite: the third kind that
    /// accepts the key, `openai_compatible`, is second-tier by construction.
    #[test]
    fn only_the_two_launch_kinds_have_rows() {
        let with_rows: Vec<&str> = ProviderKind::ALL
            .iter()
            .filter(|kind| !known(**kind).is_empty())
            .map(|kind| kind.as_str())
            .collect();
        assert_eq!(with_rows, ["anthropic", "openai"]);
        assert!(
            ProviderKind::OpenAiCompatible.serves_server_tools(),
            "a gateway takes the key and every entry of it is unverifiable"
        );
        assert!(known(ProviderKind::OpenAiCompatible).is_empty());
    }

    /// Every kind with a row admits the key, and every kind that admits the key
    /// is one of the three q30 names.
    #[test]
    fn the_table_and_the_key_row_agree() {
        for kind in ProviderKind::ALL {
            if !known(*kind).is_empty() {
                assert!(
                    kind.serves_server_tools(),
                    "`{}` has server tools in the table and does not take the key",
                    kind.as_str()
                );
            }
            assert_eq!(
                kind.serves_server_tools(),
                kind.keys().contains(&"server_tools"),
                "`{}` disagrees with its own key row about `server_tools:`",
                kind.as_str()
            );
        }
    }

    /// Every Messages-wire row pins its `name:` to the one canonical name the
    /// vendor pairs with that `type:` — a closed choice of exactly one, not
    /// free text (see [`WEB_SEARCH_NAME`]).
    ///
    /// Stated over the table so a row added later has to answer it: the
    /// Messages API 400s on a `type:`/`name:` pair that disagrees, and a shape
    /// that only asked for *a* string would let that through to the first model
    /// call, which is the failure the strict tier exists to move here.
    #[test]
    fn every_messages_wire_tool_pins_its_canonical_name() {
        for tool in known(ProviderKind::Anthropic) {
            assert!(
                tool.shape.required.contains(&"name"),
                "`{}` requires the `name:` the wire requires beside its `type:`",
                tool.type_name
            );
            match tool.shape.field("name") {
                Some(FieldShape::Choice([_])) => {}
                other => panic!(
                    "`{}`'s `name:` is {other:?} rather than the one name the wire pairs with it",
                    tool.type_name
                ),
            }
        }
        assert_eq!(
            lookup(ProviderKind::Anthropic, "web_search_20250305")
                .expect("the launch scope has web search")
                .shape
                .field("name"),
            Some(FieldShape::Choice(&["web_search"]))
        );
        assert_eq!(
            lookup(ProviderKind::Anthropic, "code_execution_20250825")
                .expect("the launch scope has the newer code execution")
                .shape
                .field("name"),
            Some(FieldShape::Choice(&["code_execution"])),
            "both dated revisions carry the same canonical name"
        );
    }

    /// [`canonical_name`] reads the pinning above rather than a second list, so
    /// the two dated code-execution revisions answer one name — which is what
    /// makes a suite declaring both a collision the compiler can see.
    #[test]
    fn the_canonical_name_is_the_one_the_row_pins() {
        assert_eq!(
            canonical_name(ProviderKind::Anthropic, "web_search_20250305"),
            Some("web_search")
        );
        assert_eq!(
            canonical_name(ProviderKind::Anthropic, "code_execution_20250522"),
            canonical_name(ProviderKind::Anthropic, "code_execution_20250825"),
            "two dated revisions of one tool occupy one slot of the `tools` array"
        );
        assert_eq!(
            canonical_name(ProviderKind::Anthropic, "code_execution_20250825"),
            Some("code_execution")
        );
        // A Responses built-in is addressed by its `type:`: no row pins a name,
        // because the wire has no key to pin.
        assert_eq!(canonical_name(ProviderKind::OpenAi, "web_search"), None);
        assert_eq!(
            canonical_name(ProviderKind::Anthropic, "a_tool_this_release_predates"),
            None
        );
    }

    /// A documented parameter of a tabled tool is **in** its row, however little
    /// the row can say about it.
    ///
    /// The two below are the launch scope's awkward cases, and they are here
    /// because leaving a documented field out has a cost the second tier only
    /// softens: `agent-compose validate` warns about a config the service serves
    /// happily, and the published schema — derived from this table by
    /// `tests/schema_conformance.rs` — describes it to an editor as a key nobody
    /// knows. `filters:` is the shape [`FieldShape`] cannot state and
    /// `input_fidelity:` is the one an earlier revision of this table simply
    /// predated.
    #[test]
    fn a_documented_field_is_tabled_even_where_its_interior_is_not() {
        let file_search =
            lookup(ProviderKind::OpenAi, "file_search").expect("the launch scope has file search");
        assert_eq!(
            file_search.shape.field("filters"),
            Some(FieldShape::Opaque),
            "`file_search` takes the vendor's attribute filter, whose interior is a union this \
             vocabulary does not state"
        );
        let image = lookup(ProviderKind::OpenAi, "image_generation")
            .expect("the launch scope has image generation");
        assert_eq!(
            image.shape.field("input_fidelity"),
            Some(FieldShape::Choice(&["high", "low"]))
        );
    }

    #[test]
    fn a_row_is_found_by_the_type_the_wire_takes() {
        let found = lookup(ProviderKind::Anthropic, "web_search_20250305")
            .expect("the launch scope has web search");
        assert_eq!(
            found.shape.field("max_uses"),
            Some(FieldShape::Integer(1, i64::MAX))
        );
        assert!(lookup(ProviderKind::Anthropic, "web_search").is_none());
        assert!(lookup(ProviderKind::OpenAi, "web_search").is_some());
        // The `file_search` of one kind is not the `file_search` of another:
        // the lookup is per kind, and a type the other vendor ships is unknown
        // here rather than borrowed.
        assert!(lookup(ProviderKind::Anthropic, "file_search").is_none());
        assert_eq!(
            type_names(ProviderKind::OpenAi),
            [
                "web_search",
                "web_search_preview",
                "file_search",
                "code_interpreter",
                "image_generation"
            ]
        );
    }
}
