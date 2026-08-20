# missing-binding

## What it protects

An input field has no binding, no name-based source, and no `default:`. The
compiler will not invent a value, and it will not run a node whose input is
half-built.

Where a field may come from depends on the target. For **in-flow** targets —
`agent:`, `exec:`, `http:`, `function:`, `human:` — the chain is an explicit
`input:` binding, then a state channel of the same name, then an enclosing flow
input of the same name, then the field's own `default:`. For **module-boundary**
targets — a `flow:` node and a `map` dispatch — the middle two steps **do not
apply**: bindings are total, and nothing falls through a module boundary
implicitly.

A string-in agent is the other half of this code: it declares no `input:`, takes
one unnamed string, and its node must supply it with the scalar form.

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
  output:
    result: { type: string }
flow.f:
  outputs: {}
  nodes:
    n:
      agent: agent.a
  edges:
    - { from: start, to: n }
    - { from: n, to: end }
```

## The fix

Bind the field. For the string-in agent above, that is the scalar form:

```yaml
n: { agent: agent.a, input: "'the thing to do'" }
```

For a `flow:` node reporting an unbound field, write the binding out — even
where the caller happens to have a channel of that name, which is exactly the
case the rule refuses to guess about. For an in-flow target, declaring a state
channel of the same name also satisfies it.

Grammar: `docs/grammar.md` §5.3, §8.0, Decisions D14, D68. Topics:
`agent-compose docs state`, `agent-compose docs agents`.
