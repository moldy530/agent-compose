# missing-registry-token

## What it protects

A `package_registry` entry that declares no `token:` at an address **at or
under** a tokened entry's.

npm does not scope a credential to a package scope at all. It writes one keyed by
an **address** — `//host/path/:_authToken=${VAR}` — and it finds one by walking
*up* the address of the request it is making, so a fetch under
`//npm.internal.example/repository/corp/` falls back to
`//npm.internal.example/repository/`, and then to `//npm.internal.example/`. Bun
does neither: a `[install.scopes]` entry carries its own `token =` or none, and
nothing is keyed by an address.

So an entry written **without** a credential, underneath one that has one, is two
different deployments out of one artifact. `npm install` walks up, finds the
parent's line and spends that token at a registry this file scoped it away from;
`bun install` in the same directory sends nothing and takes a `401`. A credential
leaving for an address the author never pointed it at is the worse of the two
directions §14.6 rule 5 refuses — the other, two variables at one address, is
`conflicting-registry-credential`.

Writing one entry to make a key required in another is rare here but not unique:
`callback_auth:` makes `callback_allow:` mandatory
(`missing-callback-allowlist`), `placements:` make `hub.join_token:` mandatory
(`missing-join-token`), and a provider reaching a vendor's own endpoint needs
`api_key:` or a gateway's `base_url:` (`missing-credential`). What they share is
the shape of the failure, and it is why this is not a plain `missing-key`:
whether the key is required is decided by a **sibling**, and the repair is a
choice of two.

**The address is the derived one, not the text.** npm builds the URI it fetches —
the registry with one trailing `/` stripped, then `/<package>` — parses it
through a WHATWG `URL` and walks up *that*. So the host folds to lowercase, the
scheme's own default port drops away, dot segments resolve, and a trailing `/`
makes no difference: `https://NPM.Internal.Example/repository` and
`https://npm.internal.example/repository/` are one address here, and an entry
under either is under both.

**What this is not.** A `token:` written as a literal string instead of an
`${ENV}` reference is `invalid-env-ref`. A `url:` spelled in a way that parse
would rewrite — a query, a fragment, a non-ASCII host, a renumbered port — is
`invalid-value`, refused at the `url:` by §14.6 rule 1. And two shapes stay
perfectly legal: an entry **beside** the tokened one rather than under it, and a
mirror nobody authenticates to at all, where no `token:` is declared anywhere and
nothing walks up to anything.

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
  url: "https://npm.internal.example/repository/"
  token: ${NPM_MIRROR_TOKEN}
  scopes:
    "@corp":
      url: "https://npm.internal.example/repository/corp/"
```

`@corp` sits inside the mirror's own directory, so npm spends
`NPM_MIRROR_TOKEN` there and Bun spends nothing.

## The fix

**Give the entry the token it should spend.** If the scope really is meant to
authenticate — and an entry inside a credentialed mirror usually is — say so, and
both installers then send the same thing. The two entries keep their own
variables, because their addresses are different.

```yaml deploy scoped
version: "0.1"

package_registry:
  url: "https://npm.internal.example/repository/"
  token: ${NPM_MIRROR_TOKEN}
  scopes:
    "@corp":
      url: "https://npm.internal.example/repository/corp/"
      token: ${NPM_CORP_TOKEN}
```

**Or move it to an address that is not under the other's.** If the scope is
genuinely read through without a credential, give it a path the walk-up never
reaches — beside the mirror rather than inside it. Nothing then falls back onto
`NPM_MIRROR_TOKEN`, and both installers go out unauthenticated for `@corp`, which
is what the file says.

```yaml deploy beside
version: "0.1"

package_registry:
  url: "https://npm.internal.example/repository/"
  token: ${NPM_MIRROR_TOKEN}
  scopes:
    "@corp":
      url: "https://npm.internal.example/corp/"
```

Grammar: `docs/grammar.md` §14.6, §4.3, Decision D145. Topic:
`agent-compose docs targets`.
