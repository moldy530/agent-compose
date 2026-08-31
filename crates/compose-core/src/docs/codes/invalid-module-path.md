# invalid-module-path

## What it protects

A `module:` binding names hand-authored TypeScript **inside the project**, and
that file travels: it ships in the artifact every worker fetches, hash-addressed
like everything else the build produced (`docs/distributed.md` §4). So its path
has one portable spelling, and six rules hold it there.

It is **project-relative**: `/`-separated segments, each `.`, `..` or a name
matching `[A-Za-z0-9_][A-Za-z0-9_.-]*`, the last ending in `.ts`. No leading
`/`, no backslashes, no whitespace, no URLs. An absolute path names a file on
the machine that ran `build` and nowhere else.

It names an **implementation**, so it is never a `.d.ts`. A declaration file
ends in `.ts` and holds no code — it states the types of a module written
somewhere else — while `src/modules.ts` imports a bound implementation for its
*value*, which the type checker refuses of a declaration outright. A binding on
one would have `build` scaffold executable code into a file that may hold none,
and emit a project that could never type-check whatever was there.

Every prefix stays **inside the project root** — the entrypoint's own directory.
A path that climbs out and returns is refused even where it lands back inside,
for the reason `imports:` refuses one: whether it re-enters the same project
depends on the checkout's parent directory, which the spec cannot see.

It **fits the artifact**: at most 100 bytes once normalized, because the tree is
served to a worker as a tar and that is what a ustar header holds
(`docs/distributed.md` §3.5). Every name the compiler emits is a short constant,
so an authored path is the only way to write an artifact no worker could fetch.

And it is **not a name `build` writes, nor inside one**. The emitted file list is
the boundary between generated and authored code: `build` overwrites exactly the
files it emits and touches nothing else, so a binding pointing at `src/graph.ts`
would be asking for an authored file the next build destroys — and one pointing
at `src/graph.ts/impl.ts` would need `src/graph.ts` to be a file and a directory
in the same tree, which is a build that fails partway through rather than a spec
that is refused.

Finally, **case does not make it a different name, and neither does nesting**.
`src/Graph.ts` and `src/graph.ts` are one file on macOS and on Windows, so a path
differing only in case from a name `build` emits is refused, and so are two
bindings differing only in case from each other — either would work on Linux and
quietly write one file over the other everywhere else. Two bindings where one
path sits inside the other are refused for the neighbouring reason: no checkout
holds a name that is a file for one tool and a directory for another.

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
