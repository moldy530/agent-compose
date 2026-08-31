# missing-join-token

## What it protects

A target that declares `placements:` and no `hub.join_token:`. Declaring a
placement is declaring that some node runs somewhere else, and the only way a
worker becomes that somewhere else is an authenticated join. The token is the
whole of that authentication, so a mesh described without one is a mesh nobody
can join.

Writing one key to require a second is rare here but not unique: `callback_auth:`
makes `callback_allow:` mandatory (`missing-callback-allowlist`), and a provider
reaching a vendor's own endpoint needs `api_key:` or a gateway's `base_url:`
(`missing-credential`). What each has in common is the shape of the failure:
whether the key is required is decided by a **sibling**, and the repair is a
choice of two.

**What the token means is worth stating plainly, because it is the v1 trust
model.** Holding the join token is being trusted with the mesh: the whole
generated artifact, the right to claim any placement, and the journal's effect
stream. It is one credential and one kind — bearer — not a per-placement
capability, and the per-placement env manifest checked at join is self-reported
presence rather than proof. Per-placement credentials are an additive later
hardening; this key does not pretend to be them.

The token is an `${ENV}` reference and never a literal, like every other
credential in the grammar: a literal here is `invalid-env-ref`, not this code.
Nor is a `hub:` that is not a mapping at all — that is `wrong-type`. One mistake
draws one diagnostic, and telling the author of a malformed block to declare the
block they wrote would be a repair that is false about the file in front of
them.

A `hub:` block on its own is legal, with or without placements. `public_url:` is
useful alone — it is the base every ingress URL this deployment hands out
derives from — and a target that places nothing needs no token, because nothing
joins it.

## A spec that triggers it

The composition is beside the point; the deploy file is what refuses:

```yaml triggers
version: "0.1"

provider.vendor:
  kind: openai
  api_key: ${OPENAI_API_KEY}

model.smart:
  provider: provider.vendor
  id: gpt-4o-mini

agent.embedder:
  model: model.smart
  prompt: Summarize the document for indexing.
  input:
    text: { type: string }
  output:
    summary: { type: string }
```

```yaml deploy mesh
version: "0.1"

hub:
  public_url: "https://hub.example"

placements:
  gpu:
    members: [agent.embedder]
```

## The fix

**Declare the token**, as an environment reference. The variable holds the
bearer credential every worker presents at `POST /workers/join`, and both sides
read it from their own environment — the hub to verify, the worker to offer.

**Or remove the placements**, if this target runs in one process. A component in
no placement executes on the hub, which is the default: a target with no
`placements:` is a single-process deployment and needs no mesh at all.

```yaml deploy fixed
version: "0.1"

hub:
  join_token: ${MESH_JOIN_TOKEN}
  public_url: "https://hub.example"

placements:
  gpu:
    members: [agent.embedder]
```

Grammar: `docs/grammar.md` §14.1, §14.2, §4.3, Decision D130. Topic:
`agent-compose docs targets`. Protocol: `docs/distributed.md`.
