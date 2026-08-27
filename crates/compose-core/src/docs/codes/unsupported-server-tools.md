# unsupported-server-tools

## What it protects

A `server_tools:` array reaches the provider by riding the `tools` of every
request that provider serves. Three kinds carry it today — `anthropic` on the
Messages wire, `openai` on the Responses wire a declared suite switches it to,
and `openai_compatible`, whose gateway may honour any vocabulary at all. The
other three kinds' wires have not been taught the shape.

So a suite declared on one of them would be **dropped on the floor**: a
composition that reads as configured, a model that is never offered the tools,
and no way for the author to tell from the outside. That is the silent no-op the
grammar refuses everywhere else, which is why this is an error rather than a
warning — unlike a tool *name* the compiler does not recognise, which is carried
verbatim precisely because the wire underneath it works.

## A spec that triggers it

```yaml triggers
version: "0.1"

provider.azure:
  kind: azure_openai
  base_url: ${AZURE_ENDPOINT}
  api_key: ${AZURE_API_KEY}
  api_version: "2024-10-21"
  server_tools:
    - type: web_search
```

## The fix

Reach the models through a kind whose wire carries server tools. An Azure,
Bedrock or Vertex connection that genuinely needs a provider-side tool is
reaching an HTTP endpoint that speaks one of the two launch wires, and
`openai_compatible` is the row for that.

Or drop the key: an agent's own `tools:` are dispatched by the runtime and work
on every kind. A server tool is the one that does not — it runs *inside* the
model call, on the provider's side, which is why the wire has to know it.

## The fix, applied

The same connection with the key removed. Its models still run; what they no
longer have is a provider-side search.

```yaml spec
version: "0.1"

provider.azure:
  kind: azure_openai
  base_url: ${AZURE_ENDPOINT}
  api_key: ${AZURE_API_KEY}
  api_version: "2024-10-21"
```

Grammar: `docs/grammar.md` §12.1, Decision D122. Topic:
`agent-compose docs models`.
