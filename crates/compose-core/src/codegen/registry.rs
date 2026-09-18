//! `bunfig.toml` and `.npmrc`: where a generated project's installer resolves
//! packages from (grammar 14.6, PRD resolved q59).
//!
//! # Why the compiler writes these at all
//!
//! A hand-placed `bunfig.toml` beside an emitted project survives a build — the
//! manifest is the boundary (PRD resolved q47) and nothing here touches a file
//! the compiler did not write — but it is **not in the artifact**, so on a mesh
//! every worker's `bun install` at materialise (`docs/distributed.md` §4 step 4)
//! resolves against the public registry. On the corporate network that mandates
//! a mirror, that is exactly what is blocked. Where a package resolves from is a
//! placement fact, and placement facts belong in deploy files (PRD 5.10) — the
//! same sentence that placed the journal (resolved q27) and the trace sink
//! (resolved q50).
//!
//! # Why both files
//!
//! Resolved q18: **Bun by default, Node and npm a supported fallback.** An
//! emitted project has to install under either, so the configuration is written
//! in both spellings — `bunfig.toml` for Bun, `.npmrc` for npm and pnpm. This
//! **configures** the installer rather than pinning one, so it does not collide
//! with q18's refusal of a `packageManager` field, and nothing here emits a
//! lockfile.
//!
//! Bun reads `.npmrc` as well as its own `bunfig.toml`, which is not a conflict:
//! both files are written from one IR and say the same thing.
//!
//! # The token is a spelling, not a byte
//!
//! Each file carries the environment-variable **reference** and the installer
//! expands it when it runs: `${NAME}` in `.npmrc`, `$NAME` in `bunfig.toml`, the
//! two spellings each tool documents. So no secret enters the artifact, the
//! artifact hash (resolved q40) is stable across a token rotation, and nothing
//! secret transits the mesh (resolved q41).
//!
//! **Neither installer fails when the variable is unset.** Both send the
//! reference as text and the registry answers `401` — so the presence check that
//! catches it early is the launch check over the environment manifest (resolved
//! q15) and a worker's join-time `env_ok` report (`docs/distributed.md` §3.1,
//! §9.2), which is why the token joins those manifests (`super::env`). `build`
//! reads no environment and checks nothing here.
//!
//! # The two spellings, verified rather than recalled
//!
//! * **Bun** — `[install] registry = { url = "…", token = "$VAR" }` and
//!   `[install.scopes]` entries `"@scope" = { url = "…", token = "$VAR" }`,
//!   with `$variable` substitution. Bun's documented `bunfig.toml` contract.
//! * **npm** — `registry=…` and `@scope:registry=…`, with the credential on a
//!   separate **address-scoped** line, `//host/path/:_authToken=${VAR}`: npm
//!   documents that `_authToken` "must be scoped to a specific registry", and
//!   that environment variables are replaced using `${VARIABLE_NAME}`. The key
//!   is the address npm itself looks a credential up under ([`npm_auth_key`]),
//!   which is the form npm's own documentation shows
//!   (`//somewhere-else.com/myorg/:_authToken=…`).

use std::fmt::Write as _;

use crate::ir::Ir;
use crate::ir::deploy::PackageRegistry;

/// `bunfig.toml` and `.npmrc`, or nothing at all.
///
/// **Nothing at all** when the target declares no `package_registry:`: an empty
/// stub would be a file claiming a name for no reason, and the emitted file set
/// is the compiler's claim on a directory (PRD resolved q47). A project built
/// for a target with no mirror installs the way it always did.
#[must_use]
pub fn files(ir: &Ir) -> Vec<super::GeneratedFile> {
    let Some(registry) = ir.deploy.package_registry.as_ref() else {
        return Vec::new();
    };
    vec![npmrc(ir, registry), bunfig_toml(ir, registry)]
}

/// `bunfig.toml` — the Bun half.
fn bunfig_toml(ir: &Ir, registry: &PackageRegistry) -> super::GeneratedFile {
    let mut contents = super::header(ir, "# ");
    contents.push_str(BUNFIG_DOC);
    contents.push_str("\n[install]\n");
    contents.push_str(&bunfig_entry(
        "registry",
        &registry.url.value,
        token(registry.token.as_ref()),
    ));
    if !registry.scopes.is_empty() {
        contents.push_str("\n[install.scopes]\n");
        for scope in registry.scopes.values() {
            contents.push_str(&bunfig_entry(
                &toml_string(&scope.name.value),
                &scope.url.value,
                token(scope.token.as_ref()),
            ));
        }
    }
    super::GeneratedFile {
        path: "bunfig.toml".to_string(),
        contents,
    }
}

/// One `key = …` line of a bunfig `[install]` or `[install.scopes]` table.
///
/// The bare URL where there is no credential and the object form where there is
/// — both of them spellings Bun's own documentation shows, rather than one
/// normalized form of this compiler's invention.
fn bunfig_entry(key: &str, url: &str, token: Option<&str>) -> String {
    match token {
        Some(name) => format!(
            "{key} = {{ url = {}, token = {} }}\n",
            toml_string(url),
            toml_string(&format!("${name}"))
        ),
        None => format!("{key} = {}\n", toml_string(url)),
    }
}

/// `.npmrc` — the npm and pnpm half, which is resolved q18's fallback getting
/// the same configuration rather than a lesser one.
fn npmrc(ir: &Ir, registry: &PackageRegistry) -> super::GeneratedFile {
    let mut contents = super::header(ir, "# ");
    contents.push_str(NPMRC_DOC);
    contents.push('\n');
    let _ = writeln!(contents, "registry={}", registry.url.value);
    if let Some(name) = token(registry.token.as_ref()) {
        let _ = writeln!(
            contents,
            "{}:_authToken=${{{name}}}",
            npm_auth_key(&registry.url.value)
        );
    }
    for scope in registry.scopes.values() {
        let _ = writeln!(
            contents,
            "{}:registry={}",
            scope.name.value, scope.url.value
        );
        if let Some(name) = token(scope.token.as_ref()) {
            let _ = writeln!(
                contents,
                "{}:_authToken=${{{name}}}",
                npm_auth_key(&scope.url.value)
            );
        }
    }
    super::GeneratedFile {
        path: ".npmrc".to_string(),
        contents,
    }
}

/// The variable a credential names, or `None` where there is no credential.
fn token(reference: Option<&crate::diag::Spanned<crate::ast::common::EnvRef>>) -> Option<&str> {
    reference.map(|token| token.value.name.as_str())
}

/// The address an `.npmrc` credential line is keyed by, for a registry URL.
///
/// npm scopes auth to an **address** rather than to a package scope, and the key
/// is the registry's authority plus its **whole path**, with a trailing `/`: a
/// registry at `https://npm.example/repo` — or at `https://npm.example/repo/`,
/// which is the same registry — authenticates under `//npm.example/repo/`, and
/// one at the host root under `//npm.example/`. The two spellings of that path
/// are one key, because npm strips a trailing `/` from the registry before it
/// appends the package name.
///
/// It is public because `parse::deploy` asks the same question before a build
/// exists: two entries whose key is this one, carrying two different variables,
/// would write one line twice and let an ini parser pick — so the parser refuses
/// them, and it has to ask about the *emitted* key rather than about the URL.
///
/// # Why this is derived rather than copied
///
/// npm never compares this key against the text of a `registry=` line. To fetch
/// a package it builds the request URI itself — the registry with any trailing
/// `/` stripped, then `/`, then the package name — parses that through a WHATWG
/// `URL`, and then **walks up** the resulting address looking for a credential:
/// for `https://npm.example/repo` and package `lodash` it tries
/// `//npm.example/repo/lodash`, `//npm.example/repo/`, `//npm.example/repo`,
/// `//npm.example/` and `//npm.example`, longest first, and spends the first
/// token it finds (`npm-registry-fetch`'s `regFromURI` and `regFetch`). So the
/// key written here has to be one of the addresses on that walk, spelled the way
/// the `URL` constructor spells it. A `_authToken` line npm cannot match is worse
/// than no line at all: the install goes out unauthenticated while the
/// `bunfig.toml` beside it authenticates, which is the one artifact / two answers
/// divergence grammar 14.6 rule 5 exists to prevent.
///
/// Two things follow, and they are the whole of what this function does:
///
/// * **the last path segment stays.** The package name is appended *after* the
///   registry's path, not written over its final segment, so a `url:` of
///   `https://npm.example/repo` keys at `//npm.example/repo/` and not at the host
///   root. (npm's config-**writing** side, the `nerfDart` behind `npm login`,
///   does drop a last segment that has no trailing `/` — but that is a different
///   function from the lookup, and keying by it would scope a declared
///   repository's credential to its entire host.)
/// * **the address is canonical rather than as-written**, because `new URL` has
///   already normalized it by the time npm walks:
///   * the host is **lowercased** (`//NPM.Example/` never matches);
///   * the scheme's own **default port** is dropped, so `https://npm.example:443/`
///     and `https://npm.example/` are one address;
///   * `.` and `..` segments are **resolved**, and an empty segment is kept,
///     because that is what `new URL` does to a path.
///
/// The path's own case is left alone: a WHATWG `URL` lowercases the host and
/// nothing else, and so does a registry that serves `/Repo/`.
///
/// # Why three normalizations are the whole list
///
/// Because the parser refuses every other spelling that parse would rewrite,
/// rather than this function reimplementing a WHATWG `URL`. A punycoded host,
/// a percent-escape decoded out of an authority, a renumbered port, an IPv4
/// literal re-serialized as a dotted quad, a `\` turned into a `/`, a
/// percent-encoded path character, a dot segment spelled with a `%2e`, and a
/// query or a fragment that ends the path early are all compile errors at
/// `package_registry.url:` (`parse::deploy::respelled_address_problem`,
/// grammar 14.6 rule 1). So what reaches here is an address whose host differs
/// from the parse's by case alone, whose port is written back unchanged or is
/// the scheme's own default, and whose path a WHATWG `URL` alters only by
/// resolving dot segments written in dots — which is exactly the list above.
/// A rule relaxed there without a normalization
/// added here would put this claim back in the compiler's mouth, so the two
/// sides are pinned together from the parser's, where both are visible:
/// `parse::deploy::tests::an_address_a_url_parse_respells_is_refused_only_where_one_is_derived`.
#[must_use]
pub fn npm_auth_key(url: &str) -> String {
    // The scheme is not part of the key (npm's own spelling starts at `//`) but
    // it decides which port is the default one, and a query or fragment is not
    // part of an address npm would authenticate to.
    let (scheme, rest) = url
        .split_once("://")
        .map_or(("", url), |(scheme, rest)| (scheme, rest));
    let rest = rest.split(['?', '#']).next().unwrap_or(rest);
    let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
    format!(
        "//{}/{}",
        canonical_authority(scheme, authority),
        canonical_directory(path)
    )
}

/// The authority half of an `.npmrc` key, spelled the way a WHATWG `URL` spells
/// it: ASCII lowercase, with the scheme's own default port dropped.
fn canonical_authority(scheme: &str, authority: &str) -> String {
    let lowered = authority.to_ascii_lowercase();
    let default_port = match scheme.to_ascii_lowercase().as_str() {
        "http" => ":80",
        "https" => ":443",
        _ => return lowered,
    };
    match lowered.strip_suffix(default_port) {
        Some(host) => host.to_string(),
        None => lowered,
    }
}

/// The path half of an `.npmrc` key: the registry's own path, dot segments
/// resolved and a trailing `/` guaranteed.
///
/// npm appends `/<package>` to the registry — after stripping one trailing `/`
/// from it — and then walks up the address of that request. The directory the
/// walk reaches first is therefore the registry's **whole** path, which is why
/// nothing here drops its last segment and why `repo` and `repo/` are one key.
///
/// A trailing `.` or `..` is resolved rather than kept, because the `/<package>`
/// npm appends makes `new URL` resolve it on the request: `…/a/b/..` fetches
/// `…/a/b/../lodash`, which is `…/a/lodash`, which keys at `//host/a/`.
///
/// Only the two spellings written *in dots* are matched here, because they are
/// the only ones that reach this function: a WHATWG `URL` reads a whole segment
/// of `%2e` as a `.` too — `%2e`, `.%2e`, `%2e.`, `%2e%2e`, in either case — and
/// `parse::deploy::respelled_address_problem` refuses all four at the `url:`
/// rather than have this function decode a percent-escape (grammar 14.6 rule 1).
fn canonical_directory(path: &str) -> String {
    let segments: Vec<&str> = path.split('/').collect();
    let last = segments.len() - 1;
    let mut directory: Vec<&str> = Vec::new();
    for (index, segment) in segments.iter().enumerate() {
        match *segment {
            // A path's final empty segment *is* the trailing `/`, which every
            // segment below writes for itself. An **interior** empty segment is
            // an ordinary segment that a WHATWG `URL` keeps, and so does the
            // address npm walks: `/a//b/` is `//host/a//b/`, not `//host/a/b/`.
            "" if index == last => {}
            "." => {}
            ".." => {
                directory.pop();
            }
            other => directory.push(other),
        }
    }
    let mut canonical = String::new();
    for segment in directory {
        canonical.push_str(segment);
        canonical.push('/');
    }
    canonical
}

/// One TOML basic string.
///
/// Every value written here is already narrow — a URL with no whitespace
/// (grammar 14.6), a scope of lowercase letters, digits and three punctuation
/// marks, an environment name of `[A-Z_][A-Z0-9_]*` — so this escapes the two
/// characters a basic string cannot carry and nothing else needs a table.
fn toml_string(value: &str) -> String {
    let mut text = String::with_capacity(value.len() + 2);
    text.push('"');
    for character in value.chars() {
        match character {
            '"' => text.push_str("\\\""),
            '\\' => text.push_str("\\\\"),
            other => text.push(other),
        }
    }
    text.push('"');
    text
}

const BUNFIG_DOC: &str = "\
#
# Where `bun install` resolves this project's packages from, written from the
# `package_registry:` of the deploy file this project was built for
# (`docs/grammar.md` §14.6). The `.npmrc` beside this file says the same thing
# to npm and pnpm, which is what keeps the Node fallback a real one.
#
# A credential is the **name** of an environment variable, which Bun expands
# when it installs — so this file carries no secret, and rotating the token does
# not change this artifact's hash. Nothing verifies the variable here: an unset
# one is sent as text and the registry refuses it, so presence is checked where
# the environment manifest is (`readEnvironment()` at process start, and a
# worker's join).
";

const NPMRC_DOC: &str = "\
#
# Where `npm install` and `pnpm install` resolve this project's packages from,
# written from the `package_registry:` of the deploy file this project was built
# for (`docs/grammar.md` §14.6). The `bunfig.toml` beside this file says the
# same thing to Bun, which is the default runtime and installer.
#
# A credential is the **name** of an environment variable, which npm expands
# when it installs — so this file carries no secret, and rotating the token does
# not change this artifact's hash. An `_authToken` is keyed by address rather
# than by package scope, which npm requires and which is why each line below
# names a host and a path.
";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codegen::test_support::ir_of_mesh;

    /// A composition resolved under `mesh` with this deploy file.
    fn registry_of(deploy: &str) -> Ir {
        ir_of_mesh("version: \"0.1\"\n", deploy)
    }

    const MIRROR: &str = r#"version: "0.1"

package_registry:
  url: "https://npm.internal.example/repository/npm-group/"
  token: ${NPM_MIRROR_TOKEN}
  scopes:
    "@corp":
      url: "https://npm.internal.example/repository/corp/"
      token: ${NPM_CORP_TOKEN}
"#;

    #[test]
    fn a_target_with_no_slot_emits_neither_file() {
        assert_eq!(files(&registry_of("version: \"0.1\"\n")), Vec::new());
    }

    /// The emitted bytes carry the **reference**, in each installer's own
    /// spelling, and the artifact is therefore stable across a rotation.
    ///
    /// The other half of that claim — that a `build` running with the variable
    /// *set* still writes the name — cannot be made honestly in-process, because
    /// mutating the environment of a test binary races every thread beside it.
    /// It is made where it belongs, over a real `agent-compose build` with the
    /// value in its environment:
    /// `crates/agent-compose/tests/build_cli.rs::build_writes_the_registry_reference_rather_than_the_token_it_resolves_to`.
    #[test]
    fn the_emitted_files_carry_the_reference_rather_than_a_value() {
        let emitted = files(&registry_of(MIRROR));
        assert_eq!(
            emitted
                .iter()
                .map(|file| file.path.as_str())
                .collect::<Vec<_>>(),
            [".npmrc", "bunfig.toml"]
        );
        assert!(emitted[0].contents.contains("${NPM_MIRROR_TOKEN}"));
        assert!(emitted[0].contents.contains("${NPM_CORP_TOKEN}"));
        assert!(emitted[1].contents.contains("\"$NPM_MIRROR_TOKEN\""));
        assert!(emitted[1].contents.contains("\"$NPM_CORP_TOKEN\""));
    }

    /// Each installer gets its own documented spelling, not one shared guess.
    #[test]
    fn each_installer_gets_the_spelling_its_own_contract_documents() {
        let emitted = files(&registry_of(MIRROR));
        let npmrc = &emitted[0].contents;
        let bunfig = &emitted[1].contents;

        assert!(
            npmrc.contains("registry=https://npm.internal.example/repository/npm-group/\n"),
            "{npmrc}"
        );
        assert!(
            npmrc.contains(
                "//npm.internal.example/repository/npm-group/:_authToken=${NPM_MIRROR_TOKEN}\n"
            ),
            "npm keys a credential by address: {npmrc}"
        );
        assert!(
            npmrc.contains("@corp:registry=https://npm.internal.example/repository/corp/\n"),
            "{npmrc}"
        );
        assert!(
            npmrc
                .contains("//npm.internal.example/repository/corp/:_authToken=${NPM_CORP_TOKEN}\n"),
            "{npmrc}"
        );

        assert!(
            bunfig.contains(
                "[install]\nregistry = { url = \"https://npm.internal.example/repository/npm-group/\", token = \"$NPM_MIRROR_TOKEN\" }\n"
            ),
            "{bunfig}"
        );
        assert!(
            bunfig.contains(
                "[install.scopes]\n\"@corp\" = { url = \"https://npm.internal.example/repository/corp/\", token = \"$NPM_CORP_TOKEN\" }\n"
            ),
            "Bun's substitution is `$VAR` and its scope keys carry the `@`: {bunfig}"
        );
    }

    /// A mirror that needs no credential gets the shorter spelling of each file
    /// rather than an empty token.
    #[test]
    fn a_registry_with_no_token_writes_no_credential_line() {
        let emitted = files(&registry_of(
            "version: \"0.1\"\npackage_registry:\n  url: \"https://npm.internal.example/mirror/\"\n",
        ));
        let npmrc = &emitted[0].contents;
        let bunfig = &emitted[1].contents;
        assert!(
            !npmrc.contains(":_authToken="),
            "an absent credential writes no line: {npmrc}"
        );
        assert!(
            bunfig.contains("registry = \"https://npm.internal.example/mirror/\"\n"),
            "{bunfig}"
        );
        assert!(!bunfig.contains("token = "), "{bunfig}");
        assert!(
            !bunfig.contains("[install.scopes]"),
            "a section with no scopes is not written empty: {bunfig}"
        );
    }

    /// The address a credential is keyed by, over the shapes an operator writes.
    #[test]
    fn a_credential_is_keyed_by_the_registrys_whole_address() {
        for (url, key) in [
            ("https://npm.example/repo/", "//npm.example/repo/"),
            // The ordinary spelling of a repository on a mirror. npm appends
            // `/lodash` *after* `repo` and walks up from
            // `//npm.example/repo/lodash`, so the credential is the
            // repository's and not the whole host's.
            ("https://npm.example/repo", "//npm.example/repo/"),
            (
                "https://nexus.example/repository/npm-group",
                "//nexus.example/repository/npm-group/",
            ),
            ("https://npm.example/", "//npm.example/"),
            ("https://npm.example", "//npm.example/"),
            ("https://npm.example:8443/a/b/", "//npm.example:8443/a/b/"),
            ("http://localhost:4873/", "//localhost:4873/"),
            // A host an operator wrote in the case their runbook uses. npm
            // derives its key through a WHATWG `URL`, which lowercases a host,
            // so a key copied verbatim from here is one npm never looks up —
            // the install would go out unauthenticated while `bunfig.toml`,
            // which keys nothing by address, authenticates.
            (
                "https://NPM.Internal.Example/repository/npm-group/",
                "//npm.internal.example/repository/npm-group/",
            ),
        ] {
            assert_eq!(npm_auth_key(url), key, "the key derived for `{url}`");
        }
    }

    /// The key is the address a WHATWG `URL` produces, because that is the one
    /// npm looks up — not the text of the `url:` the author wrote.
    ///
    /// Each row is a spelling that reaches the *same* registry and would, copied
    /// verbatim, write a `_authToken` line npm cannot match. The failure is
    /// silent in the worst direction: `npm install` 401s (or resolves
    /// anonymously) while `bun install` from the same artifact succeeds, since
    /// Bun carries the token inside its registry object rather than keyed by an
    /// address at all.
    #[test]
    fn the_key_is_the_address_npm_derives_rather_than_the_url_as_written() {
        for (url, key) in [
            // Case: the host folds, the path does not — `new URL` lowercases a
            // host and leaves a pathname alone, and so does a registry serving
            // `/Repo/`.
            ("https://NPM.Example/Repo/", "//npm.example/Repo/"),
            ("https://npm.EXAMPLE:8443/a/", "//npm.example:8443/a/"),
            // A default port is not part of a WHATWG `URL`'s host.
            ("https://npm.example:443/repo/", "//npm.example/repo/"),
            ("http://npm.example:80/repo/", "//npm.example/repo/"),
            // …and a non-default one is.
            ("https://npm.example:80/repo/", "//npm.example:80/repo/"),
            ("http://npm.example:443/repo/", "//npm.example:443/repo/"),
            // Dot segments resolve before npm sees the path.
            ("https://npm.example/a/b/../c/", "//npm.example/a/c/"),
            ("https://npm.example/a/./b/", "//npm.example/a/b/"),
            ("https://npm.example/a/b/..", "//npm.example/a/"),
            ("https://npm.example/../", "//npm.example/"),
            // A segment that merely ends in dots is an ordinary segment.
            ("https://npm.example/a/x../", "//npm.example/a/x../"),
            // An **empty** segment is an ordinary segment too: `new URL` keeps
            // it, and npm's walk-up therefore passes through `//host/a//b/`
            // before it ever reaches `//host/a/`.
            ("https://npm.example/a//b/", "//npm.example/a//b/"),
            ("https://npm.example/a//../b/", "//npm.example/a/b/"),
        ] {
            assert_eq!(npm_auth_key(url), key, "the key derived for `{url}`");
        }
    }

    /// The **lookup** side of npm, not the config-writing side.
    ///
    /// `npm login` writes a credential under `@npmcli/config`'s `nerfDart`,
    /// which resolves `new URL(".", registry)` and so drops a last segment
    /// written without a trailing `/`. `npm install` reads one under
    /// `npm-registry-fetch`'s `regFromURI`, which walks up the URI of the
    /// request it is making — and that URI is the registry with one trailing
    /// `/` stripped, then `/`, then the package. The two disagree about exactly
    /// one URL shape, and it is the shape an operator writes most often.
    ///
    /// Keying by the writer would put a credential declared for one repository
    /// on the mirror's whole host, and — because the two spellings of one
    /// registry would then derive two keys — let
    /// `parse::deploy::one_credential_per_address` pass a deploy file whose
    /// `.npmrc` and `bunfig.toml` spend two different variables at one registry,
    /// which is the divergence grammar 14.6 rule 5 exists to refuse.
    #[test]
    fn a_registrys_last_segment_is_part_of_its_address_rather_than_the_packages_slot() {
        assert_eq!(
            npm_auth_key("https://npm.internal.example/repo"),
            "//npm.internal.example/repo/",
            "npm fetches `https://npm.internal.example/repo/lodash` and walks up from there"
        );
        assert_eq!(
            npm_auth_key("https://npm.internal.example/repo"),
            npm_auth_key("https://npm.internal.example/repo/"),
            "one registry, written the two ways it is written, is one address"
        );
    }
}
