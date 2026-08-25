# unknown-key

## What it protects

Unknown keys are errors everywhere except plugin config objects. A key nothing
reads is a key whose author expected an effect that is not happening — a
misspelled `retry`, a `tools:` on a construct that does not take one, a setting
copied from another kind's row. Accepting it silently leaves a composition that
looks configured and is not.

The same code covers several closed vocabularies: a top-level key that is
neither a section nor a definition, a key a construct does not define, a
provider key belonging to another `kind`'s row, a field a curated server tool
does not have, and a store-op parameter its op does not take.

One of those vocabularies is decided by the **connection** rather than by its
kind. An `openai` provider that declares `server_tools:` speaks the Responses
API for all of its calls, and `stop:` and `seed:` are Chat Completions keys with
no equivalent there — so a model bound to such a provider is refused those two,
with a diagnostic naming the wire and the provider that moved it. Both stay
legal on an `openai` provider with no suite.

## A spec that triggers it

```yaml triggers
version: "0.1"

trigger:
  cli:
    type: manual
    flow: flow.review_loop
```

## The fix

Read the `help:` line — where the misspelling is close to a real key, the
diagnostic says which one. Otherwise look the construct up: `agent-compose docs
<topic>` lists the keys each one takes.

The two places a key is genuinely open are a model's `settings:` and the config
objects under `storage_backends` and `event_sources`, and even there the keys
are checked against the plugin's published schema.

Grammar: `docs/grammar.md` §1.5, §11.4, §12.1, Decisions D50, D106. Topic:
`agent-compose docs getting-started`.
