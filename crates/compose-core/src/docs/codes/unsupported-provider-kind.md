# unsupported-provider-kind

## What it protects

A `coder:` node's `model:` carries its provider's **connection** into the harness
run: the endpoint, the credential and the headers are mapped into the harness's
own connection surface by a curated table per harness. On `cc` that surface is
the run's environment — `ANTHROPIC_BASE_URL`, `ANTHROPIC_API_KEY`,
`ANTHROPIC_CUSTOM_HEADERS`. On `codex` it is the SDK client's own `baseUrl` and
`apiKey`, which reach the CLI as its OpenAI base URL and `CODEX_API_KEY`.

Read those two lists again and the thing the slots have in common is not that
they are slots — it is **whose wire they are**. `ANTHROPIC_BASE_URL` is where an
Anthropic client is pointed and `ANTHROPIC_API_KEY` is what authenticates there;
the Codex options are OpenAI's. So a table keyed on the harness alone would map
any provider into any harness: an `openai` connection bound through a `cc` node
would write an OpenAI gateway's URL and an OpenAI key into the two variables the
bundled Claude runtime reads, and the run would authenticate nowhere.

That is the same class as a fact with no slot at all, one level up, so it is
refused the same way — at `validate`, naming both halves of the pairing, rather
than as a `401` on the first live call. Each row of the table therefore names the
provider kinds its slots speak for:

* **`cc`** carries `anthropic`.
* **`codex`** carries `openai` and `openai_compatible` — the two kinds that are
  an OpenAI-wire endpoint with a key, which is exactly what its two options are.
  `azure_openai` is not among them: its endpoint carries a deployment path and a
  required `api_version:`, and the SDK has no slot for either.
* **`bedrock` and `vertex` are on neither row.** Their connections are a cloud
  provider's own credential chain — `access_key_id:`, `secret_access_key:`,
  `session_token:`, `credentials_json:` — and neither harness SDK has anywhere
  to put one. Those keys are refused here, with the pairing, rather than
  declared and silently dropped on the way into a run.

The rows are curated, not grammar: a kind a vendor teaches its SDK tomorrow is an
edit to the table in a compiler release, never a new keyword to learn.

## A spec that triggers it

```yaml triggers
version: "0.1"

provider.gateway:
  kind: openai
  base_url: ${LLM_GATEWAY_URL}
  api_key: ${GATEWAY_KEY}

model.implementer:
  provider: provider.gateway
  id: gpt-5-codex

state:
  summary: { type: string, default: "" }

flow.fix:
  inputs:
    goal: { type: string }
  outputs:
    summary: { type: string }
  nodes:
    implement:
      coder:
        harness: cc
        model: model.implementer
        workspace: ${REPO_ROOT}
        prompt: Fix the failing test, then say what you changed.
        output:
          summary: { type: string }
      input: "input.goal"
  edges:
    - { from: start, to: implement }
    - { from: implement, to: end }
```

Nothing about that composition is misspelled, and that is why it is worth
refusing: every value is legal on its own, and only the pair is wrong.

## The fix

Two repairs, and which one is right depends on which half was intended.

* **The harness is the point** — this node runs Claude Code. Then its `model:`
  belongs on an `anthropic` provider, and the gateway one goes on serving the
  agent nodes and the `codex` nodes that were already using it. Providers are
  cheap on purpose.
* **The connection is the point** — this node has to go through that gateway.
  Then it is a `codex` node, which is the harness whose connection surface
  speaks that wire.

What is **not** a repair is hand-carrying the endpoint and the key into the
node's `env:` under the other vendor's variable names. That is the per-node
duplication the mapping exists to remove, and the run would still be a Claude
harness pointed at an OpenAI endpoint.

## The fix, applied

The coder node keeps its harness and binds a model on a provider whose wire that
harness speaks — through the same gateway, which is the point of the crossing:
the endpoint is still one line in `providers.yml`.

```yaml spec
version: "0.1"

provider.anthropic:
  kind: anthropic
  base_url: ${LLM_GATEWAY_URL}
  api_key: ${GATEWAY_KEY}

model.implementer:
  provider: provider.anthropic
  id: claude-sonnet-4-5

state:
  summary: { type: string, default: "" }

flow.fix:
  inputs:
    goal: { type: string }
  outputs:
    summary: { type: string }
  nodes:
    implement:
      coder:
        harness: cc
        model: model.implementer
        workspace: ${REPO_ROOT}
        prompt: Fix the failing test, then say what you changed.
        output:
          summary: { type: string }
      input: "input.goal"
  edges:
    - { from: start, to: implement }
    - { from: implement, to: end }
```

Grammar: `docs/grammar.md` §8.9, §12.1, Decision D143. Topic:
`agent-compose docs agents`.
