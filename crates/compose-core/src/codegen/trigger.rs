//! `src/triggers.ts`: the composition's declared trigger table (grammar 13).
//!
//! [`super::serve`] emits the app; this emits what it serves. One entry per
//! declared **`http`** trigger, carrying everything grammar 13.3 makes a
//! property of the route rather than of the flow: its `path:` and `method:`, its
//! `respond:` mode and the sync budget that goes with it, and the CEL that turns
//! one request payload into the flow's inputs, its session identity, and its
//! completion webhook.
//!
//! # Why only `http`
//!
//! The other three trigger types contribute no route.
//!
//! * **`manual`** is not a route at all, and declaring one enables nothing:
//!   every flow is runnable from the CLI whether or not a trigger names it
//!   (Decision D64), so `src/cli.ts` reads the flow registry rather than this
//!   table. A declared `manual` trigger's `session_key:` is its own `"payload.session"`
//!   default, which is `--session`.
//! * **`schedule`** and **`event`** are reserved grammar (grammar 15): parsed,
//!   type-checked and carried into the IR, executed by nothing in v0. Emitting a
//!   route for one would be a route nothing ever calls.
//!
//! # The payload is bound through a declared shape
//!
//! Grammar 13.3 fixes what a request payload holds — `body`, `query`, `headers`,
//! `path`, `method` — and the validator types every trigger expression against
//! exactly that. The emitted shape is the same one, so an expression the
//! validator accepted binds here through the same declaration: `payload.query.x`
//! is a `string` on both sides, and a number in a decoded body is whatever the
//! body says it is.

use crate::ast::trigger::{Respond, TriggerMethod};
use crate::ir::Ir;
use crate::ir::trigger::{HttpTrigger, TriggerKind};

use super::{names, policy};

/// `src/triggers.ts`.
#[must_use]
pub fn module(ir: &Ir) -> super::GeneratedFile {
    let mut contents = super::header(ir, "// ");
    contents.push_str(PREAMBLE);
    contents.push_str(&table(ir));
    super::GeneratedFile {
        path: "src/triggers.ts".to_string(),
        contents,
    }
}

/// The part of the module that is the same in every project.
const PREAMBLE: &str = r#"//
// The composition's declared triggers (grammar 13, PRD 5.11).
//
// `./serve.ts` is the app; this is the table it serves. Only `http` triggers
// appear here: `manual` invocation is a property of the CLI rather than a
// declared route (Decision D64), and `schedule` and `event` are reserved grammar
// that executes as a no-op in v0 (grammar 15).

import * as runtime from "./runtime.ts";

/**
 * The payload shape grammar 13.3 fixes, as the emitted evaluator reads it.
 *
 * The same declaration the validator typed every trigger expression against, so
 * an expression it accepted binds here through the same types.
 */
const httpPayload: runtime.Shape = {
  properties: {
    body: { properties: {}, rest: "any" },
    query: { properties: {}, rest: "string" },
    headers: { properties: {}, rest: "string" },
    path: "string",
    method: "string",
  },
};

/** One declared `http` trigger (grammar 13.3). */
export interface HttpTrigger {
  /** The trigger's own name, which is what a diagnostic and a report call it. */
  readonly name: string;
  /** `flow:` — the execution's entry module. */
  readonly flow: string;
  /** `path:` — the route, defaulting to `/triggers/<name>`. */
  readonly path: string;
  /** `method:` — defaulting to `POST`. */
  readonly method: "GET" | "POST" | "PUT";
  /** `respond:` — defaulting to `async`. */
  readonly respond: "sync" | "async";
  /** `timeout:` — the sync response budget, in milliseconds (`sync` only). */
  readonly timeoutMs?: number;
  /**
   * Whether the app decodes a body for this route.
   *
   * `false` on a `GET`, which has none: `payload.body` is then `{}` — present
   * and readable, so `has(payload.body.goal)` answers `false` instead of
   * erroring (Decision D117).
   */
  readonly readsBody: boolean;
  /** `input:` — the flow inputs this trigger binds from one payload. */
  input(payload: unknown): Record<string, unknown>;
  /** `session_key:` — the session identity for session-scoped stores (grammar 11.3). */
  sessionKey?(payload: unknown): string;
  /** `callback:` — the completion webhook, on an `async` trigger. */
  callback?(payload: unknown): string;
}

/** The roots one trigger expression is evaluated against. */
function roots(payload: unknown): runtime.Roots {
  return { payload: runtime.bind(payload, httpPayload) };
}

/** One trigger expression whose declared result type is `string`. */
function text(source: string, payload: unknown, site: string): string {
  const answer = runtime.toJson(runtime.evaluate(source, roots(payload)));
  if (typeof answer !== "string") {
    throw new runtime.CelError(`${site} answered ${JSON.stringify(answer)} rather than a string`);
  }
  return answer;
}

"#;

/// The table itself.
fn table(ir: &Ir) -> String {
    let mut text = String::from("export const httpTriggers: readonly HttpTrigger[] = [\n");
    for trigger in triggers(ir) {
        let name = trigger.name;
        text.push_str("  {\n");
        text.push_str(&format!("    name: {},\n", names::string(name)));
        text.push_str(&format!("    flow: {},\n", names::string(&trigger.flow)));
        text.push_str(&format!("    path: {},\n", names::string(&trigger.path)));
        text.push_str(&format!("    method: {},\n", names::string(trigger.method)));
        text.push_str(&format!(
            "    respond: {},\n",
            names::string(trigger.respond)
        ));
        if let Some(timeout) = trigger.timeout_ms {
            text.push_str(&format!("    timeoutMs: {timeout},\n"));
        }
        text.push_str(&format!("    readsBody: {},\n", trigger.reads_body));
        if trigger.input.is_empty() {
            text.push_str("    input: () => ({}),\n");
        } else {
            // The roots are bound **once** per request rather than once per
            // binding: `runtime.bind` copies the value it is given, and a body
            // read by three fields would otherwise be copied three times.
            text.push_str(
                "    input: (payload) => {\n      const bound = roots(payload);\n      return {\n",
            );
            for (field, expression) in &trigger.input {
                text.push_str(&format!(
                    "        {}: runtime.toJson(runtime.evaluate({}, bound)),\n",
                    names::string(field),
                    names::string(expression)
                ));
            }
            text.push_str("      };\n    },\n");
        }
        for (key, spelling, expression) in [
            ("sessionKey", "session_key", trigger.session_key.as_ref()),
            ("callback", "callback", trigger.callback.as_ref()),
        ] {
            let Some(expression) = expression else {
                continue;
            };
            text.push_str(&format!(
                "    {key}: (payload) => text({}, payload, {}),\n",
                names::string(expression),
                names::string(&format!("`{spelling}` of the trigger `{name}`"))
            ));
        }
        text.push_str("  },\n");
    }
    text.push_str("];\n");
    text
}

/// One `http` trigger, lowered to what the emitted table says about it.
struct Emitted<'ir> {
    name: &'ir str,
    flow: String,
    path: String,
    method: &'static str,
    respond: &'static str,
    timeout_ms: Option<u64>,
    reads_body: bool,
    input: Vec<(&'ir str, &'ir str)>,
    session_key: Option<&'ir str>,
    callback: Option<&'ir str>,
}

/// The declared `http` triggers, in the IR's canonical order.
fn triggers(ir: &Ir) -> Vec<Emitted<'_>> {
    let Some(section) = ir.triggers.as_ref() else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for trigger in section.entries.values() {
        let TriggerKind::Http(http) = &trigger.kind else {
            continue;
        };
        let name = trigger.name.value.as_str();
        let method = method(http);
        found.push(Emitted {
            name,
            flow: trigger.flow.value.to_string(),
            // Grammar 13.3's default path is the trigger's own name.
            path: http
                .path
                .as_ref()
                .map_or_else(|| format!("/triggers/{name}"), |path| path.value.clone()),
            method,
            respond: match http.respond {
                Some(Respond::Sync) => "sync",
                Some(Respond::Async) | None => "async",
            },
            // Grammar 13.3: `60s` is the default of the *effective sync* budget,
            // and an `async` trigger carries none at all (Decision D81).
            timeout_ms: match http.respond {
                Some(Respond::Sync) => Some(
                    http.timeout
                        .as_ref()
                        .map_or(DEFAULT_SYNC_TIMEOUT_MS, |timeout| {
                            policy::milliseconds(&timeout.value)
                        }),
                ),
                _ => None,
            },
            reads_body: method != "GET",
            input: http
                .input
                .as_ref()
                .map(|bindings| {
                    bindings
                        .entries
                        .iter()
                        .map(|binding| (binding.name.value.as_str(), binding.value.value.as_str()))
                        .collect()
                })
                .unwrap_or_default(),
            session_key: trigger.session_key.as_ref().map(|key| key.value.as_str()),
            callback: http
                .callback
                .as_ref()
                .map(|callback| callback.value.as_str()),
        });
    }
    found
}

/// Grammar 13.3's `60s` default for a sync trigger's response budget.
const DEFAULT_SYNC_TIMEOUT_MS: u64 = 60_000;

const fn method(http: &HttpTrigger) -> &'static str {
    match http.method {
        Some(TriggerMethod::Get) => "GET",
        Some(TriggerMethod::Put) => "PUT",
        Some(TriggerMethod::Post) | None => "POST",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codegen::test_support::ir_of;

    const PROJECT: &str = r#"version: "0.1"

state:
  answer: { type: string, default: "" }

triggers:
  on_request:
    type: http
    flow: flow.ask
    path: /answers
    input:
      question: "payload.body.question"
    session_key: "payload.headers['x-session-id']"
    callback: "payload.body.callback_url"

  quick:
    type: http
    flow: flow.ask
    method: GET
    respond: sync
    timeout: 5s
    input:
      question: "payload.query.q"

  cli:
    type: manual
    flow: flow.ask

flow.ask:
  inputs:
    question: { type: string }
  outputs:
    answer: { type: string }
  nodes:
    reply:
      exec: { command: printf, args: ["an answer"] }
      writes: { stdout: answer }
  edges:
    - { from: start, to: reply }
    - { from: reply, to: end }
"#;

    #[test]
    fn a_project_with_no_triggers_emits_an_empty_table() {
        let emitted = module(&ir_of("version: \"0.1\"\n")).contents;
        assert!(emitted.contains("export const httpTriggers: readonly HttpTrigger[] = [\n];\n"));
    }

    /// Every key grammar 13.3 makes a property of the route, and the defaults
    /// for the ones a trigger left out.
    #[test]
    fn each_http_trigger_carries_its_route_its_mode_and_its_bindings() {
        let emitted = module(&ir_of(PROJECT)).contents;
        assert!(emitted.contains("name: \"on_request\","), "{emitted}");
        assert!(emitted.contains("path: \"/answers\","));
        assert!(emitted.contains("method: \"POST\","), "the default method");
        assert!(emitted.contains("respond: \"async\","), "the default mode");
        assert!(emitted.contains("readsBody: true,"));
        assert!(
            emitted.contains(
                "\"question\": runtime.toJson(runtime.evaluate(\"payload.body.question\", bound)),"
            ),
            "{emitted}"
        );
        assert!(
            emitted.contains("sessionKey: (payload) => text(\"payload.headers['x-session-id']\"")
        );
        assert!(emitted.contains("callback: (payload) => text(\"payload.body.callback_url\""));

        // The sync one: its budget is a number of milliseconds, and a `GET`
        // decodes no body (Decision D117).
        assert!(emitted.contains("respond: \"sync\","));
        assert!(emitted.contains("timeoutMs: 5000,"));
        assert!(emitted.contains("method: \"GET\","));
        assert!(emitted.contains("readsBody: false,"));

        // A `manual` trigger contributes no route (Decision D64).
        assert!(!emitted.contains("\"cli\""), "{emitted}");
    }

    /// A sync trigger that declares no `timeout:` takes grammar 13.3's default.
    #[test]
    fn a_sync_trigger_with_no_timeout_takes_the_declared_default() {
        let source = PROJECT.replace("    timeout: 5s\n", "");
        let emitted = module(&ir_of(&source)).contents;
        assert!(emitted.contains("timeoutMs: 60000,"), "{emitted}");
    }
}
