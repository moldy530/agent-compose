//! The schema language (grammar 3): field maps and type nodes.
//!
//! Schemas are the load-bearing construct — they make routing decidable, fan-out
//! bounded, edges serializable, and store ops checkable — so the AST keeps them
//! fully typed rather than as opaque YAML. Which surface a schema was written
//! at is recorded too ([`Surface`]), because several rules key off it: results
//! forbid `default:` and require `max_items` on arrays, inputs allow both.

use crate::diag::{Span, Spanned};

use super::common::{Ident, Literal};

/// The declaration surface a schema was written at (grammar 3.6, 3.9).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Surface {
    /// `agent.input`, `tool.input`, `flow.inputs`, `human.input`: `default:` is
    /// legal on every form but a union, `max_items` is optional.
    Input,
    /// `agent.output`, `tool.output`, `flow.outputs`, `human.output`, store
    /// schemas, and inline `exec:`/`http:` node outputs: `default:` is illegal
    /// and every array declares `max_items`.
    Result,
    /// A `state:` channel: `default:` is the channel's initial value,
    /// `max_items` is optional (grammar 10.1).
    Channel,
}

impl Surface {
    /// Whether a type node may carry `default:` here (grammar 3.6,
    /// Decision D77).
    ///
    /// Everywhere but a result surface. The form matters too — a discriminated
    /// union never takes one, at any surface, because a union default would
    /// have to manufacture a discriminator tag — but that is a property of the
    /// form rather than of the place it was written, so it lives with the union
    /// rather than here.
    #[must_use]
    pub const fn allows_default(self) -> bool {
        !matches!(self, Self::Result)
    }

    /// Whether arrays must declare `max_items` at this surface.
    #[must_use]
    pub const fn requires_max_items(self) -> bool {
        matches!(self, Self::Result)
    }
}

/// A field map: field name to type node, denoting a closed object (grammar 3.1).
#[derive(Clone, Debug, PartialEq)]
pub struct FieldMap {
    /// The declared fields, in declaration order.
    pub fields: Vec<Field>,
    /// The surface this map was written at.
    pub surface: Surface,
    /// The mapping's own span.
    pub span: Span,
}

impl FieldMap {
    /// The field with this name, if declared.
    #[must_use]
    pub fn field(&self, name: &str) -> Option<&Field> {
        self.fields.iter().find(|f| f.name.value.as_str() == name)
    }

    /// Whether the map declares no fields (`{}`).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }
}

/// One entry of a [`FieldMap`].
#[derive(Clone, Debug, PartialEq)]
pub struct Field {
    /// The field name.
    pub name: Spanned<Ident>,
    /// Its type.
    pub ty: TypeNode,
}

/// A type node: exactly one discriminating key selects the form (grammar 3.2).
#[derive(Clone, Debug, PartialEq)]
pub struct TypeNode {
    /// The form and its keys.
    pub form: TypeForm,
    /// `description:`, legal on every form.
    pub description: Option<Spanned<String>>,
    /// The mapping's own span.
    pub span: Span,
}

/// The five type-node forms (grammar 3.2).
///
/// A scalar is the largest variant because it carries the constraint
/// vocabulary; boxing it would put an allocation behind every schema field for
/// the sake of the rarer forms, which is the wrong trade for a tree that is
/// built once and then read.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq)]
pub enum TypeForm {
    /// `type: string | integer | number | boolean`.
    Scalar(ScalarType),
    /// `enum: [...]`.
    Enum(EnumType),
    /// `type: object`.
    Object(ObjectType),
    /// `type: array`.
    Array(ArrayType),
    /// `discriminator: <field>`.
    Union(UnionType),
    /// The node declared no discriminating key, or one the grammar does not
    /// define. A diagnostic was reported; the node is kept so the surrounding
    /// construct still parses.
    Invalid,
}

/// The four scalar types (grammar 3.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScalarKind {
    /// `string`
    String,
    /// `integer`
    Integer,
    /// `number`
    Number,
    /// `boolean`
    Boolean,
}

impl ScalarKind {
    /// The keyword that names this type.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::String => "string",
            Self::Integer => "integer",
            Self::Number => "number",
            Self::Boolean => "boolean",
        }
    }

    /// Whether string constraints (`min_length`, `pattern`, `format`, …) apply.
    #[must_use]
    pub const fn is_string(self) -> bool {
        matches!(self, Self::String)
    }

    /// Whether numeric constraints (`minimum`, `multiple_of`, …) apply.
    #[must_use]
    pub const fn is_numeric(self) -> bool {
        matches!(self, Self::Integer | Self::Number)
    }
}

/// The closed `format:` vocabulary (grammar 3.3, Decision D12).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StringFormat {
    /// `date-time`
    DateTime,
    /// `date`
    Date,
    /// `time`
    Time,
    /// `duration`
    Duration,
    /// `email`
    Email,
    /// `uri`
    Uri,
    /// `uuid`
    Uuid,
    /// `hostname`
    Hostname,
    /// `ipv4`
    Ipv4,
    /// `ipv6`
    Ipv6,
}

impl StringFormat {
    /// Every format, in the order grammar 3.3 lists them.
    pub const ALL: &'static [Self] = &[
        Self::DateTime,
        Self::Date,
        Self::Time,
        Self::Duration,
        Self::Email,
        Self::Uri,
        Self::Uuid,
        Self::Hostname,
        Self::Ipv4,
        Self::Ipv6,
    ];

    /// The keyword that names this format.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::DateTime => "date-time",
            Self::Date => "date",
            Self::Time => "time",
            Self::Duration => "duration",
            Self::Email => "email",
            Self::Uri => "uri",
            Self::Uuid => "uuid",
            Self::Hostname => "hostname",
            Self::Ipv4 => "ipv4",
            Self::Ipv6 => "ipv6",
        }
    }
}

/// A number literal, as written.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Number {
    /// An integer literal.
    Int(i64),
    /// A floating-point literal.
    Float(f64),
}

/// A scalar type node with its constraints (grammar 3.3).
///
/// `Debug` prints only the constraints that were declared: a scalar carries
/// nine optional keys, and printing all of them buries the one or two an author
/// actually wrote under a wall of `None` in every AST snapshot.
#[derive(Clone, PartialEq)]
pub struct ScalarType {
    /// Which scalar.
    pub kind: Spanned<ScalarKind>,
    /// `min_length`, string only.
    pub min_length: Option<Spanned<i64>>,
    /// `max_length`, string only.
    pub max_length: Option<Spanned<i64>>,
    /// `pattern`, RE2 syntax, string only. Kept raw: compiling it is the
    /// validator's job.
    pub pattern: Option<Spanned<String>>,
    /// `format`, string only.
    pub format: Option<Spanned<StringFormat>>,
    /// `minimum`, numeric only.
    pub minimum: Option<Spanned<Number>>,
    /// `maximum`, numeric only.
    pub maximum: Option<Spanned<Number>>,
    /// `exclusive_minimum`, numeric only.
    pub exclusive_minimum: Option<Spanned<Number>>,
    /// `exclusive_maximum`, numeric only.
    pub exclusive_maximum: Option<Spanned<Number>>,
    /// `multiple_of`, numeric only, greater than zero.
    pub multiple_of: Option<Spanned<Number>>,
    /// `default:`, input surfaces and channels only.
    pub default: Option<Spanned<Literal>>,
}

/// An enum type node: unique non-empty string variants (grammar 3.3, D9).
#[derive(Clone, Debug, PartialEq)]
pub struct EnumType {
    /// The variants, in declaration order.
    pub variants: Vec<Spanned<String>>,
    /// `default:`, input surfaces and channels only.
    pub default: Option<Spanned<Literal>>,
}

/// A nested object type node (grammar 3.4).
#[derive(Clone, Debug, PartialEq)]
pub struct ObjectType {
    /// The object's properties.
    pub properties: FieldMap,
    /// Properties that are not required (grammar 3.4, Decision D7).
    pub optional: Vec<Spanned<Ident>>,
    /// `default:`, input surfaces and channels only; the literal supplies every
    /// required property (grammar 3.6, Decision D77).
    pub default: Option<Spanned<Literal>>,
}

/// An array type node (grammar 3.5).
///
/// `Debug` prints only the keys that were declared, as [`ScalarType`] does.
#[derive(Clone, PartialEq)]
pub struct ArrayType {
    /// The element type.
    pub items: Box<TypeNode>,
    /// `max_items`, required in result surfaces (Decision D10).
    pub max_items: Option<Spanned<i64>>,
    /// `min_items`.
    pub min_items: Option<Spanned<i64>>,
    /// `unique_items`, default `false`.
    pub unique_items: Option<Spanned<bool>>,
    /// `default:`, input surfaces and channels only; the literal is an array of
    /// the `items:` type (grammar 3.6, Decision D77).
    pub default: Option<Spanned<Literal>>,
}

/// A discriminated union type node (grammar 3.7, Decision D11).
#[derive(Clone, Debug, PartialEq)]
pub struct UnionType {
    /// The tag field's name. A `map`'s `route_by` must equal it.
    pub discriminator: Spanned<Ident>,
    /// The variants, in declaration order; at least two.
    pub variants: Vec<UnionVariant>,
}

/// One variant of a [`UnionType`].
#[derive(Clone, Debug, PartialEq)]
pub struct UnionVariant {
    /// The variant tag.
    pub tag: Spanned<Ident>,
    /// The variant's payload. It may not redeclare the discriminator field —
    /// the compiler synthesizes that as a string constant.
    pub fields: FieldMap,
}

/// Add the fields that were declared, skipping the ones that were not.
fn declared<'a, T: std::fmt::Debug + 'a>(
    builder: &mut std::fmt::DebugStruct<'_, '_>,
    fields: impl IntoIterator<Item = (&'static str, Option<&'a T>)>,
) {
    for (label, value) in fields {
        if let Some(value) = value {
            builder.field(label, value);
        }
    }
}

impl std::fmt::Debug for ScalarType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut builder = f.debug_struct("ScalarType");
        builder.field("kind", &self.kind);
        declared(
            &mut builder,
            [
                ("min_length", self.min_length.as_ref()),
                ("max_length", self.max_length.as_ref()),
            ],
        );
        declared(&mut builder, [("pattern", self.pattern.as_ref())]);
        declared(&mut builder, [("format", self.format.as_ref())]);
        declared(
            &mut builder,
            [
                ("minimum", self.minimum.as_ref()),
                ("maximum", self.maximum.as_ref()),
                ("exclusive_minimum", self.exclusive_minimum.as_ref()),
                ("exclusive_maximum", self.exclusive_maximum.as_ref()),
                ("multiple_of", self.multiple_of.as_ref()),
            ],
        );
        declared(&mut builder, [("default", self.default.as_ref())]);
        builder.finish()
    }
}

impl std::fmt::Debug for ArrayType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut builder = f.debug_struct("ArrayType");
        builder.field("items", &self.items);
        declared(
            &mut builder,
            [
                ("max_items", self.max_items.as_ref()),
                ("min_items", self.min_items.as_ref()),
            ],
        );
        declared(&mut builder, [("unique_items", self.unique_items.as_ref())]);
        declared(&mut builder, [("default", self.default.as_ref())]);
        builder.finish()
    }
}
