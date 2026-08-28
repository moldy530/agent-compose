# missing-credential

## What it protects

A provider that reaches a vendor's own endpoint and carries no key. `anthropic`
and `openai` are the two kinds with a **default endpoint**: omit `base_url:` and
the connection resolves to `https://api.anthropic.com` or
`https://api.openai.com`, where nothing but `api_key:` authenticates. So on those
two kinds the key is required when `base_url:` is absent and optional when it is
present, and declaring neither is this diagnostic.

The optional half is the point of the rule. A corporate deployment routes model
traffic through a gateway that injects the vendor credential server-side, and
nobody running the graph holds a key at all — so `kind: anthropic` with a
`base_url:` and no `api_key:` is a legal, ordinary connection. What is refused is
only the pair being absent *together*, which is a spec that builds, ships, and
401s on its first live call.

The other four kinds are untouched: `azure_openai` reaches a per-resource
deployment with no default to fall back to and keeps all three of its keys
required, `openai_compatible` already paired an optional `api_key` with a
required `base_url`, and `bedrock` and `vertex` authenticate through their
cloud's own credential chain.

## A spec that triggers it

```yaml triggers
version: "0.1"
provider.claude:
  kind: anthropic
model.smart:
  provider: provider.claude
  id: claude-sonnet-4-5
```

## The fix

Two repairs, and either one is complete.

**Declare the key** — `api_key: ${ANTHROPIC_API_KEY}` — when the graph really is
calling the vendor. Credentials are env-ref values (the whole string is one
`${NAME}`), they survive unresolved into the artifact, and presence is checked at
process start rather than at compile.

**Or name the gateway** — `base_url: ${LLM_GATEWAY}` — when something in front of
the vendor supplies the credential. A provider with a `base_url:` and no
`api_key:` sends **no** authentication header at all, not an empty one, so the
gateway sees a request that never claimed to authenticate. A gateway wanting a
token of its own takes it through `headers:`, whose values interpolate:
`authorization: "Bearer ${PROXY_TOKEN}"`.

## The fix, applied

The gateway repair, since it is the one whose shape is new. Adding
`api_key: ${ANTHROPIC_API_KEY}` to the spec above instead clears the same
diagnostic.

```yaml spec
version: "0.1"
provider.claude:
  kind: anthropic
  base_url: ${LLM_GATEWAY}
  headers:
    authorization: "Bearer ${PROXY_TOKEN}"
model.smart:
  provider: provider.claude
  id: claude-sonnet-4-5
```

Grammar: `docs/grammar.md` §12.1, §4.3, Decision D120. Topics:
`agent-compose docs models`.
