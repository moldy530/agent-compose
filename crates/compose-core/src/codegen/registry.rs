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
//!   separate **host-scoped** line, `//host/path/:_authToken=${VAR}`: npm
//!   documents that `_authToken` "must be scoped to a specific registry", and
//!   that environment variables are replaced using `${VARIABLE_NAME}`. The key
//!   is the registry URL's address and directory ([`npm_auth_key`]), which is
//!   the form npm's own documentation shows
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
/// is the registry's authority plus the *directory* of its path: a registry at
/// `https://npm.example/repo/` authenticates under `//npm.example/repo/`, and
/// one at the host root under `//npm.example/`. A URL written without a trailing
/// slash names the same directory as one written with it, because npm appends
/// the package name to it either way.
///
/// It is public because `parse::deploy` asks the same question before a build
/// exists: two entries whose key is this one, carrying two different variables,
/// would write one line twice and let an ini parser pick — so the parser refuses
/// them, and it has to ask about the *emitted* key rather than about the URL.
#[must_use]
pub fn npm_auth_key(url: &str) -> String {
    // The scheme is not part of the key (npm's own spelling starts at `//`), and
    // a query or fragment is not part of an address npm would authenticate to.
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let rest = rest.split(['?', '#']).next().unwrap_or(rest);
    let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
    // The directory: everything up to and including the last `/`, which for a
    // path with no `/` of its own — `repo` — is the host root, and for `repo/`
    // is `repo/` itself.
    let directory = path.rfind('/').map_or("", |end| &path[..=end]);
    format!("//{authority}/{directory}")
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
    fn a_credential_is_keyed_by_the_registrys_own_directory() {
        for (url, key) in [
            ("https://npm.example/repo/", "//npm.example/repo/"),
            ("https://npm.example/repo", "//npm.example/"),
            ("https://npm.example/", "//npm.example/"),
            ("https://npm.example", "//npm.example/"),
            ("https://npm.example:8443/a/b/", "//npm.example:8443/a/b/"),
            ("http://localhost:4873/", "//localhost:4873/"),
        ] {
            assert_eq!(npm_auth_key(url), key, "the key derived for `{url}`");
        }
    }
}
