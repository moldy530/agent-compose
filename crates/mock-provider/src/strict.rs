//! Strict request checking, and the two dialects it complains in.
//!
//! The server's core job is catching bad codegen output, so leniency is a bug:
//! a request that a real provider would refuse must be refused here, and it must
//! be refused *in the provider's own words*, because the client library reading
//! the answer is the real one. What differs between the two surfaces is only the
//! wording — the walk over the body is the same walk — so the checker is one
//! type parameterized by a [`Dialect`].
//!
//! Every complaint carries two things: a `pointer`, which is the harness's
//! uniform dotted address into the body (`messages.1.content.0.type`), and a
//! `message`, which is the provider's dialect. Tests assert on the pointer;
//! generated code sees the message.
//!
//! A checker never stops at the first mistake. A codegen bug usually shows up as
//! several fields at once, and one round trip that names all of them is worth
//! more to whoever is reading the transcript than five that each name one.

use serde_json::{Map, Value};

use crate::control::ValidationFailure;

/// How a surface words its complaints.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Dialect {
    /// Anthropic's Messages API, which validates with pydantic and says so:
    /// `messages.0.content: Field required`.
    Anthropic,
    /// OpenAI's Chat Completions API:
    /// `Unrecognized request argument supplied: temperatur`.
    OpenAi,
}

/// The JSON shapes a request field can be required to have.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Object,
    Array,
    String,
    Integer,
    Boolean,
}

impl Kind {
    /// What this shape is called in an error message.
    fn name(self) -> &'static str {
        match self {
            Self::Object => "object",
            Self::Array => "array",
            Self::String => "string",
            Self::Integer => "integer",
            Self::Boolean => "boolean",
        }
    }
}

/// What a value actually is, for the dialect that says so.
fn found(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(number) if number.is_i64() || number.is_u64() => "integer",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// Join a parent pointer and a member into the uniform dotted address.
pub(crate) fn at(parent: &str, member: impl std::fmt::Display) -> String {
    if parent.is_empty() {
        member.to_string()
    } else {
        format!("{parent}.{member}")
    }
}

/// A strict walk over one request body.
pub(crate) struct Checker {
    dialect: Dialect,
    failures: Vec<ValidationFailure>,
}

impl Checker {
    pub(crate) fn new(dialect: Dialect) -> Self {
        Self {
            dialect,
            failures: Vec::new(),
        }
    }

    /// Everything wrong with the request, in the order it was found.
    pub(crate) fn into_failures(self) -> Vec<ValidationFailure> {
        self.failures
    }

    /// Record a complaint in the surface's own words.
    pub(crate) fn fail(&mut self, pointer: &str, message: impl Into<String>) {
        self.failures
            .push(ValidationFailure::new(pointer, message.into()));
    }

    /// A key the request must carry.
    pub(crate) fn required<'a>(
        &mut self,
        parent: &str,
        object: &'a Map<String, Value>,
        key: &str,
    ) -> Option<&'a Value> {
        let pointer = at(parent, key);
        match object.get(key) {
            Some(Value::Null) | None => {
                let message = match self.dialect {
                    Dialect::Anthropic => format!("{pointer}: Field required"),
                    Dialect::OpenAi => format!("Missing required parameter: '{pointer}'."),
                };
                self.fail(&pointer, message);
                None
            }
            Some(value) => Some(value),
        }
    }

    /// A value that must have this shape.
    pub(crate) fn typed<'a>(
        &mut self,
        pointer: &str,
        value: &'a Value,
        kind: Kind,
    ) -> Option<&'a Value> {
        let matches = match kind {
            Kind::Object => value.is_object(),
            Kind::Array => value.is_array(),
            Kind::String => value.is_string(),
            Kind::Integer => value.is_i64() || value.is_u64(),
            Kind::Boolean => value.is_boolean(),
        };
        if matches {
            return Some(value);
        }
        let message = match self.dialect {
            Dialect::Anthropic => format!("{pointer}: Input should be a valid {}", kind.name()),
            Dialect::OpenAi => format!(
                "Invalid type for '{pointer}': expected {}, but got {} instead.",
                kind.name(),
                found(value)
            ),
        };
        self.fail(pointer, message);
        None
    }

    /// A required key that must be an object.
    pub(crate) fn required_object<'a>(
        &mut self,
        parent: &str,
        object: &'a Map<String, Value>,
        key: &str,
    ) -> Option<&'a Map<String, Value>> {
        let value = self.required(parent, object, key)?;
        self.typed(&at(parent, key), value, Kind::Object)?
            .as_object()
    }

    /// A required key that must be an array.
    pub(crate) fn required_array<'a>(
        &mut self,
        parent: &str,
        object: &'a Map<String, Value>,
        key: &str,
    ) -> Option<&'a Vec<Value>> {
        let value = self.required(parent, object, key)?;
        self.typed(&at(parent, key), value, Kind::Array)?.as_array()
    }

    /// A required key that must be a string.
    pub(crate) fn required_string<'a>(
        &mut self,
        parent: &str,
        object: &'a Map<String, Value>,
        key: &str,
    ) -> Option<&'a str> {
        let value = self.required(parent, object, key)?;
        self.typed(&at(parent, key), value, Kind::String)?.as_str()
    }

    /// An optional key that must have this shape when it is present.
    ///
    /// `null` counts as absent: both APIs accept an explicitly null optional.
    pub(crate) fn optional<'a>(
        &mut self,
        parent: &str,
        object: &'a Map<String, Value>,
        key: &str,
        kind: Kind,
    ) -> Option<&'a Value> {
        match object.get(key) {
            None | Some(Value::Null) => None,
            Some(value) => self.typed(&at(parent, key), value, kind),
        }
    }

    /// The closed key set of an object.
    ///
    /// This is the check that catches the codegen mistake nothing else can: a
    /// key the API does not know is not an ignored setting, it is a request the
    /// provider refuses, and a mock that accepted it would let a graph pass CI
    /// and fail in production.
    pub(crate) fn closed(&mut self, parent: &str, object: &Map<String, Value>, allowed: &[&str]) {
        for key in object.keys() {
            if allowed.contains(&key.as_str()) {
                continue;
            }
            let pointer = at(parent, key);
            let message = match self.dialect {
                Dialect::Anthropic => format!("{pointer}: Extra inputs are not permitted"),
                Dialect::OpenAi if parent.is_empty() => {
                    format!("Unrecognized request argument supplied: {key}")
                }
                Dialect::OpenAi => format!("Unrecognized request argument supplied: {pointer}"),
            };
            self.fail(&pointer, message);
        }
    }

    /// A string that must be one of a closed set of values.
    pub(crate) fn one_of<'a>(
        &mut self,
        pointer: &str,
        value: &'a Value,
        allowed: &[&str],
    ) -> Option<&'a str> {
        let text = self.typed(pointer, value, Kind::String)?.as_str()?;
        if allowed.contains(&text) {
            return Some(text);
        }
        let list = allowed
            .iter()
            .map(|value| format!("'{value}'"))
            .collect::<Vec<_>>()
            .join(", ");
        let message = match self.dialect {
            Dialect::Anthropic => {
                format!("{pointer}: Input should be one of {list}")
            }
            Dialect::OpenAi => {
                format!("Invalid value: '{text}'. Supported values are: {list}.")
            }
        };
        self.fail(pointer, message);
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn object(value: Value) -> Map<String, Value> {
        value.as_object().expect("an object").clone()
    }

    /// Both dialects find the same mistakes at the same addresses, and word them
    /// differently — which is the whole reason the dialect is a parameter.
    #[test]
    fn one_walk_two_dialects() {
        let body = object(json!({ "messages": 7 }));

        let mut anthropic = Checker::new(Dialect::Anthropic);
        anthropic.required_array("", &body, "messages");
        anthropic.required("", &body, "model");
        let anthropic = anthropic.into_failures();

        let mut openai = Checker::new(Dialect::OpenAi);
        openai.required_array("", &body, "messages");
        openai.required("", &body, "model");
        let openai = openai.into_failures();

        let pointers: Vec<&str> = anthropic
            .iter()
            .map(|failure| failure.pointer.as_str())
            .collect();
        assert_eq!(pointers, ["messages", "model"]);
        assert_eq!(
            pointers,
            openai
                .iter()
                .map(|failure| failure.pointer.as_str())
                .collect::<Vec<_>>(),
            "the addresses are the harness's, not the provider's"
        );

        assert_eq!(
            anthropic[0].message,
            "messages: Input should be a valid array"
        );
        assert_eq!(anthropic[1].message, "model: Field required");
        assert_eq!(
            openai[0].message,
            "Invalid type for 'messages': expected array, but got integer instead."
        );
        assert_eq!(openai[1].message, "Missing required parameter: 'model'.");
    }

    /// A checker names every mistake, not the first one.
    #[test]
    fn every_mistake_is_reported() {
        let body = object(json!({ "model": 1, "max_tokens": "many", "nope": true }));
        let mut checker = Checker::new(Dialect::Anthropic);
        checker.required_string("", &body, "model");
        checker.optional("", &body, "max_tokens", Kind::Integer);
        checker.closed("", &body, &["model", "max_tokens"]);
        let failures = checker.into_failures();
        assert_eq!(
            failures
                .iter()
                .map(|failure| failure.pointer.as_str())
                .collect::<Vec<_>>(),
            ["model", "max_tokens", "nope"]
        );
    }

    /// An explicit `null` on an optional key is an absent key, not a type error.
    #[test]
    fn an_explicit_null_optional_is_absent() {
        let body = object(json!({ "stop_sequences": null }));
        let mut checker = Checker::new(Dialect::OpenAi);
        assert!(
            checker
                .optional("", &body, "stop_sequences", Kind::Array)
                .is_none()
        );
        assert!(checker.into_failures().is_empty());
    }

    /// A required key present as `null` is missing: both APIs treat it that way,
    /// and a codegen bug that emits `"model": null` is the case this catches.
    #[test]
    fn a_required_key_that_is_null_is_missing() {
        let body = object(json!({ "model": null }));
        let mut checker = Checker::new(Dialect::Anthropic);
        assert!(checker.required("", &body, "model").is_none());
        assert_eq!(checker.into_failures()[0].message, "model: Field required");
    }

    /// Pointers nest, and the OpenAI dialect's unknown-key wording changes with
    /// depth: the top-level one is the API's own sentence.
    #[test]
    fn unknown_keys_are_addressed_by_depth() {
        let nested = object(json!({ "wrong": 1 }));
        let mut checker = Checker::new(Dialect::OpenAi);
        checker.closed("", &object(json!({ "bad": 1 })), &[]);
        checker.closed("tools.0.function", &nested, &[]);
        let failures = checker.into_failures();
        assert_eq!(
            failures[0].message,
            "Unrecognized request argument supplied: bad"
        );
        assert_eq!(failures[1].pointer, "tools.0.function.wrong");
        assert_eq!(
            failures[1].message,
            "Unrecognized request argument supplied: tools.0.function.wrong"
        );
    }

    /// A closed value set names what was allowed, so the answer says how to fix
    /// the request rather than only that it is wrong.
    #[test]
    fn a_closed_value_set_lists_its_members() {
        let mut checker = Checker::new(Dialect::OpenAi);
        assert!(
            checker
                .one_of("messages.0.role", &json!("wizard"), &["user", "assistant"])
                .is_none()
        );
        assert_eq!(
            checker.into_failures()[0].message,
            "Invalid value: 'wizard'. Supported values are: 'user', 'assistant'."
        );
    }

    /// A number that is not an integer is not an integer.
    #[test]
    fn a_float_is_not_an_integer() {
        let mut checker = Checker::new(Dialect::Anthropic);
        assert!(
            checker
                .typed("max_tokens", &json!(1.5), Kind::Integer)
                .is_none()
        );
        assert!(
            checker
                .typed("max_tokens", &json!(4), Kind::Integer)
                .is_some()
        );
        assert_eq!(checker.into_failures().len(), 1);
    }
}
