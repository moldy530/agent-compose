# invalid-module-path

## What it protects

A `module:` binding names hand-authored TypeScript **inside the project**, and
that file travels: it ships in the artifact every worker fetches, hash-addressed
like everything else the build produced (`docs/distributed.md` §4). So its path
has one portable spelling, and three rules hold it there.

It is **project-relative**: `/`-separated segments, each `.`, `..` or a name
matching `[A-Za-z0-9_][A-Za-z0-9_.-]*`, the last ending in `.ts`. No leading
`/`, no backslashes, no whitespace, no URLs. An absolute path names a file on
the machine that ran `build` and nowhere else.

Every prefix stays **inside the project root** — the entrypoint's own directory.
A path that climbs out and returns is refused even where it lands back inside,
for the reason `imports:` refuses one: whether it re-enters the same project
depends on the checkout's parent directory, which the spec cannot see.

And it is **not a name `build` writes**. The emitted file list is the
boundary between generated and authored code: `build` overwrites exactly the
files it emits and touches nothing else, so a binding pointing at `src/graph.ts`
would be asking for an authored file the next build destroys.

## A spec that triggers it

```yaml triggers
version: "0.1"

tool.sign:
  description: Sign a payload.
  input:
    payload: { type: string }
  output:
    signature: { type: string }
  module: /opt/tools/sign.ts
```

## The fix

Write the in-project spelling, under a name the compiler does not emit:
`module: ./src/tools/sign.ts`. `src/tools/` is the conventional place — it is
where `build` scaffolds a missing implementation — but any project-relative
`.ts` path outside the generated file list works. For code genuinely outside the
project, publish it as a package and `import` it from a module here, declaring
it under the binding's `dependencies:`.

Grammar: `docs/grammar.md` §6.1, Decision D132. PRD: resolved q47, q48. Topic:
`agent-compose docs tools`.
