//! Reading the cgroup v2 hierarchy, and parsers for the formats found there.
//!
//! The counterpart to [`crate::proc`], and the same split applies: this module
//! knows how cgroup files are *shaped* and how containers are *found*, not what
//! any tool wants to say about them.
//!
//! cgroups are the enforcement layer rather than a reporting layer, so limits
//! and accounting read identically whatever created the container — Docker,
//! podman and Kubernetes all just write these files (decision 22). Only
//! discovery is runtime-specific, and it is confined to [`classify`].

use crate::proc;

/// How deep the walk descends below `/sys/fs/cgroup`.
///
/// Kubernetes nests deepest of the layouts in decision 22 — a QoS slice, a pod
/// slice, then the container scope — and a bound keeps a pathological hierarchy
/// from turning discovery into an unbounded walk.
const MAX_DEPTH: usize = 6;

/// Shortest container ID treated as real.
///
/// Every runtime in [`classify`] uses a 64-character hex ID; the bound is well
/// below that so a runtime using short IDs still works, while staying long
/// enough that a stray word cannot pass as one.
const MIN_ID_LEN: usize = 12;

/// A container found in the hierarchy.
pub struct Found {
    pub id: String,
    pub runtime: &'static str,
    pub path: String,
}

/// Recognize a container from its cgroup directory name.
///
/// This is the one runtime-specific part of the container tools. The prefixes
/// come from decision 22, where only the Docker/systemd form is verified
/// against a real host; the others are from documentation and will need
/// confirming. A prefix that is wrong fails visibly — nothing is found — rather
/// than silently mis-attributing anything.
fn classify(dir_name: &str) -> Option<(&'static str, String)> {
    const SCOPES: &[(&str, &str)] = &[
        ("docker-", "docker"),
        ("libpod-", "podman"),
        ("cri-containerd-", "containerd"),
        ("crio-", "cri-o"),
    ];
    let base = dir_name.strip_suffix(".scope")?;
    for (prefix, runtime) in SCOPES {
        if let Some(id) = base.strip_prefix(prefix) {
            // A prefix match is not enough. podman gives its per-container
            // monitor process a sibling `libpod-conmon-<id>.scope`, which
            // strips to `conmon-<id>` and would otherwise be reported as a
            // second, non-existent container. Every runtime here names a
            // container with a hex ID, so requiring one rejects that scope and
            // any comparable infrastructure cgroup a future runtime adds.
            if id.len() >= MIN_ID_LEN && id.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Some((runtime, id.to_owned()));
            }
        }
    }
    None
}

/// Walk the cgroup tree and return every container in it.
///
/// Ordered by path so repeated calls agree, which matters because the caller
/// pages through the result and a model may compare two readings.
pub async fn discover() -> Vec<Found> {
    let mut found = Vec::new();
    let mut frontier = vec!["/sys/fs/cgroup".to_owned()];
    for _ in 0..MAX_DEPTH {
        let mut next = Vec::new();
        for dir in frontier {
            let Some(children) = proc::read_dir(&dir).await else {
                continue;
            };
            for child in children {
                let name = child.rsplit('/').next().unwrap_or_default();
                match classify(name) {
                    Some((runtime, id)) => found.push(Found {
                        id,
                        runtime,
                        path: child,
                    }),
                    // Not a container: keep descending, since container scopes
                    // sit below slices that look like nothing in particular.
                    None => next.push(child),
                }
            }
        }
        if next.is_empty() {
            break;
        }
        frontier = next;
    }
    found.sort_by(|a, b| a.path.cmp(&b.path));
    found
}

/// Distinct process names in a cgroup, and how many processes it holds.
///
/// `comm` is the kernel's own short name for a process: fifteen characters,
/// structurally incapable of holding a command line, and therefore incapable of
/// leaking the credentials that command lines routinely carry (decision 23).
///
/// Every process is named, with no sampling. A `comm` read is a memcpy out of
/// kernel memory: measured at 7us, so a deliberately pathological 300-process
/// container costs about 2ms, and a machine able to schedule that many
/// processes reads that many tiny files without noticing. Truncating would
/// trade exactness for microseconds and could report the wrong names on a
/// container whose distinguishing process happens to sort late.
pub async fn identify(path: &str) -> (Vec<String>, usize) {
    let Some(procs) = proc::read_optional(&format!("{path}/cgroup.procs")).await else {
        return (Vec::new(), 0);
    };
    let pids: Vec<&str> = procs.split_whitespace().collect();
    let mut names = Vec::new();
    for pid in &pids {
        if let Some(name) = proc::read_optional(&format!("/proc/{pid}/comm")).await {
            let name = name.trim().to_owned();
            if !name.is_empty() && !names.contains(&name) {
                names.push(name);
            }
        }
    }
    names.sort();
    (names, pids.len())
}

/// One `key value` line, as found in `cpu.stat` and `memory.events`.
pub fn keyed(text: &str, key: &str) -> Option<u64> {
    text.lines()
        .find_map(|l| l.strip_prefix(key)?.strip_prefix(' '))?
        .trim()
        .parse()
        .ok()
}

/// A file holding one number, as `memory.current` and `pids.current` do.
pub fn single(text: &str) -> Option<u64> {
    text.trim().parse().ok()
}

/// A limit file, where the literal `max` means no limit is set.
///
/// `None` for `max` is the whole point: an unset limit is a fact worth
/// reporting, not a missing reading. The caller distinguishes the two by
/// whether the surrounding block is present at all.
pub fn limit(text: &str) -> Option<u64> {
    match text.trim() {
        "max" => None,
        n => n.parse().ok(),
    }
}

/// `cpu.max`, which is `quota period` where quota may be the literal `max`.
///
/// Returns the pair only when a quota is actually set; the period alone says
/// nothing, since it is 100000 by default whether or not anything is limited.
pub fn cpu_max(text: &str) -> Option<(u64, u64)> {
    let mut f = text.split_whitespace();
    let quota = limit(f.next()?)?;
    let period = f.next()?.parse().ok()?;
    Some((quota, period))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_each_runtime_naming_convention() {
        let cases = [
            ("docker-abc123def456.scope", "docker", "abc123def456"),
            ("libpod-def456abc789.scope", "podman", "def456abc789"),
            (
                "cri-containerd-789aaa012bcd.scope",
                "containerd",
                "789aaa012bcd",
            ),
            ("crio-bbb222ccc333.scope", "cri-o", "bbb222ccc333"),
        ];
        for (dir, runtime, id) in cases {
            let (r, i) = classify(dir).unwrap_or_else(|| panic!("should classify {dir}"));
            assert_eq!((r, i.as_str()), (runtime, id), "for {dir}");
        }
    }

    #[test]
    fn ignores_podman_infrastructure_scopes() {
        // Verified against a real rootless podman container: podman creates a
        // sibling monitor scope whose name strips to `conmon-<id>`.
        assert!(
            classify("libpod-conmon-b013beb7ae662d0d287e1df78087cff4859a4fb5.scope").is_none(),
            "conmon scope must not be reported as a container"
        );
        // The container's own scope alongside it still classifies.
        let (r, id) =
            classify("libpod-b013beb7ae662d0d287e1df78087cff4859a4fb5.scope").expect("real");
        assert_eq!(r, "podman");
        assert_eq!(id, "b013beb7ae662d0d287e1df78087cff4859a4fb5");
    }

    #[test]
    fn requires_a_hex_id() {
        assert!(classify("docker-not-a-hex-id.scope").is_none());
        assert!(classify("docker-abc.scope").is_none(), "too short");
        assert!(classify("libpod-zzzzzzzzzzzzzz.scope").is_none(), "not hex");
        assert!(
            classify("docker-abcdef012345.scope").is_some(),
            "12 hex is fine"
        );
    }

    #[test]
    fn ignores_cgroups_that_are_not_containers() {
        for dir in [
            "system.slice",
            "user.slice",
            "init.scope",
            "sshd.service",
            "kubepods-burstable.slice",
            "docker.service",
            "docker-no-suffix",
        ] {
            assert!(classify(dir).is_none(), "should not classify {dir}");
        }
    }

    #[test]
    fn an_unset_limit_is_none_not_a_number() {
        // The whole point: `max` means no limit is configured, which is a fact
        // to report, not a reading that failed.
        assert_eq!(limit("max"), None);
        assert_eq!(limit("max\n"), None);
        assert_eq!(limit("536870912"), Some(536870912));
        assert_eq!(limit("536870912\n"), Some(536870912));
        assert_eq!(limit("0"), Some(0));
    }

    #[test]
    fn cpu_max_reports_a_quota_only_when_one_is_set() {
        // The period is 100000 by default whether or not anything is limited,
        // so the period alone must never be mistaken for a limit.
        assert_eq!(cpu_max("max 100000"), None);
        assert_eq!(cpu_max("max 100000\n"), None);
        assert_eq!(cpu_max("200000 100000"), Some((200000, 100000)));
        assert_eq!(cpu_max("50000 100000"), Some((50000, 100000)));
        assert_eq!(cpu_max("garbage"), None);
        assert_eq!(cpu_max(""), None);
    }

    #[test]
    fn reads_keyed_lines() {
        let stat = "usage_usec 120943150\nuser_usec 68151710\nnr_throttled 0\nthrottled_usec 42\n";
        assert_eq!(keyed(stat, "usage_usec"), Some(120943150));
        assert_eq!(keyed(stat, "throttled_usec"), Some(42));
        assert_eq!(keyed(stat, "nr_throttled"), Some(0));
        assert_eq!(keyed(stat, "absent_key"), None);
        // A prefix of a real key must not match it.
        assert_eq!(keyed(stat, "usage"), None);
    }

    #[test]
    fn reads_memory_events() {
        let events = "low 0\nhigh 3\nmax 7\noom 2\noom_kill 1\n";
        assert_eq!(keyed(events, "max"), Some(7));
        assert_eq!(keyed(events, "oom"), Some(2));
        assert_eq!(keyed(events, "oom_kill"), Some(1));
        assert_eq!(keyed(events, "high"), Some(3));
    }

    #[test]
    fn reads_single_value_files() {
        assert_eq!(single("102903808"), Some(102903808));
        assert_eq!(single("9\n"), Some(9));
        assert_eq!(single("max"), None);
        assert_eq!(single(""), None);
    }
}
