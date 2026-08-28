# missing-callback-allowlist

## What it protects

An `http` trigger that authenticates its outbound callbacks and does not say
where they may go. `callback_auth:` is opt-in, and declaring it makes
`callback_allow:` mandatory. Writing one key to require a second is rare here
but not unique: a `human:` node's `timeout:` and `on_timeout:` are declared
together or not at all (`missing-key`), and an edge's `max_iterations:` is legal
only where a `when:` is (`invalid-value`). What is unique is the reason.

The reason is what a callback URL *is*. It comes from the request payload
(`callback: "payload.body.callback_url"`), so whoever calls the trigger chooses
where the deployment will later POST — including the body of a parked or settled
execution, and including the credential `callback_auth:` attaches to it. A
deployment careful enough to sign its deliveries must not hand them, signature
and all, to whatever host a payload named.

Declaring **neither** key stays legal and is the documented test posture: a
trigger with no `callback_auth:` may POST anywhere, carries no identity, and gets
no allowlist ceremony. The rule fires only on the half-configured case, which is
the one that looks careful and is not.

## A spec that triggers it

```yaml triggers
version: "0.1"
flow.f:
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
    flow: flow.f
    callback: "payload.body.callback_url"
    callback_auth:
      hmac:
        secret: ${CALLBACK_SECRET}
```

## The fix

Two repairs, and either one is complete.

**Declare the allowlist** — the receivers this trigger is allowed to deliver to,
as absolute `http`/`https` URL patterns where `*` stands for any run of
characters. A callback URL outside the list will be refused when it is read, at
parking or at settle, and recorded as a refused delivery rather than as anybody's
failure.

**Anchor the host against the scheme.** `*` runs over anything, `/` and `?`
included, so `https://hooks.example.com/*` constrains a host and
`https://*.hooks.example.com/*` constrains none: its leading `*` is free to
swallow `attacker.test/collect?x=` on the way to the dot, and a payload naming
that URL is admitted. Give a second subdomain a second entry.

**This check is live; the matching it demands is not.** `callback_auth:` and
`callback_allow:` are reserved grammar in v0 (grammar §15): parsed, checked, and
carried into the IR, and read by nothing generated yet — a built app signs no
delivery and refuses no URL. Declaring them says what the deployment will
enforce; anything that needs the guarantee today keeps a gateway in front.

**Or drop `callback_auth:`** when the trigger is a development or test entry
point. Then the deployment signs nothing, claims nothing, and may POST wherever
the payload says — which is a posture worth choosing deliberately and worth
never shipping.

## The fix, applied

```yaml spec
version: "0.1"
flow.f:
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
    flow: flow.f
    callback: "payload.body.callback_url"
    callback_auth:
      hmac:
        secret: ${CALLBACK_SECRET}
    callback_allow:
      - "https://hooks.example.com/*"
```

Grammar: `docs/grammar.md` §13.3, §4.3, §15, Decision D126. Topic:
`agent-compose docs triggers`.
