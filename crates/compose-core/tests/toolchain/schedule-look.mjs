// A schedule's look at its newest claim, driven directly against a journal
// whose `lastFired` refuses on cue (`docs/durability.md` §3.11, PRD resolved
// q78 ruling b, Decision D162).
//
// The look is the one read that finds a tick claimed for an execution that
// never began, and it reads only the **newest** `fired` row. So the scheduler
// may claim no tick while the look cannot reach the journal: claimed, the
// tick's own row would become the newest, the stranded claim below it would
// never be read again, and its tick would fire zero times. Nothing a test can
// do to a real journal refuses that one read and admits the claim after it — a
// standby's journal refuses both until the `serve` holding it goes, and opens
// to both at once — so this runner holds the journal's `lastFired` itself, and
// stands in for the admission the scheduler hands its ticks to. Two cases:
//
//   * **bypass**: the clock runs sixty times fast, so the every-minute
//     trigger ticks once a real second. The newest row is a claim whose
//     execution never began, and the scheduler's first two looks are refused —
//     the start's, and the first retry's. Every call the admission receives is
//     recorded in order: the claim is handed back for its start before any
//     tick is claimed, and a tick fires after it.
//   * **standby**: the clock runs at its own rate, so no tick is due for up to
//     a minute — a daily cron's case, in small. The newest row is a claim a
//     `serve` that held the journal made and died before starting, and the
//     look is refused twice, as a standby's is while the guard is held. The
//     claim is started on the look's own backoff, seconds in, with no tick
//     claimed and none due: not at the next tick's look a period later.
//
// Usage: bun schedule-look.mjs <built project of the `schedules` fixture>
// Prints one JSON object, `{ "<case>": [<call>, …] }`, read by
// `compiled_graph_acceptance.rs`; the serve log the scheduler writes goes to
// stderr, as `serve`'s does.

import { pathToFileURL } from "node:url";
import path from "node:path";
import process from "node:process";

const [, , project] = process.argv;
if (project === undefined) {
  throw new Error("usage: bun schedule-look.mjs <generated project directory>");
}

const at = (relative) => pathToFileURL(path.resolve(project, relative)).href;
const journalModule = await import(at("src/journal.ts"));
const sources = await import(at("src/sources.ts"));
const journal = await journalModule.openJournal();

const TRIGGER = "every_minute";
const REFUSED_LOOKS = 2;
const MINUTE = 60_000;

/** One case: a fired table holding `stranded`, and the calls the admission received. */
async function scenario(name, rate, stranded, until, budgetMs) {
  process.env.AGENT_COMPOSE_SCHEDULE_CLOCK_RATE = String(rate);
  const rows = [stranded];
  const begun = new Set();
  const calls = [];
  let looks = 0;
  journal.lastFired = async (trigger) => {
    looks += 1;
    if (looks <= REFUSED_LOOKS) {
      calls.push({ look: looks, refused: true });
      throw new Error(`this journal refuses look ${looks}`);
    }
    calls.push({ look: looks, refused: false });
    const own = rows.filter((row) => row.trigger === trigger);
    return own.reduce((newest, row) => (row.scheduledAt > newest.scheduledAt ? row : newest));
  };
  let claims = 0;
  const intake = {
    async tick(trigger, instant, payload) {
      claims += 1;
      const execution = `exec_${name}_claim_${claims}`;
      rows.push({
        trigger: trigger.name,
        scheduledAt: new Date(instant).toISOString(),
        execution,
        firedAt: new Date().toISOString(),
      });
      begun.add(execution);
      calls.push({ tick: payload.scheduled_at, execution });
      return { outcome: "started", execution };
    },
    async unbegun(trigger, fired, payload) {
      if (begun.has(fired.execution)) {
        calls.push({ unbegun: fired.execution, begun: true });
        return undefined;
      }
      begun.add(fired.execution);
      calls.push({ unbegun: fired.execution, begun: false, scheduled_at: payload.scheduled_at });
      return { outcome: "started", execution: fired.execution };
    },
    async event() {
      throw new Error("no event source in this composition");
    },
  };
  const deadline = Date.now() + budgetMs;
  await sources.startSources(intake);
  while (!until(calls) && Date.now() < deadline) {
    await new Promise((resolve) => setTimeout(resolve, 50));
  }
  await sources.stopSources();
  return calls;
}

/** The minute `minutes` before this one, as a `fired` row records it. */
function minutesAgo(minutes) {
  const now = Date.now();
  return new Date(now - (now % MINUTE) - minutes * MINUTE).toISOString();
}

const results = {};

// Done once a tick has been claimed after the stranded claim was handed back,
// or once a tick was claimed with no hand-back before it — the regression.
results.bypass = await scenario(
  "bypass",
  60,
  {
    trigger: TRIGGER,
    scheduledAt: minutesAgo(3),
    execution: "exec_stranded_by_a_stopped_serve",
    firedAt: minutesAgo(3),
  },
  (calls) => calls.some((call) => call.tick !== undefined),
  20_000,
);

// Done once the stranded claim was handed back, or once the budget is spent —
// well short of a period, and long enough for the look's backoff of one
// second and then two.
results.standby = await scenario(
  "standby",
  1,
  {
    trigger: TRIGGER,
    scheduledAt: minutesAgo(0),
    execution: "exec_claimed_by_a_dead_primary",
    firedAt: minutesAgo(0),
  },
  (calls) => calls.some((call) => call.unbegun !== undefined),
  10_000,
);

await journalModule.closeJournal?.();
process.stdout.write(JSON.stringify(results));
process.exit(0);
