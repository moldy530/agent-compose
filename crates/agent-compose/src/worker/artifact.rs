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
//! <data-dir>/held                  the hash this worker is executing out of
//! <data-dir>/artifacts/<hash>/     one tree per hash, so a rollback survives
//! <data-dir>/artifacts/<hash>/.materialised   written last: this tree is whole
//! ```
//!
//! Keyed by hash because §4 step 3 says so — "so the previous artifact survives a
//! rollback" — and `held` because a worker that has just started has to know
//! which of them it is holding without asking anything.
//!
//! **Surviving a rollback is a property of two functions and not of the layout
//! alone.** A tree kept under a name nothing ever looks up again is a directory
//! this worker pays for and never reads, so:
//!
//! * [`adopt`] is what §4 step 1 compares against on the way *back*: a hash this
//!   worker has materialised before is one it holds, whether or not it is the one
//!   it was executing out of, so a hub rotated back to yesterday's artifact costs
//!   a pointer write rather than a download.
//! * [`materialise`] and [`adopt`] both **prune**: what survives is the tree in
//!   hand and the one it replaced, which is exactly the pair §4 step 3's sentence
//!   names. Without it a worker on a mesh that redeploys daily keeps one complete
//!   project tree per deployment for ever.
//!
//! The marker file is what makes an adoption safe. Files are written in path
//! order and the marker after all of them, so a worker killed mid-materialise
//! leaves a tree that [`adopt`] refuses — the alternative, adopting whatever
//! directory carries the right name, would execute half an artifact under a hash
//! that promises the whole of it. It records a verification that has already
//! happened: the bytes were hashed before any of them were written, and the
//! marker says which hash they answered to. Its name is one no build emits
//! (`ARTIFACT_FILES` is the emitter's own list), and a tarball that carried one
//! anyway would have it overwritten rather than believed — it is written last,
//! after every entry the archive held.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::Value;

/// Where a worker keeps its trees, under the data directory it was given.
fn trees(data_dir: &Path) -> PathBuf {
    data_dir.join("artifacts")
}

/// The file naming the hash this worker is executing out of.
fn held_path(data_dir: &Path) -> PathBuf {
    data_dir.join("held")
}

/// The file a whole tree carries, written after every other one.
const MARKER: &str = ".materialised";

/// The artifact this worker is holding, if any — its hash and its tree.
///
/// `None` for a cold start, which is what §3.1 makes a join that omits
/// `artifact_hash` altogether: "a worker that holds no artifact at all — a cold
/// start, the first minute of a new machine's life".
#[must_use]
pub(crate) fn held(data_dir: &Path) -> Option<(String, PathBuf)> {
    let hash = pointer(data_dir)?;
    // The pointer and the tree have to agree: a directory removed under a
    // worker is a worker that holds nothing, not one that holds a name.
    let tree = whole(data_dir, &hash)?;
    Some((hash, tree))
}

/// The hash `held` names, where it names a well-formed one.
fn pointer(data_dir: &Path) -> Option<String> {
    let hash = std::fs::read_to_string(held_path(data_dir)).ok()?;
    let hash = hash.trim().to_string();
    well_formed(&hash).then_some(hash)
}

/// The tree for `hash` under this data directory, where a whole one is there.
///
/// Whole means the marker is present and names this hash — see the module
/// header for why that and not the presence of a file the artifact happens to
/// carry.
fn whole(data_dir: &Path, hash: &str) -> Option<PathBuf> {
    if !well_formed(hash) {
        return None;
    }
    let tree = trees(data_dir).join(hash);
    let stamped = std::fs::read_to_string(tree.join(MARKER)).ok()?;
    (stamped.trim() == hash).then_some(tree)
}

/// Take up an artifact this worker has materialised before, without a download.
///
/// The other half of §4 step 1: the join names a hash, and what this worker
/// holds is not only the one it was last executing out of. A hub rolled back to
/// an artifact still on this disk is answered from disk — which is the property
/// §4 step 3 keys the trees by hash *for*, and without this the retention would
/// be storage nobody ever reads.
///
/// `None` where no whole tree of that hash is here, which sends the caller to
/// the fetch. A pointer this could not write is `None` too: re-downloading an
/// artifact is slow and correct, and the write is attempted again — and reported
/// properly — by [`materialise`].
pub(crate) fn adopt(data_dir: &Path, hash: &str) -> Option<PathBuf> {
    let tree = whole(data_dir, hash)?;
    hold(data_dir, hash).ok()?;
    Some(tree)
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
    // The marker after every file of the tree, so what [`adopt`] later takes up
    // is a tree that was finished rather than one that was started.
    std::fs::write(tree.join(MARKER), format!("{hash}\n"))
        .map_err(|error| format!("cannot mark `{}` materialised: {error}", tree.display()))?;
    // …and the pointer last of all, so a worker killed mid-write comes back
    // holding the artifact it had rather than a name with half a tree behind it.
    hold(data_dir, hash)?;
    Ok(tree)
}

/// Point `held` at `hash`, and drop every tree but this one and the one it
/// replaces (§4 step 3).
///
/// Two, and not one: "so the previous artifact survives a rollback" is a promise
/// about the artifact this one replaced, and a store that kept every tree it
/// ever wrote would honour it by never reclaiming anything — one complete
/// generated project per deployment, for the life of the machine.
///
/// The prune runs **before** the pointer moves, so a worker killed between them
/// comes back holding a tree that is still there: what it would have dropped is
/// what it is no longer pointing at.
fn hold(data_dir: &Path, hash: &str) -> Result<(), String> {
    let replaced = pointer(data_dir);
    let keeping = [Some(hash), replaced.as_deref()];
    if let Ok(entries) = std::fs::read_dir(trees(data_dir)) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let named = name.to_string_lossy();
            if keeping.iter().any(|kept| *kept == Some(named.as_ref())) {
                continue;
            }
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
    std::fs::write(held_path(data_dir), format!("{hash}\n"))
        .map_err(|error| format!("cannot record which artifact this worker holds: {error}"))
}

/// `bun install`, in a materialised tree (§4 step 4).
///
/// Skipped where the pinned dependency set **already resolves at the version
/// this artifact pins** ([`resolves`]). Module resolution walks up, so an
/// artifact materialised beneath a directory that holds that install needs no
/// second copy of it — the arrangement a monorepo and this suite's own toolchain
/// both are. A worker that installed anyway would be re-downloading a dependency
/// set it can already import, on every artifact it is ever served.
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

/// The one package every emitted project imports, and the one §4 step 4 is
/// really about.
const SUBSTRATE: &str = "@langchain/langgraph";

/// Whether the pinned dependency set already resolves from this tree — **at the
/// version this artifact pins**.
///
/// Looked for the way the runtime looks for it: up the directory chain from
/// where the import is made, taking the nearest `node_modules` and no other,
/// because that is the copy an import from inside the tree reaches.
///
/// The version is the half a bare "is the directory there" would drop, and it is
/// the half that matters here. `agent-compose run` asks the same question of a
/// project **built on the machine that runs it** (`launch::dependencies`), and
/// answers a missing install by refusing; a served artifact is neither built
/// here nor refused — it arrived over the wire and was materialised under a data
/// directory the operator chose, which may sit inside an unrelated JavaScript
/// project whose own install is a LangGraph this compiler release never pinned.
/// §4.1: "what pins the required major is the compiler release — the same
/// release that pins the LangGraph version", and the handshake triple refuses a
/// mismatched pair at join for exactly that reason. A skew reached *after* a
/// clean join would be the same fault arriving later and quieter — as a runtime
/// error inside a dispatched node, if it says anything at all — so what an
/// ancestor holds has to be the version the artifact asks for, and where it is
/// not, step 4 runs.
fn resolves(tree: &Path) -> bool {
    let Some(wanted) = pinned(tree) else {
        return false;
    };
    for directory in tree.ancestors() {
        let installed = directory.join("node_modules").join(SUBSTRATE);
        if !installed.is_dir() {
            continue;
        }
        return installed_version(&installed).is_some_and(|found| found == wanted);
    }
    false
}

/// What this artifact's own `package.json` pins [`SUBSTRATE`] at.
///
/// Read out of the tree rather than taken from this binary's own constant: the
/// artifact is what states what it needs, and a worker that trusted its own copy
/// of the pin would be checking the install against a number the tree never
/// mentioned. `None` — an unreadable manifest, or one naming no such dependency
/// — sends the caller to the install, which is the answer that cannot be wrong.
fn pinned(tree: &Path) -> Option<String> {
    let text = std::fs::read_to_string(tree.join("package.json")).ok()?;
    let document: Value = serde_json::from_str(&text).ok()?;
    document
        .get("dependencies")?
        .get(SUBSTRATE)?
        .as_str()
        .map(str::to_string)
}

/// The `version` an installed package's own `package.json` declares.
fn installed_version(package: &Path) -> Option<String> {
    let text = std::fs::read_to_string(package.join("package.json")).ok()?;
    let document: Value = serde_json::from_str(&text).ok()?;
    document.get("version")?.as_str().map(str::to_string)
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

    /// A data directory of this test's own.
    fn scratch(purpose: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "agent-compose-worker-{purpose}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("a scratch directory");
        path
    }

    /// One artifact whose content is `marking`, as a tarball and its hash.
    fn artifact(marking: &str) -> (String, Vec<u8>) {
        let package = format!("{{ \"name\": \"{marking}\", \"private\": true }}\n");
        let manifest = b"{ \"node_runner\": \"src/worker-node.ts\", \"placements\": [] }\n";
        let mut blocks = entry("manifest.json", manifest, b'0');
        blocks.extend(entry("package.json", package.as_bytes(), b'0'));
        blocks.extend(entry("src/graph.ts", marking.as_bytes(), b'0'));
        blocks.extend(vec![0u8; 1024]);
        let tarball = gzipped(blocks);
        let read = entries(&tarball).expect("the archive reads");
        let hash = compose_core::codegen::artifact::hash_of(
            read.iter().map(|(path, bytes)| (path.as_str(), &bytes[..])),
        );
        (hash, tarball)
    }

    /// A hub rolled back to an artifact this worker still has costs no download
    /// (§4 step 3, §3.5's rollback row).
    ///
    /// The reason the trees are keyed by hash at all: "so the previous artifact
    /// survives a rollback". Surviving means being **taken up** — a store that
    /// kept the tree and could not answer out of it would be paying for a
    /// property it does not have, and the fetch would run again over bytes
    /// already on the disk.
    #[test]
    fn a_rollback_to_a_tree_this_worker_still_holds_is_taken_up_without_a_download() {
        let scratch = scratch("rollback");
        let (before, yesterday) = artifact("yesterday");
        let (after, today) = artifact("today");
        let previous = materialise(&scratch, &before, &yesterday).expect("yesterday materialises");
        materialise(&scratch, &after, &today).expect("today materialises");
        assert_eq!(held(&scratch).map(|(hash, _)| hash), Some(after.clone()));

        // The rollback: the hub names the artifact this worker replaced, and it
        // is answered off the disk.
        assert_eq!(
            adopt(&scratch, &before),
            Some(previous.clone()),
            "the tree this worker kept for a rollback was not taken up"
        );
        assert_eq!(
            held(&scratch),
            Some((before.clone(), previous)),
            "the adoption did not become the artifact this worker executes out of"
        );
        // …and the one it rolled back *from* is still there, because that is now
        // the artifact a roll-forward would ask for.
        assert!(trees(&scratch).join(&after).is_dir());
        let _ = std::fs::remove_dir_all(&scratch);
    }

    /// The store is the artifact in hand and the one it replaced, and never a
    /// third (§4 step 3).
    ///
    /// A worker on a mesh that redeploys daily would otherwise keep one complete
    /// generated project per deployment for the life of the machine: nothing in
    /// this module ever removed a tree it was not about to rewrite.
    #[test]
    fn the_store_keeps_the_artifact_in_hand_and_the_one_it_replaced() {
        let scratch = scratch("retention");
        let mut written = Vec::new();
        for marking in ["first", "second", "third"] {
            let (hash, tarball) = artifact(marking);
            materialise(&scratch, &hash, &tarball).expect("it materialises");
            written.push(hash);
        }
        let kept: std::collections::BTreeSet<String> = std::fs::read_dir(trees(&scratch))
            .expect("the store is there")
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            kept,
            [written[1].clone(), written[2].clone()]
                .into_iter()
                .collect(),
            "the store kept something other than the artifact in hand and the one it replaced"
        );
        assert_eq!(
            adopt(&scratch, &written[0]),
            None,
            "a tree this worker dropped was taken up as though it were still there"
        );
        let _ = std::fs::remove_dir_all(&scratch);
    }

    /// A tree that was started and not finished is not one to execute out of.
    ///
    /// What the marker is for: the files are written in path order, so a worker
    /// killed part way through leaves a directory named by a hash whose content
    /// is not that hash's. Adopting it would run half an artifact.
    #[test]
    fn a_tree_left_half_written_is_neither_held_nor_taken_up() {
        let scratch = scratch("half-written");
        let (hash, tarball) = artifact("interrupted");
        let tree = materialise(&scratch, &hash, &tarball).expect("it materialises");
        std::fs::remove_file(tree.join(MARKER)).expect("the marker is removed");
        assert_eq!(adopt(&scratch, &hash), None);
        assert_eq!(
            held(&scratch),
            None,
            "a worker holding a pointer at a tree that was never finished said it holds it"
        );
        let _ = std::fs::remove_dir_all(&scratch);
    }

    /// The version this compiler release pins the execution substrate at, which
    /// is the version an emitted `package.json` names.
    fn pin() -> &'static str {
        compose_core::codegen::project::PINS
            .iter()
            .find(|(package, _)| *package == SUBSTRATE)
            .map(|(_, version)| *version)
            .expect("every emitted project pins the execution substrate")
    }

    /// Give `tree` a `package.json` pinning `wants`, and stage an install of
    /// `holds` where an import from inside the tree would find one.
    fn stage(tree: &Path, wants: &str, holds: &str) {
        std::fs::write(
            tree.join("package.json"),
            format!("{{ \"dependencies\": {{ \"{SUBSTRATE}\": \"{wants}\" }} }}\n"),
        )
        .expect("the artifact's manifest is written");
        let installed = tree.join("node_modules").join(SUBSTRATE);
        std::fs::create_dir_all(&installed).expect("the package is staged");
        std::fs::write(
            installed.join("package.json"),
            format!("{{ \"name\": \"{SUBSTRATE}\", \"version\": \"{holds}\" }}\n"),
        )
        .expect("the staged package's manifest is written");
    }

    /// An install above a materialised tree stands in for §4 step 4 only where
    /// it is the version **this artifact** pins (§4.1).
    ///
    /// What this is written against is a worker started inside an unrelated
    /// JavaScript project — or with a stray `node_modules` anywhere above its
    /// data directory. A skip that asked only whether the directory exists would
    /// leave the artifact's own pins uninstalled and run a dispatched node
    /// against whatever that other install holds: "what pins the required major
    /// is the compiler release — the same release that pins the LangGraph
    /// version", and the handshake triple refuses a mixed pair at join precisely
    /// so that skew cannot happen. Reached this way it would arrive *after* a
    /// clean join, as a runtime error inside a node if it said anything at all.
    #[test]
    fn an_install_above_the_tree_stands_in_only_at_the_version_the_artifact_pins() {
        let scratch = scratch("resolution");
        let (hash, tarball) = artifact("resolvable");
        let tree = materialise(&scratch, &hash, &tarball).expect("it materialises");

        stage(&tree, pin(), "0.0.1-not-the-pin");
        assert!(
            !resolves(&tree),
            "an install of another LangGraph stood in for the one this artifact pins"
        );

        // The same directory, holding what the artifact asks for: this is the
        // copy the import reaches, so step 4 would be a download of what is
        // already there.
        stage(&tree, pin(), pin());
        assert!(
            resolves(&tree),
            "the install this artifact's own pin names was not taken up"
        );

        // An install with no manifest to read a version out of says nothing
        // about which LangGraph it is, and nothing is not the right one.
        std::fs::remove_file(
            tree.join("node_modules")
                .join(SUBSTRATE)
                .join("package.json"),
        )
        .expect("the staged manifest is removed");
        assert!(!resolves(&tree));

        // …and an artifact whose own manifest names no such dependency is one
        // nothing above it can answer for either.
        stage(&tree, pin(), pin());
        std::fs::write(tree.join("package.json"), "{ \"private\": true }\n")
            .expect("the manifest is rewritten");
        assert!(!resolves(&tree));
        let _ = std::fs::remove_dir_all(&scratch);
    }

    /// §4 step 4 runs, in a materialised tree, and is skipped where the pinned
    /// set already resolves.
    ///
    /// The step every other suite elides: `tests/distributed_mesh_acceptance.rs`
    /// roots its workers' data directories under the installed toolchain, so
    /// `resolves` is true there and no install is ever entered. Here it is
    /// false, so this is the one place the command in [`install`] is really run
    /// — over an artifact whose dependency set is empty, which is what makes it
    /// a check on the *step* rather than on a network.
    #[test]
    fn a_materialised_tree_has_its_dependency_set_installed() {
        let Ok(bun) = crate::worker::bun() else {
            assert!(
                std::env::var_os("CI").is_none_or(|value| value.is_empty()),
                "CI runs with Bun installed (`docs/distributed.md` §4.2), so §4 step 4 is \
                 unchecked here rather than skipped"
            );
            eprintln!("skipping: no `bun` on this machine, so §4 step 4 cannot be run");
            return;
        };
        let scratch = scratch("install");
        let (hash, tarball) = artifact("installable");
        let tree = materialise(&scratch, &hash, &tarball).expect("it materialises");
        assert!(
            !resolves(&tree),
            "this scratch tree already resolves what it pins, so the install below would be \
             skipped and this test would check nothing: {}",
            tree.display()
        );
        install(&bun, &tree).expect("the install runs in the materialised tree");
        assert!(
            tree.join("node_modules").is_dir(),
            "`bun install` answered success and left no install behind: {}",
            tree.display()
        );
        // …and a tree that already resolves the pinned set skips the step rather
        // than repeating it: "a worker that installed anyway would be
        // re-downloading a dependency set it can already import". The program
        // handed over is one that does not exist, so an install that ran at all
        // would fail here.
        stage(&tree, pin(), pin());
        assert!(resolves(&tree));
        install(Path::new("/no/such/bun"), &tree).expect("a resolvable tree runs no install");

        // An install that cannot run is the provisioning cycle's failure, and it
        // says which tree and what the command said.
        let broken = trees(&scratch).join("broken");
        std::fs::create_dir_all(&broken).expect("a second tree");
        std::fs::write(broken.join("package.json"), "{ not a manifest\n").expect("it is written");
        let refused = install(&bun, &broken).expect_err("a tree that cannot install");
        assert!(
            refused.contains(&broken.display().to_string()),
            "the failure does not name the tree it happened in: {refused}"
        );
        let _ = std::fs::remove_dir_all(&scratch);
    }
}
