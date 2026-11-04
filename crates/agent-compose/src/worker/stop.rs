//! What a stop signal sent to `agent-compose worker` does to the node runners
//! it started (`docs/distributed.md` §3.2, PRD resolved q75).
//!
//! Each runner is the leader of a **process group of its own** ([`super::node`]'s
//! `spawn`), so that a cancel it does not answer in time takes everything left
//! in it. A group of its own is one the terminal no longer reaches: the `SIGINT`
//! a `Ctrl-C` sends goes to the terminal's foreground group — the worker's — and
//! a runner outside it, with its node, its `exec:` groups and its harness
//! processes, would run on with nothing supervising it, still writing into the
//! workspace. So the worker delivers the stop itself, which is what the
//! terminal did when the runners were in its group:
//!
//! 1. every runner's group is sent `SIGINT` — the stop a runner already
//!    answers by sweeping the `exec:` groups it detached and ending
//!    (`src/runtime.ts`'s `armCommandSweep`), and never a `SIGTERM`, which a
//!    runner reads as the hub's cancel and answers by running finally nodes
//!    whole for a worker that is no longer there to read the result;
//! 2. the worker waits up to [`GRACE`] for them to be reaped, and kills what
//!    is left of each group;
//! 3. and it ends **by the signal it was sent**, its default disposition
//!    restored first, so a caller reading its exit status sees what it saw
//!    before the runners had groups of their own.
//!
//! What the runners were doing is the hub's to recover, exactly as when a
//! worker dies any other way: its session stops making requests, and §6.3's
//! sweep supersedes what it held.
//!
//! `SIGINT`, `SIGTERM` and `SIGHUP` are the three: the two ways a caller asks a
//! process to stop, and the one a closing terminal sends. `SIGKILL` cannot be
//! caught, which a caller that must end the whole tree without the worker's
//! cooperation answers by signalling the runners' groups itself.
//!
//! The handler makes one call, `write`, which POSIX lists as async-signal-safe,
//! into a pipe a thread of this module waits on; everything else happens on
//! that thread.

#[cfg(unix)]
pub(crate) use imp::{ended, install, started};

#[cfg(not(unix))]
pub(crate) use portable::{ended, install, started};

/// How long a runner sent `SIGINT` by a stopping worker is given to end before
/// its group is killed.
///
/// A runner answers `SIGINT` at once — it kills the `exec:` groups it has
/// running and ends — so this is for one that does not: a host function that
/// has wedged the event loop, a harness process that traps the signal.
#[cfg_attr(not(unix), allow(dead_code))]
const GRACE: std::time::Duration = std::time::Duration::from_secs(2);

#[cfg(unix)]
mod imp {
    use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
    use std::sync::{Mutex, PoisonError};
    use std::time::{Duration, Instant};

    use super::GRACE;

    /// The runners this process has running, by pid — each the leader of its
    /// own group — from spawn until its dispatch thread has reaped it.
    static GROUPS: Mutex<Vec<libc::pid_t>> = Mutex::new(Vec::new());

    /// Set once a stop signal has arrived, so a runner spawned while the stop
    /// is under way is stopped as it is put on the books.
    static STOPPING: AtomicBool = AtomicBool::new(false);

    /// The pipe's write end, or `-1` before [`install`] made one.
    static WAKE: AtomicI32 = AtomicI32::new(-1);

    /// The signals a worker stops on.
    const STOPS: [libc::c_int; 3] = [libc::SIGINT, libc::SIGTERM, libc::SIGHUP];

    extern "C" fn caught(signal: libc::c_int) {
        let fd = WAKE.load(Ordering::SeqCst);
        if fd < 0 {
            return;
        }
        let byte = u8::try_from(signal).unwrap_or(u8::MAX);
        // SAFETY: `write` is async-signal-safe, `fd` is the pipe `install` made
        // and never closes, and `byte` outlives the call.
        unsafe { libc::write(fd, (&raw const byte).cast(), 1) };
    }

    /// Install the handlers, and the thread that acts on them.
    ///
    /// Called once, before any runner exists. A process that cannot make the
    /// pipe keeps the default dispositions it had, which is what a worker did
    /// before its runners had groups of their own.
    pub(crate) fn install() {
        let mut fds: [libc::c_int; 2] = [-1, -1];
        // SAFETY: `fds` is two writable ints, which is what `pipe` fills.
        if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
            return;
        }
        for fd in fds {
            // SAFETY: an fd `pipe` just returned. Close-on-exec, so no runner —
            // nor any other child — inherits either end.
            unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) };
        }
        let [read, write] = fds;
        WAKE.store(write, Ordering::SeqCst);
        std::thread::spawn(move || {
            let signal = loop {
                let mut byte = 0u8;
                // SAFETY: `read` is the pipe's read end, and `byte` is one
                // writable byte.
                let got = unsafe { libc::read(read, (&raw mut byte).cast(), 1) };
                if got == 1 {
                    break libc::c_int::from(byte);
                }
                if got < 0 && std::io::Error::last_os_error().raw_os_error() == Some(libc::EINTR) {
                    continue;
                }
                return;
            };
            stop(signal);
        });
        for signal in STOPS {
            // SAFETY: `action` is fully initialised before use, and `caught`
            // has the signature `sigaction` expects.
            unsafe {
                let mut action: libc::sigaction = std::mem::zeroed();
                action.sa_sigaction = caught as *const () as libc::sighandler_t;
                libc::sigemptyset(&raw mut action.sa_mask);
                // The poll's socket read, a runner's `wait`: restarted rather
                // than answering `EINTR`.
                action.sa_flags = libc::SA_RESTART;
                libc::sigaction(signal, &raw const action, std::ptr::null_mut());
            }
        }
    }

    /// Put one runner on the books — stopping it at once if the worker is
    /// already stopping.
    pub(crate) fn started(pid: u32) {
        let Ok(pid) = libc::pid_t::try_from(pid) else {
            return;
        };
        let mut groups = GROUPS.lock().unwrap_or_else(PoisonError::into_inner);
        groups.push(pid);
        if STOPPING.load(Ordering::SeqCst) {
            // SAFETY: the group of a runner this process spawned and has not
            // reaped.
            unsafe { libc::kill(-pid, libc::SIGINT) };
        }
    }

    /// Take one runner off the books, once its dispatch thread has reaped it:
    /// past that its pid is not this process's to signal.
    pub(crate) fn ended(pid: u32) {
        let Ok(pid) = libc::pid_t::try_from(pid) else {
            return;
        };
        GROUPS
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|held| *held != pid);
    }

    /// The stop itself: see the module docs.
    fn stop(signal: libc::c_int) -> ! {
        let held: Vec<libc::pid_t> = {
            let groups = GROUPS.lock().unwrap_or_else(PoisonError::into_inner);
            STOPPING.store(true, Ordering::SeqCst);
            for pid in groups.iter() {
                // SAFETY: the group of a runner this process spawned and has
                // not reaped — it leaves the books before it is.
                unsafe { libc::kill(-*pid, libc::SIGINT) };
            }
            groups.clone()
        };
        let deadline = Instant::now() + GRACE;
        let running = || {
            let groups = GROUPS.lock().unwrap_or_else(PoisonError::into_inner);
            held.iter().any(|pid| groups.contains(pid))
        };
        while running() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        for pid in &held {
            // SAFETY: whatever is left of a group a runner this process spawned
            // led; `ESRCH` for one already empty is the ordinary case.
            unsafe { libc::kill(-*pid, libc::SIGKILL) };
        }
        // SAFETY: restoring the default disposition and delivering the signal
        // that arrived, so this process ends by it.
        unsafe {
            libc::signal(signal, libc::SIG_DFL);
            libc::raise(signal);
        }
        // Unreachable for any of the three, whose default ends the process.
        std::process::exit(128 + signal);
    }
}

/// Where signals are not a POSIX facility, nothing is forwarded: a runner there
/// has no group of its own to fall outside of.
#[cfg(not(unix))]
mod portable {
    pub(crate) const fn install() {}

    pub(crate) const fn started(_pid: u32) {}

    pub(crate) const fn ended(_pid: u32) {}
}
