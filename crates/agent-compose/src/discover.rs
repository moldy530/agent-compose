//! The discovery verbs that touch the file system.
//!
//! Everything else in the discovery surface writes a document to stdout and is
//! therefore one line in `main.rs`. `init` **writes a file**, which makes it
//! one of the two that need a refusal story: a compiler that silently replaced
//! a file a person wrote is worse than one that did nothing, and this runs in a
//! directory the user already lives in.

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
