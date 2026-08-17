// Reports which channel names LangGraph cannot hold, and which names every
// JavaScript object already carries.
//
// `compose_core::codegen::diagnostics` refuses a `build` whose `state:` declares
// a channel named after an own property of `Object.prototype`, because the
// emitted `channels` object is read with a plain property lookup: `StateGraph`
// asks `this.channels[key] !== undefined` over a `this.channels` that is `{}`,
// so `constructor` reads as already installed and the next line calls
// `.equals(…)` on `Object.prototype.constructor`.
//
// A refusal is an over-refusal until something says the runtime really cannot
// take the name, so this runner is the evidence rather than the claim: it builds
// a two-channel state model per name and reports whether constructing it throws.
// A LangGraph release that stopped caring turns the Rust-side gate red, which is
// the signal to drop the refusal rather than to keep it out of habit.
//
// Usage: node inherited-channel-names.mjs '["constructor", …]'
// Output: { "inherited": [own property names of Object.prototype],
//           "rejected": [the argument names `new StateGraph` throws on] }

import process from "node:process";

import { Annotation, StateGraph } from "@langchain/langgraph";

const [, , namesJson] = process.argv;
if (namesJson === undefined) {
  throw new Error("usage: node inherited-channel-names.mjs <names as JSON>");
}
const names = JSON.parse(namesJson);

const channel = () =>
  Annotation({
    reducer: (_left, right) => right,
    default: () => "",
  });

const rejected = names.filter((name) => {
  // A computed key defines an own property even for `__proto__`, which an
  // ordinary `{__proto__: …}` literal would spend on setting the prototype —
  // and an own property is what the emitted object literal has.
  const channels = { [name]: channel(), ordinary: channel() };
  try {
    new StateGraph(Annotation.Root(channels));
    return false;
  } catch {
    return true;
  }
});

process.stdout.write(
  JSON.stringify({
    inherited: Object.getOwnPropertyNames(Object.prototype).sort(),
    rejected,
  }),
);
