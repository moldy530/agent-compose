//! The expanded explanations: `agent-compose explain <code>`.
//!
//! rustc's `--explain` for this compiler. A diagnostic is one line and a span;
//! an explanation is what the check protects, a minimal spec that triggers it,
//! the fix, and where the grammar says so. The second of those is what makes it
//! worth the bytes — a reader who can *reproduce* a failure in six lines
//! understands the rule, and one reading prose about it may not.
//!
//! # The bind
//!
//! [`explanation`] is an **exhaustive `match`** over [`DiagnosticCode`], and
//! that is deliberate: it is a compile error, not a test failure, so a new
//! failure class does not build until it has an explanation. Nothing about
//! shipping a code can outrun the document that explains it.
//!
//! The other direction — an explanation whose file nothing reads — compiles
//! fine, so it is a test:
//! `crates/agent-compose/tests/discovery_surface_inventory.rs`. And the examples
//! themselves are **run**, in
//! `crates/compose-core/tests/documented_examples_validate.rs`: a block marked
//! ```` ```yaml triggers ```` must report the code its file is named for, so an
//! example that stopped triggering is a test failure rather than a lie inside a
//! released binary.

use crate::diag::DiagnosticCode;
use crate::parse::reader::suggest;

/// The diagnostic code spelled `name`, if it is one.
///
/// The spelling is the one the code is *reported* under, so `explain` takes
/// exactly what a reader copies out of a diagnostic.
#[must_use]
pub fn named(name: &str) -> Option<DiagnosticCode> {
    DiagnosticCode::ALL
        .iter()
        .copied()
        .find(|code| code.as_str() == name)
}

/// Every code, in the order the registry declares them.
#[must_use]
pub fn names() -> Vec<&'static str> {
    DiagnosticCode::ALL
        .iter()
        .map(|code| code.as_str())
        .collect()
}

/// The closest code to `name`, when one is close enough to suggest.
#[must_use]
pub fn nearest(name: &str) -> Option<&'static str> {
    suggest(name, &names())
}

/// The expanded explanation of one diagnostic code.
///
/// Exhaustive on purpose — see the module header.
#[must_use]
pub fn explanation(code: DiagnosticCode) -> &'static str {
    match code {
        DiagnosticCode::IoError => include_str!("codes/io-error.md"),
        DiagnosticCode::InvalidEncoding => include_str!("codes/invalid-encoding.md"),
        DiagnosticCode::YamlSyntax => include_str!("codes/yaml-syntax.md"),
        DiagnosticCode::EmptyDocument => include_str!("codes/empty-document.md"),
        DiagnosticCode::MultipleDocuments => include_str!("codes/multiple-documents.md"),
        DiagnosticCode::RootNotMapping => include_str!("codes/root-not-mapping.md"),
        DiagnosticCode::NonStringKey => include_str!("codes/non-string-key.md"),
        DiagnosticCode::DuplicateKey => include_str!("codes/duplicate-key.md"),
        DiagnosticCode::MergeKey => include_str!("codes/merge-key.md"),
        DiagnosticCode::YamlTag => include_str!("codes/yaml-tag.md"),
        DiagnosticCode::MisplacedSection => include_str!("codes/misplaced-section.md"),
        DiagnosticCode::UnsupportedVersion => include_str!("codes/unsupported-version.md"),
        DiagnosticCode::InvalidImportPath => include_str!("codes/invalid-import-path.md"),
        DiagnosticCode::InvalidModulePath => include_str!("codes/invalid-module-path.md"),
        DiagnosticCode::InvalidDependency => include_str!("codes/invalid-dependency.md"),
        DiagnosticCode::UnknownKey => include_str!("codes/unknown-key.md"),
        DiagnosticCode::MissingKey => include_str!("codes/missing-key.md"),
        DiagnosticCode::WrongType => include_str!("codes/wrong-type.md"),
        DiagnosticCode::InvalidValue => include_str!("codes/invalid-value.md"),
        DiagnosticCode::ValueOutOfRange => include_str!("codes/value-out-of-range.md"),
        DiagnosticCode::UnknownVariant => include_str!("codes/unknown-variant.md"),
        DiagnosticCode::ConflictingKeys => include_str!("codes/conflicting-keys.md"),
        DiagnosticCode::MissingCredential => include_str!("codes/missing-credential.md"),
        DiagnosticCode::InvalidIdentifier => include_str!("codes/invalid-identifier.md"),
        DiagnosticCode::InvalidReference => include_str!("codes/invalid-reference.md"),
        DiagnosticCode::InvalidDuration => include_str!("codes/invalid-duration.md"),
        DiagnosticCode::InvalidPathExpression => include_str!("codes/invalid-path-expression.md"),
        DiagnosticCode::InvalidEnvRef => include_str!("codes/invalid-env-ref.md"),
        DiagnosticCode::UnexpectedEnvRef => include_str!("codes/unexpected-env-ref.md"),
        DiagnosticCode::ReservedName => include_str!("codes/reserved-name.md"),
        DiagnosticCode::DuplicateDefinition => include_str!("codes/duplicate-definition.md"),
        DiagnosticCode::DuplicateSection => include_str!("codes/duplicate-section.md"),
        DiagnosticCode::UndefinedReference => include_str!("codes/undefined-reference.md"),
        DiagnosticCode::VersionMismatch => include_str!("codes/version-mismatch.md"),
        DiagnosticCode::InvalidExpression => include_str!("codes/invalid-expression.md"),
        DiagnosticCode::UnknownRoot => include_str!("codes/unknown-root.md"),
        DiagnosticCode::UnknownField => include_str!("codes/unknown-field.md"),
        DiagnosticCode::TypeMismatch => include_str!("codes/type-mismatch.md"),
        DiagnosticCode::UndefinedChannel => include_str!("codes/undefined-channel.md"),
        DiagnosticCode::UnreducedWrite => include_str!("codes/unreduced-write.md"),
        DiagnosticCode::ConflictingWrites => include_str!("codes/conflicting-writes.md"),
        DiagnosticCode::MissingBinding => include_str!("codes/missing-binding.md"),
        DiagnosticCode::UnboundedFanOut => include_str!("codes/unbounded-fan-out.md"),
        DiagnosticCode::NonExhaustive => include_str!("codes/non-exhaustive.md"),
        DiagnosticCode::DetachedWrite => include_str!("codes/detached-write.md"),
        DiagnosticCode::UnkeyedMapWrite => include_str!("codes/unkeyed-map-write.md"),
        DiagnosticCode::ToolNameCollision => include_str!("codes/tool-name-collision.md"),
        DiagnosticCode::MissingSessionKey => include_str!("codes/missing-session-key.md"),
        DiagnosticCode::MissingCapability => include_str!("codes/missing-capability.md"),
        DiagnosticCode::UnsupportedServerTools => {
            include_str!("codes/unsupported-server-tools.md")
        }
        DiagnosticCode::UnknownServerTool => include_str!("codes/unknown-server-tool.md"),
        DiagnosticCode::UnknownServerToolField => {
            include_str!("codes/unknown-server-tool-field.md")
        }
        DiagnosticCode::MismatchedServerTools => {
            include_str!("codes/mismatched-server-tools.md")
        }
        DiagnosticCode::UnsupportedHarness => include_str!("codes/unsupported-harness.md"),
        DiagnosticCode::UnsupportedPermissionMode => {
            include_str!("codes/unsupported-permission-mode.md")
        }
        DiagnosticCode::WideningPermissionMode => {
            include_str!("codes/widening-permission-mode.md")
        }
        DiagnosticCode::UnknownHarnessSetting => {
            include_str!("codes/unknown-harness-setting.md")
        }
        DiagnosticCode::ReservedHarnessSetting => {
            include_str!("codes/reserved-harness-setting.md")
        }
        DiagnosticCode::SharedWorkspace => include_str!("codes/shared-workspace.md"),
        DiagnosticCode::UnsupportedConnectionFact => {
            include_str!("codes/unsupported-connection-fact.md")
        }
        DiagnosticCode::UnsupportedProviderKind => {
            include_str!("codes/unsupported-provider-kind.md")
        }
        DiagnosticCode::ConflictingConnectionVariable => {
            include_str!("codes/conflicting-connection-variable.md")
        }
        DiagnosticCode::DeadEnd => include_str!("codes/dead-end.md"),
        DiagnosticCode::UnboundedCycle => include_str!("codes/unbounded-cycle.md"),
        DiagnosticCode::UnbalancedConvergence => include_str!("codes/unbalanced-convergence.md"),
        DiagnosticCode::UnreachableNode => include_str!("codes/unreachable-node.md"),
        DiagnosticCode::RecursiveFlow => include_str!("codes/recursive-flow.md"),
        DiagnosticCode::SyncTriggerInterrupt => include_str!("codes/sync-trigger-interrupt.md"),
        DiagnosticCode::NonDominatingSource => include_str!("codes/non-dominating-source.md"),
        DiagnosticCode::UnsupportedDetach => include_str!("codes/unsupported-detach.md"),
        DiagnosticCode::DetachedInterrupt => include_str!("codes/detached-interrupt.md"),
        DiagnosticCode::DuplicateRoute => include_str!("codes/duplicate-route.md"),
        DiagnosticCode::ConflictingSessionKey => include_str!("codes/conflicting-session-key.md"),
        DiagnosticCode::MissingCallbackAllowlist => {
            include_str!("codes/missing-callback-allowlist.md")
        }
        DiagnosticCode::UnsupportedPlacement => include_str!("codes/unsupported-placement.md"),
        DiagnosticCode::ConflictingPlacement => include_str!("codes/conflicting-placement.md"),
        DiagnosticCode::MissingJoinToken => include_str!("codes/missing-join-token.md"),
        DiagnosticCode::ProcessLocalStore => include_str!("codes/process-local-store.md"),
        DiagnosticCode::ConflictingRegistryCredential => {
            include_str!("codes/conflicting-registry-credential.md")
        }
        DiagnosticCode::MissingRegistryToken => include_str!("codes/missing-registry-token.md"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A code is looked up by the same spelling it is reported under, and a
    /// near miss is answered rather than refused blankly (PRD G3).
    #[test]
    fn codes_round_trip_through_their_kebab_spelling() {
        for code in DiagnosticCode::ALL {
            assert_eq!(named(code.as_str()), Some(*code));
        }
        assert_eq!(named("no-such-code"), None);
        assert_eq!(nearest("unknwon-key"), Some("unknown-key"));
    }

    /// The four parts every explanation is written in, held to the code's own
    /// name: a heading that is the code, what the check protects, an example,
    /// and a cross-reference into the normative text.
    ///
    /// Held here rather than in an integration test because the `match` above
    /// is the only thing that says *which* document belongs to a code — a test
    /// walking the directory would check the same files without checking that
    /// pairing.
    #[test]
    fn every_explanation_is_written_in_the_same_four_parts() {
        for code in DiagnosticCode::ALL {
            let text = explanation(*code);
            let name = code.as_str();
            assert!(
                text.starts_with(&format!("# {name}\n")),
                "`{name}`'s explanation opens with its own code"
            );
            for heading in [
                "## What it protects",
                "## A spec that triggers it",
                "## The fix",
            ] {
                assert!(
                    text.contains(heading),
                    "`{name}`'s explanation has a `{heading}` section"
                );
            }
            let closing = text
                .lines()
                .rev()
                .find(|line| !line.trim().is_empty())
                .unwrap_or_default();
            assert!(
                text.contains("\nGrammar: "),
                "`{name}`'s explanation cross-references the normative text"
            );
            assert!(
                closing.contains("agent-compose docs "),
                "`{name}`'s explanation ends by naming a topic to read, found `{closing}`"
            );
        }
    }
}
