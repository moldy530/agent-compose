//! `src/artifact.ts`: what this generated tree **is**, as the worker wire names
//! it (`docs/distributed.md` §3.5, §4, §4.1).
//!
//! # The hash
//!
//! An artifact is identified by a hash **over its tree's content**, so that "two
//! hubs built from one composition serve one artifact" (§4) is a property of the
//! bytes rather than of who built them. The rule is:
//!
//! ```text
//! sha256( for each file, sorted by path: path "\0" hex(sha256(bytes)) "\n" )
//! ```
//!
//! written `sha256:<hex>`, which is the spelling §3.5 hash-addresses the fetch
//! route by. It is over a *list of digests* rather than over a concatenation of
//! the files themselves for the reason every content-addressed tree format does
//! it: a path and its bytes have to be separable, or two trees whose files were
//! renamed and re-cut to compensate would hash alike.
//!
//! It is deliberately **not** a hash of the tarball §3.5 serves. A tarball
//! carries modification times, ownership and block padding, none of which is the
//! project; two hubs would serve two hashes for one build, and a worker that
//! already held the tree would re-download it after every restart. So the hash is
//! over the tree and the transfer format is free to change without moving it —
//! which is also what lets a worker verify by hashing **the entries it unpacked**
//! before materialising them (§4 step 2).
//!
//! # The one file the hash cannot cover
//!
//! This one. A file carrying the hash of a tree it is part of has no fixed point,
//! so `src/artifact.ts` is excluded from the digest and everything else is
//! inside it — including [`super::deployment`], which is why a renamed placement
//! or a variable that moved between manifests moves the artifact hash. The
//! exclusion is a **constant**, [`SELF`], read by the emitter here and by the
//! worker that re-derives the hash from what it unpacked, so neither end has to
//! guess which entry to leave out.
//!
//! # The file list
//!
//! `ARTIFACT_FILES` is every path this build emitted, sorted — what the hub tars
//! when a worker fetches (§3.5). It is the emitter's list rather than a directory
//! walk so that what is served is what `build` wrote: a `node_modules/` an
//! install left beside it, a `.env`, and the `.agent-compose/` a run filled are
//! not the artifact, and a walk would have to enumerate exclusions the emitter
//! already knows the complement of.

use sha2::{Digest, Sha256};

use crate::ir::Ir;

use super::{GeneratedFile, names};

/// The one emitted path outside the artifact's own digest: this module.
///
/// Read by both ends — the emitter, which skips it when hashing, and a worker,
/// which skips it when verifying what it unpacked (`docs/distributed.md` §4).
pub const SELF: &str = "src/artifact.ts";

/// The content hash of a set of emitted files, as `sha256:<hex>`.
///
/// [`SELF`] is skipped, for the reason the module docs give.
#[must_use]
pub fn hash(files: &[GeneratedFile]) -> String {
    let mut listing = Vec::new();
    let mut paths: Vec<&GeneratedFile> = files.iter().filter(|file| file.path != SELF).collect();
    paths.sort_by(|left, right| left.path.cmp(&right.path));
    for file in paths {
        listing.push(format!(
            "{}\0{}",
            file.path,
            digest(file.contents.as_bytes())
        ));
    }
    format!("sha256:{}", digest(listing.join("\n").as_bytes()))
}

/// One SHA-256, as lowercase hex.
fn digest(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// `src/artifact.ts`, over the rest of the project.
#[must_use]
pub fn module(ir: &Ir, files: &[GeneratedFile]) -> GeneratedFile {
    let mut contents = super::header(ir, "// ");
    contents.push_str(MODULE_DOC);
    contents.push_str(&format!(
        "\nexport const ARTIFACT_HASH = {};\n",
        names::string(&hash(files))
    ));
    contents.push_str(&format!(
        "\nexport const COMPILER_VERSION = {};\n",
        names::string(super::COMPILER_VERSION)
    ));
    contents.push_str("\nexport const ARTIFACT_FILES: readonly string[] = [\n");
    let mut paths: Vec<&str> = files.iter().map(|file| file.path.as_str()).collect();
    paths.push(SELF);
    paths.sort_unstable();
    paths.dedup();
    for path in paths {
        contents.push_str(&format!("  {},\n", names::string(path)));
    }
    contents.push_str("];\n");

    GeneratedFile {
        path: SELF.to_string(),
        contents,
    }
}

const MODULE_DOC: &str = "\
//
// This artifact's identity on the worker wire (`docs/distributed.md` §3.5, §4).
//
// `ARTIFACT_HASH` is a hash over the tree's content — every emitted file's path
// and the digest of its bytes, sorted, hashed — and is what a join agrees on and
// what `/workers/artifact/{hash}` is addressed by. `ARTIFACT_FILES` is the set
// the hub tars for that route: what `agent-compose build` wrote, and nothing an
// install, a run or an operator put beside it.
//
// This file is the one entry the hash does not cover, because a file carrying
// the hash of a tree containing it has no fixed point. Everything else is
// inside it, `./deployment.ts` included — so a placement renamed or a variable
// that moved between manifests is a new artifact, which is what §4.1's handshake
// needs of it.
//
// `COMPILER_VERSION` is the second member of §4.1's handshake triple: the
// agent-compose release that generated this tree, which a join carries and a
// mismatch is refused on, naming both sides.
";

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str, contents: &str) -> GeneratedFile {
        GeneratedFile {
            path: path.to_string(),
            contents: contents.to_string(),
        }
    }

    /// The hash is over the tree, so the order files arrive in cannot move it.
    #[test]
    fn the_hash_is_the_same_whichever_order_the_files_are_emitted_in() {
        let forwards = [file("a.ts", "one\n"), file("b/c.ts", "two\n")];
        let backwards = [file("b/c.ts", "two\n"), file("a.ts", "one\n")];
        assert_eq!(hash(&forwards), hash(&backwards));
        assert!(
            hash(&forwards).starts_with("sha256:"),
            "{}",
            hash(&forwards)
        );
        assert_eq!(hash(&forwards).len(), "sha256:".len() + 64);
    }

    /// …and a byte anywhere in it does move it.
    #[test]
    fn a_changed_byte_anywhere_in_the_tree_is_a_different_artifact() {
        let before = [file("a.ts", "one\n"), file("b/c.ts", "two\n")];
        let after = [file("a.ts", "one\n"), file("b/c.ts", "two!\n")];
        assert_ne!(hash(&before), hash(&after));
    }

    /// A path is separable from its bytes: moving content between two files
    /// without changing the concatenation is still a different tree.
    #[test]
    fn a_file_renamed_is_a_different_artifact_even_at_the_same_bytes() {
        let before = [file("a.ts", "one\n")];
        let after = [file("z.ts", "one\n")];
        assert_ne!(hash(&before), hash(&after));
    }

    /// The self-exclusion is real: whatever this module says about the hash, the
    /// hash does not depend on it.
    #[test]
    fn the_module_that_carries_the_hash_is_outside_it() {
        let tree = [file("a.ts", "one\n")];
        let mut with_self = tree.to_vec();
        with_self.push(file(SELF, "export const ARTIFACT_HASH = \"anything\";\n"));
        assert_eq!(hash(&tree), hash(&with_self));
    }

    /// The known-answer test that says this is SHA-256 rather than some other
    /// 32-byte function: the empty string's digest, as FIPS 180-4 publishes it.
    #[test]
    fn the_digest_is_sha256() {
        assert_eq!(
            digest(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            digest(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
