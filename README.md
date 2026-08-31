# agent-compose

A declarative YAML DSL for agent graphs, compiled to a LangGraph TypeScript
project. You author components — models, agents, tools, flows — and a single
static binary checks that the graph they form can actually run, before any of it
does: reference and schema resolution across every edge, routing exhaustiveness,
cycle termination, fan-out bounds, and a few dozen other things.

The compiler is one Rust binary with no runtime dependency. A JavaScript runtime
(Bun, or Node >= 22.18) is needed only to *run* what it emits.

## Install

```sh
curl -fsSLO https://raw.githubusercontent.com/moldy530/agent-compose/main/install.sh
sh install.sh
```

That picks the artifact for this machine, checks it against the release's
`SHA256SUMS`, and installs it into `$HOME/.local/bin` — printing a `PATH` hint if
that directory is not on yours. **A tarball whose checksum does not match is
never installed.**

> **Until the first release.** Those URLs resolve once this repository has a
> published release *and* is publicly readable. Today it is neither — it is
> private and has cut no release — so `curl` answers `404`. Until then there are
> two routes: build from source (`cargo build --release -p agent-compose`, and
> the [section below](#build-from-source)), or take a tarball from the Actions
> tab, where the *Release dry run* uploads the two Linux archives on a pull
> request; all four from a manual *Release dry run* (`workflow_dispatch`).

Two things it takes:

```sh
# a particular release rather than the latest
sh install.sh 0.2.0

# somewhere else to put it
AGENT_COMPOSE_INSTALL="$HOME/bin" sh install.sh
```

Name a directory you can write to. A system one like `/usr/local/bin` needs
`sudo`, and `sudo` does not carry that variable — run
`sudo env AGENT_COMPOSE_INSTALL=/usr/local/bin sh install.sh`.

Two commands rather than `curl … | sh`, for two reasons. A pipe throws `curl`'s
exit status away: a URL that answers `404` pipes an empty body into a shell,
which runs the nothing it was given and exits `0` — no compiler installed, no
failure reported. And a script about to run as you is worth reading first. It is
[`install.sh`](install.sh) in this repository, and it is the same file the
release pipeline runs against freshly built artifacts on every pull request.

### From the tarball

Every release carries four archives and one `SHA256SUMS` over all of them:

| target | archive |
|---|---|
| Linux x86_64 | `agent-compose-<version>-x86_64-unknown-linux-musl.tar.gz` |
| Linux arm64 | `agent-compose-<version>-aarch64-unknown-linux-musl.tar.gz` |
| macOS Intel | `agent-compose-<version>-x86_64-apple-darwin.tar.gz` |
| macOS Apple silicon | `agent-compose-<version>-aarch64-apple-darwin.tar.gz` |

The Linux binaries are statically linked against musl, so they run on an alpine
container and on an old glibc alike.

```sh
version=0.2.0
target=x86_64-unknown-linux-musl
base="https://github.com/moldy530/agent-compose/releases/download/v$version"

curl -fsSLO "$base/agent-compose-$version-$target.tar.gz"
curl -fsSLO "$base/SHA256SUMS"

# macOS: shasum -a 256 -c -
grep "agent-compose-$version-$target.tar.gz" SHA256SUMS | sha256sum -c -

tar -xzf "agent-compose-$version-$target.tar.gz"
mv agent-compose ~/.local/bin/
```

Check the checksum before unpacking, not after. The `grep` is what narrows one
`SHA256SUMS` covering four archives to the one you downloaded.

### Verify

```sh
agent-compose --version
```

It prints the release, the commit it was built from, and the target it was built
for — `agent-compose 0.2.0 (a1b2c3d, x86_64-unknown-linux-musl)` — which is what
a bug report should carry. If the command is not found, the install directory is
not on your `PATH`.

## Start

```sh
agent-compose init hello          # one commented main.yml that already validates
agent-compose validate hello/main.yml
agent-compose docs                # the topic index — the binary teaches its grammar
agent-compose build hello/main.yml
agent-compose run hello/main.yml flow.summarize --input document="some text"
```

`flow.summarize` is the flow the scaffold writes; `run` needs a key for the
provider it names, and `validate` and `build` need none.

`agent-compose docs <topic>` is the curriculum, and it ships inside the binary:
nothing to look up, and always in step with the compiler you are holding.
`agent-compose explain <code>` expands any diagnostic code a report carried.

### Code you write by hand

Most of a composition is declarative, and where it is not, a `tool.*` can bind a
TypeScript file in the emitted project:

```yaml
tool.sign:
  description: Sign a payload.
  input:  { payload: { type: string } }
  output: { signature: { type: string } }
  module: ./src/tools/sign.ts
```

`build` writes that file **once** — the typed signature, the contract as a doc
comment, a body that throws — and never writes it again. It overwrites and
`--check`s only the files it emits, so your implementation sits in the same tree
with no marker comments and nothing to merge; the tool's declared `input:` and
`output:` are the contract, and `tsc` is what holds you to it. See
`agent-compose docs tools`.

### With a coding agent

```sh
agent-compose skill --agent claude
```

writes the packaged skill into the agent's own skills directory (`--agent codex`
for Codex, `--global` to install under `$HOME`). It teaches the loop — `init`,
edit, `validate`, `explain`, `docs`, `plan`, `build`/`run` — rather than the
grammar, which the agent reads from the binary as it goes.

## Build from source

Requires the Rust toolchain pinned in `rust-toolchain.toml`; nothing else.

```sh
cargo build --release -p agent-compose
cargo test --workspace          # needs Bun and Node for the generated-code gates
```

`prd.md` is the source of truth for design decisions, `docs/grammar.md` is
normative for the language, and `CLAUDE.md` describes how this repository is
worked on.
