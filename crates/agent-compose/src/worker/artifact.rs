//! The artifact, on a worker's disk (`docs/distributed.md` §3.5, §4).
//!
//! Four steps of §4's five live here — fetch, **verify**, materialise, install —
//! and the fifth (join again) is [`super`]'s, because it is a protocol step
//! rather than a filesystem one.
//!
//! # Verify before materialise
//!
//! §4 step 2 is the one instruction in this file with a security property behind
//! it: "verifies the hash it computed against the hash it asked for before
//! unpacking anything". The tarball is read into memory, expanded into a set of
//! `(path, bytes)` pairs, hashed with **the compiler's own function**
//! ([`compose_core::codegen::artifact::hash_of`]) and only then written to disk.
//! A worker that wrote first and hashed afterwards would have executed nothing —
//! but it would have put a hub's answer inside the directory it executes out of,
//! and the whole point of hash-addressing the route (§3.5) is that a body is
//! checkable against the name it was asked for.
//!
//! There is no second implementation of the hash on this side, and that is
//! deliberate: `crates/compose-core/src/codegen/artifact.rs` computes it when the
//! tree is emitted, this reads it back over bytes, and `src/mesh.ts`'s
//! `contentHash` is the third — one rule, three call sites, pinned by
//! `the_artifact_hash_is_the_same_in_both_languages`.
//!
//! # The tar reader
//!
//! Written by hand, exactly as `src/mesh.ts` writes the archive by hand and for
//! the same reason it gives: the hub emits a ustar archive of regular files with
//! every header field but the name and the size fixed, and reading one back is
//! forty lines of a format that has not moved since 1988. What a dependency
//! would add here is a general reader for a format this worker only ever meets
//! one writer of.
//!
//! It is still **hostile input** — a tarball is a hub's answer, and §9.3 makes
//! holding the join token being trusted with the mesh, not the other way round —
//! so [`entries`] refuses an absolute path, a `..` component and a name that
//! escapes the tree, and answers what it read rather than writing as it goes.
//!
//! # The data directory
//!
//! ```text
//! <data-dir>/held                  the hash this worker materialised last
//! <data-dir>/artifacts/<hash>/     one tree per hash, so a rollback survives
//! ```
//!
//! Keyed by hash because §4 step 3 says so — "so the previous artifact survives a
//! rollback" — and `held` because a worker that has just started has to know
//! which of them it is holding without asking anything.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::Value;

/// Where a worker keeps its trees, under the data directory it was given.
fn trees(data_dir: &Path) -> PathBuf {
    data_dir.join("artifacts")
}

/// The file naming the hash this worker materialised last.
fn held_path(data_dir: &Path) -> PathBuf {
    data_dir.join("held")
}

/// The artifact this worker is holding, if any — its hash and its tree.
///
/// `None` for a cold start, which is what §3.1 makes a join that omits
/// `artifact_hash` altogether: "a worker that holds no artifact at all — a cold
/// start, the first minute of a new machine's life".
#[must_use]
pub(crate) fn held(data_dir: &Path) -> Option<(String, PathBuf)> {
    let hash = std::fs::read_to_string(held_path(data_dir)).ok()?;
    let hash = hash.trim().to_string();
    if !well_formed(&hash) {
        return None;
    }
    let tree = trees(data_dir).join(&hash);
    // The pointer and the tree have to agree: a directory removed under a
    // worker is a worker that holds nothing, not one that holds a name.
    tree.join("manifest.json").is_file().then_some((hash, tree))
}

/// Whether a string is an artifact hash as §3.5 writes one.
fn well_formed(hash: &str) -> bool {
    hash.len() == "sha256:".len() + 64
        && hash.starts_with("sha256:")
        && hash[7..].bytes().all(|byte| byte.is_ascii_hexdigit())
        && hash[7..].bytes().all(|byte| !byte.is_ascii_uppercase())
}

/// Unpack, verify and materialise one artifact under `data_dir` (§4 steps 2–4).
///
/// Answers the tree it wrote. The install of step 4 is [`install`], run by the
/// caller, because a failed install leaves a materialised tree that the next
/// attempt can install into rather than re-download.
pub(crate) fn materialise(data_dir: &Path, hash: &str, tarball: &[u8]) -> Result<PathBuf, String> {
    let files = entries(tarball)?;
    if files.is_empty() {
        return Err("the artifact this hub served holds no files".to_string());
    }
    // **Before anything is written.** See the module header.
    let computed = compose_core::codegen::artifact::hash_of(
        files
            .iter()
            .map(|(path, bytes)| (path.as_str(), &bytes[..])),
    );
    if computed != hash {
        return Err(format!(
            "the artifact this hub served hashes to `{computed}` and was asked for as `{hash}`: \
             the body does not match the name it is addressed by, so nothing was written \
             (docs/distributed.md §3.5, §4)"
        ));
    }

    let tree = trees(data_dir).join(hash);
    // A previous half-materialised tree of this hash is replaced rather than
    // merged: what is on disk has to be exactly what the hash names.
    let _ = std::fs::remove_dir_all(&tree);
    for (path, bytes) in &files {
        let target = tree.join(path);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("cannot create `{}`: {error}", parent.display()))?;
        }
        std::fs::write(&target, bytes)
            .map_err(|error| format!("cannot write `{}`: {error}", target.display()))?;
    }
    // The pointer last, so a worker killed mid-write comes back holding the
    // artifact it had rather than a name with half a tree behind it.
    std::fs::write(held_path(data_dir), format!("{hash}\n"))
        .map_err(|error| format!("cannot record which artifact this worker holds: {error}"))?;
    Ok(tree)
}

/// `bun install`, in a materialised tree (§4 step 4).
///
/// Skipped where the pinned dependency set **already resolves** from the tree.
/// Module resolution walks up, so an artifact materialised beneath a directory
/// that holds the install needs no second copy of it — which is the same reading
/// `agent-compose run` takes of the same question (`launch::dependencies`), and
/// the arrangement a monorepo and this suite's own toolchain both are. A worker
/// that installed anyway would be re-downloading a dependency set it can already
/// import, on every artifact it is ever served.
pub(crate) fn install(bun: &Path, tree: &Path) -> Result<(), String> {
    if resolves(tree) {
        return Ok(());
    }
    let produced = Command::new(bun)
        .arg("install")
        .current_dir(tree)
        .stdin(Stdio::null())
        .output()
        .map_err(|error| format!("cannot run `{} install`: {error}", bun.display()))?;
    if produced.status.success() {
        return Ok(());
    }
    Err(format!(
        "`{} install` failed in `{}`: {}",
        bun.display(),
        tree.display(),
        String::from_utf8_lossy(&produced.stderr).trim()
    ))
}

/// Whether the pinned dependency set already resolves from this tree.
///
/// The one package every emitted project imports, looked for the way the
/// runtime looks for it: up the directory chain from where the import is made.
fn resolves(tree: &Path) -> bool {
    tree.ancestors()
        .any(|directory| directory.join("node_modules/@langchain/langgraph").is_dir())
}

/// What one materialised artifact says a worker needs — `manifest.json`.
pub(crate) struct Manifest {
    /// The entry a dispatch is executed through, relative to the tree.
    pub(crate) runner: String,
    /// Each placement's environment, as `docs/distributed.md` §9.1 partitions it.
    pub(crate) placements: BTreeMap<String, Vec<String>>,
}

impl Manifest {
    /// Read the manifest out of a materialised tree.
    pub(crate) fn read(tree: &Path) -> Result<Self, String> {
        let path = tree.join("manifest.json");
        let text = std::fs::read_to_string(&path)
            .map_err(|error| format!("cannot read `{}`: {error}", path.display()))?;
        let document: Value = serde_json::from_str(&text)
            .map_err(|error| format!("`{}` is not JSON: {error}", path.display()))?;
        let runner = document
            .get("node_runner")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("`{}` names no node runner", path.display()))?
            .to_string();
        let mut placements = BTreeMap::new();
        for entry in document
            .get("placements")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(name) = entry.get("name").and_then(Value::as_str) else {
                continue;
            };
            let variables = entry
                .get("environment")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect();
            placements.insert(name.to_string(), variables);
        }
        Ok(Self { runner, placements })
    }

    /// The `env_ok` report for these claims: the variables of their manifests
    /// **this machine has set** (§3.1, §9.2).
    ///
    /// Names only, and only names this manifest already holds: "a worker that
    /// shipped the names of its whole environment would be telling the hub about
    /// every unrelated credential on the box" (§9.2). A claim naming no
    /// placement in this manifest contributes nothing, because the join it is
    /// carried on is about to be refused `400` for exactly that (§3.1).
    #[must_use]
    pub(crate) fn env_ok(&self, claims: &[String]) -> Vec<String> {
        let mut reported: Vec<String> = Vec::new();
        for claim in claims {
            let Some(variables) = self.placements.get(claim) else {
                continue;
            };
            for name in variables {
                if std::env::var_os(name).is_none() || reported.contains(name) {
                    continue;
                }
                reported.push(name.clone());
            }
        }
        reported.sort();
        reported
    }
}

/// Every regular file in a gzipped ustar archive, by path.
///
/// Answers what it read; nothing is written. See the module header for why the
/// reader is here and what it refuses.
pub(crate) fn entries(tarball: &[u8]) -> Result<Vec<(String, Vec<u8>)>, String> {
    let mut expanded = Vec::new();
    flate2::read::GzDecoder::new(tarball)
        .read_to_end(&mut expanded)
        .map_err(|error| format!("the artifact this hub served is not gzip: {error}"))?;

    let mut found: Vec<(String, Vec<u8>)> = Vec::new();
    let mut at = 0usize;
    while at + 512 <= expanded.len() {
        let header = &expanded[at..at + 512];
        if header.iter().all(|byte| *byte == 0) {
            break;
        }
        at += 512;
        let name = field(header, 0, 100);
        let size = octal(header, 124, 12)
            .ok_or_else(|| format!("`{name}` carries a size this reader cannot read"))?;
        let size = usize::try_from(size).map_err(|_| format!("`{name}` is larger than memory"))?;
        let blocks = size.div_ceil(512) * 512;
        if at + blocks > expanded.len() {
            return Err(format!("`{name}` runs past the end of the archive"));
        }
        let kind = header[156];
        let body = expanded[at..at + size].to_vec();
        at += blocks;
        // `0` and NUL are both a regular file; everything else — a directory, a
        // link, a device, one of GNU's extension headers — is not something this
        // hub writes, and a reader that guessed at one would be reading a format
        // the writer does not produce.
        if kind != b'0' && kind != 0 {
            return Err(format!(
                "`{name}` is not a regular file, and this artifact's archive holds only files"
            ));
        }
        let path = safe(&name)?;
        found.push((path, body));
    }
    found.sort_by(|left, right| left.0.cmp(&right.0));
    found.dedup_by(|left, right| left.0 == right.0);
    Ok(found)
}

/// One NUL-terminated header field, as text.
fn field(header: &[u8], at: usize, len: usize) -> String {
    let slice = &header[at..at + len];
    let end = slice.iter().position(|byte| *byte == 0).unwrap_or(len);
    String::from_utf8_lossy(&slice[..end]).into_owned()
}

/// One octal header field.
fn octal(header: &[u8], at: usize, len: usize) -> Option<u64> {
    let text = field(header, at, len);
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Some(0);
    }
    u64::from_str_radix(trimmed, 8).ok()
}

/// One entry's path, refused where it would not stay inside the tree.
fn safe(name: &str) -> Result<String, String> {
    if name.is_empty() {
        return Err("the archive holds an entry with no name".to_string());
    }
    if name.starts_with('/') || name.contains('\\') || name.contains('\0') {
        return Err(format!("`{name}` is not a path inside this artifact"));
    }
    for component in name.split('/') {
        if component.is_empty() || component == "." || component == ".." {
            return Err(format!("`{name}` is not a path inside this artifact"));
        }
    }
    Ok(name.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One ustar entry, written the way `src/mesh.ts` writes it.
    fn entry(name: &str, body: &[u8], kind: u8) -> Vec<u8> {
        let mut header = vec![0u8; 512];
        header[..name.len()].copy_from_slice(name.as_bytes());
        header[100..108].copy_from_slice(b"0000644\0");
        header[108..116].copy_from_slice(b"0000000\0");
        header[116..124].copy_from_slice(b"0000000\0");
        let size = format!("{:011o}\0", body.len());
        header[124..136].copy_from_slice(size.as_bytes());
        header[136..148].copy_from_slice(b"00000000000\0");
        header[148..156].copy_from_slice(b"        ");
        header[156] = kind;
        header[257..263].copy_from_slice(b"ustar\0");
        header[263..265].copy_from_slice(b"00");
        let checksum: u32 = header.iter().map(|byte| u32::from(*byte)).sum();
        let written = format!("{checksum:06o}\0 ");
        header[148..156].copy_from_slice(written.as_bytes());
        let mut block = header;
        block.extend_from_slice(body);
        let padding = (512 - body.len() % 512) % 512;
        block.extend(std::iter::repeat_n(0u8, padding));
        block
    }

    fn gzipped(blocks: Vec<u8>) -> Vec<u8> {
        use std::io::Write;
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(&blocks).expect("the encoder takes it");
        encoder.finish().expect("the encoder finishes")
    }

    #[test]
    fn a_two_file_archive_reads_back_as_its_two_files() {
        let mut blocks = entry("src/graph.ts", b"one\n", b'0');
        blocks.extend(entry("package.json", b"{}\n", b'0'));
        blocks.extend(vec![0u8; 1024]);
        let read = entries(&gzipped(blocks)).expect("the archive reads");
        assert_eq!(
            read,
            [
                ("package.json".to_string(), b"{}\n".to_vec()),
                ("src/graph.ts".to_string(), b"one\n".to_vec()),
            ]
        );
    }

    /// A file whose size is an exact multiple of the block size still leaves the
    /// reader at the next header — the padding rule's boundary.
    #[test]
    fn a_file_that_fills_its_blocks_exactly_is_followed_by_the_next_entry() {
        let body = vec![b'x'; 1024];
        let mut blocks = entry("src/big.ts", &body, b'0');
        blocks.extend(entry("src/after.ts", b"after\n", b'0'));
        blocks.extend(vec![0u8; 1024]);
        let read = entries(&gzipped(blocks)).expect("the archive reads");
        assert_eq!(read.len(), 2, "{read:?}");
        assert_eq!(read[0].0, "src/after.ts");
        assert_eq!(read[1].1, body);
    }

    /// An entry that would be written outside the tree is refused, and refused
    /// before anything is written — [`entries`] writes nothing at all.
    #[test]
    fn an_entry_that_escapes_the_tree_is_refused() {
        for name in ["../escape.ts", "/etc/passwd", "src/../../out.ts"] {
            let mut blocks = entry(name, b"x\n", b'0');
            blocks.extend(vec![0u8; 1024]);
            let refused = entries(&gzipped(blocks)).expect_err("the path is refused");
            assert!(refused.contains(name), "{refused}");
        }
    }

    /// …and so is an entry that is not a regular file.
    #[test]
    fn an_entry_that_is_not_a_regular_file_is_refused() {
        let mut blocks = entry("src/link.ts", b"", b'2');
        blocks.extend(vec![0u8; 1024]);
        let refused = entries(&gzipped(blocks)).expect_err("the type is refused");
        assert!(refused.contains("regular file"), "{refused}");
    }

    /// A body that does not hash to the name it was asked for writes nothing
    /// (§4 step 2).
    #[test]
    fn a_tarball_that_does_not_match_its_hash_materialises_nothing() {
        let scratch = std::env::temp_dir().join(format!(
            "agent-compose-worker-artifact-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(&scratch).expect("a scratch directory");
        let mut blocks = entry("src/graph.ts", b"one\n", b'0');
        blocks.extend(vec![0u8; 1024]);
        let asked = format!("sha256:{}", "0".repeat(64));
        let refused = materialise(&scratch, &asked, &gzipped(blocks)).expect_err("refused");
        assert!(refused.contains(&asked), "{refused}");
        assert!(!trees(&scratch).join(&asked).exists());
        assert!(held(&scratch).is_none());
        let _ = std::fs::remove_dir_all(&scratch);
    }

    /// …and one that does is written, pointed at, and read back.
    #[test]
    fn a_matching_tarball_becomes_the_tree_this_worker_holds() {
        let scratch = std::env::temp_dir().join(format!(
            "agent-compose-worker-materialise-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(&scratch).expect("a scratch directory");
        let manifest = b"{\n  \"node_runner\": \"src/worker-node.ts\",\n  \"placements\": [\n    { \"name\": \"mac\", \"environment\": [\"KEYCHAIN_PASSWORD\"] }\n  ]\n}\n";
        let mut blocks = entry("manifest.json", manifest, b'0');
        blocks.extend(entry("src/graph.ts", b"one\n", b'0'));
        // The one entry the hash never covers, carried like any other file.
        blocks.extend(entry(
            "src/artifact.ts",
            b"export const ARTIFACT_HASH = \"\";\n",
            b'0',
        ));
        blocks.extend(vec![0u8; 1024]);
        let tarball = gzipped(blocks);
        let hash = compose_core::codegen::artifact::hash_of(
            entries(&tarball)
                .expect("the archive reads")
                .iter()
                .map(|(path, bytes)| (path.as_str(), &bytes[..]))
                .collect::<Vec<_>>()
                .into_iter(),
        );
        let tree = materialise(&scratch, &hash, &tarball).expect("it materialises");
        assert_eq!(held(&scratch), Some((hash.clone(), tree.clone())));
        let read = Manifest::read(&tree).expect("the manifest reads");
        assert_eq!(read.runner, "src/worker-node.ts");
        assert_eq!(
            read.placements.get("mac").map(Vec::as_slice),
            Some(["KEYCHAIN_PASSWORD".to_string()].as_slice())
        );
        // The report is what this machine has, never what the manifest names:
        // `KEYCHAIN_PASSWORD` is not set here, so nothing is reported.
        assert!(read.env_ok(&["mac".to_string()]).is_empty());
        let _ = std::fs::remove_dir_all(&scratch);
    }
}
