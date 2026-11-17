//! The conversions and `join` the Rust column supplies where the pinned crate
//! has no overload (PRD resolved q85, Decision D168).
//!
//! PRD resolved q85 put CEL's four standard conversions — `string(x)`,
//! `int(x)`, `double(x)`, `bool(x)` — and the strings extension's
//! `list.join(separator)` on the authored surface, held to the conformance
//! corpus like everything else on it. The corpus is only evidence if both of its
//! columns can run every case, and `cel` 0.14.3 carries the conversions for
//! *some* argument types and `join` not at all:
//!
//! | call | the crate's own overloads | supplied here |
//! |---|---|---|
//! | `string(x)` | `string`, `int`, `uint`, `double`, `bytes` | `bool`, `null`, a list, a map |
//! | `int(x)` | `int`, `uint`, `double`, `string` | `bool` |
//! | `double(x)` | `double`, `int`, `uint`, `string` | `bool` |
//! | `bool(x)` | none | every argument |
//! | `l.join(s)` | none | every argument |
//!
//! The crate consults its own typed overloads first and the context's registry
//! only when none of them matches the argument
//! ([`super::evaluation_context`] says why that also means a built-in cannot be
//! replaced), so each function here is reached exactly for the argument types
//! the right-hand column names, and for the spellings the surface refuses — a
//! receiver on a conversion, a global `join` — which it refuses again rather
//! than answer: an evaluator that runs what `validate` refuses is drift pointing
//! the other way.
//!
//! What a conversion of a list or a map writes is its **canonical JSON** (PRD
//! resolved q85): object keys in code-point order, no whitespace, strings
//! escaped as `JSON.stringify` escapes them, an `int` without a fraction, and a
//! `double` the way `string()` writes one on its own — the crate's shortest
//! round-trip digits, never an exponent. A value JSON has no spelling for — a
//! `NaN` or an infinity, `bytes`, a map keyed by anything but strings — is an
//! evaluation error rather than a guess. The emitted evaluator writes the same
//! bytes (`src/codegen/js/cel.ts`), and `conversions.json` in the corpus is what
//! holds the two to it. Because the text is a function of the value alone, a
//! replayed execution that converts the same value converts it to the same
//! string (PRD resolved q29).

use std::sync::Arc;

use ::cel::extractors::{Arguments, This};
use ::cel::objects::{Key, Value};
use ::cel::{ExecutionError, FunctionContext};

/// Register every function this module supplies.
pub(super) fn register(context: &mut ::cel::Context<'_>) {
    context.add_function("string", string);
    context.add_function("int", int);
    context.add_function("double", double);
    context.add_function("bool", bool);
    context.add_function("join", join);
}

/// The one spelling the specification gives a conversion: a global call of one
/// argument. A receiver, or a second argument, has no overload to call.
fn global(ftx: &FunctionContext) -> Result<(), ExecutionError> {
    if ftx.this.is_some() || ftx.args.len() != 1 {
        return Err(ftx.error(format!(
            "`{name}` is a global call of one argument — `{name}(x)`",
            name = ftx.name
        )));
    }
    Ok(())
}

/// `string(x)` for what the crate does not convert: a bool, `null`, a list or a
/// map.
fn string(ftx: &FunctionContext, This(value): This<Value>) -> Result<Value, ExecutionError> {
    global(ftx)?;
    let text = match &value {
        Value::Bool(flag) => flag.to_string(),
        Value::Null => "null".to_string(),
        Value::String(text) => text.to_string(),
        Value::Int(number) => number.to_string(),
        Value::UInt(number) => number.to_string(),
        Value::Float(number) => number.to_string(),
        Value::Bytes(bytes) => String::from_utf8_lossy(bytes).into_owned(),
        Value::List(_) | Value::Map(_) => {
            let mut text = String::new();
            canonical(&value, &mut text).map_err(|why| ftx.error(why))?;
            text
        }
        other => return Err(ftx.error(format!("cannot convert {} to a string", kind(other)))),
    };
    Ok(Value::String(Arc::new(text)))
}

/// `int(x)` for a bool: `1` and `0`. Every other argument type the
/// specification converts is the crate's own overload.
fn int(ftx: &FunctionContext, This(value): This<Value>) -> Result<Value, ExecutionError> {
    global(ftx)?;
    match value {
        Value::Bool(flag) => Ok(Value::Int(i64::from(flag))),
        other => Err(ftx.error(format!("cannot convert {} to an int", kind(&other)))),
    }
}

/// `double(x)` for a bool: `1.0` and `0.0`.
fn double(ftx: &FunctionContext, This(value): This<Value>) -> Result<Value, ExecutionError> {
    global(ftx)?;
    match value {
        Value::Bool(flag) => Ok(Value::Float(if flag { 1.0 } else { 0.0 })),
        other => Err(ftx.error(format!("cannot convert {} to a double", kind(&other)))),
    }
}

/// `bool(x)`: a bool is itself, and a string is `true` or `false` exactly —
/// case-sensitive, and nothing else. A number has no overload: `bool(1)` is a
/// mismatch at validate and an error here, never a guess at what `1` means.
fn bool(ftx: &FunctionContext, This(value): This<Value>) -> Result<Value, ExecutionError> {
    global(ftx)?;
    match value {
        Value::Bool(flag) => Ok(Value::Bool(flag)),
        Value::String(text) => match text.as_str() {
            "true" => Ok(Value::Bool(true)),
            "false" => Ok(Value::Bool(false)),
            other => Err(ftx.error(format!(
                "`{other}` is not a bool: the string a bool converts from is `true` or `false`"
            ))),
        },
        other => Err(ftx.error(format!("cannot convert {} to a bool", kind(&other)))),
    }
}

/// `list.join(separator)` and `list.join()`, the strings extension's: every
/// element a string, joined by the separator or by nothing.
fn join(
    ftx: &FunctionContext,
    This(list): This<Value>,
    Arguments(arguments): Arguments,
) -> Result<Value, ExecutionError> {
    if ftx.this.is_none() || arguments.len() > 1 {
        return Err(ftx.error(
            "`join` is written on the list it joins, with one separator or none — `names.join(', ')`",
        ));
    }
    let Value::List(items) = list else {
        return Err(ftx.error(format!(
            "`join` joins a list of strings, and its receiver is {}",
            kind(&list)
        )));
    };
    let separator = match arguments.first() {
        None => String::new(),
        Some(Value::String(separator)) => separator.to_string(),
        Some(other) => {
            return Err(ftx.error(format!(
                "`join`'s separator is a string, and this one is {}",
                kind(other)
            )));
        }
    };
    let mut parts = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let Value::String(text) = item else {
            return Err(ftx.error(format!(
                "`join` joins a list of strings, and element {index} is {}",
                kind(item)
            )));
        };
        parts.push(text.as_str());
    }
    Ok(Value::String(Arc::new(parts.join(&separator))))
}

/// The canonical JSON of one value, appended to `out`.
fn canonical(value: &Value, out: &mut String) -> Result<(), String> {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(flag) => out.push_str(if *flag { "true" } else { "false" }),
        Value::Int(number) => out.push_str(&number.to_string()),
        Value::UInt(number) => out.push_str(&number.to_string()),
        Value::Float(number) => {
            if !number.is_finite() {
                return Err(format!(
                    "{number} has no JSON spelling, so a list or a map holding it has no canonical JSON"
                ));
            }
            out.push_str(&number.to_string());
        }
        Value::String(text) => quote(text, out),
        Value::List(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                canonical(item, out)?;
            }
            out.push(']');
        }
        Value::Map(map) => {
            let mut entries = Vec::with_capacity(map.map.len());
            for (key, entry) in map.map.iter() {
                let Key::String(key) = key else {
                    return Err(
                        "a map keyed by anything but strings has no canonical JSON".to_string()
                    );
                };
                entries.push((key.as_str(), entry));
            }
            // A Rust `str` orders by byte, which for UTF-8 is code-point
            // order — the order the emitted evaluator's `compareStrings`
            // sorts by, rather than JavaScript's UTF-16 code units.
            entries.sort_unstable_by(|left, right| left.0.cmp(right.0));
            out.push('{');
            for (index, (key, entry)) in entries.into_iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                quote(key, out);
                out.push(':');
                canonical(entry, out)?;
            }
            out.push('}');
        }
        other => {
            return Err(format!(
                "{} has no JSON spelling, so a list or a map holding it has no canonical JSON",
                kind(other)
            ));
        }
    }
    Ok(())
}

/// One string as a JSON string literal, escaped as `JSON.stringify` escapes it:
/// the quote, the backslash and the C0 controls, the five with a short form
/// written short and the rest as lowercase `\u00XX`.
fn quote(text: &str, out: &mut String) {
    out.push('"');
    for character in text.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            control if u32::from(control) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", u32::from(control)));
            }
            other => out.push(other),
        }
    }
    out.push('"');
}

/// A value's kind, as a message names it.
fn kind(value: &Value) -> &'static str {
    match value {
        Value::List(_) => "a list",
        Value::Map(_) => "a map",
        Value::Function(..) => "a function",
        Value::Int(_) => "an int",
        Value::UInt(_) => "a uint",
        Value::Float(_) => "a double",
        Value::String(_) => "a string",
        Value::Bytes(_) => "bytes",
        Value::Bool(_) => "a bool",
        Value::Null => "null",
        _ => "a value of another type",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(source: &str) -> String {
        let program = ::cel::Program::compile(source).expect("the expression parses");
        match program
            .execute(&super::super::evaluation_context())
            .expect("the expression evaluates")
        {
            Value::String(text) => text.to_string(),
            other => panic!("`{source}` is not a string: {other:?}"),
        }
    }

    /// Canonical JSON is a function of the value: two maps holding the same
    /// entries write the same bytes whatever order they were built in, which is
    /// what a `HashMap` underneath would otherwise leave to chance.
    #[test]
    fn canonical_json_is_a_function_of_the_value() {
        let one = text("string({'b': 1, 'a': [true, null], 'c': {'z': 'x', 'y': 2.5}})");
        let other = text("string({'c': {'y': 2.5, 'z': 'x'}, 'a': [true, null], 'b': 1})");
        assert_eq!(one, r#"{"a":[true,null],"b":1,"c":{"y":2.5,"z":"x"}}"#);
        assert_eq!(one, other);
    }

    /// The escape table is `JSON.stringify`'s, the C0 controls included.
    #[test]
    fn a_string_inside_canonical_json_is_escaped_as_json_stringify_escapes_it() {
        assert_eq!(
            text(r#"string(['"\\/\b\f\n\r\t\x01\x1f\x7f é'])"#),
            "[\"\\\"\\\\/\\b\\f\\n\\r\\t\\u0001\\u001f\u{7f} é\"]"
        );
    }
}
