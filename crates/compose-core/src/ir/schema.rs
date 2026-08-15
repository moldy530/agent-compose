//! The schema language in the IR (grammar 3).
//!
//! One difference from the AST: there is no `Invalid` form. A type node the
//! parser could not read is a diagnostic, and the resolver emits no IR when a
//! diagnostic was raised — so every type node in an artifact declares one of the
//! five forms the grammar defines.
//!
//! Constraint keys (`min_length`, `max_items`, `multiple_of`, …) are written as
//! plain values rather than spanned ones. Each is decided in full by the parser,
//! and the checks a later pass runs *over* them — `max_items` required on the
//! array a `map.over` resolves to (Decision D10), a channel with no `max_items`
//! feeding an `outputs:` field (Decision D111) — report against the type node,
//! whose span is here. `default:` keeps its span, because its literal is checked
//! against the very node it sits on (grammar 3.6).

use serde::Serialize;

use crate::ast::common::{Ident, Literal};
use crate::ast::schema::{Number, ScalarKind, StringFormat, Surface};
use crate::diag::{Span, Spanned};

/// A field map: field name to type node, denoting a closed object (grammar 3.1).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct FieldMap {
    /// Which class of surface this map was written at, and so which of
    /// grammar 3.5's and 3.6's opposite rules apply inside it.
    pub surface: Surface,
    /// The declared fields, in declaration order.
    pub fields: Vec<Field>,
    /// The mapping's own span.
    pub span: Span,
}

/// One entry of a [`FieldMap`].
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Field {
    /// The field name.
    pub name: Spanned<Ident>,
    /// Its type.
    #[serde(rename = "type")]
    pub ty: TypeNode,
}

/// A type node: one of the five forms of grammar 3.2, plus the two keys every
/// form accepts.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TypeNode {
    /// `description:` — legal on every form.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<Spanned<String>>,
    /// The mapping's own span.
    pub span: Span,
    /// The form and its keys.
    #[serde(flatten)]
    pub form: TypeForm,
}

/// The five type-node forms (grammar 3.2), tagged by `form` in the artifact.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "form", rename_all = "snake_case")]
pub enum TypeForm {
    /// `type: string | integer | number | boolean` (grammar 3.3).
    Scalar(Scalar),
    /// `enum: [...]` (grammar 3.3, Decision D9).
    Enum(EnumType),
    /// `type: object` (grammar 3.4).
    Object(ObjectType),
    /// `type: array` (grammar 3.5).
    Array(ArrayType),
    /// `discriminator: <field>` (grammar 3.7, Decision D11).
    Union(UnionType),
}

/// A scalar type node with its constraint vocabulary (grammar 3.3).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Scalar {
    /// Which scalar.
    #[serde(rename = "type")]
    pub kind: ScalarKind,
    /// `min_length`, string only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_length: Option<i64>,
    /// `max_length`, string only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_length: Option<i64>,
    /// `pattern`, RE2 syntax, string only. Kept raw: compiling it is a later
    /// pass's job (Decision D12).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,
    /// `format`, string only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<StringFormat>,
    /// `minimum`, numeric only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub minimum: Option<Number>,
    /// `maximum`, numeric only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub maximum: Option<Number>,
    /// `exclusive_minimum`, numeric only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exclusive_minimum: Option<Number>,
    /// `exclusive_maximum`, numeric only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exclusive_maximum: Option<Number>,
    /// `multiple_of`, numeric only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub multiple_of: Option<Number>,
    /// `default:`, input surfaces and channels only (grammar 3.6).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<Spanned<Literal>>,
}

/// An enum type node: unique non-empty string variants (grammar 3.3).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct EnumType {
    /// The variants, in declaration order — which is the order they reach a
    /// structured-output schema in, so it is the author's to choose.
    pub variants: Vec<Spanned<String>>,
    /// `default:`, input surfaces and channels only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<Spanned<Literal>>,
}

/// A nested object type node (grammar 3.4).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ObjectType {
    /// The object's properties.
    pub properties: FieldMap,
    /// Properties that are not required (Decision D7); each must name one the
    /// object declares (Decision D89).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub optional: Vec<Spanned<Ident>>,
    /// `default:`, input surfaces and channels only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<Spanned<Literal>>,
}

/// An array type node (grammar 3.5).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ArrayType {
    /// The element type.
    pub items: Box<TypeNode>,
    /// `max_items` — required inside every result surface and on the array a
    /// `map.over` resolves to (Decision D10).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_items: Option<i64>,
    /// `min_items`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_items: Option<i64>,
    /// `unique_items`, default `false`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unique_items: Option<bool>,
    /// `default:`, input surfaces and channels only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<Spanned<Literal>>,
}

/// A discriminated union type node (grammar 3.7).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct UnionType {
    /// The tag field's name. A `map`'s `route_by` must equal it.
    pub discriminator: Spanned<Ident>,
    /// The variants, in declaration order; at least two.
    pub variants: Vec<UnionVariant>,
}

/// One variant of a [`UnionType`].
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct UnionVariant {
    /// The variant tag.
    pub tag: Spanned<Ident>,
    /// The variant's payload, which never redeclares the discriminator field —
    /// the compiler synthesizes that as a string constant equal to the tag.
    pub fields: FieldMap,
}

impl FieldMap {
    /// The field declared under this name, if the map declares one.
    #[must_use]
    pub fn field(&self, name: &str) -> Option<&Field> {
        self.fields
            .iter()
            .find(|field| field.name.value.as_str() == name)
    }

    /// Whether the map declares no fields (`{}`, grammar 3.1).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }
}
