//! `agent-compose run` and `agent-compose serve`: building a project and then
//! **starting** it (PRD 5.11, §7 M1).
//!
//! Both verbs are the same four steps, and only the last differs:
//!
//! 1. **validate**, and refuse on anything reported — a run of a composition the
//!    compiler had something to say about would report that thing again, later,
//!    with no span to point at (the same rule `build` follows);
//! 2. **build**, through `build::write` and `build::default_out`, so a run leaves
//!    behind exactly the directory `agent-compose build` would have written and a
//!    reader can inspect, re-run or eject it;
//! 3. **check the environment**, which is PRD §9.15's second half — "`run`/`serve`
//!    fail fast before invoking the graph, naming the missing variable". The
//!    emitted project checks itself at process start too (`src/index.ts` calls
//!    `readEnvironment()`), and both are wanted: the compiler knows every
//!    reference statically, so it can name all of them and the site each is
//!    written at *before* spawning anything;
//! 4. **launch** the emitted project's own command line — `bun src/index.ts run …`
//!    or `… serve …`, falling back to `node` where Bun is absent (PRD §9.18).
//!
//! # Why the verbs launch rather than link
//!
//! Nothing about invoking a compiled graph lives in this binary. The emitted
//! project is a plain TypeScript project with its own entry point, which is what
//! makes PRD 5.12's eject path real: `agent-compose run` runs the command a
//! reader could have typed, and the emitted `README.md` documents that command
//! first. A compiler that had reimplemented invocation would be a second
//! implementation of the runtime, drifting from the one an ejected project keeps.
//!
//! # Exit codes
//!
//! The table `main` documents, unchanged: `0` clean, `1` "the answer is no" — a
//! composition that reported diagnostics, or a run that produced no answer — and
//! `2` for a command that could not run at all, which is what a missing
//! entrypoint, an unresolvable JavaScript runtime, an unresolved dependency set
//! and a missing environment variable all are. The child's own code is passed
//! through unchanged, and `src/cli.ts` gives the three the same meanings — plus
//! the fourth only it can reach: `3`, a run with nobody to ask that stopped at a
//! `human` pause (grammar 8.7). Nothing here decides that one; it arrives as the
//! child's code like the rest.
//!
//! # Why standard input is inherited
//!
//! [`spawn`] inherits all three streams, and stdin is the one that has become
//! load-bearing: a pause has a second delivery surface, and it is the terminal
//! this command was launched from (grammar 8.7, PRD §9.21). The child asks its
//! question there and reads the answer there, so `agent-compose run` at a
//! terminal completes a flow with a `human` node in it — and a redirected or
//! absent stdin is what leaves the child on the exit-`3` path above. This
//! process reads nothing and copies nothing: the terminal *is* the child's.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use compose_core::Ir;

/// Where a launch stopped, when it stopped before the child ran.
pub(crate) enum Refusal {
    /// The command's own precondition failed: exit `2` with this message.
    Unusable(String),
}

/// The JavaScript runtime an emitted project is launched with (PRD §9.18).
///
/// Bun is the default — it is what the emitted `README.md` documents first and
/// what the generated-code gates run — and Node ≥ 22.18 is the supported
/// fallback. The search is the same one the test toolchain performs, and for the
/// same reason: `$BUN_INSTALL/bin/bun` and `~/.bun/bin/bun` are the difference
/// between "Bun is installed" and "this shell knows about it".
pub(crate) fn runtime() -> Result<PathBuf, Refusal> {
    if answers("bun") {
        return Ok(PathBuf::from("bun"));
    }
    let installed = std::env::var_os("BUN_INSTALL")
        .map(PathBuf::from)
        .into_iter()
        .chain(std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".bun")));
    for prefix in installed {
        let candidate = prefix.join("bin/bun");
        if answers(&candidate.to_string_lossy()) {
            return Ok(candidate);
        }
    }
    if answers("node") {
        return Ok(PathBuf::from("node"));
    }
    Err(Refusal::Unusable(
        "no JavaScript runtime was found: an emitted project runs on Bun by default and on \
         Node >= 22.18 as a fallback, and neither `bun` nor `node` answered `--version`. \
         Install one, or run the built project yourself — it is a plain TypeScript project \
         and its README says how"
            .to_string(),
    ))
}

/// Whether a program answers `--version`.
fn answers(program: &str) -> bool {
    Command::new(program)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// Refuse before spawning when a `${ENV}` reference the composition makes is not
/// set (PRD 5.9, §9.15).
///
/// Every reference at once, each with the site it is written at: a run that
/// stopped at the first would take as many attempts to configure as the project
/// has secrets. The emitted project performs the same check at process start,
/// which is what covers a project run by hand; this one covers the case the
/// milestone names, and covers it before a graph is built.
pub(crate) fn environment(ir: &Ir) -> Result<(), Refusal> {
    let references = compose_core::codegen::env::References::of(ir);
    let mut missing: Vec<String> = Vec::new();
    for name in references.names() {
        if std::env::var_os(name).is_none() {
            missing.push(format!(
                "`{name}` (referenced by {})",
                references
                    .sites(name)
                    .iter()
                    .map(|site| format!("`{site}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    }
    if missing.is_empty() {
        return Ok(());
    }
    Err(Refusal::Unusable(format!(
        "{} this composition references {} not set: {}. Environment references are resolved \
         when the graph runs, never at build (PRD 5.9), so this is the first point the values \
         are needed",
        if missing.len() == 1 {
            "an environment variable"
        } else {
            "environment variables"
        },
        if missing.len() == 1 { "is" } else { "are" },
        missing.join(", ")
    )))
}

/// Refuse before spawning when the pinned dependency set is not installed.
///
/// An emitted project is source, not a bundle: it imports LangGraph, Zod and the
/// rest of `codegen::project::PINS` by name, and a launch into a directory with
/// no `node_modules` above it fails inside the runtime with a resolution error
/// naming a package rather than a step. Node resolution walks up, so this walks
/// up too — a project built under a directory that already has the install is
/// exactly the arrangement the acceptance suite and a monorepo both use.
pub(crate) fn dependencies(out: &Path) -> Result<(), Refusal> {
    for directory in out.ancestors() {
        if directory.join("node_modules/@langchain/langgraph").is_dir() {
            return Ok(());
        }
    }
    Err(Refusal::Unusable(format!(
        "the pinned dependency set is not installed for `{}`: run `bun install` there (or \
         `npm install`) before running the graph. `agent-compose build` writes the project \
         and its `package.json`; installing is yours, because the lockfile is yours",
        out.display()
    )))
}

/// Spawn the emitted project's own command line and answer with its exit code.
///
/// stdin, stdout and stderr are **inherited**, which is what makes the child's
/// answer the command's answer: `run` prints the flow's outputs on stdout and
/// its report on stderr, and `serve` announces its address on stdout as one JSON
/// line — and a caller reading either is reading the child directly rather than
/// something this process copied.
///
/// # Stopping this command stops the graph
///
/// The child is the thing that is *running*: a `serve` holds a listening socket
/// for as long as it lives, and this process only waits on it. So `SIGINT` and
/// `SIGTERM` are forwarded to it ([`signals`]) rather than being left to end
/// this process alone — which would leave the app listening with nothing above
/// it, on a port an operator believes they released and holding the store data
/// directory of a project they believe they stopped. The emitted app installs
/// handlers of its own for both (`serve()` in `src/serve.ts` closes the app and
/// exits), and they are unreachable unless something delivers the signal.
///
/// A `SIGKILL` cannot be forwarded, and nothing here pretends otherwise: a
/// caller that must be able to end the whole tree without its cooperation puts
/// the command in a process group of its own and signals the **group**, which is
/// what this suite's own harness does.
pub(crate) fn spawn(runtime: &Path, out: &Path, arguments: &[String]) -> Result<u8, Refusal> {
    let mut command = Command::new(runtime);
    command
        .arg("src/index.ts")
        .args(arguments)
        .current_dir(out)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    let cannot_start = |error: std::io::Error| {
        Refusal::Unusable(format!(
            "cannot start `{} src/index.ts` in `{}`: {error}",
            runtime.display(),
            out.display()
        ))
    };
    // Installed **before** the child exists, so a signal that arrives while it
    // is being spawned is remembered rather than dropped on a default
    // disposition that would end this process and orphan it.
    let forwarding = signals::forwarding();
    let mut child = command.spawn().map_err(cannot_start)?;
    forwarding.adopt(child.id());
    let waited = child.wait();
    // The pid stops being this process's to signal the moment `wait` reaps it,
    // rather than at the end of this scope: a handler that ran in between would
    // be signalling whatever the system had given the number to next.
    drop(forwarding);
    let status = waited.map_err(cannot_start)?;
    // A child killed by a signal reports no code. It did not answer, which is
    // what `1` means here.
    Ok(u8::try_from(status.code().unwrap_or(1)).unwrap_or(1))
}

/// Handing this process's stop signals to the child it is waiting on.
///
/// Two states and one hazard. The signal may arrive before there is a child to
/// forward it to — between installing the handler and `spawn` answering — and a
/// handler that only forwarded would drop it, leaving a command that was asked
/// to stop running until the graph finished. So the handler records the signal
/// as well as forwarding it, and [`Forwarding::adopt`] replays what it recorded
/// the moment a pid exists. Whichever order the two happen in, the child is
/// signalled exactly once or twice, and twice is harmless.
///
/// The handler makes one call, `kill`, which POSIX lists as async-signal-safe.
/// The two `AtomicI32`s it reads are lock-free on every platform this compiler
/// targets, which is what makes reading them from a handler sound.
///
/// The state is process-wide because signal dispositions are, so one
/// [`Forwarding`] may exist at a time. `run` and `serve` each launch one child
/// and wait for it, which is the only shape this binary has.
#[cfg(unix)]
mod signals {
    use std::sync::atomic::{AtomicI32, Ordering};

    /// The child to forward to, or `0` before there is one.
    static CHILD: AtomicI32 = AtomicI32::new(0);
    /// The signal that arrived before there was a child, or `0`.
    static PENDING: AtomicI32 = AtomicI32::new(0);

    /// What is forwarded: the two ways a caller asks a command to stop.
    ///
    /// `SIGQUIT` and the rest are left at their default disposition, which is
    /// what a caller sending them is asking for — a core dump from this process
    /// is not a graceful shutdown of the graph.
    const FORWARDED: [libc::c_int; 2] = [libc::SIGINT, libc::SIGTERM];

    extern "C" fn forward(signal: libc::c_int) {
        PENDING.store(signal, Ordering::SeqCst);
        let child = CHILD.load(Ordering::SeqCst);
        if child > 0 {
            // SAFETY: `kill` is async-signal-safe, and `child` is a pid this
            // process spawned and has not yet reaped — `spawn` drops the guard,
            // which clears this, in the statement after `wait` answers.
            unsafe { libc::kill(child, signal) };
        }
    }

    /// The installed handlers, which restore the default disposition when
    /// dropped: past the child's exit there is nothing to forward to, and a
    /// process that answered the *next* `SIGTERM` by doing nothing would be
    /// worse than one that never installed a handler.
    pub(super) struct Forwarding;

    /// Install the handlers.
    pub(super) fn forwarding() -> Forwarding {
        for signal in FORWARDED {
            // SAFETY: `action` is fully initialized before use, and `forward`
            // has the signature `sigaction` expects.
            unsafe {
                let mut action: libc::sigaction = std::mem::zeroed();
                action.sa_sigaction = forward as *const () as libc::sighandler_t;
                libc::sigemptyset(&raw mut action.sa_mask);
                // System calls this process is inside — the `wait` below — are
                // restarted rather than answering `EINTR`.
                action.sa_flags = libc::SA_RESTART;
                libc::sigaction(signal, &raw const action, std::ptr::null_mut());
            }
        }
        Forwarding
    }

    impl Forwarding {
        /// Name the child, and deliver whatever arrived before it existed.
        pub(super) fn adopt(&self, child: u32) {
            let pid = libc::pid_t::try_from(child).unwrap_or(0);
            CHILD.store(pid, Ordering::SeqCst);
            let pending = PENDING.load(Ordering::SeqCst);
            if pid > 0 && pending > 0 {
                // SAFETY: as in `forward` — a pid this process spawned and has
                // not reaped.
                unsafe { libc::kill(pid, pending) };
            }
        }
    }

    impl Drop for Forwarding {
        fn drop(&mut self) {
            CHILD.store(0, Ordering::SeqCst);
            PENDING.store(0, Ordering::SeqCst);
            for signal in FORWARDED {
                // SAFETY: restoring the default disposition, which is what was
                // installed before `forwarding()` ran.
                unsafe {
                    libc::signal(signal, libc::SIG_DFL);
                }
            }
        }
    }
}

/// The same, where signals are not a POSIX facility: the child is waited on and
/// nothing is forwarded, which is what this command has always done.
#[cfg(not(unix))]
mod signals {
    pub(super) struct Forwarding;

    pub(super) const fn forwarding() -> Forwarding {
        Forwarding
    }

    impl Forwarding {
        pub(super) const fn adopt(&self, _child: u32) {}
    }

    /// Nothing to restore, and the impl says so: `spawn` drops the guard where
    /// a pid stops being safe to signal, and a platform with no handler to
    /// uninstall still has that moment.
    impl Drop for Forwarding {
        fn drop(&mut self) {}
    }
}
