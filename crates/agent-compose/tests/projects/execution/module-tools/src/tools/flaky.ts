import type { ToolFlakyModule } from "../modules.ts";

/**
 * How many times this process has called the tool.
 *
 * Module-level state, which is the whole point of the fixture: a `module:` tool
 * runs **in the graph's own process**, so a counter here counts real calls. An
 * `exec:` tool would have to write a file to say the same thing.
 */
let calls = 0;

/**
 * `tool.flaky` — fail the first call and answer the second.
 *
 * The node's `retry:` is what gets a run to the second one, which is grammar
 * §9.3's chain applying to this binding exactly as it applies to the other
 * three.
 */
const toolFlaky: ToolFlakyModule = () => {
  calls += 1;
  if (calls === 1) {
    throw new Error("the first call always fails");
  }
  return { attempts: calls };
};

export default toolFlaky;
