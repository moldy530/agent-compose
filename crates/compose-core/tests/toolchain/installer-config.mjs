// The two installer-configuration files, read back by the **parsers their own
// installers read them with** (`docs/grammar.md` §14.6, PRD resolved q59).
//
// Every other format this compiler emits has a gate that makes the tool which
// consumes it read the bytes: the `.ts` modules and `tsconfig.json` go through
// `tsc`, `manifest.json` is parsed by the worker that materialises an artifact.
// `bunfig.toml` and `.npmrc` had a byte-for-byte golden and a paragraph of prose
// — which is a human having checked once, the manual verification gate
// CLAUDE.md's validation strategy rules out. An emitted `bunfig.toml` Bun cannot
// parse, or an `.npmrc` whose keys an ini reader splits somewhere other than
// where the emitter meant them to, fails nothing but a golden comparison that
// the next contributor re-blesses.
//
// Two readers, because the two files have two installers:
//
//   * `bunfig` — `Bun.TOML.parse`, Bun's own TOML parser, over `bunfig.toml`.
//   * `npmrc` — the `ini` package **out of the npm installation on this
//     machine**, over `.npmrc`. npm reads its configuration files with that
//     package, so resolving it from npm's own tree is what makes this npm's
//     reader rather than a second implementation of one. It is resolved through
//     `npm root -g` and a `createRequire` rooted at npm's own entry point, and a
//     layout that stops answering is an error naming what it looked for rather
//     than a quiet fallback onto something else.
//
// Neither reader touches the network — both parse a file and print what they
// parsed — which is what keeps this inside PRD resolved q59 ruling e: the gates
// stay hermetic and CI never installs against a private registry.
//
// Usage: bun  installer-config.mjs bunfig <generated project directory>
//        node installer-config.mjs npmrc  <generated project directory>
// Output: { "read": { … } } — the file as that installer's own parser sees it.

import { execFileSync } from "node:child_process";
import fs from "node:fs";
import { createRequire } from "node:module";
import path from "node:path";
import process from "node:process";

const [, , reader, project] = process.argv;
if (reader === undefined || project === undefined) {
  throw new Error(
    "usage: installer-config.mjs <bunfig|npmrc> <generated project directory>",
  );
}

/** The `ini` package npm itself reads `.npmrc` with. */
function npmsOwnIni() {
  const root = execFileSync("npm", ["root", "-g"], { encoding: "utf8" }).trim();
  // npm's own entry point, which is what `createRequire` resolves `ini` from.
  // More than one spelling because the file has moved between major versions
  // and the point is to reach npm's bundled copy, not to know its layout.
  const entries = ["lib/cli.js", "index.js", "bin/npm-cli.js"];
  for (const entry of entries) {
    const candidate = path.join(root, "npm", entry);
    if (!fs.existsSync(candidate)) {
      continue;
    }
    try {
      return createRequire(candidate)("ini");
    } catch (cause) {
      throw new Error(`npm at ${candidate} does not bundle \`ini\``, { cause });
    }
  }
  throw new Error(
    `no npm entry point under ${root} answered to any of ${entries.join(", ")}`,
  );
}

function read(name) {
  return fs.readFileSync(path.resolve(project, name), "utf8");
}

let parsed;
switch (reader) {
  case "bunfig": {
    if (typeof Bun === "undefined") {
      throw new Error("the `bunfig` reader is Bun's own TOML parser; run it under bun");
    }
    parsed = Bun.TOML.parse(read("bunfig.toml"));
    break;
  }
  case "npmrc": {
    parsed = npmsOwnIni().parse(read(".npmrc"));
    break;
  }
  default: {
    throw new Error(`\`${reader}\` is not a reader: expected \`bunfig\` or \`npmrc\``);
  }
}

process.stdout.write(`${JSON.stringify({ read: parsed })}\n`);
