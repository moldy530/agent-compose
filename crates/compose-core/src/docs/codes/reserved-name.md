# reserved-name

## What it protects

Seven identifiers already mean something inside an expression:
`input`, `state`, `execution`, `item`, `messages`, `output`, `payload`. Reusing
one as a state channel name or a flow-local node id is refused.

Shadowing is an error rather than a precedence rule because there is no reading
of `input.output.verdict` that is obviously right when a node is named `input`,
and a document that had to name a winner would be teaching a trap.

Two of the seven are on the list for their own reasons. `messages` names the
implicit conversation-history channel, which already exists. `output` is the
fixed **selector** of a node-output path — the middle segment of every
`<node>.output.<field>` — so reserving it keeps the token to one meaning
wherever an expression is written.

`start` and `end` are additionally reserved as node ids. Definition *names* have
no reserved words: `agent.state` is fine, because a namespaced address is never
a bare token in expression position.

## A spec that triggers it

```yaml triggers
version: "0.1"
state:
  output: { type: string }
```

## The fix

Rename. `item` is the one exception, and only in one position — it is the
default name of a `map`'s per-item binding, so `as: item` is legal.

Grammar: `docs/grammar.md` §2.5, Decision D74. Topics:
`agent-compose docs getting-started`, `agent-compose docs state`.
