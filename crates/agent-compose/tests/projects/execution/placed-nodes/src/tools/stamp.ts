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
 * `STAMP_MARKER` is read here and declared in the YAML, which is why the
 * environment partition carries it to `mac` and to nothing else. The fallback
 * below is what a run says when it did not: a marker that never arrived is
 * evidence rather than a silent difference.
 */
const toolStamp: ToolStampModule = (input) => ({
  stamped: `${input.path} ${process.env.STAMP_MARKER ?? "unmarked"}`,
});

export default toolStamp;
