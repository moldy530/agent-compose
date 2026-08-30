// The artifact's content hash, computed by the **generated project's own**
// implementation of it (`docs/distributed.md` §3.5, §4).
//
// A hash over a tree has to mean the same thing at both ends of the worker wire:
// the compiler writes it into `src/artifact.ts` when it emits, and a worker
// re-derives it from the entries it unpacked before materialising them (§4
// step 2). Two implementations of one rule is exactly the shape the CEL corpus
// exists for, so this is the same discipline applied to the other one: the Rust
// side hashes the emitted files, this hashes the files on disk with `contentHash`
// out of the project's own `src/mesh.ts`, and the gate compares the two against
// the constant the emitter wrote.
//
// The file list is `ARTIFACT_FILES` rather than a directory walk, because that
// is what the hub tars: what is being checked is that the *served* set hashes to
// the *declared* hash, not that a directory happens to.
//
// Usage: node artifact-content-hash.mjs <generated project directory>
// Output: { "declared": "sha256:…", "computed": "sha256:…", "files": [ … ] }

import fs from "node:fs";
import path from "node:path";
import process from "node:process";
import { pathToFileURL } from "node:url";

const [, , project] = process.argv;
if (project === undefined) {
  throw new Error("usage: node artifact-content-hash.mjs <generated project directory>");
}

const artifact = await import(pathToFileURL(path.resolve(project, "src/artifact.ts")).href);
const mesh = await import(pathToFileURL(path.resolve(project, "src/mesh.ts")).href);

const files = new Map();
for (const relative of artifact.ARTIFACT_FILES) {
  files.set(relative, fs.readFileSync(path.resolve(project, relative)));
}

process.stdout.write(
  `${JSON.stringify({
    declared: artifact.ARTIFACT_HASH,
    computed: mesh.contentHash(files),
    files: [...files.keys()],
  })}\n`,
);
