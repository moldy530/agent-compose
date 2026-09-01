# invalid-dependency

## What it protects

A `module:` binding declares what its TypeScript imports, and `build` folds
those declarations into the generated `package.json`. The artifact ships **no
lockfile** — a lockfile is what an install produces, not what a build writes —
so the pin in the spec is the only thing that makes the hub's `bun install` and
every worker's resolve the same tree (`docs/distributed.md` §4).

That gives three rules. A key is an **npm package name**: at most 214 characters
of lowercase letters, digits, `-`, `_` and `.`, optionally scoped as
`@scope/name`, starting with none of `.`, `_` or `-`. A value is an **exact
version**: `MAJOR.MINOR.PATCH`, with an optional `-prerelease` and `+build`, each
tail a `.`-separated list of non-empty identifiers as semver writes them.
`^1.2.3`, `~1.2.3`, `>=1`, `1.x`, `*`, `latest`, and every `git`, `file`, `npm`
and `workspace` specifier are refused — each of them resolves to a different
tree on a different day or on a different machine — and so are the degenerate
spellings `npm` itself rejects, like `1.2.3-` or `1.2.3-+`, which would otherwise
fail at the install rather than here.

And the pin has to be **the only one**. One `package.json` holds one version of
a package, so a module pinning a package the generated project already pins, at
another version, is refused naming both; so are two tools pinning one package at
two versions, naming both tools.

## A spec that triggers it

```yaml triggers
version: "0.1"

tool.sign:
  description: Sign a payload.
  input:
    payload: { type: string }
  output:
    signature: { type: string }
  module:
    path: ./src/tools/sign.ts
    dependencies:
      "@noble/hashes": "^1.4.0"
```

## The fix

Write the exact version you want installed — `"@noble/hashes": "1.4.0"` — and
where two declarations disagree, make them agree. A package the generated
runtime already pins takes that release's version or nothing: the generated
`package.json` is pure-generated, and a build cannot hold two answers for one
name.

Grammar: `docs/grammar.md` §6.1, Decision D133. PRD: resolved q49. Topic:
`agent-compose docs tools`.
