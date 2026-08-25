# unknown-server-tool

## What it protects

**A warning, and the composition still builds.** This is the one diagnostic that
exists to say what the compiler *did not* check.

The compiler keeps a curated table of the server tools each provider kind is
known to serve, and a config naming one is checked strictly — every field
against its documented shape, every constraint the vendor states — because a
config the provider will refuse is a run that dies on its first model call with
a 400 and no span to point at.

A `type:` outside that table is carried to the wire verbatim. That is the whole
design: a server tool the vendor ships tomorrow has to be usable the day it
ships, not one compiler release later. The warning is the honest half of that
bargain — it names exactly what could not be verified, so a typo does not look
like a working configuration, and it suggests the near miss when there is one.

## A spec that triggers it

```yaml triggers
version: "0.1"

provider.anthropic:
  kind: anthropic
  api_key: ${ANTHROPIC_API_KEY}
  server_tools:
    - type: web_search
      name: web_search
```

`web_search` is the *OpenAI* spelling. The Messages wire takes a dated type,
`web_search_20250305`, so this config reaches Anthropic and is refused there.

## The fix

Read the `help:` line. Where the spelling is close to a tool the table knows,
the diagnostic names it; otherwise it lists what the kind's table holds.

Then decide which case you are in:

* **a typo or the other vendor's spelling** — correct it, and the strict tier
  checks the config from then on;
* **a tool this release predates** — leave it. The warning is the record that
  the config is unchecked, and `build` emits the project anyway.

## The fix, applied

The dated type the Messages wire actually takes, which puts the entry back in
the checked tier.

```yaml spec
version: "0.1"

provider.anthropic:
  kind: anthropic
  api_key: ${ANTHROPIC_API_KEY}
  server_tools:
    - type: web_search_20250305
      name: web_search
      max_uses: 5
```

Grammar: `docs/grammar.md` §12.1, Decision D122. Topic:
`agent-compose docs models`.
