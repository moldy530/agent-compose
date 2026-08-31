import type { ToolStampModule } from "../modules.ts";

/**
 * `tool.stamp` — stamp a release with the marker the signing machine holds.
 *
 * Authored code inside the emitted project (grammar 6.1, PRD resolved q48), and
 * the one file of this composition the compiler did not write. It travels with
 * the artifact because the composition references it (PRD resolved q49), so what
 * runs it is a worker that fetched this tree by hash and never saw the checkout
 * it was committed in.
 *
 * `STAMP_MARKER` is declared in the YAML and arrives as the third argument,
 * which is why the environment partition carries it to `mac` and to nothing
 * else. Reading it out of `process.env` would work on whatever machine happened
 * to have the variable; reading it here works exactly where the composition said
 * this tool runs, and a process the partition did not give it to fails the call
 * naming the variable rather than quietly stamping something else.
 */
const toolStamp: ToolStampModule = (input, _context, env) => ({
  stamped: `${input.path} ${env.STAMP_MARKER}`,
});

export default toolStamp;
