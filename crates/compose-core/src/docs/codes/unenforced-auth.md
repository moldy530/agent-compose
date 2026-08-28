# unenforced-auth

## What it protects

**A warning, and the composition still builds — that is the whole point of it.**

`auth:`, `callback_auth:` and `callback_allow:` are reserved grammar in v0
(`docs/grammar.md` §15): fully specified, parsed, checked, and carried into the
IR, and read by nothing this release generates. A served trigger declaring
`auth:` is exactly as open as one declaring none, and a callback is still
delivered to whatever URL the payload named.

Every other reserved construct is content to say so in a document. A `schedule`
trigger that runs nothing is visible the first morning it does not fire; an
`event_sources:` block that consumes nothing has an empty queue to show for it.
An access control that runs nothing shows **nothing at all** — the route answers,
the flow starts, the deployment looks healthy, and it is indistinguishable from
outside from a route that verifies its callers. A reader who has to learn that
from a document learns it only if they read the document.

So the report says it. `validate` and `build` name the trigger and the keys, and
the verdict that follows says the composition is valid **with a warning** rather
than valid — so a deployment whose only access control is inert cannot pass
through a pipeline that reads verdicts without something having mentioned it.

The warning is **not** advice to drop the keys. Declaring them is how a spec
records what the deployment will enforce, and the M3 runtime is written against
exactly what §13.3 specifies. What the warning buys is that nobody mistakes the
declaration for the enforcement in the meantime — and it is deleted by the same
change that makes it false, since a report claiming a live control is unenforced
is the same wrong claim pointing the other way.

## A spec that triggers it

```yaml triggers
version: "0.1"

flow.support:
  outputs: {}
  nodes:
    approve:
      human:
        input: {}
        output:
          decision: { enum: [approve, reject] }
  edges:
    - { from: start, to: approve }
    - { from: approve, to: end }

triggers:
  intake:
    type: http
    flow: flow.support
    auth:
      bearer:
        token: ${WEBHOOK_TOKEN}
```

## The fix

Put a gateway in front of the generated app, and keep the keys. Until the
runtime lands, the guarantee has to come from something outside the artifact —
an API gateway, an ingress, a reverse proxy — and the trigger's `auth:` block is
the specification that thing is configured to match. That is v0's standing
posture for a deployment that needs the guarantee (`docs/grammar.md` §15).

Deleting the block is the other way to quiet the report, and it is the worse
one: it makes the spec say nothing about who may invoke the flow, which is the
same route with one less thing written down. The warning is a statement about
this release, not about the spec.

`callback_allow:` reads the same way. Its shape is checked here —
`missing-callback-allowlist` is a live error, and every entry is validated — but
no delivery is refused against it yet, so a callback still goes wherever the
payload said.

Grammar: `docs/grammar.md` §13.3, §15, PRD resolved q32 and q33. Topic:
`agent-compose docs triggers`.
