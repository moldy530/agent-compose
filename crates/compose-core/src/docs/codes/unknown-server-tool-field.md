# unknown-server-tool-field

## What it protects

**A warning, and the composition still builds.** It is `unknown-server-tool`'s
neighbour, one level in: the `type:` is one the compiler's curated table knows,
and a key beside it is not.

The table is keyed on `type:` alone, so a row is a snapshot of one tool taken
when the compiler was released. Vendors add parameters to tools they already
ship. If a key the row predates were an error, every author of that tool would
wait for a new compiler binary to use it — the manual support treadmill the two
tiers exist to avoid, arriving at field granularity instead of tool
granularity, and with no way out: renaming the `type:` to duck into the
unchecked tier would change which tool runs.

So the key is carried to the wire as written, and the warning says exactly that.
What stays an error is everything the table can genuinely speak for: a field it
*does* model given the wrong kind of value, a value outside a range or a closed
set the vendor states, a required field left out, two fields the vendor refuses
together.

## A spec that triggers it

```yaml triggers
version: "0.1"

provider.anthropic:
  kind: anthropic
  api_key: ${ANTHROPIC_API_KEY}
  server_tools:
    - type: web_search_20250305
      name: web_search
      max_usages: 5
```

`web_search_20250305` is in the table, and the field it takes is `max_uses`.
Nothing in the compiler can tell that from a parameter shipped after this
release — so it is warned about, the near miss is named, and `max_usages: 5`
reaches Anthropic, where it is what the service makes of it.

## The fix

Read the `help:` line, then decide which case you are in:

* **a typo** — the help names the near miss where the spelling is close. Correct
  it and the field is checked from then on;
* **a parameter this release predates** — leave it. The warning is the record
  that one key of the config is unchecked, and `build` emits the project anyway.

The one thing worth ruling out first: that the whole entry is for a tool this
kind does not serve. A `type:` outside the table reports `unknown-server-tool`
instead, and then *nothing* about the config was checked.

## The fix, applied

The spelling the Messages wire takes, which puts the field back in the checked
tier — where `max_uses: 0` would now be an error, because the table states the
bound.

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
