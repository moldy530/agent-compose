import type { ToolStampModule } from "../modules.ts";

/**
 * `tool.stamp` — stamp a payload with this deployment's own marker.
 *
 * Authored, and held to the tool's declared `input:`/`output:` by the type
 * annotation: `ToolStampModule` is generated from those two schemas, so a change
 * to either stops this file compiling (grammar 6.1, PRD resolved q48).
 *
 * `STAMP_MARKER` arrives as the third argument rather than out of
 * `process.env`, and `ToolStampModuleEnv` admits that name because the binding
 * declares it: `References::of` cannot walk a variable read inside TypeScript,
 * so the YAML is both where the environment partition learns of it and where
 * this file's right to read it comes from (PRD resolved q49).
 */
const toolStamp: ToolStampModule = (input, _context, env) => ({
  stamped: `${input.payload} ${env.STAMP_MARKER}`,
});

export default toolStamp;
