# invalid-identifier

## What it protects

One identifier class covers every name in this grammar: definition names, node
ids, state channel names, schema field names, variant tags, trigger names,
backend aliases, and a `map`'s `as:` binding. **1–64 characters of lowercase
letters, digits and `_`, starting with a letter.**

One class means one rule and one message. It also survives every codegen target
without mangling, which matters because these names become TypeScript
identifiers, environment variable names, and JSON keys.

## A spec that triggers it

```yaml triggers
version: "0.1"
agent.Reviewer:
  model: model.smart
  prompt: Review the draft.
  output:
    verdict: { enum: [approve, revise] }
```

## The fix

Lowercase it and replace anything that is not a letter, digit or underscore:
`agent.reviewer`. Names are case-sensitive and only lowercase is legal, so
`Reviewer` and `reviewer` are not two spellings of one name — one of them is
not a name.

Grammar: `docs/grammar.md` §2.1, Decision D5. Topic:
`agent-compose docs getting-started`.
