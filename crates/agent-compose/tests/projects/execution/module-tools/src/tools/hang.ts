import type { ToolHangModule } from "../modules.ts";

/**
 * `tool.hang` — never answer, and never look at `context.signal` either.
 *
 * Grammar §9.2 bounds one node execution with no exemption for a kind, so what
 * stops this is the node's own `timeout:` rather than anything in here — and the
 * node's `on_error:` fallback then runs while this call is still outstanding.
 * It is `activities`' `flow.stubborn` with a `module:` binding in place of a
 * `function:` one, and the two have to behave identically.
 *
 * The timer is `unref`'d so that a run whose node walked away can still exit:
 * what is being tested is the deadline, not how long a process is willing to
 * wait for a promise nobody is holding.
 */
const toolHang: ToolHangModule = () =>
  new Promise<{ waited: boolean }>((resolve) => {
    setTimeout(() => resolve({ waited: true }), 60_000).unref();
  });

export default toolHang;
