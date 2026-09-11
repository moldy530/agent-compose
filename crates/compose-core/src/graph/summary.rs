//! Grammar 3's type language, written as one line a reader can take in.
//!
//! A detail pane has a column of them, so what matters is that the line is
//! short and that nothing an author declared has been dropped. The shape comes
//! first — `string`, `array<string>`, `union on kind` — and every constraint
//! the node carries follows it in one parenthesis, in the order grammar 3
//! declares them. A type with no constraints is one word.
//!
//! This is a **derived** spelling and not a second schema language: nothing
//! reads it back, and the fields it summarizes are the IR's own. Where the
//! reader needs the whole declaration, the composition is where it lives.

use std::fmt::Write as _;

use crate::ast::common::Literal;
use crate::ast::schema::Number;
use crate::ir::schema::{FieldMap, TypeForm, TypeNode};

use super::document::FieldView;

/// One field map, as a column of lines.
pub(super) fn fields(map: &FieldMap) -> Vec<FieldView> {
    map.fields
        .iter()
        .map(|field| FieldView {
            name: field.name.value.as_str().to_string(),
            ty: type_of(&field.ty),
            description: field.ty.description.as_ref().map(|text| text.value.clone()),
        })
        .collect()
}

/// One type node, as one line.
pub(super) fn type_of(node: &TypeNode) -> String {
    let mut text = match &node.form {
        TypeForm::Scalar(scalar) => scalar.kind.as_str().to_string(),
        TypeForm::Enum(held) => format!(
            "enum [{}]",
            held.variants
                .iter()
                .map(|variant| variant.value.clone())
                .collect::<Vec<_>>()
                .join(", ")
        ),
        TypeForm::Object(object) => format!(
            "object {{ {} }}",
            object
                .properties
                .fields
                .iter()
                .map(|field| field.name.value.as_str().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ),
        TypeForm::Array(array) => format!("array<{}>", type_of(&array.items)),
        TypeForm::Union(union) => format!(
            "union on {} [{}]",
            union.discriminator.value,
            union
                .variants
                .iter()
                .map(|variant| variant.tag.value.as_str().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    };
    let constraints = constraints(node);
    if !constraints.is_empty() {
        let _ = write!(text, " ({})", constraints.join(", "));
    }
    text
}

/// Every constraint key this node declares, in grammar 3's own order.
fn constraints(node: &TypeNode) -> Vec<String> {
    let mut held: Vec<String> = Vec::new();
    match &node.form {
        TypeForm::Scalar(scalar) => {
            if let Some(bound) = scalar.min_length {
                held.push(format!("min_length {bound}"));
            }
            if let Some(bound) = scalar.max_length {
                held.push(format!("max_length {bound}"));
            }
            if let Some(pattern) = &scalar.pattern {
                held.push(format!("pattern {pattern}"));
            }
            if let Some(format) = scalar.format {
                held.push(format!("format {}", format.as_str()));
            }
            for (name, bound) in [
                ("minimum", scalar.minimum.as_ref()),
                ("maximum", scalar.maximum.as_ref()),
                ("exclusive_minimum", scalar.exclusive_minimum.as_ref()),
                ("exclusive_maximum", scalar.exclusive_maximum.as_ref()),
                ("multiple_of", scalar.multiple_of.as_ref()),
            ] {
                if let Some(bound) = bound {
                    held.push(format!("{name} {}", number(bound)));
                }
            }
        }
        TypeForm::Array(array) => {
            if let Some(bound) = array.max_items {
                held.push(format!("max_items {bound}"));
            }
            if let Some(bound) = array.min_items {
                held.push(format!("min_items {bound}"));
            }
            if array.unique_items == Some(true) {
                held.push("unique_items".to_string());
            }
        }
        TypeForm::Object(object) if !object.optional.is_empty() => {
            held.push(format!(
                "optional {}",
                object
                    .optional
                    .iter()
                    .map(|name| name.value.as_str().to_string())
                    .collect::<Vec<_>>()
                    .join(" ")
            ));
        }
        _ => {}
    }
    if let Some(default) = crate::check::model::default_of(node) {
        held.push(format!("default {}", literal(&default.value)));
    }
    held
}

/// A numeric constraint, in the spelling the author wrote it in.
fn number(value: &Number) -> String {
    match value {
        Number::Int(held) => held.to_string(),
        Number::Float(held) => held.to_string(),
    }
}

/// A literal, short enough to sit inside a parenthesis.
///
/// Sequences and mappings are summarized rather than printed: a `default:` that
/// is a whole object belongs in the composition, and a line that unrolled one
/// would stop being a line.
fn literal(value: &Literal) -> String {
    match value {
        Literal::Null => "null".to_string(),
        Literal::Bool(held) => held.to_string(),
        Literal::Int(held) => held.to_string(),
        Literal::Float(held) => held.to_string(),
        Literal::String(held) => format!("{held:?}"),
        Literal::Sequence(held) => format!("[{} items]", held.len()),
        Literal::Mapping(held) => format!("{{{} keys}}", held.len()),
    }
}

/// A literal as JSON — a `settings:` value, which is an open object the
/// provider plugin's own schema decides (Decision D40).
pub(super) fn literal_json(value: &Literal) -> serde_json::Value {
    match value {
        Literal::Null => serde_json::Value::Null,
        Literal::Bool(held) => serde_json::Value::Bool(*held),
        Literal::Int(held) => serde_json::Value::from(*held),
        // The IR refuses an infinity and a NaN where a float is read, so the
        // fallback is unreachable; it is written rather than unwrapped because
        // a picture of a graph is not the place to panic.
        Literal::Float(held) => serde_json::Number::from_f64(*held)
            .map_or(serde_json::Value::Null, serde_json::Value::Number),
        Literal::String(held) => serde_json::Value::String(held.clone()),
        Literal::Sequence(held) => serde_json::Value::Array(
            held.iter()
                .map(|entry| literal_json(&entry.value))
                .collect(),
        ),
        Literal::Mapping(held) => serde_json::Value::Object(
            held.iter()
                .map(|entry| (entry.key.value.clone(), literal_json(&entry.value.value)))
                .collect(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resolve::resolve;

    /// The forms a reader meets most, written the way the pane prints them.
    #[test]
    fn every_form_summarizes_to_one_line() {
        let project = tempdir("summary");
        std::fs::write(
            project.join("main.yml"),
            r#"version: "0.1"

state:
  counted:
    type: array
    max_items: 20
    unique_items: true
    items: { type: string, min_length: 1 }

provider.p:
  kind: anthropic
  api_key: ${K}

model.m:
  provider: provider.p
  id: some-model

agent.a:
  model: model.m
  prompt: p
  input:
    verdict: { enum: [ok, no], default: ok }
    nested: { type: object, properties: { a: { type: string } }, optional: [a] }
  output:
    items:
      type: array
      max_items: 5
      items:
        discriminator: kind
        variants:
          one: { a: { type: string } }
          two: { b: { type: integer, minimum: 0 } }

flow.f:
  outputs: {}
  nodes:
    only: { agent: agent.a }
  edges:
    - { from: start, to: only }
    - { from: only, to: end }
"#,
        )
        .expect("the fixture is writable");
        let resolution = resolve(project.join("main.yml"));
        assert!(
            resolution.diagnostics.is_empty(),
            "{:#?}",
            resolution.diagnostics
        );
        let ir = resolution.ir.expect("a clean resolution has an artifact");

        let channel = &ir
            .state
            .as_ref()
            .expect("a state section")
            .entries
            .get("counted")
            .expect("the channel")
            .ty;
        assert_eq!(
            type_of(channel),
            "array<string (min_length 1)> (max_items 20, unique_items)"
        );

        let crate::ir::definition::DefinitionBody::Agent(agent) =
            &ir.definition("agent.a").expect("the agent").body
        else {
            panic!("not an agent");
        };
        let input = fields(agent.input.as_ref().expect("declared inputs"));
        assert_eq!(input[0].ty, "enum [ok, no] (default \"ok\")");
        assert_eq!(input[1].ty, "object { a } (optional a)");
        let output = fields(&agent.output);
        assert_eq!(
            output[0].ty,
            "array<union on kind [one, two]> (max_items 5)"
        );

        let _ = std::fs::remove_dir_all(&project);
    }

    fn tempdir(purpose: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "agent-compose-graph-{purpose}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("a scratch directory");
        path
    }
}
