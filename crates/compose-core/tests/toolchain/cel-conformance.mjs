// Runs the shared CEL conformance corpus against the evaluator a generated
// project embeds.
//
// CLAUDE.md's validation strategy names CEL semantic drift between the Rust
// validator and the JS runtime as a risk, and one shared corpus run against both
// interpreters as the mitigation. `crates/compose-core/tests/cel_conformance.rs`
// is the Rust runner; this is the other half, and it is deliberately the *same*
// files with no port of anything.
//
// It lives in the shared toolchain fixture rather than beside one suite because
// two suites run it, under the two runtimes PRD §9.18 supports: the acceptance
// suite's `the_generated_cel_evaluator_agrees_with_the_validator_on_the_
// conformance_corpus` under Bun, and gate 15 of
// `crates/compose-core/tests/generated_code_gates.rs` under the Node fallback.
// The verdicts here are an *engine's* — `BigInt`, `RegExp`, number formatting —
// so one runtime answering the corpus would leave the other's readers unchecked.
// It imports only `src/cel.ts`, which has no imports of its own, so neither run
// needs an installed dependency set.
//
// Usage: <bun|node> cel-conformance.mjs <generated project directory> <corpus directory>
// Output: a JSON array of divergences, empty when the two agree.
//
// # Why the corpus is not read with `JSON.parse`
//
// The corpus distinguishes CEL's numeric types the way CEL does — `1` is an
// `int` and `1.0` a `double`, and arithmetic across them is an error rather than
// a coercion. `JSON.parse` cannot: both arrive as the `number` 1, and a runner
// built on it would bind every integer as a double and quietly stop testing the
// int64 cases the corpus exists to pin. So the runner carries a JSON reader that
// keeps the distinction the text makes, exactly as `serde_json` does on the Rust
// side.

import { readdirSync, readFileSync } from "node:fs";
import path from "node:path";
import process from "node:process";
import { pathToFileURL } from "node:url";

const [, , project, corpus] = process.argv;
if (project === undefined || corpus === undefined) {
  throw new Error("usage: cel-conformance.mjs <project directory> <corpus directory>");
}

const cel = await import(pathToFileURL(path.resolve(project, "src/cel.ts")).href);

// ---------------------------------------------------------------------------
// A JSON reader that keeps `1` and `1.0` apart
// ---------------------------------------------------------------------------

function readJson(text) {
  let at = 0;

  const fail = (message) => {
    throw new Error(`${message} at offset ${at}`);
  };
  const skip = () => {
    while (at < text.length && /[\s]/.test(text[at])) at += 1;
  };
  const literal = (word, value) => {
    if (!text.startsWith(word, at)) fail(`expected \`${word}\``);
    at += word.length;
    return value;
  };

  const value = () => {
    skip();
    const character = text[at];
    if (character === "{") {
      at += 1;
      const object = {};
      skip();
      if (text[at] === "}") {
        at += 1;
        return object;
      }
      for (;;) {
        skip();
        const key = value();
        skip();
        if (text[at] !== ":") fail("expected `:`");
        at += 1;
        object[key] = value();
        skip();
        if (text[at] === ",") {
          at += 1;
          continue;
        }
        if (text[at] !== "}") fail("expected `}`");
        at += 1;
        return object;
      }
    }
    if (character === "[") {
      at += 1;
      const items = [];
      skip();
      if (text[at] === "]") {
        at += 1;
        return items;
      }
      for (;;) {
        items.push(value());
        skip();
        if (text[at] === ",") {
          at += 1;
          continue;
        }
        if (text[at] !== "]") fail("expected `]`");
        at += 1;
        return items;
      }
    }
    if (character === '"') {
      at += 1;
      let out = "";
      for (;;) {
        const next = text[at];
        if (next === undefined) fail("unterminated string");
        at += 1;
        if (next === '"') return out;
        if (next !== "\\") {
          out += next;
          continue;
        }
        const escaped = text[at];
        at += 1;
        if (escaped === "u") {
          out += String.fromCharCode(Number.parseInt(text.slice(at, at + 4), 16));
          at += 4;
          continue;
        }
        out += { n: "\n", t: "\t", r: "\r", b: "\b", f: "\f" }[escaped] ?? escaped;
      }
    }
    if (character === "t") return literal("true", true);
    if (character === "f") return literal("false", false);
    if (character === "n") return literal("null", null);

    const start = at;
    if (text[at] === "-") at += 1;
    while (at < text.length && /[0-9]/.test(text[at])) at += 1;
    let integral = true;
    if (text[at] === ".") {
      integral = false;
      at += 1;
      while (at < text.length && /[0-9]/.test(text[at])) at += 1;
    }
    if (text[at] === "e" || text[at] === "E") {
      integral = false;
      at += 1;
      if (text[at] === "+" || text[at] === "-") at += 1;
      while (at < text.length && /[0-9]/.test(text[at])) at += 1;
    }
    const source = text.slice(start, at);
    if (source === "" || source === "-") fail("expected a value");
    // The corpus's own rule: an integer is an `int`, and anything written with a
    // fraction or an exponent is a `double`.
    return integral ? BigInt(source) : Number(source);
  };

  const answer = value();
  skip();
  if (at !== text.length) fail("trailing input");
  return answer;
}

/** One already-typed JSON value, as the CEL value the corpus means by it. */
function toCel(value) {
  if (value === null || typeof value === "boolean" || typeof value === "string") return value;
  if (typeof value === "bigint" || typeof value === "number") return value;
  if (Array.isArray(value)) return value.map(toCel);
  const map = new cel.CelMap();
  for (const [key, entry] of Object.entries(value)) map.set(key, toCel(entry));
  return map;
}

/** A CEL value as the JSON text the corpus would have written for it. */
function spell(value) {
  if (typeof value === "bigint") return value.toString();
  if (value instanceof cel.CelUint) return value.value.toString();
  if (typeof value === "number") {
    // A double is spelled with a fraction so that `1.0` and `1` do not compare
    // equal as text — the whole point of the distinction above.
    return Number.isInteger(value) ? `${value}.0` : String(value);
  }
  if (value === null || typeof value === "boolean") return String(value);
  if (typeof value === "string") return JSON.stringify(value);
  if (value instanceof Uint8Array) return `bytes(${[...value].join(",")})`;
  if (Array.isArray(value)) return `[${value.map(spell).join(",")}]`;
  if (value instanceof cel.CelMap) {
    const entries = [...value.entries.values()].map(
      (entry) => `${spell(entry.key)}:${spell(entry.value)}`,
    );
    return `{${entries.join(",")}}`;
  }
  return String(value);
}

const divergences = [];
for (const file of readdirSync(corpus).filter((name) => name.endsWith(".json"))) {
  for (const testCase of readJson(readFileSync(path.join(corpus, file), "utf8"))) {
    const roots = {};
    for (const [name, value] of Object.entries(testCase.input ?? {})) {
      roots[name] = toCel(value);
    }

    let answer;
    try {
      answer = { ok: cel.evaluate(testCase.expression, roots) };
    } catch (error) {
      answer = { failed: String(error) };
    }

    if (testCase.error === true) {
      if ("ok" in answer) {
        divergences.push({
          name: testCase.name,
          expected: "an evaluation error",
          actual: spell(answer.ok),
        });
      }
      continue;
    }
    const expected = spell(toCel(testCase.result));
    if ("failed" in answer) {
      divergences.push({ name: testCase.name, expected, threw: answer.failed });
      continue;
    }
    const actual = spell(answer.ok);
    if (expected !== actual) {
      divergences.push({ name: testCase.name, expected, actual });
    }
  }
}

process.stdout.write(JSON.stringify(divergences, null, divergences.length > 0 ? 1 : 0));
