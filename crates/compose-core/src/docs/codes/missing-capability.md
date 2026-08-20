# missing-capability

## What it protects

A provider cannot serve what something asks of it. Provider plugins publish
their capabilities, and three checks read that table: every model an agent
references must come from a provider serving structured output and tool use, all
members of a failover `route:` must be capability-equivalent, and a `vector`
store's `embed.provider` must serve embeddings.

The route rule is the subtle one: failover that silently dropped structured
output would turn a rate-limit into a parse failure two nodes later.

The embeddings rule is where most reports come from, because it is easy to
assume the storage backend computes vectors. It does not. `backend:` says
*where the vectors live* and forks per target; `embed.provider` says *what turns
text into a vector* and does not, so one store's embeddings are identical under
`local` and under `staging` with only their home changing.

## A spec that triggers it

```yaml triggers
version: "0.1"
provider.p:
  kind: anthropic
  api_key: ${K}
store.docs:
  kind: vector
  scope: global
  embed:
    model: text-embedding-3-small
    provider: provider.p
```

## The fix

Name a provider whose kind serves what you are asking for. For embeddings that
means a second `provider.*` beside the inference one:

```yaml
provider.embeddings:
  kind: openai
  api_key: ${OPENAI_API_KEY}
```

Providers are cheap — they are a connection, not a deployment — and having two
is the normal shape for a project that both calls a model and embeds text.

Grammar: `docs/grammar.md` §11.2, §12.2, Decision D116. Topics:
`agent-compose docs stores`, `agent-compose docs models`.
