//! The read guard: the one place that decides which paths this server may open.
//!
//! Every read in the crate goes through [`crate::proc::read`] or
//! [`crate::proc::read_optional`], and both ask this module first. Nothing else
//! opens a file.
//!
//! The control is an **allowlist**, not a denylist. A denylist has to enumerate
//! every dangerous file forever, and it loses outright to renaming: a symlink at
//! `/tmp/x` pointing at `/proc/1/environ` has the basename `x` and passes any
//! check on basenames. An allowlist rejects `/tmp/x` because it is not a shape
//! this server reads. [`FORBIDDEN`] exists underneath it only so that widening
//! the allowlist carelessly still fails closed.
//!
//! Paths are normalized *lexically*, without touching the filesystem.
//! `std::fs::canonicalize` would resolve symlinks and `..` correctly but
//! requires the file to exist, which would turn every legitimately absent
//! optional file into a resolution failure rather than the "missing" answer
//! decision 18 rule 7 depends on. Rejecting `.`, `..` and empty components
//! outright is enough: the server's paths never contain them, so their presence
//! is either a bug or an attempt to get around this function.
//!
//! A denial is always a programming error — no path here is model-controlled —
//! so the tests at the bottom of this file, and the CI job that flags any edit
//! to it, are as much of the control as the runtime check is.

/// Files this server reads at a fixed, known path.
const EXACT: &[&str] = &[
    "/proc/uptime",
    "/proc/stat",
    "/proc/loadavg",
    "/proc/meminfo",
    "/proc/pressure/cpu",
    "/proc/pressure/memory",
    "/proc/pressure/io",
    "/proc/sys/kernel/hostname",
    "/proc/sys/kernel/osrelease",
];

/// The only per-process file this server may ever read (decision 23).
///
/// Everything else under a `/proc/<pid>/` directory is denied by omission,
/// including `status`, `stat` and `limits`, which are harmless but unused.
const PER_PROCESS: &str = "comm";

/// Basenames readable anywhere beneath the cgroup hierarchy (decision 22).
///
/// The path above the basename is not constrained because cgroup paths are
/// discovered at runtime and differ by container runtime and cgroup driver.
const CGROUP_FILES: &[&str] = &[
    "cpu.stat",
    "cpu.max",
    "cpu.weight",
    "cpu.pressure",
    "memory.current",
    "memory.max",
    "memory.high",
    "memory.events",
    "memory.swap.current",
    "memory.pressure",
    "io.stat",
    "io.max",
    "io.pressure",
    "pids.current",
    "pids.max",
    "cgroup.procs",
];

/// Names that are never a legitimate read, checked before the allowlist.
///
/// Redundant by construction: nothing here could match [`EXACT`],
/// [`PER_PROCESS`] or [`CGROUP_FILES`]. It is a backstop against a future
/// widening of those lists, and it is deliberately narrow — `io`, `fd` and
/// `fdinfo` are *absent* because `/proc/<pid>/io` is already denied by the
/// per-process rule, and listing a bare `io` here would block the legitimate
/// `/proc/pressure/io`.
const FORBIDDEN: &[&str] = &[
    "environ",
    "cmdline",
    "mem",
    "maps",
    "smaps",
    "stack",
    "syscall",
    "auxv",
    "personality",
    "kcore",
];

/// Why a path was refused. Reported to the operator on stderr, never to the model.
#[derive(Debug, PartialEq, Eq)]
pub enum Denied {
    /// Not an absolute path.
    NotAbsolute,
    /// Contained an empty, `.` or `..` component.
    BadComponent,
    /// Named a file this server must never read.
    Forbidden,
    /// Not a path shape this server reads.
    NotAllowed,
}

impl std::fmt::Display for Denied {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Denied::NotAbsolute => "not an absolute path",
            Denied::BadComponent => "contains an empty, '.' or '..' component",
            Denied::Forbidden => "names a file the server must never read",
            Denied::NotAllowed => "not a path this server reads",
        };
        f.write_str(s)
    }
}

/// Decide whether this server may open `path`.
pub fn check(path: &str) -> Result<(), Denied> {
    if !path.starts_with('/') {
        return Err(Denied::NotAbsolute);
    }
    let parts: Vec<&str> = path[1..].split('/').collect();
    if parts
        .iter()
        .any(|c| c.is_empty() || *c == "." || *c == "..")
    {
        return Err(Denied::BadComponent);
    }
    // `parts` cannot be empty: splitting a non-empty string always yields one
    // element, and an empty first element was rejected above.
    let base = *parts.last().expect("path has at least one component");
    if FORBIDDEN.contains(&base) {
        return Err(Denied::Forbidden);
    }

    if EXACT.contains(&path) {
        return Ok(());
    }
    if parts.len() == 3
        && parts[0] == "proc"
        && parts[1].bytes().all(|b| b.is_ascii_digit())
        && parts[2] == PER_PROCESS
    {
        return Ok(());
    }
    if parts.len() >= 4 && parts[..3] == ["sys", "fs", "cgroup"] && CGROUP_FILES.contains(&base) {
        return Ok(());
    }
    Err(Denied::NotAllowed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn allowed(path: &str) {
        assert_eq!(check(path), Ok(()), "should be ALLOWED: {path}");
    }
    fn denied(path: &str) {
        assert!(check(path).is_err(), "should be DENIED: {path}");
    }

    #[test]
    fn allows_every_path_the_tools_actually_read() {
        for p in EXACT {
            allowed(p);
        }
    }

    #[test]
    fn allows_per_process_comm_and_cgroup_files() {
        allowed("/proc/1/comm");
        allowed("/proc/4242/comm");
        allowed("/sys/fs/cgroup/system.slice/some-container.scope/cpu.stat");
        allowed("/sys/fs/cgroup/system.slice/some-container.scope/memory.events");
        allowed("/sys/fs/cgroup/user.slice/user-1000.slice/cgroup.procs");
    }

    #[test]
    fn denies_environ_and_cmdline() {
        denied("/proc/1/environ");
        denied("/proc/1/cmdline");
        denied("/proc/self/environ");
        denied("/proc/self/cmdline");
        denied("/proc/1/task/1/environ");
    }

    #[test]
    fn denies_traversal_and_malformed_paths() {
        denied("/proc/self/../1/environ");
        denied("/proc/1/./environ");
        denied("/proc//1/environ");
        denied("/proc/1/root/proc/1/environ");
        denied("/sys/fs/cgroup/../../proc/1/environ");
        denied("proc/1/environ");
        denied("./environ");
        denied("");
    }

    #[test]
    fn denies_paths_outside_the_substrate() {
        // A symlink can hide a basename; the allowlist rejects the path anyway.
        denied("/tmp/looks-harmless");
        denied("/home/someone/link-to-environ");
        denied("/etc/shadow");
    }

    #[test]
    fn denies_other_sensitive_per_process_files() {
        for f in [
            "maps", "mem", "fd", "fdinfo", "auxv", "io", "smaps", "stack",
        ] {
            denied(&format!("/proc/1/{f}"));
        }
        denied("/proc/kcore");
    }

    #[test]
    fn denies_unused_but_harmless_per_process_files() {
        // The per-process rule admits `comm` only, so these are denied by omission.
        denied("/proc/1/status");
        denied("/proc/1/stat");
        denied("/proc/1/limits");
    }

    #[test]
    fn denies_unknown_cgroup_files() {
        denied("/sys/fs/cgroup/system.slice/x.scope/cgroup.subtree_control");
        denied("/sys/fs/cgroup/system.slice/x.scope/memory.numa_stat");
    }

    #[test]
    fn decides_without_touching_the_filesystem() {
        // An allowed path that does not exist still passes: the guard must not
        // conflate "may not read" with "is not there".
        let absent = "/proc/pressure/cpu";
        allowed(absent);
        denied("/proc/pressure/definitely-not-a-real-file");
    }

    /// No module outside this one may name a forbidden path.
    ///
    /// This catches the case the runtime guard cannot: someone adding a literal
    /// read of `/proc/<pid>/environ` in a new tool. Comment lines are skipped so
    /// that prose may still discuss the rule.
    #[test]
    fn no_other_module_names_a_forbidden_path() {
        let mut hits = Vec::new();
        for entry in std::fs::read_dir("src").expect("src/ is readable") {
            let path = entry.expect("readable dir entry").path();
            if path.extension().is_none_or(|e| e != "rs") {
                continue;
            }
            if path.file_name().is_some_and(|n| n == "guard.rs") {
                continue;
            }
            let text = std::fs::read_to_string(&path).expect("source file is readable");
            for (n, line) in text.lines().enumerate() {
                if line.trim_start().starts_with("//") {
                    continue;
                }
                for f in ["environ", "cmdline", "kcore"] {
                    if line.contains(f) {
                        hits.push(format!("{}:{}: {}", path.display(), n + 1, line.trim()));
                    }
                }
            }
        }
        assert!(
            hits.is_empty(),
            "a forbidden path is named outside the guard:\n{}",
            hits.join("\n")
        );
    }
}
