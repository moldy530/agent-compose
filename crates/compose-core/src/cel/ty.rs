//! The types an expression is checked against (grammar 4.1).
//!
//! This is the CEL side of the schema language: every declared type node
//! (grammar 3) maps onto one [`Type`], and every expression the front-end walks
//! is given one. It is deliberately **coarser** than the schema language —
//! `min_length`, `max_items`, and the rest do not appear here, because an
//! expression is typed by its *result* (grammar 8.0) and a result carries no
//! declaration to compare constraints against. Where two *declarations* meet
//! with no expression between them, the comparison is the schema-level
//! `satisfies` relation of `check::model`, which does read them.
//!
//! [`Type::Dyn`] is the answer wherever the front-end cannot know: a decoded
//! JSON body, a value behind a construct the walk does not model. Nothing is
//! ever reported against `Dyn`, which is what keeps the pass conservative in
//! the one direction it must be — it asks for a declaration, it never invents
//! one.

use std::fmt;
use std::sync::Arc;

use crate::ast::schema::ScalarKind;

/// The type of a CEL value.
#[derive(Clone, Debug, PartialEq)]
pub enum Type {
    /// Unknown: no check is possible, and none is made.
    Dyn,
    /// `bool`.
    Bool,
    /// `int` — the type of a schema `integer` and of an integer literal.
    Int,
    /// `uint` — reachable only from a `3u` literal; no schema produces one.
    Uint,
    /// `double` — the type of a schema `number`.
    Double,
    /// `string` with no known value set.
    String,
    /// A string literal, which is a `string` whose one value is known. Carried
    /// separately so that a comparison against an `enum` field and a binding
    /// into one can both be decided (grammar 4.1, 7.3.1).
    StringLiteral(Arc<str>),
    /// A schema `enum`: a string drawn from a closed variant set.
    Enum(Arc<Vec<String>>),
    /// `bytes`.
    Bytes,
    /// `null`.
    Null,
    /// A homogeneous list.
    List(Arc<Type>),
    /// A string-keyed map of homogeneous values — what a decoded payload
    /// member is, and what no declaration in this grammar produces.
    Map(Arc<Type>),
    /// A closed object: a field map, a nested `type: object`, or one of the
    /// synthesized roots (grammar 3.1, 3.4, 4.1).
    Object(Arc<ObjectShape>),
    /// A discriminated union (grammar 3.7).
    Union(Arc<UnionShape>),
}

/// A closed object's members, and what kind of thing it is.
#[derive(Clone, Debug, PartialEq)]
pub struct ObjectShape {
    /// What this object is, for the message an unknown member draws.
    pub origin: Origin,
    /// Its members, in declaration order.
    pub properties: Vec<Property>,
}

/// One member of an [`ObjectShape`].
#[derive(Clone, Debug, PartialEq)]
pub struct Property {
    /// The member's name.
    pub name: String,
    /// Its type.
    pub ty: Type,
    /// Whether a value may legally omit it (grammar 3.4, 11.4). Presence is a
    /// runtime property either way — reading an absent value fails the
    /// execution (Decision D110) — so this changes no static answer here; it is
    /// carried because the schema-level relation needs it.
    pub optional: bool,
}

/// What an [`ObjectShape`] is, which is what decides how an unknown member is
/// reported.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Origin {
    /// A declared field map or nested object: an agent's `output`, a flow's
    /// `inputs`, a variant's payload. The string names it in a diagnostic.
    Declared(String),
    /// The `state` root, whose members are the declared channels — so an
    /// unknown member is an undefined state channel (grammar 10.3).
    State,
    /// The `execution` root (grammar 4.1).
    Execution,
    /// A node id, which is a root only as the `<node>.output` of an edge guard
    /// or a `map.over` (grammar 4.1, Decision D42).
    Node {
        /// The node's flow-local id.
        id: String,
        /// Whether the node has an output at all: a `map` node has none of its
        /// own (grammar 8.6 rule 9).
        has_output: bool,
    },
    /// A trigger payload, named by its trigger type (grammar 13).
    Payload(&'static str),
}

/// A discriminated union's shape (grammar 3.7).
#[derive(Clone, Debug, PartialEq)]
pub struct UnionShape {
    /// The discriminator field's name.
    pub discriminator: String,
    /// The variants, in declaration order.
    pub variants: Vec<UnionVariant>,
}

/// One variant of a [`UnionShape`].
#[derive(Clone, Debug, PartialEq)]
pub struct UnionVariant {
    /// The variant tag.
    pub tag: String,
    /// The variant's payload, without the synthesized discriminator field.
    pub properties: Vec<Property>,
}

impl ObjectShape {
    /// The member with this name, if the object declares one.
    #[must_use]
    pub fn property(&self, name: &str) -> Option<&Property> {
        self.properties.iter().find(|p| p.name == name)
    }

    /// Every member name, in declaration order.
    #[must_use]
    pub fn names(&self) -> Vec<&str> {
        self.properties.iter().map(|p| p.name.as_str()).collect()
    }
}

impl UnionShape {
    /// The variant with this tag, if the union declares one.
    #[must_use]
    pub fn variant(&self, tag: &str) -> Option<&UnionVariant> {
        self.variants.iter().find(|v| v.tag == tag)
    }

    /// Every variant tag, in declaration order.
    #[must_use]
    pub fn tags(&self) -> Vec<&str> {
        self.variants.iter().map(|v| v.tag.as_str()).collect()
    }
}

impl Type {
    /// A closed object of these properties.
    #[must_use]
    pub fn object(origin: Origin, properties: Vec<Property>) -> Self {
        Self::Object(Arc::new(ObjectShape { origin, properties }))
    }

    /// A list of this element type.
    #[must_use]
    pub fn list(items: Self) -> Self {
        Self::List(Arc::new(items))
    }

    /// The scalar a schema `type:` keyword names (grammar 3.3).
    #[must_use]
    pub const fn scalar(kind: ScalarKind) -> Self {
        match kind {
            ScalarKind::String => Self::String,
            ScalarKind::Integer => Self::Int,
            ScalarKind::Number => Self::Double,
            ScalarKind::Boolean => Self::Bool,
        }
    }

    /// Whether this type is a string in the CEL sense — a plain string, a
    /// literal, or an `enum` (grammar 3.3: `enum` implies `type: string`).
    #[must_use]
    pub const fn is_stringy(&self) -> bool {
        matches!(
            self,
            Self::String | Self::StringLiteral(_) | Self::Enum(_) | Self::Dyn
        )
    }

    /// Whether this type is one of CEL's numeric types.
    #[must_use]
    pub const fn is_numeric(&self) -> bool {
        matches!(self, Self::Int | Self::Uint | Self::Double)
    }

    /// Whether a value of this type may be used where `target` is expected.
    ///
    /// This is the *expression* half of grammar 8.0's typing: an explicit
    /// binding is typed by its result, so what is compared is the result's
    /// shape against the destination's, with no constraint anywhere in it.
    /// [`Dyn`](Self::Dyn) on either side answers `true`, which is what keeps an
    /// unmodelled expression from being reported as a mismatch.
    #[must_use]
    pub fn assignable_to(&self, target: &Self) -> bool {
        match (self, target) {
            (Self::Dyn, _) | (_, Self::Dyn) => true,
            (Self::Bool, Self::Bool) | (Self::Bytes, Self::Bytes) | (Self::Null, Self::Null) => {
                true
            }
            // CEL's integers land in a schema `number` as JSON's do; a `double`
            // never lands in an `integer`.
            (Self::Int | Self::Uint, Self::Int | Self::Uint | Self::Double)
            | (Self::Double, Self::Double) => true,
            // A known string value is accepted by an `enum` that declares it.
            (Self::StringLiteral(value), Self::Enum(variants)) => {
                variants.iter().any(|variant| variant.as_str() == &**value)
            }
            (Self::Enum(source), Self::Enum(target)) => {
                source.iter().all(|variant| target.contains(variant))
            }
            // Every enum member is a string, so an enum lands in a string; the
            // reverse needs a value the expression does not promise.
            (Self::String | Self::StringLiteral(_) | Self::Enum(_), Self::String)
            | (Self::StringLiteral(_), Self::StringLiteral(_)) => true,
            (Self::List(source), Self::List(target)) => source.assignable_to(target),
            (Self::Map(source), Self::Map(target)) => source.assignable_to(target),
            // A map is what a decoded payload member is: its shape is unknown,
            // so an object destination can neither be proved nor refused.
            (Self::Map(_), Self::Object(_)) | (Self::Object(_), Self::Map(_)) => true,
            (Self::Object(source), Self::Object(target)) => {
                target.properties.iter().all(|want| {
                    source.property(&want.name).is_some_and(|have| {
                        have.ty.assignable_to(&want.ty) && (want.optional || !have.optional)
                    })
                }) && source
                    .properties
                    .iter()
                    .all(|have| target.property(&have.name).is_some())
            }
            (Self::Union(source), Self::Union(target)) => {
                source.discriminator == target.discriminator
                    && source.variants.iter().all(|variant| {
                        target.variant(&variant.tag).is_some_and(|want| {
                            variant.properties.iter().all(|have| {
                                want.properties
                                    .iter()
                                    .any(|w| w.name == have.name && have.ty.assignable_to(&w.ty))
                            }) && want.properties.len() == variant.properties.len()
                        })
                    })
            }
            _ => false,
        }
    }
}

impl Type {
    /// The type's name with no article, as it reads inside "a list of …".
    #[must_use]
    pub fn bare(&self) -> String {
        match self {
            Self::Dyn => "unknown type".to_string(),
            Self::Bool => "booleans".to_string(),
            Self::Int => "integers".to_string(),
            Self::Uint => "unsigned integers".to_string(),
            Self::Double => "numbers".to_string(),
            Self::String | Self::StringLiteral(_) => "strings".to_string(),
            Self::Enum(variants) => format!("one of [{}]", variants.join(", ")),
            Self::Bytes => "bytes".to_string(),
            Self::Null => "nulls".to_string(),
            Self::List(items) => format!("lists of {}", items.bare()),
            Self::Map(values) => format!("maps of {}", values.bare()),
            Self::Object(_) => "objects".to_string(),
            Self::Union(shape) => format!("`{}` unions", shape.discriminator),
        }
    }
}

/// A type renders as the noun phrase a diagnostic wants: "expected a string,
/// found a list of integers".
impl fmt::Display for Type {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Dyn => f.write_str("a value of unknown type"),
            Self::Bool => f.write_str("a boolean"),
            Self::Int => f.write_str("an integer"),
            Self::Uint => f.write_str("an unsigned integer"),
            Self::Double => f.write_str("a number"),
            Self::String => f.write_str("a string"),
            Self::StringLiteral(value) => write!(f, "the string `{value}`"),
            Self::Enum(variants) => write!(f, "one of [{}]", variants.join(", ")),
            Self::Bytes => f.write_str("bytes"),
            Self::Null => f.write_str("null"),
            Self::List(items) => write!(f, "a list of {}", items.bare()),
            Self::Map(values) => write!(f, "a map of {}", values.bare()),
            Self::Object(shape) => match &shape.origin {
                Origin::Declared(what) => write!(f, "{what}"),
                Origin::State => f.write_str("the state object"),
                Origin::Execution => f.write_str("the execution object"),
                Origin::Node { id, .. } => write!(f, "the node `{id}`"),
                Origin::Payload(kind) => write!(f, "the `{kind}` trigger payload"),
            },
            Self::Union(shape) => write!(
                f,
                "a `{}` union of [{}]",
                shape.discriminator,
                shape.tags().join(", ")
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enumeration(variants: &[&str]) -> Type {
        Type::Enum(Arc::new(
            variants.iter().map(|v| (*v).to_string()).collect(),
        ))
    }

    #[test]
    fn dyn_is_assignable_in_both_directions() {
        assert!(Type::Dyn.assignable_to(&Type::Bool));
        assert!(Type::Bool.assignable_to(&Type::Dyn));
    }

    #[test]
    fn integers_widen_to_numbers_but_numbers_do_not_narrow() {
        assert!(Type::Int.assignable_to(&Type::Double));
        assert!(!Type::Double.assignable_to(&Type::Int));
    }

    #[test]
    fn a_literal_lands_in_an_enum_that_declares_it() {
        let target = enumeration(&["approve", "revise"]);
        assert!(Type::StringLiteral("approve".into()).assignable_to(&target));
        assert!(!Type::StringLiteral("aprove".into()).assignable_to(&target));
        // A string of unknown value promises nothing about the variant set.
        assert!(!Type::String.assignable_to(&target));
        // Every variant is a string, so the other direction always holds.
        assert!(target.assignable_to(&Type::String));
    }

    #[test]
    fn an_enum_lands_in_an_enum_that_covers_it() {
        assert!(enumeration(&["a"]).assignable_to(&enumeration(&["a", "b"])));
        assert!(!enumeration(&["a", "c"]).assignable_to(&enumeration(&["a", "b"])));
    }

    #[test]
    fn objects_are_compared_structurally_and_stay_closed() {
        let property = |name: &str, ty: Type, optional: bool| Property {
            name: name.to_string(),
            ty,
            optional,
        };
        let target = Type::object(
            Origin::Declared("the target".into()),
            vec![property("a", Type::String, false)],
        );
        let exact = Type::object(
            Origin::Declared("exact".into()),
            vec![property("a", Type::String, false)],
        );
        let extra = Type::object(
            Origin::Declared("extra".into()),
            vec![
                property("a", Type::String, false),
                property("b", Type::Int, false),
            ],
        );
        let wrong = Type::object(
            Origin::Declared("wrong".into()),
            vec![property("a", Type::Int, false)],
        );
        assert!(exact.assignable_to(&target));
        // Objects are closed (Decision D8), so a value carrying `b` is not a
        // legal value of a target that declares only `a`.
        assert!(!extra.assignable_to(&target));
        assert!(!wrong.assignable_to(&target));
        // A required destination is not satisfied by an optional source.
        let optional = Type::object(
            Origin::Declared("optional".into()),
            vec![property("a", Type::String, true)],
        );
        assert!(!optional.assignable_to(&target));
        assert!(exact.assignable_to(&optional));
    }

    #[test]
    fn types_render_as_noun_phrases() {
        assert_eq!(Type::list(Type::String).to_string(), "a list of strings");
        assert_eq!(
            enumeration(&["low", "high"]).to_string(),
            "one of [low, high]"
        );
        assert_eq!(
            Type::StringLiteral("revise".into()).to_string(),
            "the string `revise`"
        );
    }
}
