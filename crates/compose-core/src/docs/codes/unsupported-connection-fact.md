# unsupported-connection-fact

## What it protects

A `coder:` node's `model:` is a registry address, and the adapter reads more off
it than the model id: the **connection facts** of the provider behind it —
`base_url:`, the credential, and `headers:` — cross into the run and are mapped
into the harness's own connection surface. That is what keeps one promise true
for a coder node that was already true for every agent node: pointing a project
at a gateway is a one-line edit in `providers.yml`, not a per-node duplication in
harness-native spellings.

The map is a **curated table per harness**, because the two harnesses do not
offer the same surface:

* **`cc`** reaches the Claude Agent SDK, whose `Options.env` replaces the
  subprocess environment outright and whose embedded runtime reads its endpoint,
  its API key and its custom headers out of that environment. All three facts
  cross, as `ANTHROPIC_BASE_URL`, `ANTHROPIC_API_KEY` and
  `ANTHROPIC_CUSTOM_HEADERS`.
* **`codex`** reaches the Codex SDK, whose client options are `baseUrl` and
  `apiKey` and nothing else that speaks to a connection. **There is no header
  slot**: a thread takes none, the client takes none, and the only way a header
  could reach that CLI is a `model_providers.*` config table this composition
  never wrote — with its own name, its own endpoint and its own credential
  variable. Inventing one would be a slot that is not there.

So a fact a harness has no slot for is an **error**, not a dropped value with a
warning. An unknown `settings:` key travels to the SDK and is warned about,
because the cost of being wrong is one option nothing reads. A connection fact is
the other thing entirely: it decides *where the traffic goes* and *whether it
authenticates*. A `headers:` quietly dropped on the way into a run is a gateway
token that never reaches the wire, discovered by a `401` on the first live call
— which is exactly the failure moved from production back to `validate` when the
keyless-gateway rule was written.

## A spec that triggers it

```yaml triggers
version: "0.1"

provider.gateway:
  kind: openai
  base_url: ${LLM_GATEWAY_URL}
  api_key: ${GATEWAY_KEY}
  headers:
    x-team: platform

model.reviewer:
  provider: provider.gateway
  id: gpt-5-codex

state:
  summary: { type: string, default: "" }

flow.review:
  inputs:
    goal: { type: string }
  outputs:
    summary: { type: string }
  nodes:
    review:
      coder:
        harness: codex
        model: model.reviewer
        workspace: ${REPO_ROOT}
        access: read_only
        prompt: Read the working tree and report what it does.
        output:
          summary: { type: string }
      input: "input.goal"
  edges:
    - { from: start, to: review }
    - { from: review, to: end }
```

`base_url:` and `api_key:` both have a slot on this harness and cross without
comment. `headers:` does not, so the node is refused rather than run against an
endpoint that never sees the header.

## The fix

Two repairs, and which one is right depends on why the header is there.

* **The header is for the agents, not for this run.** Define a second
  `provider.*` — providers are cheap, and scoping a connection to one consumer
  by defining another is the move the two-tier tables already lean on — and bind
  the coder node's `model.*` to that one. The headered provider goes on serving
  every agent that needs it.
* **The header is really this connection's.** Then the harness that can carry it
  is the one to run the node under, or the header belongs somewhere the harness
  reads it from — `cc` carries all three facts.

What is **not** a repair is writing the header into the node's `env:` by hand.
That is the per-node duplication this whole mapping exists to remove, and under
`codex` there is no variable to write it into anyway.

## The fix, applied

The same connection with the header taken off, so every fact it declares has a
slot on the harness the node binds.

```yaml spec
version: "0.1"

provider.gateway:
  kind: openai
  base_url: ${LLM_GATEWAY_URL}
  api_key: ${GATEWAY_KEY}

model.reviewer:
  provider: provider.gateway
  id: gpt-5-codex

state:
  summary: { type: string, default: "" }

flow.review:
  inputs:
    goal: { type: string }
  outputs:
    summary: { type: string }
  nodes:
    review:
      coder:
        harness: codex
        model: model.reviewer
        workspace: ${REPO_ROOT}
        access: read_only
        prompt: Read the working tree and report what it does.
        output:
          summary: { type: string }
      input: "input.goal"
  edges:
    - { from: start, to: review }
    - { from: review, to: end }
```

Grammar: `docs/grammar.md` §8.9, §12.1, Decision D143. Topic:
`agent-compose docs agents`.
