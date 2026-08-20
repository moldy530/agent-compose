//! The two discovery verbs that touch the file system.
//!
//! Everything else in the discovery surface writes a document to stdout and is
//! therefore one line in `main.rs`. `init` and `skill --agent claude` **write
//! files**, which makes them the two that need a refusal story: a compiler that
//! silently replaced a file a person wrote is worse than one that did nothing,
//! and both of these run in a directory the user already lives in.
//!
//! The two refusals differ because what they are protecting differs. `init`
//! scaffolds a *project*, so what it protects is the **directory**: anything
//! already in it is somebody's, and this command has no way to tell a stale
//! build from a checkout. `skill` installs one known document into a directory
//! whose whole purpose is holding such documents, so what it protects is that
//! one file's **contents** — a re-install of the same skill is the common case
//! and must not be a failure, while a file that differs is a customization
//! nobody asked this command to discard.

use std::fs;
use std::path::{Path, PathBuf};

use compose_core::docs;

/// Why a discovery verb wrote nothing.
pub(crate) enum Refusal {
    /// The command's own precondition failed — exit `2`.
    Unusable(String),
    /// There was something here already, and it is not this command's to
    /// replace — exit `1`.
    Occupied(String),
}

/// What a write did, for the line the command prints afterwards.
pub(crate) enum Wrote {
    /// The file was created.
    Created(PathBuf),
    /// The file was already exactly this document.
    Unchanged(PathBuf),
}

/// `agent-compose init [<dir>]`: write the scaffold into an empty directory.
///
/// The directory must be **empty or absent**, and "empty" counts every entry
/// including hidden ones. The rule is deliberately blunt: this command cannot
/// tell a directory holding a stale `build/` from one holding a checkout, and
/// the two remedies a blunt refusal leaves — name a subdirectory, or empty the
/// one you meant — are both one command, while a wrong guess replaces
/// somebody's `main.yml`. A user who wanted to scaffold beside existing work
/// types `agent-compose init <name>`, which the refusal says.
pub(crate) fn init(directory: &Path) -> Result<PathBuf, Refusal> {
    match fs::read_dir(directory) {
        Ok(entries) => {
            let mut held: Vec<String> = entries
                .filter_map(|entry| entry.ok())
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect();
            held.sort();
            if !held.is_empty() {
                return Err(Refusal::Occupied(format!(
                    "`{}` is not empty (it holds {}): `init` writes into an empty or absent \
                     directory only. Name one of its own — `agent-compose init <name>` — or clear \
                     this one",
                    directory.display(),
                    named(&held),
                )));
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir_all(directory).map_err(|error| {
                Refusal::Unusable(format!("cannot create `{}`: {error}", directory.display()))
            })?;
        }
        Err(error) => {
            return Err(Refusal::Unusable(format!(
                "cannot read `{}`: {error}",
                directory.display()
            )));
        }
    }

    let path = directory.join(docs::SCAFFOLD_NAME);
    fs::write(&path, docs::SCAFFOLD).map_err(|error| {
        Refusal::Unusable(format!("cannot write `{}`: {error}", path.display()))
    })?;
    Ok(path)
}

/// At most three names, so a refusal about a directory of two hundred files is
/// still one line.
fn named(held: &[String]) -> String {
    let shown: Vec<String> = held
        .iter()
        .take(3)
        .map(|name| format!("`{name}`"))
        .collect();
    match held.len().saturating_sub(shown.len()) {
        0 => shown.join(", "),
        rest => format!("{}, and {rest} more", shown.join(", ")),
    }
}

/// Write `document` to `path`, creating the directories above it.
///
/// **Identical content is success and writes nothing.** Re-running an install
/// is how a user picks up a newer skill, and the run that finds nothing to do
/// should say so rather than fail — a refusal there would make "keep this in
/// step" a command that fails whenever it is already in step.
///
/// A file that **differs** is refused with its path. This command cannot tell a
/// customization from a stale copy and the user can, so it names the file and
/// leaves the decision with them; `agent-compose skill` prints the document for
/// a merge.
///
/// The comparison is over **bytes**, not text, and that is the whole reason
/// this reads with [`fs::read`]. What the refusal protects is that one file's
/// contents, so a file whose bytes are not this document is the refusal's case
/// whatever those bytes are — including a file that is not UTF-8 at all.
/// Decoding first would sort that file under "cannot read", which exits `2` and
/// tells a supervisor the command could not run, when the true answer is `1`:
/// the command ran, and there is something here that is not ours to replace.
/// A genuine I/O failure — no permission, a directory in the way — is still
/// `2`, because then the question really was unanswerable.
pub(crate) fn install(path: &Path, document: &str) -> Result<Wrote, Refusal> {
    match fs::read(path) {
        Ok(existing) if existing == document.as_bytes() => {
            return Ok(Wrote::Unchanged(path.to_path_buf()));
        }
        Ok(_) => {
            return Err(Refusal::Occupied(format!(
                "`{}` already holds a different document: this command replaces nothing it did \
                 not write. Move it aside, or print the skill with `agent-compose skill` and \
                 merge it yourself",
                path.display()
            )));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(Refusal::Unusable(format!(
                "cannot read `{}`: {error}",
                path.display()
            )));
        }
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            Refusal::Unusable(format!("cannot create `{}`: {error}", parent.display()))
        })?;
    }
    fs::write(path, document).map_err(|error| {
        Refusal::Unusable(format!("cannot write `{}`: {error}", path.display()))
    })?;
    Ok(Wrote::Created(path.to_path_buf()))
}
