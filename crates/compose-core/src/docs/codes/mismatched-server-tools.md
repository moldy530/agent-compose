# mismatched-server-tools

## What it protects

**A warning, and the composition still builds.**

A server tool runs on the provider's side, so a failover route's members each
offer their own suite: the array is declared on `provider.*`, and a route is a
ladder of models bound to different providers. Which tools the model was offered
therefore depends on **which member answered** — and a route exists precisely so
that a caller does not have to know which one did.

That is a legal thing to want. A fallback vendor that publishes no web search is
still a fallback, and refusing the composition would leave an author choosing
between failover and server tools. So the difference is made **visible** rather
than refused: the warning names both providers and what they disagree about, and
a reader who meant it carries on.

Capability equivalence (`missing-capability`) is the neighbouring rule and is an
error, because structured output is what an agent's contract is *made of* — a
route that lost it produces a parse failure two nodes later. A missing search
tool produces a worse answer, which is a different kind of bad.

## A spec that triggers it

```yaml triggers
version: "0.1"

provider.anthropic:
  kind: anthropic
  api_key: ${ANTHROPIC_API_KEY}
  server_tools:
    - type: web_search_20250305
      name: web_search

provider.fallback:
  kind: anthropic
  api_key: ${FALLBACK_API_KEY}

model.primary:
  provider: provider.anthropic
  id: claude-sonnet-4-6

model.spare:
  provider: provider.fallback
  id: claude-haiku-4-5

model.resilient:
  route: [model.primary, model.spare]
```

## The fix

Declare the same suite on both providers, if both vendors serve it — providers
are cheap, and two connections carrying one array is the ordinary shape.

Or keep the route as it is. The warning is the record that a failover changes
what the model can do, not an instruction to remove one.

Scoping a suite to one agent is done the same way: define a second provider. A
`server_tools:` array belongs to a connection, so every agent whose model
resolves to that provider holds it.

Grammar: `docs/grammar.md` §12.1, §12.2, Decision D122. Topic:
`agent-compose docs models`.
