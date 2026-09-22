//! The grammar's env-ref classification and the compiler's, held together
//! (grammar 4.3, Decisions D41, D92).
//!
//! §4.3 is **total**: every string-valued surface falls in exactly one of three
//! classes, and class 3 — no references at all — is the *default* for anything
//! the section does not place in class 1 or class 2 (D92). That totality is what
//! makes the section answerable rather than a list somebody keeps extending, and
//! it is also what makes an omission dangerous in a direction no negative corpus
//! can see.
//!
//! A credential the compiler holds to the env-ref value form, whose name §4.3's
//! class-1 table does not carry, is classified **class 3** by the rule that makes
//! the section complete. The document then says a literal is required exactly
//! where the compiler refuses one, and the reader who resolves the question the
//! way the grammar tells them to resolve it writes the secret into the spec
//! text. Nothing else in the repository fails when that happens: the fixture
//! corpus pins the compiler, and the compiler is not the half that went wrong.
//!
//! So the two halves are bound here. The compiler's own list of connection and
//! credential field names is read directly, and the one credential that lives
//! outside it — `hub.join_token:`, which is a closed key of `hub:` rather than a
//! field of a plugin-config object — is checked from both ends: the table names
//! it, and `validate` enforces the class the table puts it in.

use std::fs;
use std::path::{Path, PathBuf};

use compose_core::ast::deploy::SECRET_FIELDS;
use compose_core::parse_str;

/// The repository root.
fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("the manifest directory has a grandparent")
        .to_path_buf()
}

/// §4.3's class-1 table: the rows between the sentence that introduces it and
/// the paragraph that qualifies it.
///
/// Sliced rather than searched, because the failure this file exists for is a
/// field name that appears *somewhere* in the grammar — in §14.2's own key
/// table, say — and nowhere in the classification. A whole-document `contains`
/// would pass on exactly the shape that is broken.
fn class_one_table(grammar: &str) -> &str {
    const OPENER: &str = "**Secret-bearing fields take the env-ref value form only**";
    const CLOSER: &str = "The table classifies these field *names* wherever they occur";
    let start = grammar
        .find(OPENER)
        .expect("grammar 4.3 introduces its class-1 table");
    let rest = &grammar[start..];
    let end = rest
        .find(CLOSER)
        .expect("grammar 4.3 qualifies its class-1 table");
    &rest[..end]
}

/// Every connection and credential field the compiler refuses a literal in is
/// tabulated as class 1.
#[test]
fn every_secret_field_the_compiler_knows_is_tabulated_as_class_one() {
    let grammar =
        fs::read_to_string(repository().join("docs/grammar.md")).expect("the grammar is readable");
    let table = class_one_table(&grammar);

    for field in SECRET_FIELDS {
        assert!(
            table.contains(&format!("`{field}`")),
            "the compiler holds `{field}:` to the env-ref value form and grammar 4.3's class-1 \
             table does not name it, so §4.3's own totality rule (D92) classifies it class 3 — \
             which says a literal is required exactly where the compiler refuses one"
        );
    }
}

/// …and so is `hub.join_token:`, which no such list carries.
///
/// It is a closed key of `hub:` (grammar 14.2 rule 1) rather than a field of an
/// open plugin-config object, so the parser reaches for `lexical::env_ref`
/// directly and there is no constant to read. That is precisely why it is the
/// row likeliest to be missed: the credential exists in the compiler with
/// nothing enumerating it.
#[test]
fn the_hub_join_token_is_tabulated_as_class_one() {
    let grammar =
        fs::read_to_string(repository().join("docs/grammar.md")).expect("the grammar is readable");
    let table = class_one_table(&grammar);
    assert!(
        table.contains("`join_token`"),
        "grammar 4.3's class-1 table does not name `join_token`, so §4.3 classifies the mesh's \
         one credential class 3 — the reverse of §14.2 rule 1, of what `validate` enforces, and \
         of the published schema's `$ref: envRef`"
    );
    assert!(
        table.contains("§14.2"),
        "the `join_token` row does not say where the key lives; §4.3's rows carry their section"
    );
}

/// …and so is the `trace_sink.auth:` credential, which the class-1 table has to
/// name **where it lives** rather than only by spelling.
///
/// `token` and `secret` were already class-1 names, so a table row that stopped
/// at the spelling would look complete while saying nothing about the deploy
/// layer's second credential-bearing block: §4.3's own sentence is that the
/// table "classifies these field *names* wherever they occur; it never makes one
/// legal where its section's own key rules do not admit it", which cuts both
/// ways — a reader asking "may `trace_sink.auth.bearer.token:` be a literal?"
/// is entitled to find the answer by looking up the section, not by inferring
/// that a name borrowed from §13.3 carries §13.3's rule with it.
#[test]
fn the_trace_sink_credential_is_tabulated_as_class_one() {
    let grammar =
        fs::read_to_string(repository().join("docs/grammar.md")).expect("the grammar is readable");
    let table = class_one_table(&grammar);
    let row = table
        .lines()
        .find(|line| line.contains("`token`, `secret`"))
        .expect("grammar 4.3's class-1 table carries the outbound-signing row");
    assert!(
        row.contains("§14.5"),
        "grammar 4.3's `token`/`secret` row does not name §14.5, so the deploy layer's trace-sink \
         credential is classified by nothing: the row reads as §13.3's alone, and §4.3's totality \
         rule (D92) then puts `trace_sink.auth.bearer.token:` in class 3 — a literal required \
         exactly where the compiler refuses one"
    );
}

/// …and the class the table names is the class `validate` enforces, on that
/// block's two credentials.
#[test]
fn validate_enforces_the_trace_sink_credentials_class() {
    const SINK: &str =
        "version: \"0.1\"\ntrace_sink:\n  url: \"https://collector.internal.example/v1/traces\"\n";
    for (scheme, key) in [("bearer", "token"), ("hmac", "secret")] {
        let literal = format!("{SINK}  auth:\n    {scheme}:\n      {key}: \"s3cret\"\n");
        assert_eq!(
            parse_str(&literal, "deploy/staging.yml")
                .diagnostics
                .iter()
                .map(|diagnostic| diagnostic.code.as_str())
                .collect::<Vec<_>>(),
            ["invalid-env-ref"],
            "a literal `trace_sink.auth.{scheme}.{key}:` must be refused as class 1 requires"
        );

        let reference =
            format!("{SINK}  auth:\n    {scheme}:\n      {key}: ${{SINK_CREDENTIAL}}\n");
        assert!(
            parse_str(&reference, "deploy/staging.yml")
                .diagnostics
                .is_empty(),
            "the value form is the form class 1 asks for and it must be accepted for \
             `trace_sink.auth.{scheme}.{key}:`"
        );
    }
}

/// …and so is the `package_registry:` credential, which the class-1 table has to
/// name **where it lives** for the reason the trace sink's row does.
///
/// `token` was already a class-1 name, so a reader asking "may
/// `package_registry.token:` be a literal?" would find a row about §13.3 and
/// §14.5 and have to infer that a name borrowed from them carries their rule.
/// §4.3's totality rule (D92) makes that inference the *wrong* one to have to
/// make: a surface the section does not place in class 1 or 2 is class 3, so an
/// unnamed credential reads as one a literal is required in — which is the
/// secret in the spec text, in the one block whose whole point is that the
/// secret never enters the artifact (PRD resolved q59).
#[test]
fn the_package_registry_credential_is_tabulated_as_class_one() {
    let grammar =
        fs::read_to_string(repository().join("docs/grammar.md")).expect("the grammar is readable");
    let table = class_one_table(&grammar);
    let row = table
        .lines()
        .find(|line| line.contains("§14.6"))
        .expect("grammar 4.3's class-1 table carries the package-registry row");
    assert!(
        row.contains("`token`"),
        "grammar 4.3's §14.6 row does not name `token`: {row}"
    );
    assert!(
        row.contains("scopes"),
        "the row names the block but not its `scopes:` entries, whose `token:` is the same key \
         under the same rule: {row}"
    );
}

/// …and the class the table names is the class `validate` enforces, on both
/// positions that take that credential.
#[test]
fn validate_enforces_the_package_registry_credentials_class() {
    const REGISTRY: &str = "version: \"0.1\"\npackage_registry:\n  url: \"https://npm.internal.example/repository/npm-group/\"\n";
    let scope =
        "  scopes:\n    \"@corp\":\n      url: \"https://npm.internal.example/repository/corp/\"\n";

    for (what, literal, reference) in [
        (
            "package_registry.token",
            format!("{REGISTRY}  token: \"npm_s3cret\"\n"),
            format!("{REGISTRY}  token: ${{NPM_MIRROR_TOKEN}}\n"),
        ),
        (
            "package_registry.scopes.@corp.token",
            format!("{REGISTRY}{scope}      token: \"npm_s3cret\"\n"),
            format!("{REGISTRY}{scope}      token: ${{NPM_CORP_TOKEN}}\n"),
        ),
    ] {
        assert_eq!(
            parse_str(&literal, "deploy/staging.yml")
                .diagnostics
                .iter()
                .map(|diagnostic| diagnostic.code.as_str())
                .collect::<Vec<_>>(),
            ["invalid-env-ref"],
            "a literal `{what}:` must be refused as class 1 requires"
        );
        assert!(
            parse_str(&reference, "deploy/staging.yml")
                .diagnostics
                .is_empty(),
            "the value form is the form class 1 asks for and it must be accepted for `{what}:`"
        );
    }
}

/// …and so is the `journal:` block's address, which is the one deploy-layer
/// credential whose field name was already in the table for **other** sections.
///
/// `url` was class 1 in `provider.*`, `storage_backends.*` and `event_sources.*`
/// — and class **3** at the deploy layer's three other URL keys, which are
/// addresses a deployment writes out and this compiler shape-checks
/// (`hub.public_url:`, `trace_sink.url:`, `package_registry`'s). So a reader
/// asking "may `journal.url:` be a literal?" meets a name that is classified
/// both ways one section apart, and §4.3's totality rule (D92) makes the
/// unnamed answer the wrong one: class 3 is the default, and class 3 here is a
/// connection string — a **password** — written into the spec text and into
/// every artifact built from it. The row has to name §14.7 for the answer to be
/// findable rather than inferable (PRD resolved q62).
#[test]
fn the_journal_address_is_tabulated_as_class_one() {
    let grammar =
        fs::read_to_string(repository().join("docs/grammar.md")).expect("the grammar is readable");
    let table = class_one_table(&grammar);
    let row = table
        .lines()
        .find(|line| line.contains("§14.7"))
        .expect("grammar 4.3's class-1 table carries the journal row");
    assert!(
        row.contains("`url`"),
        "grammar 4.3's §14.7 row does not name `url`: {row}"
    );
}

/// …and the class the table names is the class `validate` enforces, on both
/// providers that take an address.
#[test]
fn validate_enforces_the_journal_addresss_class() {
    for provider in ["postgres", "mysql"] {
        let block = format!("version: \"0.1\"\njournal:\n  provider: {provider}\n");
        let literal =
            format!("{block}  url: \"{provider}://compose:hunter2@db.example/journal\"\n");
        assert_eq!(
            parse_str(&literal, "deploy/staging.yml")
                .diagnostics
                .iter()
                .map(|diagnostic| diagnostic.code.as_str())
                .collect::<Vec<_>>(),
            ["invalid-env-ref"],
            "a literal `journal.url:` must be refused as class 1 requires, on `{provider}`"
        );
        let reference = format!("{block}  url: ${{JOURNAL_URL}}\n");
        assert!(
            parse_str(&reference, "deploy/staging.yml")
                .diagnostics
                .is_empty(),
            "the value form is the form class 1 asks for and it must be accepted for \
             `journal.url:` on `{provider}`"
        );
    }
}

/// The other end of the bind: the class the table names is the class `validate`
/// enforces.
///
/// A table row nothing checks is a claim, and this file's whole subject is a
/// claim drifting from the compiler. Both directions are asserted because both
/// are failures: a literal accepted would be the secret in the spec text that
/// PRD 5.9 forbids, and an `${ENV}` reference refused would make the mesh
/// unwritable.
#[test]
fn validate_enforces_the_class_the_table_names() {
    const LITERAL: &str = "version: \"0.1\"\nhub:\n  join_token: \"s3cret\"\n";
    const REFERENCE: &str = "version: \"0.1\"\nhub:\n  join_token: ${MESH_JOIN_TOKEN}\n";

    let refused = parse_str(LITERAL, "deploy/mesh.yml").diagnostics;
    assert_eq!(
        refused
            .iter()
            .map(|diagnostic| diagnostic.code.as_str())
            .collect::<Vec<_>>(),
        ["invalid-env-ref"],
        "a literal `hub.join_token:` must be refused as class 1 requires"
    );
    assert!(
        parse_str(REFERENCE, "deploy/mesh.yml")
            .diagnostics
            .is_empty(),
        "the value form is the form class 1 asks for and it must be accepted"
    );
}
