import type { ToolSealedModule } from "../modules.ts";

/**
 * `tool.sealed` — answer with the marker this binding declared.
 *
 * `SEALED_MARKER` is a name the process running this graph does not have: the
 * binding maps it to the value of a variable the process *does* have, which is
 * the spelling grammar §6.1 offers for "these two can differ". So the only place
 * that name exists is the argument below, and `flow.sealed` checks the other
 * half — that a subprocess started after this call cannot see it.
 */
const toolSealed: ToolSealedModule = (_input, _context, env) => ({
  marker: env.SEALED_MARKER,
});

export default toolSealed;
