# unknown-field

## What it protects

A path from a typed root names a field the declared schema does not have. Every
expression here is checked against **declared** schemas, so a guard reading an
output field the agent never declared is a compile error rather than a silent
`false` at run time.

The same code covers a `writes:` key that is not an output field of the node, an
`input:` binding naming a field the target does not declare, and a trigger
`input:` binding naming a field the flow's `inputs:` does not declare.

## A spec that triggers it

```yaml triggers
version: "0.1"
provider.p:
  kind: anthropic
  api_key: ${K}
model.m:
  provider: provider.p
  id: some-model
agent.a:
  model: model.m
  prompt: Do the thing.
  input:
    text: { type: string }
  output:
    result: { type: string }
flow.f:
  outputs: {}
  nodes:
    n:
      agent: agent.a
      input: { text: "'x'" }
  edges:
    - { from: start, to: n }
    - { from: n, to: end, when: "n.output.verdict == 'approve'" }
```

## The fix

The `help:` line lists the declared fields. Either read one of those, or declare
the field you meant on the agent's `output:`.

If you want to branch on a decision, declare it as an **`enum`** — enum-typed
output fields are what routing exhaustiveness is computed over, and a `string`
gives the compiler nothing to check coverage against.

Grammar: `docs/grammar.md` §4.1, §8.0. Topics:
`agent-compose docs cel`, `agent-compose docs routing`.
