# human

A `human` node pauses the execution and asks a person. The grammar and the
runtime are both active: a compiled project really stops at the node, reports
the execution as `interrupted`, publishes what the human is shown, and holds
the answer to a declared schema — so the answer is routable structured output
like any other node's.

```yaml spec
version: "0.1"
provider.p:
  kind: anthropic
  api_key: ${ANTHROPIC_API_KEY}
model.m:
  provider: provider.p
  id: claude-sonnet-4-5

agent.drafter:
  model: model.m
  prompt: Draft the announcement.
  output:
    draft: { type: string }

agent.escalation:
  model: model.m
  prompt: Nobody answered in time; write a handover note.
  output:
    note: { type: string }

state:
  draft:          { type: string, default: "" }
  human_decision: { enum: [approve, reject, revise], default: approve }

flow.publish:
  outputs: {}
  nodes:
    write: { agent: agent.drafter, input: "'the announcement'" }
    approve:
      human:
        input:                                  # what the human is shown
          draft: { type: string }
        output:                                 # what they return; routable
          decision: { enum: [approve, reject, revise] }
          note:     { type: string }
        timeout: 24h
        on_timeout: escalate
      input: { draft: "state.draft" }
      writes: { decision: human_decision }
    escalate: { agent: agent.escalation, input: "'nobody answered'" }
  edges:
    - { from: start, to: write }
    - { from: write, to: approve }
    - { from: approve, to: end }
    - { from: escalate, to: end }
```

## Keys inside `human:`

| Key | Required | Notes |
|---|---|---|
| `input` | yes | a field map, rendered for the human |
| `output` | yes | a result schema; answers are validated against it |
| `timeout` | no | a wall-clock wait budget |
| `on_timeout` | with `timeout`, never without | a flow-local node id, or `end` |

`timeout:` and `on_timeout:` are **jointly optional and jointly required**.
`timeout:` alone leaves the expiry with no route; `on_timeout:` alone declares a
route that can never be taken. Either half alone is a compile error naming the
missing one.

Node-level `timeout:` and `retry:` are **illegal** on a `human` node — a wait is
not an activity timeout, and re-prompting a person is not a retry. The exemption
is the whole chain's, not just this level's: a `human` node resolves no
`timeout` and no `retry` from a `flow:` node's `policy:` or from `defaults:`
either, so a composition-wide budget can never cut a wait short.

Nor does the budget of a node *above* a wait cut it short. A `flow:` node's
`timeout:` is **held still** for as long as a wait inside it is open, and
resumes with the time it had left: `timeout: 60s` on a `flow:` node whose
subflow pauses for an hour still means sixty seconds of running.

`on_error:` **is** legal and resolves through all four levels — it covers
delivery failures, which are ordinary node errors.

## Two ways to answer

Which surface is available is a property of the *invocation*, not of the
composition. What differs between them is delivery and nothing else: both
address a pause by its instance path, both hold the answer to the node's
`output:`, both refuse an answer that does not fit **without consuming the
wait**, and both leave the same trace record.

| surface | available when | how the answer arrives |
|---|---|---|
| `POST /executions/:id/resume` | the execution was started by the generated app (`agent-compose serve`) | the request body; `?wait=` names which pause where an execution holds more than one |
| the terminal of an `agent-compose run` | standard input is a terminal, or `AGENT_COMPOSE_INTERACTIVE=1` | one JSON value per line, at a prompt naming the pause |

A `run` with **neither** — standard input is not a terminal, or
`AGENT_COMPOSE_INTERACTIVE=0` forces it — cannot answer, so a run that reaches a
pause reports it and exits **3** rather than waiting or carrying on. So does one
whose terminal goes away.

Four details of the terminal surface are its own:

- the framing is **one JSON value per line**; a value spanning lines has no
  terminator a prompt could recognize without guessing or hanging;
- a line that is not JSON, or that the `output:` refuses, **re-prompts** and
  does not consume the wait;
- an execution holding more than one pause is asked **one at a time**, each
  question being the lowest-id pause open when it is asked — a pause that opens
  while a question is on the screen is asked after it;
- **standard input ending withdraws the surface**: every pause still waiting,
  and every one opened after, becomes the same interrupt a run with no surface
  raises, so a script that answered too few questions ends where it stood
  instead of parking for ever.

`AGENT_COMPOSE_INTERACTIVE` takes `1` or `0` and nothing else; any other value
is refused before the run starts, as a command that could not be run rather than
a setting nobody read.

A `timeout:` is not one of the four. It keeps running while the prompt is on the
screen, and an expiry routes through `on_timeout:` there and then — taking the
question with it, so the prompt is withdrawn and the next pause is asked.

## Where a pause may not be

Two constructs may not **reach** a `human` node — including one inside a
`map`-dispatched flow, and one inside a flow attached to an agent's `tools:`:

- the flow a **`respond: sync`** http trigger targets, because a pause in the
  middle of a synchronous request has no answer the request can wait for
  (`sync-trigger-interrupt`). Declare `respond: async` and take the result
  through `callback:` instead;
- a **detached** `map` dispatch's target, because the join never observes the
  instance and the execution can end while it is still in flight
  (`detached-interrupt`).

## Durability

A wait lives in the **process** holding it. Durable execution is a later
milestone, so a `serve` restarted while a human was thinking has lost it. That
boundary is stated in the emitted project's `README.md` where a reader meets the
resume route.

Normative source: `docs/grammar.md` §8.7
