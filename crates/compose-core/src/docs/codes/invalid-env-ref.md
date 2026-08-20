# invalid-env-ref

## What it protects

**The spec never contains credentials.** Secret-bearing and connection-bearing
fields take the env-ref *value* form only — the whole string is exactly one
`${NAME}` reference — and a literal there is refused.

The field list is classified by name wherever it occurs: `api_key`,
`api_secret`, `token`, `password`, `access_key_id`, `secret_access_key`,
`session_token`, `credentials_json`, `url`, `base_url`, `endpoint`, `dsn`.

This is what makes a spec committable and an artifact portable. A build succeeds
in CI holding no secrets, because env refs survive **unresolved** into the
artifact and presence is checked at process start instead.

The same code covers a malformed reference: an env name is upper-case letters,
digits and underscores, starting with a letter or an underscore.

## A spec that triggers it

```yaml triggers
version: "0.1"
provider.anthropic:
  kind: anthropic
  api_key: sk-ant-not-a-reference
```

## The fix

Write the reference:

```yaml
provider.anthropic:
  kind: anthropic
  api_key: ${ANTHROPIC_API_KEY}
```

Quote it when it appears inside `{ … }` or `[ … ]` — YAML forbids the braces in
a plain scalar in flow context. Quoting is always legal, so quoting every
reference is the safe habit.

Grammar: `docs/grammar.md` §4.3, Decision D41. Topics:
`agent-compose docs cel`, `agent-compose docs models`.
