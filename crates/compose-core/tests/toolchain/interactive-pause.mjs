// Drives a generated project's terminal answer surface directly, and reports
// what it did (grammar 8.7, PRD 5.11).
//
// The acceptance suite answers pauses through the real `agent-compose run`,
// which is where "the command behaves" is decided. This is the other half, and
// it is here rather than there for one reason: **standard input is the engine's**
// and this project runs on two of them (PRD §9.18). A `data` event's chunking, a
// stream's `end`, the encoding a chunk arrives in and what a `setEncoding` does
// to it are Bun's and Node's separately, and the prompt loop is nothing but a
// reader over exactly those. So the same runner answers under both, and gate 13
// asserts the same observations with the same function this gate does.
//
// Eight sections, and each is a claim a served app cannot make:
//
//   * a pause is **rendered** — the wait id, the flow and node, what the human is
//     shown, the shape their answer has to fit, and the deadline where the node
//     declares one — and one line of JSON answers it;
//   * a line that is not JSON, and one the node's `output:` refuses, are refusals
//     that **re-prompt**: the wait is not consumed, which is the resume route's
//     `400` rule at the other surface;
//   * a blank line is not an answer and re-prompts with no refusal at all;
//   * two pauses that are waiting **together** are asked one at a time and in
//     wait-id order — the order the status route publishes them in, and the one
//     that does not depend on how the scheduler interleaved the instances;
//   * a pause that opens **while a question is on the screen** is asked after it,
//     whatever its id sorts as: the guarantee is over the pauses open when a
//     question is asked, not over every pause a run makes, because the only way
//     to put a latecomer first would be to take back a question somebody is
//     already answering;
//   * an **expiry** takes the question away while it is on the screen: the prompt
//     is withdrawn saying so, and the loop moves on rather than reading an answer
//     into a wait nothing is holding;
//   * standard input **ending** withdraws the whole surface: every pause still
//     waiting becomes the interrupt a run with no surface raises, and so does the
//     next one the run opens;
//   * …and an end that arrives while **no** question is outstanding is noticed
//     before the next one is rendered, rather than after a whole prompt block has
//     been printed under a surface that is already gone.
//
// `src/cli.ts` and `src/runtime.ts` are compiler constants, byte-identical in
// every project this release builds, so driving them directly is driving what
// every project runs.
//
// Usage: node interactive-pause.mjs <generated project directory>
// Output: one JSON object of observations; the expectations live in the Rust
// test that reads it (`generated_code_gates.rs`).

import path from "node:path";
import process from "node:process";
import { PassThrough } from "node:stream";
import { pathToFileURL } from "node:url";

const [, , project] = process.argv;
if (project === undefined) {
  throw new Error("usage: node interactive-pause.mjs <generated project directory>");
}

const runtime = await import(pathToFileURL(path.resolve(project, "src/runtime.ts")).href);
const cli = await import(pathToFileURL(path.resolve(project, "src/cli.ts")).href);

const settle = () => new Promise((resolve) => setTimeout(resolve, 0));

/** Wait until something has happened, rather than for a number of milliseconds. */
async function until(ready, ms = 5_000) {
  const deadline = Date.now() + ms;
  while (!ready() && Date.now() < deadline) {
    await new Promise((resolve) => setTimeout(resolve, 5));
  }
}

/**
 * A sign-off's `output:`, with one required property and one that is not — so
 * the sketch a prompt renders has both a variant list and the `?` that marks a
 * property the schema does not require.
 */
const SCHEMA = {
  type: "object",
  properties: { decision: { enum: ["approve", "reject"] }, note: { type: "string" } },
  required: ["decision"],
  additionalProperties: false,
};

function parse(payload) {
  if (typeof payload !== "object" || payload === null) {
    throw new Error("the answer is an object");
  }
  if (!["approve", "reject"].includes(payload.decision)) {
    throw new Error("`decision` is `approve` or `reject`");
  }
  const answer = { decision: payload.decision };
  if (payload.note !== undefined) {
    if (typeof payload.note !== "string") throw new Error("`note` is a string");
    answer.note = payload.note;
  }
  return answer;
}

/** A `human` node's descriptor, as a compiled `graph.ts` emits one. */
function descriptor(fields = {}) {
  return { flow: "flow.sign_off", node: "sign", schema: SCHEMA, parse, ...fields };
}

/** A `NodeView` at `instancePath`, which is what a wait id is derived from. */
function viewAt(instancePath, id) {
  const run = {
    ...runtime.emptyRun(),
    execution: { id, session_key: "" },
    path: instancePath,
  };
  return { state: {}, run, roots: { input: runtime.bind({}, { properties: {} }) } };
}

/** What a parked task ended as: `"pending"`, `"resolved"`, or the error's name. */
function outcomeOf(promise) {
  const held = { state: "pending", value: undefined };
  promise.then(
    (value) => {
      held.state = "resolved";
      held.value = value;
    },
    (error) => {
      held.state = error?.name ?? "Error";
      held.value = error?.message ?? "";
    },
  );
  return held;
}

/** Park one pause and answer the handle its outcome is read through. */
function park(execution, instancePath, shown = { question: "ship it?" }, fields = {}) {
  const context = { execution: { id: execution, session_key: "" }, node: "sign" };
  return outcomeOf(
    runtime.runHuman(descriptor(fields), shown, context, viewAt(instancePath, execution)),
  );
}

/** Everything one prompt loop writes, in the order it wrote it. */
function collector() {
  const written = [];
  return {
    write(text) {
      written.push(String(text));
    },
    text: () => written.join(""),
  };
}

/** The prompt loop over a stream this section feeds, and the handle to end it. */
function driving(execution) {
  const input = new PassThrough();
  const output = collector();
  let stop;
  const finished = new Promise((resolve) => {
    stop = resolve;
  });
  return {
    input,
    output,
    stop,
    prompting: cli.answerPauses(execution, finished, { input, output }),
  };
}

const observed = {};

// A pause is rendered and one line of JSON answers it. The pause reaches the
// node's answer exactly as a resume does — `human.settled` is `"resumed"` — and
// what was typed is nowhere in the record (docs/trace.md §11).
{
  const execution = "exec_prompted";
  runtime.openHumanWaits(execution, true);
  const held = park(execution, ["review", "0"], { question: "ship it?", draft: "a draft" });
  await settle();
  const driver = driving(execution);
  driver.input.write('{"decision":"approve","note":"looks right"}\n');
  await until(() => held.state !== "pending");
  driver.stop();
  await driver.prompting;

  observed.prompted = {
    said: driver.output.text(),
    settled: held.state,
    output: held.value?.output,
    pause: held.value?.human?.settled,
    published: runtime.humanWaits(execution).map((wait) => wait.id),
  };
  runtime.releaseHumanWaits(execution);
}

// A line that is not JSON and an answer the `output:` refuses are both refusals
// that leave the wait waiting, and a blank line is not an answer at all. Fed in
// one go, because the loop reads them in order and the third answer is what says
// the two before it consumed nothing.
{
  const execution = "exec_refused";
  runtime.openHumanWaits(execution, true);
  const held = park(execution, ["review", "0"]);
  await settle();
  const driver = driving(execution);
  driver.input.write("this is not JSON\n");
  driver.input.write('{"decision":"maybe"}\n');
  driver.input.write("   \n");
  driver.input.write('{"decision":"reject"}\n');
  await until(() => held.state !== "pending");
  driver.stop();
  await driver.prompting;

  const said = driver.output.text();
  observed.refused = {
    said,
    // The prompt is repeated after every refusal, so the count is what says the
    // wait was still there to be answered each time: one for the question and
    // one after each of the three lines that did not answer it.
    prompts: said.split("answer `review/0/sign/0` with one line of JSON: ").length - 1,
    settled: held.state,
    output: held.value?.output,
  };
  runtime.releaseHumanWaits(execution);
}

// Two pauses waiting at once, asked one at a time and in wait-id order — which
// is the order the status route publishes them in, and not the order they
// opened.
{
  const execution = "exec_two";
  runtime.openHumanWaits(execution, true);
  // Opened in the reverse of the order they must be asked in, so the ordering
  // under test is the ids' rather than the board's insertion order.
  const second = park(execution, ["fan", "0", "1"], { question: "b" });
  const first = park(execution, ["fan", "0", "0"], { question: "a" });
  await settle();
  const driver = driving(execution);
  driver.input.write('{"decision":"approve"}\n');
  await until(() => first.state !== "pending");
  const afterTheFirst = {
    first: first.state,
    second: second.state,
    published: runtime.humanWaits(execution).map((wait) => wait.id),
  };
  driver.input.write('{"decision":"reject"}\n');
  await until(() => second.state !== "pending");
  driver.stop();
  await driver.prompting;

  const said = driver.output.text();
  observed.two = {
    asked: [...said.matchAll(/pause `([^`]+)`/g)].map((match) => match[1]),
    after_the_first: afterTheFirst,
    outputs: [first.value?.output, second.value?.output],
  };
  runtime.releaseHumanWaits(execution);
}

// …and the other half of that guarantee, which is the half it does *not* make: a
// pause that opens while a question is on the screen is asked after it, even
// where its id sorts first. What is ordered is the set of pauses open when a
// question is asked; putting a latecomer first would mean withdrawing a question
// somebody may already be typing an answer to. The section above cannot see this
// — its two pauses park together — and no composition can ask for the timing, so
// it is staged: the second id is parked, waited for on the screen, and only then
// is the first id's pause opened under it.
{
  const execution = "exec_later";
  runtime.openHumanWaits(execution, true);
  const asked_first = park(execution, ["fan", "0", "1"], { question: "b" });
  await settle();
  const driver = driving(execution);
  await until(() => driver.output.text().includes("pause `fan/0/1/sign/0`"));

  // Open while that question is on the screen, and with the id that sorts
  // before it.
  const opened_later = park(execution, ["fan", "0", "0"], { question: "a" });
  await settle();
  driver.input.write('{"decision":"approve"}\n');
  await until(() => asked_first.state !== "pending");
  await until(() => driver.output.text().includes("pause `fan/0/0/sign/0`"));
  driver.input.write('{"decision":"reject"}\n');
  await until(() => opened_later.state !== "pending");
  driver.stop();
  await driver.prompting;

  const said = driver.output.text();
  observed.later = {
    asked: [...said.matchAll(/pause `([^`]+)`/g)].map((match) => match[1]),
    // The first line answered the question that was on the screen, which is the
    // same claim from the answers' side: a loop that had re-ordered on the
    // latecomer would have given `approve` to `fan/0/0/sign/0`.
    outputs: [asked_first.value?.output, opened_later.value?.output],
  };
  runtime.releaseHumanWaits(execution);
}

// A budget that runs out while the question is on the screen takes the question
// away: the prompt is withdrawn with the sentence a late answer would have been
// refused with, and the loop asks the next pause rather than reading a line into
// a wait nothing is holding. Nothing is ever written to this stream before the
// expiry, which is what makes the withdrawal the thing that ended the prompt.
{
  const execution = "exec_expired";
  runtime.openHumanWaits(execution, true);
  const expiring = park(execution, ["review", "0"], { question: "ship it?" }, {
    timeoutMs: 50,
    onTimeout: "escalate",
  });
  await settle();
  const driver = driving(execution);
  await until(() => expiring.state !== "pending");
  await settle();

  // …and the loop really did move on: a pause opened after the withdrawal is
  // asked and answered on the same stream.
  const next = park(execution, ["review", "1"]);
  await until(() => driver.output.text().includes("pause `review/1/sign/0`"));
  driver.input.write('{"decision":"approve"}\n');
  await until(() => next.state !== "pending");
  driver.stop();
  await driver.prompting;

  observed.expired = {
    said: driver.output.text(),
    settled: expiring.state,
    expires_at_was_shown: /\n {2}expires: \d{4}-\d{2}-\d{2}T/.test(driver.output.text()),
    next: next.state,
    next_output: next.value?.output,
  };
  runtime.releaseHumanWaits(execution);
}

// Standard input ending is the answer surface going away, which is the same
// shape as a run that never had one: every pause still waiting becomes a
// `HumanInterrupt`, and so does the next one the run opens.
{
  const execution = "exec_input_ended";
  runtime.openHumanWaits(execution, true);
  const held = park(execution, ["review", "0"]);
  await settle();
  const driver = driving(execution);
  await until(() => driver.output.text().includes("pause `review/0/sign/0`"));
  driver.input.end();
  await until(() => held.state !== "pending");
  await driver.prompting;

  const later = park(execution, ["review", "1"]);
  await settle();

  observed.input_ended = {
    said: driver.output.text(),
    settled: held.state,
    message: held.value,
    later: later.state,
    published: runtime.humanWaits(execution).map((wait) => wait.id),
  };
  driver.stop();
  runtime.releaseHumanWaits(execution);
}

// An end that arrives **between** questions is noticed before the next one is
// rendered. The loop learns about the end from the stream's own event, and
// parked with no pause open it has no read outstanding for that event to answer
// — so without a check before the block is written, the next pause to open would
// be printed in full (the id, what the human is shown, the schema, the deadline)
// and withdrawn on the line under it: a question that was never askable.
//
// Staged rather than raced: the `end` listener the loop installed runs before
// this one, because it was added first, so awaiting this one is awaiting the
// loop having seen the end.
{
  const execution = "exec_ended_between";
  runtime.openHumanWaits(execution, true);
  const answered = park(execution, ["review", "0"]);
  await settle();
  const driver = driving(execution);
  driver.input.write('{"decision":"approve"}\n');
  await until(() => answered.state !== "pending");

  const closed = new Promise((resolve) => driver.input.once("end", resolve));
  driver.input.end();
  await closed;

  const later = park(execution, ["review", "1"]);
  await until(() => later.state !== "pending");
  await driver.prompting;

  observed.ended_between = {
    said: driver.output.text(),
    answered: answered.state,
    later: later.state,
  };
  driver.stop();
  runtime.releaseHumanWaits(execution);
}

// A stream whose last line carries no newline is still an answer: a script that
// wrote `printf '{"decision":"approve"}'` answered, and reading only up to a
// newline would hang on it until the stream closed under it.
{
  const execution = "exec_unterminated";
  runtime.openHumanWaits(execution, true);
  const held = park(execution, ["review", "0"]);
  await settle();
  const driver = driving(execution);
  driver.input.end('{"decision":"approve"}');
  await until(() => held.state !== "pending");
  driver.stop();
  await driver.prompting;

  observed.unterminated = {
    settled: held.state,
    output: held.value?.output,
  };
  runtime.releaseHumanWaits(execution);
}

process.stdout.write(JSON.stringify(observed));
