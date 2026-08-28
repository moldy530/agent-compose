# missing-callback-allowlist

## What it protects

An `http` trigger that authenticates its outbound callbacks and does not say
where they may go. `callback_auth:` is opt-in, and declaring it makes
`callback_allow:` mandatory — the one place in this grammar where writing a key
makes a second key required.

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
characters. A callback URL outside the list is refused when it is read, at
parking or at settle, and recorded as a refused delivery rather than as anybody's
failure.

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

Grammar: `docs/grammar.md` §13.3, §4.3, Decision D126. Topic:
`agent-compose docs triggers`.
