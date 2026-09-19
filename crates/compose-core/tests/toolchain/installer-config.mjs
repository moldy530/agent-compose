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
// The `npmrc` reader answers a second question out of that same tree, and it is
// the one a parse cannot: **which credential npm finds** for a package fetched
// from each registry the file configures. A parse only says the `_authToken`
// line survived as one key — it cannot say the key is an address npm's own
// lookup walks, because the address is derived rather than written anywhere, so
// a reader that recomputed it with the compiler's own function would agree with
// whatever the compiler wrote. Instead this walks npm's real path: `pacote`'s
// packument URL for a package in that registry (`pickRegistry`, then
// `removeTrailingSlashes(registry)/<escaped name>`), handed to
// `npm-registry-fetch`'s `getAuth` — the function every `npm install` spends a
// token through. The token it comes back with is npm's answer, not the
// compiler's.
//
// No reader touches the network — they parse a file, build a URL as a string,
// and print what they found — which is what keeps this inside PRD resolved q59
// ruling e: the gates stay hermetic and CI never installs against a private
// registry.
//
// Usage: bun  installer-config.mjs bunfig <generated project directory>
//        node installer-config.mjs npmrc  <generated project directory>
// Output: { "read": { … } } — the file as that installer's own parser sees it —
//         and, for `npmrc`, { "auth": { "<registry key>": { registry, token } } }
//         — what npm's own credential lookup finds for each of them, with a
//         `token` of `null` where it finds none.

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

/**
 * The package the credential probe pretends to install.
 *
 * Any name does: npm walks **up** the address of its request looking for a
 * credential, so the last segment is the first thing the walk drops. It is a
 * real package name only so the URL the walk starts from is one npm could
 * really have built.
 */
const PROBE = "lodash";

/**
 * `require`, rooted at the npm installation on this machine.
 *
 * Every module below comes from npm's own tree rather than from the fixture's
 * `package.json`: a second copy of `ini` is a second parser, and a second copy
 * of `npm-registry-fetch` is a second answer to the question this file exists to
 * ask npm.
 */
function npmsOwn() {
  const root = execFileSync("npm", ["root", "-g"], { encoding: "utf8" }).trim();
  // npm's own entry point, which is what `createRequire` resolves npm's bundled
  // packages from. More than one spelling because the file has moved between
  // major versions and the point is to reach that copy, not to know its layout.
  const entries = ["lib/cli.js", "index.js", "bin/npm-cli.js"];
  for (const entry of entries) {
    const candidate = path.join(root, "npm", entry);
    if (!fs.existsSync(candidate)) {
      continue;
    }
    const resolve = createRequire(candidate);
    return (specifier) => {
      try {
        return resolve(specifier);
      } catch (cause) {
        throw new Error(`npm at ${candidate} does not bundle \`${specifier}\``, {
          cause,
        });
      }
    };
  }
  throw new Error(
    `no npm entry point under ${root} answered to any of ${entries.join(", ")}`,
  );
}

/**
 * What npm's own credential lookup finds for each registry an `.npmrc`
 * configures: `{ "<registry key>": { registry, token } }`, keyed by the line the
 * registry was configured on (`registry`, `@scope:registry`).
 *
 * `registry` is the address npm would fetch from — `pickRegistry`'s answer,
 * reported so a probe that fell through to the public default is visible rather
 * than being read as a verdict about the mirror. `token` is the credential
 * `getAuth` walks up to, or `null` where it walks the whole address and finds
 * none.
 *
 * It is the **reference** rather than a secret: npm's config layer expands
 * `${VAR}` before `getAuth` ever sees a value, and this stops one step short of
 * that on purpose, because the claim under test is about what the artifact
 * carries (PRD resolved q59: no secret enters an artifact).
 *
 * The `registry` line is withheld from the options `getAuth` reads. npm falls
 * back to the default registry's credential for any request to the same host —
 * a real behaviour, meant for a tarball served beside a registry — and it would
 * mask exactly the failure this is written against: a scope entry whose
 * `_authToken` is keyed where npm never walks would come back holding the
 * *default* entry's token and look authenticated. Each entry is therefore asked
 * about on its own.
 */
function credentials(config, fromNpm) {
  const { pickRegistry } = fromNpm("npm-registry-fetch");
  const getAuth = fromNpm("npm-registry-fetch/lib/auth.js");
  const removeTrailingSlashes = fromNpm("pacote/lib/util/trailing-slashes.js");
  const npa = fromNpm("npm-package-arg");
  // Each of those is an export npm's own installer calls, and a release that
  // moved one says so here rather than answering every registry `null` — which
  // would read as a compiler that keyed its credentials wrong.
  for (const [name, found] of Object.entries({
    pickRegistry,
    getAuth,
    removeTrailingSlashes,
    npa,
  })) {
    if (typeof found !== "function") {
      throw new Error(`npm's \`${name}\` is not a function: its own layout moved`);
    }
  }

  const walked = { ...config };
  delete walked.registry;

  const found = {};
  for (const key of Object.keys(config)) {
    let scope;
    if (key === "registry") {
      scope = null;
    } else if (key.endsWith(":registry")) {
      scope = key.slice(0, -":registry".length);
    } else {
      continue;
    }
    const spec = scope === null ? PROBE : `${scope}/${PROBE}`;
    const registry = pickRegistry(spec, config);
    // `pacote`'s own packument URL, which is the request every install of that
    // package starts with and the address `getAuth` walks up from.
    const uri = `${removeTrailingSlashes(registry)}/${npa(spec).escapedName}`;
    found[key] = { registry, token: getAuth(uri, walked).token ?? null };
  }
  return found;
}

function read(name) {
  return fs.readFileSync(path.resolve(project, name), "utf8");
}

const answer = {};
switch (reader) {
  case "bunfig": {
    if (typeof Bun === "undefined") {
      throw new Error("the `bunfig` reader is Bun's own TOML parser; run it under bun");
    }
    answer.read = Bun.TOML.parse(read("bunfig.toml"));
    break;
  }
  case "npmrc": {
    const fromNpm = npmsOwn();
    answer.read = fromNpm("ini").parse(read(".npmrc"));
    answer.auth = credentials(answer.read, fromNpm);
    break;
  }
  default: {
    throw new Error(`\`${reader}\` is not a reader: expected \`bunfig\` or \`npmrc\``);
  }
}

process.stdout.write(`${JSON.stringify(answer)}\n`);
