import type { ToolStampModule } from "../modules.ts";

/**
 * `tool.stamp` — stamp a payload with this deployment's own marker.
 *
 * Authored, and held to the tool's declared `input:`/`output:` by the type
 * annotation: `ToolStampModule` is generated from those two schemas, so a change
 * to either stops this file compiling (grammar 6.1, PRD resolved q48).
 *
 * `STAMP_MARKER` is read out of the environment, which is why the binding
 * declares it: `References::of` cannot walk a `process.env` read inside
 * TypeScript, so the YAML is where the environment partition learns of it (PRD
 * resolved q49).
 */
const toolStamp: ToolStampModule = (input) => ({
  stamped: `${input.payload} ${process.env.STAMP_MARKER ?? "unmarked"}`,
});

export default toolStamp;
