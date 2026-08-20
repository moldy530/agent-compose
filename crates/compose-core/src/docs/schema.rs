//! The published JSON Schema, embedded.
//!
//! `schemas/agent-compose.schema.json` is what an editor's `$schema` points at
//! for autocomplete and per-file structural validation (grammar Appendix B).
//! An agent working from a released binary has no checkout to point at, so the
//! binary carries the document and `agent-compose schema` writes it out.

/// The published JSON Schema, byte-identical to the committed file.
///
/// **Bytes, not a re-serialization.** A pretty-printer that agreed with the
/// committed file about the document and disagreed about the whitespace would
/// make `agent-compose schema > schemas/agent-compose.schema.json` a diff, and
/// make the URL and the binary two artifacts that are equal only in the
/// reading. [`tests::the_schema_is_the_committed_document`] pins the equality.
pub const SCHEMA: &str = include_str!("../../../../schemas/agent-compose.schema.json");

#[cfg(test)]
mod tests {
    use super::*;

    /// The one property this surface has: what the binary prints is what the
    /// repository publishes.
    #[test]
    fn the_schema_is_the_committed_document() {
        let committed = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .ancestors()
                .nth(2)
                .expect("the manifest directory has a grandparent")
                .join("schemas/agent-compose.schema.json"),
        )
        .expect("the published schema is readable");
        assert_eq!(SCHEMA, committed);
    }

    /// A document that does not parse is one an editor would refuse, and the
    /// binary would hand out anyway.
    #[test]
    fn the_schema_is_one_json_document() {
        let parsed: serde_json::Value =
            serde_json::from_str(SCHEMA).expect("the published schema is JSON");
        assert!(parsed.get("$schema").is_some(), "it declares its dialect");
        assert!(parsed.get("$id").is_some(), "it declares its identity");
    }
}
