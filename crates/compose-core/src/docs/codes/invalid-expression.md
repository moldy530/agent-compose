# invalid-expression

## What it protects

A CEL expression that does not parse, or that uses something outside the
supported surface. The message carries the CEL parser's own account of where it
went wrong.

The supported surface is standard CEL over the roots the position exposes:
comparisons, boolean logic, arithmetic, indexing, `in`, `size()`, `has()`,
`startsWith`/`endsWith`/`contains`/`matches`, and the comprehension macros
`all`, `exists`, `exists_one`, `filter`, `map`. There are no custom extension
functions.

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
      input: { text: "'x' +" }
  edges:
    - { from: start, to: n }
    - { from: n, to: end }
```

## The fix

Finish the expression. Two shapes account for most of these: an operator with
nothing after it, and a string literal quoted with the wrong quotes for the
surrounding YAML.

Remember a CEL value is always a **YAML string**, so a bare literal needs its
own quotes inside the YAML ones: `input: { text: "'a literal'" }`.

Grammar: `docs/grammar.md` §4.1. Topic: `agent-compose docs cel`.
