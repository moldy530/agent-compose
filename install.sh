#!/bin/sh
# Install the `agent-compose` binary from a GitHub release.
#
# The compiler is one static executable with no runtime dependencies (PRD 5.12),
# so installing it is: pick the artifact this machine can run, check it against
# the release's SHA256SUMS, and put it somewhere on PATH. **A tarball whose
# checksum does not match the one the release published is never installed** —
# that is the whole reason this script exists rather than a `curl … | tar -xz`
# line in the README.
#
# Usage:
#
#   curl -fsSL https://raw.githubusercontent.com/moldy530/agent-compose/main/install.sh | sh
#   curl -fsSL https://raw.githubusercontent.com/moldy530/agent-compose/main/install.sh | sh -s -- 0.1.0
#   sh install.sh [<version>]
#
# The version may be given as the first argument or as AGENT_COMPOSE_VERSION,
# with or without a leading `v`; the argument wins, and with neither the latest
# release is installed.
#
# Environment:
#
#   AGENT_COMPOSE_VERSION       the release to install (default: the latest)
#   AGENT_COMPOSE_INSTALL       where to put the binary
#                               (default: $HOME/.local/bin)
#   AGENT_COMPOSE_ARTIFACT_DIR  install from a directory holding built tarballs
#                               and their SHA256SUMS instead of from GitHub.
#                               This is what the release pipeline's dry-run gate
#                               runs on every pull request, so the script that
#                               ships is the script CI exercised — detection,
#                               checksum verification, install and `--version`,
#                               against artifacts built minutes earlier.
#
# POSIX sh on purpose: this runs before anything is installed, on whatever shell
# the machine has.

set -eu

ac_repo="moldy530/agent-compose"
ac_bin="agent-compose"
ac_sums="SHA256SUMS"

ac_die() {
  printf 'install %s: %s\n' "$ac_bin" "$1" >&2
  exit 1
}

ac_have() {
  command -v "$1" >/dev/null 2>&1
}

# The version, before anything else touches the positional parameters.
ac_version="${1:-${AGENT_COMPOSE_VERSION:-}}"
ac_version="${ac_version#v}"
ac_artifacts="${AGENT_COMPOSE_ARTIFACT_DIR:-}"

# --- which artifact this machine wants ---------------------------------------

ac_system="$(uname -s)"
case "$ac_system" in
  Linux) ac_platform="unknown-linux-musl" ;;
  Darwin) ac_platform="apple-darwin" ;;
  *) ac_die "no released binary for \`$ac_system\` — releases carry Linux and macOS. Build from source instead: \`cargo install --git https://github.com/$ac_repo agent-compose\`" ;;
esac

ac_machine="$(uname -m)"
case "$ac_machine" in
  x86_64 | amd64) ac_arch="x86_64" ;;
  aarch64 | arm64) ac_arch="aarch64" ;;
  *) ac_die "no released binary for \`$ac_machine\` — releases carry x86_64 and aarch64" ;;
esac

ac_target="$ac_arch-$ac_platform"

# --- the tools this needs ----------------------------------------------------

if ac_have sha256sum; then
  ac_sha="sha256sum"
elif ac_have shasum; then
  ac_sha="shasum"
else
  ac_die "neither \`sha256sum\` nor \`shasum\` is on PATH, so the download cannot be verified"
fi

ac_checksum() {
  case "$ac_sha" in
    sha256sum) sha256sum "$1" ;;
    *) shasum -a 256 "$1" ;;
  esac | cut -d ' ' -f 1
}

if [ -n "$ac_artifacts" ]; then
  ac_get="local"
elif ac_have curl; then
  ac_get="curl"
elif ac_have wget; then
  ac_get="wget"
else
  ac_die "neither \`curl\` nor \`wget\` is on PATH"
fi

# Fetch <url> to <path>, or copy <basename> out of the artifact directory.
ac_fetch() {
  case "$ac_get" in
    curl) curl -fsSL --retry 3 -o "$2" "$1" ;;
    wget) wget -q -O "$2" "$1" ;;
    local) cp "$ac_artifacts/$1" "$2" ;;
  esac
}

# --- which release ------------------------------------------------------------

if [ -z "$ac_version" ] && [ "$ac_get" = "local" ]; then
  # One directory of artifacts is one build, so the version is written on the
  # tarball. Two of them would make "the latest" a guess, and this script does
  # not guess about what it installs.
  set -- "$ac_artifacts/$ac_bin-"*"-$ac_target.tar.gz"
  # A glob that matched nothing is left standing as itself, which is one word
  # naming a file that is not there rather than no words at all.
  if [ "$#" -eq 1 ] && [ ! -f "$1" ]; then
    set --
  fi
  if [ "$#" -ne 1 ]; then
    ac_die "\`$ac_artifacts\` does not hold exactly one \`$ac_bin-<version>-$ac_target.tar.gz\` (found $#). Name the version to install"
  fi
  ac_version="$(basename "$1")"
  ac_version="${ac_version#"$ac_bin-"}"
  ac_version="${ac_version%"-$ac_target.tar.gz"}"
fi

if [ -z "$ac_version" ]; then
  ac_latest="$(
    case "$ac_get" in
      curl) curl -fsSL "https://api.github.com/repos/$ac_repo/releases/latest" ;;
      wget) wget -qO- "https://api.github.com/repos/$ac_repo/releases/latest" ;;
    esac | sed -n 's/.*"tag_name" *: *"\([^"]*\)".*/\1/p' | head -n 1
  )" || ac_die "could not reach the GitHub releases API. Name a version to install instead: \`AGENT_COMPOSE_VERSION=<version> sh install.sh\`"
  ac_version="${ac_latest#v}"
  [ -n "$ac_version" ] || ac_die "the GitHub releases API named no release. Name a version to install instead: \`AGENT_COMPOSE_VERSION=<version> sh install.sh\`"
fi

ac_archive="$ac_bin-$ac_version-$ac_target.tar.gz"

# --- download and verify ------------------------------------------------------

ac_tmp="$(mktemp -d 2>/dev/null || mktemp -d -t agent-compose)"
trap 'rm -rf "$ac_tmp"' EXIT INT TERM

if [ "$ac_get" = "local" ]; then
  [ -f "$ac_artifacts/$ac_archive" ] ||
    ac_die "\`$ac_artifacts\` does not hold \`$ac_archive\`"
  [ -f "$ac_artifacts/$ac_sums" ] ||
    ac_die "\`$ac_artifacts\` does not hold \`$ac_sums\`, so the archive cannot be verified"
  printf 'taking %s from %s\n' "$ac_archive" "$ac_artifacts"
  ac_fetch "$ac_archive" "$ac_tmp/$ac_archive" ||
    ac_die "could not copy \`$ac_archive\` out of \`$ac_artifacts\`"
  ac_fetch "$ac_sums" "$ac_tmp/$ac_sums" ||
    ac_die "could not copy \`$ac_sums\` out of \`$ac_artifacts\`"
else
  ac_from="https://github.com/$ac_repo/releases/download/v$ac_version"
  printf 'downloading %s\n' "$ac_from/$ac_archive"
  ac_fetch "$ac_from/$ac_archive" "$ac_tmp/$ac_archive" ||
    ac_die "could not download \`$ac_archive\` from the \`v$ac_version\` release. Check the version, and that this release carries a \`$ac_target\` binary"
  ac_fetch "$ac_from/$ac_sums" "$ac_tmp/$ac_sums" ||
    ac_die "could not download \`$ac_sums\` from the \`v$ac_version\` release, so the archive cannot be verified"
fi

# The name is matched as a whole field rather than searched for, so a checksum
# line for a *different* artifact can never be read as this one's. The two
# rewrites ahead of the comparison are the spellings the tools that write these
# files use: `*name` marks binary mode, and `./name` is what a glob that opened
# with `./` leaves behind.
ac_expected="$(awk -v name="$ac_archive" '{ sub(/^[*]/, "", $2); sub(/^\.\//, "", $2); if ($2 == name) { print $1; exit } }' "$ac_tmp/$ac_sums")"
[ -n "$ac_expected" ] ||
  ac_die "\`$ac_sums\` says nothing about \`$ac_archive\`, and unvouched-for bytes are not installed"

ac_actual="$(ac_checksum "$ac_tmp/$ac_archive")"
if [ "$ac_actual" != "$ac_expected" ]; then
  ac_die "checksum mismatch for \`$ac_archive\`
  expected $ac_expected
       got $ac_actual
Nothing was installed. Delete what you downloaded and try again; if it happens twice, do not run the binary"
fi
printf 'verified %s against %s\n' "$ac_archive" "$ac_sums"

# --- install ------------------------------------------------------------------

tar -xzf "$ac_tmp/$ac_archive" -C "$ac_tmp" ||
  ac_die "\`$ac_archive\` did not unpack"
[ -f "$ac_tmp/$ac_bin" ] ||
  ac_die "\`$ac_archive\` does not hold \`$ac_bin\`"

ac_into="${AGENT_COMPOSE_INSTALL:-}"
if [ -z "$ac_into" ]; then
  [ -n "${HOME:-}" ] ||
    ac_die "neither AGENT_COMPOSE_INSTALL nor HOME is set, so there is nowhere to install to"
  ac_into="$HOME/.local/bin"
fi
mkdir -p "$ac_into" || ac_die "could not create \`$ac_into\`"

chmod 0755 "$ac_tmp/$ac_bin"
# `mv` replaces the directory entry, which is what makes re-installing over a
# copy that is currently running work; `cp` onto a running binary is refused.
mv -f "$ac_tmp/$ac_bin" "$ac_into/$ac_bin" ||
  ac_die "could not write \`$ac_into/$ac_bin\`"

printf 'installed %s\n' "$ac_into/$ac_bin"
"$ac_into/$ac_bin" --version ||
  ac_die "\`$ac_into/$ac_bin\` was installed but would not run"

case ":${PATH:-}:" in
  *":$ac_into:"*) ;;
  *)
    printf "\n%s is not on your PATH. Add it:\n\n    export PATH=\"%s:\$PATH\"\n" "$ac_into" "$ac_into"
    ;;
esac
