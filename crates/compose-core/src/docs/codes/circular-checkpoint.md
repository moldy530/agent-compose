# circular-checkpoint

## What it protects

A `kind: command` event source is a program `serve` starts and reads one JSON
object per line from. Its `checkpoint:` is the CEL expression that says where
the program has got to — a sequence number, an offset, a cursor — and the
runtime hands the newest one back when it restarts the program, as
`AGENT_COMPOSE_CHECKPOINT` and in place of an argument written `{checkpoint}`, so
the program resumes rather than starting over.

The checkpoint is also what an event is **called**: where a source declares one,
its value is the event's `payload.id`, which is what an `event` trigger's
`dedupe_key:` reads by default. So the checkpoint is computed *first*, from the
line, and the id is its answer — and an expression that reads `payload.id`
would be defined in terms of itself. There is no value for it to read yet.

The expression is typed over what does exist when it runs: `payload.body` (the
decoded line), `payload.source` (the source's name) and `payload.attributes`
(`{}` for a command source). It must answer a string.

## A spec that triggers it

The composition is beside the point; the deploy file is what refuses:

```yaml triggers
version: "0.1"

flow.ingest:
  inputs:
    report: { type: string }
  outputs: {}
  nodes:
    store:
      exec: { command: "true" }
  edges:
    - { from: start, to: store }
    - { from: store, to: end }

triggers:
  reports:
    type: event
    flow: flow.ingest
    source: feed
    input: { report: "payload.body.text" }
```

```yaml deploy staging
version: "0.1"

event_sources:
  feed:
    kind: command
    command: ./bin/feed
    checkpoint: "payload.id"
```

## The fix

Read the position out of the line itself — whatever field the program writes to
say where it is:

```yaml deploy fixed
version: "0.1"

event_sources:
  feed:
    kind: command
    command: ./bin/feed
    args: ["--after", "{checkpoint}"]
    checkpoint: "payload.body.cursor"
```

Or declare no `checkpoint:` at all. Then an event's id is the SHA-256 of its
exact line — still a key `dedupe_key:` can read — and the program is started
with no checkpoint to resume from, so it has to know for itself where it was.

Grammar: `docs/grammar.md` §14.4, Decision D162. Topic: `agent-compose docs targets`.
