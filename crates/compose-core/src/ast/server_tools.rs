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
//! * a config naming a tool **in this table** is validated **strictly** — a
//!   misspelled or mistyped field and a constraint violation are errors with
//!   diagnostics naming the repair (PRD G3);
//! * a config naming anything else is a **warning** that names exactly what
//!   could not be verified, and then travels to the wire verbatim.
//!
//! The table is therefore a convenience that buys diagnostics, never a gate. A
//! kind with **no** row — `openai_compatible`, whose gateway may honour any
//! vocabulary at all — is served by the second tier alone, which is why
//! [`known`] answers an empty slice for it rather than being unimplemented.
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
/// `type:` and which must be the tool's canonical name.
const NAME: Field = Field {
    name: "name",
    shape: FieldShape::Text,
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
        NAME,
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
        NAME,
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
    fields: &[NAME],
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
