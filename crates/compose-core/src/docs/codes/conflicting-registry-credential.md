# conflicting-registry-credential

## What it protects

Two `package_registry` entries that authenticate to **one address** with two
different environment variables.

A target that declares `package_registry:` gets two generated files beside
`package.json`: a `bunfig.toml` for Bun, the default installer, and an `.npmrc`
for the npm fallback. The two carry a credential in incompatible ways. Bun's
registry object holds its own `token = "$VAR"` per entry, keyed by nothing. npm's
is keyed by an **address** — `//host/path/:_authToken=${VAR}` — so two entries
whose `url:`s derive one address write that one key twice, and an ini parser
keeps the last line it read.

One artifact would then authenticate two different ways depending on which
installer ran it: Bun sends each entry's own variable, npm sends whichever
entry's happened to be written last. Which of them is right is a question only
the author can answer, so `validate` asks it rather than picking.

Neither value is wrong on its own, which is why this is not an `invalid-value`:
both `url:`s are legal addresses and both `${VAR}`s are legal references, and
what is refused is the **pair**. It is the shape
`conflicting-connection-variable` was minted for one layer up — two sources
writing one name, where a silent pick is a deployment nobody was told about.

**The address is derived, not compared as text.** npm never matches an
`_authToken` key against the text of a `registry=` line; it builds the URI it is
about to fetch — the registry with one trailing `/` stripped, then `/<package>` —
parses it through a WHATWG `URL`, and looks the key up from that. So this rule
compares the same derived keys: the host folds to lowercase, the scheme's own
default port drops away, dot segments resolve, and the registry's whole path is
kept with a trailing `/`. `https://NPM.Internal.Example/repo`,
`https://npm.internal.example/repo` and `https://npm.internal.example/repo/` are
therefore **one address**, and a collision between them is refused even though no
two lines in the file look alike.

**What this is not.** One variable written at one address twice is fine — writing
a line twice says what writing it once said, and both installers agree. An entry
with no `token:` under a tokened one is the other half of §14.6 rule 5 and has
its own code, `missing-registry-token`. A credential written into the `url:`
itself — `https://user:pass@host/` — is `invalid-value`, refused by §14.6 rule 1
before this rule is reached.

## A spec that triggers it

The composition is beside the point; the deploy file is what refuses:

```yaml triggers
version: "0.1"

provider.vendor:
  kind: openai
  api_key: ${OPENAI_API_KEY}

model.smart:
  provider: provider.vendor
  id: gpt-4o-mini

agent.embedder:
  model: model.smart
  prompt: Summarize the document for indexing.
  input:
    text: { type: string }
  output:
    summary: { type: string }
```

```yaml deploy mirror
version: "0.1"

package_registry:
  url: "https://npm.internal.example/repository/npm-group/"
  token: ${NPM_MIRROR_TOKEN}
  scopes:
    "@corp":
      url: "https://npm.internal.example/repository/npm-group"
      token: ${NPM_CORP_TOKEN}
```

The two URLs differ by a trailing `/` and nothing else, which is exactly the
spelling that makes this worth refusing: the file reads like two registries and
the emitted `.npmrc` holds one key.

## The fix

**Give each entry its own path on the mirror.** This is the usual repair, because
it is usually what the operator meant: a group repository for everything and a
hosted repository for the scope, which are two addresses and take two
credentials.

```yaml deploy split
version: "0.1"

package_registry:
  url: "https://npm.internal.example/repository/npm-group/"
  token: ${NPM_MIRROR_TOKEN}
  scopes:
    "@corp":
      url: "https://npm.internal.example/repository/corp/"
      token: ${NPM_CORP_TOKEN}
```

**Or give both the same variable.** If the two entries really are one registry —
the scope resolves from the same place as everything else, and only the
`@corp:registry=` line differs — then one credential is the honest description,
and both files then say the same thing.

```yaml deploy shared
version: "0.1"

package_registry:
  url: "https://npm.internal.example/repository/npm-group/"
  token: ${NPM_MIRROR_TOKEN}
  scopes:
    "@corp":
      url: "https://npm.internal.example/repository/npm-group/"
      token: ${NPM_MIRROR_TOKEN}
```

Grammar: `docs/grammar.md` §14.6, §4.3, Decision D145. Topic:
`agent-compose docs targets`.
