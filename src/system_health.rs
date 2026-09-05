//! `system_health` — resource-contention health for a target.
//!
//! Units normalized at the boundary, derived values computed alongside the
//! inputs they came from, and no verdicts in the payload.
//!
//! `///` on a nested struct or a field is model-facing: `schemars` lifts it
//! into the output schema and every character is spent on each `tools/list`.
//! It earns its place only by changing how the model reads a number. Design
//! rationale goes in `//` instead, which stays invisible to the model. A doc
//! comment on the root struct is dropped by `schemars` and so is free.

use rmcp::{ErrorData, schemars};
use serde::Serialize;

use crate::proc;

/// Resource-contention health for a target.
///
/// The four sources are read together deliberately: the useful conclusions
/// come from the combination, not from any single number. The model-facing
/// half of that lives on [`CpuHealth`], since a doc comment here is dropped
/// before the schema is generated.
#[derive(Serialize, schemars::JsonSchema)]
pub struct SystemHealth {
    /// Echoes the requested target, so a response is self-describing when several targets are queried.
    target: String,
    /// Wall-clock time of this reading, RFC 3339 in UTC, to one-second resolution, as the *target's* own clock reports it. If the target's clock is wrong then so is this field.
    ///
    /// Use it to judge staleness and to line readings up against other machines or external logs. Do *not* use it to measure the interval between two readings: subtract `uptime_seconds` instead, which is monotonic and so unaffected by clock steps from NTP.
    collected_at: String,
    /// Seconds since boot. On a machine that suspends, this includes the time spent suspended.
    uptime_seconds: f64,
    cpu: CpuHealth,
    memory: MemoryHealth,
    /// Absent when the kernel supplies no pressure-stall information at all: PSI needs Linux 4.20 or later and can be compiled out or disabled. Absence is a normal answer, not a failure.
    #[serde(skip_serializing_if = "Option::is_none")]
    pressure: Option<PressureSet>,
}

/// CPU demand.
///
/// Linux load average counts tasks in uninterruptible sleep — usually blocked on disk — as well as tasks waiting for a CPU, so high load alone does not mean CPU saturation. Read it against `pressure`, which separates the two causes: high load with low `pressure.cpu` and high `pressure.io` means work is blocked on disk rather than starved of CPU.
#[derive(Serialize, schemars::JsonSchema)]
pub struct CpuHealth {
    /// Online CPUs. The load figures below cannot be interpreted without it.
    count: u32,
    /// Load averaged over the last minute. These are exponentially damped rather than sliding windows, so they lag a change in both directions: a still-elevated average does not prove the cause is still present. The trend across the three is the signal — 1m above 15m means it is getting worse now.
    load_1m: f64,
    load_5m: f64,
    load_15m: f64,
    /// `load_1m` divided by `count`. Around 1.0 means fully subscribed; well above 1.0 means more demand than the machine can serve at once.
    load_1m_per_cpu: f64,
    /// Share of all CPU time spent doing work since boot, 0.0 to 1.0. A long-run average over `uptime_seconds`, not a current reading.
    busy_fraction_since_boot: f64,
    /// Tasks runnable at the instant of reading.
    runnable_tasks: u32,
    /// Tasks that exist, kernel threads included. A steadily climbing count across readings suggests something is spawning and not reaping.
    total_tasks: u32,
}

// `MemFree` is deliberately not reported. On a healthy busy machine it is near
// zero, because the kernel spends otherwise-idle memory on page cache — which
// makes it the most commonly misread field in `/proc/meminfo`.
// `available_bytes` is the field that answers whether the machine can take on
// more work. No `///` here: the model needs no account of a field it cannot see.
#[derive(Serialize, schemars::JsonSchema)]
pub struct MemoryHealth {
    total_bytes: u64,
    /// The kernel's own estimate of what new work could allocate without pushing the machine into swap, counting the page cache and slab it could reclaim to satisfy the request.
    available_bytes: u64,
    /// `available_bytes` as a percentage of `total_bytes`.
    available_percent: f64,
    /// Page cache. Reclaimable, so this is not memory lost.
    cached_bytes: u64,
    /// Modified pages not yet written back to disk. Large and growing across readings suggests writes backing up against a slow device.
    dirty_bytes: u64,
    swap_total_bytes: u64,
    /// Swap in use. Occupied swap is not itself a problem — evicting cold pages is correct behavior. Sustained swap *activity* is the problem, and it shows up in `pressure.memory` rather than here.
    swap_used_bytes: u64,
    /// Kernel data structures that cannot be reclaimed under pressure. Large and steadily growing across readings is the signature of a kernel or driver memory leak.
    kernel_unreclaimable_bytes: u64,
}

/// Kernel pressure-stall information. Each of the three resources is reported independently, and any one of them may be absent while the others are present.
///
/// Each average is a percentage of wall-clock time over the trailing 10, 60 or 300 seconds, already normalized: no dividing by CPU count, no second sample needed. PSI measures time in which work did not happen *because a resource was unavailable* — a task blocked on a socket, a lock, or its own sleep is not stalled in this sense.
#[derive(Serialize, schemars::JsonSchema)]
pub struct PressureSet {
    #[serde(skip_serializing_if = "Option::is_none")]
    cpu: Option<CpuPressure>,
    #[serde(skip_serializing_if = "Option::is_none")]
    memory: Option<Pressure>,
    #[serde(skip_serializing_if = "Option::is_none")]
    io: Option<Pressure>,
}

/// CPU pressure: time during which at least one task was runnable but not scheduled.
// Only `some` is reported. "Every task stalled" is structurally impossible for
// CPU at machine level, because if anything is runnable the CPU is running it —
// the kernel always reports zero there, and returning a permanently-zero field
// would invite meaning to be read into it. Per-cgroup CPU `full` *is*
// meaningful, since a whole container can be starved while others run, but that
// comes from cgroup data rather than this file.
#[derive(Serialize, schemars::JsonSchema)]
pub struct CpuPressure {
    /// Percentage of the last 10 seconds in which at least one task was waiting for a CPU. Below 10 is unremarkable; sustained above 30 is real contention.
    some_avg10: f64,
    some_avg60: f64,
    some_avg300: f64,
    /// Cumulative microseconds of stall since boot. Monotonic, so the difference between two readings is the exact stall time in between — useful when the smoothed averages are too coarse.
    some_total_us: u64,
}

/// Stall time for memory or I/O.
///
/// `some` is the share of time at least one task was stalled. `full` is the share in which *every* non-idle task was stalled simultaneously, so nothing useful happened at all — `full_avg10` above roughly 10 means the machine is losing a serious fraction of its capacity to this resource. Memory stalls count refaults, thrashing, direct reclaim and swap-in; I/O stalls count waiting on actual disk reads and writes.
#[derive(Serialize, schemars::JsonSchema)]
pub struct Pressure {
    some_avg10: f64,
    full_avg10: f64,
    some_avg60: f64,
    full_avg60: f64,
    some_avg300: f64,
    full_avg300: f64,
    /// Cumulative microseconds of stall since boot; see `CpuPressure`.
    some_total_us: u64,
    full_total_us: u64,
}

/// Collect health for the local machine.
///
/// `target` is carried only so the response can echo it; nothing here branches
/// on it. Validation happens once, in `check_target`, which is where dispatch
/// will land when there is something to dispatch to. See [`proc::read`] for
/// why no target abstraction exists yet — the reads below are the concrete
/// reason the shape of that seam is still an open question.
pub async fn collect(target: String) -> Result<SystemHealth, ErrorData> {
    // /proc/uptime: seconds since boot, then idle seconds summed over all
    // CPUs — which is why the second figure routinely exceeds the first.
    let uptime_raw = proc::read("/proc/uptime").await?;
    let mut uptime_fields = uptime_raw.split_whitespace();
    let uptime_seconds: f64 = proc::field(uptime_fields.next(), "/proc/uptime", "uptime")?;
    let idle_seconds: f64 = proc::field(uptime_fields.next(), "/proc/uptime", "idle time")?;

    // /proc/stat's per-CPU lines are the authority on how many CPUs are
    // online. Counting them keeps the whole tool a matter of reading /proc,
    // which is what makes the same parser work over SSH later.
    let stat = proc::read("/proc/stat").await?;
    let cpu_count = proc::count_cpus(&stat);
    if cpu_count == 0 {
        return Err(proc::parse_error("/proc/stat", "no per-CPU lines"));
    }

    // The same file carries boot time, so the target's wall clock costs no
    // extra read: it is boot time plus how long the machine has been up.
    let btime = proc::stat_btime(&stat).ok_or_else(|| proc::parse_error("/proc/stat", "btime"))?;
    let collected_at = wall_clock(btime, uptime_seconds)
        .ok_or_else(|| proc::parse_error("/proc/stat", "boot time out of range"))?;

    // /proc/loadavg: three averages, runnable/total tasks, and the PID of the
    // last process created.
    let loadavg_raw = proc::read("/proc/loadavg").await?;
    let loadavg: Vec<&str> = loadavg_raw.split_whitespace().collect();
    let load_1m: f64 = proc::field(loadavg.first().copied(), "/proc/loadavg", "1-minute load")?;
    let load_5m: f64 = proc::field(loadavg.get(1).copied(), "/proc/loadavg", "5-minute load")?;
    let load_15m: f64 = proc::field(loadavg.get(2).copied(), "/proc/loadavg", "15-minute load")?;
    let mut tasks = loadavg.get(3).copied().unwrap_or_default().split('/');
    let runnable_tasks: u32 = proc::field(tasks.next(), "/proc/loadavg", "runnable tasks")?;
    let total_tasks: u32 = proc::field(tasks.next(), "/proc/loadavg", "total tasks")?;

    let meminfo = proc::read("/proc/meminfo").await?;
    let mem = |key: &str| {
        proc::meminfo_bytes(&meminfo, key).ok_or_else(|| proc::parse_error("/proc/meminfo", key))
    };
    let total_bytes = mem("MemTotal")?;
    let available_bytes = mem("MemAvailable")?;
    let swap_total_bytes = mem("SwapTotal")?;
    let swap_free_bytes = mem("SwapFree")?;

    Ok(SystemHealth {
        target,
        collected_at,
        uptime_seconds,
        cpu: CpuHealth {
            count: cpu_count,
            load_1m,
            load_5m,
            load_15m,
            load_1m_per_cpu: round(load_1m / f64::from(cpu_count), 3),
            busy_fraction_since_boot: round(
                busy_fraction(idle_seconds, uptime_seconds, cpu_count),
                3,
            ),
            runnable_tasks,
            total_tasks,
        },
        memory: MemoryHealth {
            total_bytes,
            available_bytes,
            available_percent: round(percent_of(available_bytes, total_bytes), 1),
            cached_bytes: mem("Cached")?,
            dirty_bytes: mem("Dirty")?,
            swap_total_bytes,
            swap_used_bytes: swap_total_bytes.saturating_sub(swap_free_bytes),
            kernel_unreclaimable_bytes: mem("SUnreclaim")?,
        },
        pressure: read_pressure().await,
    })
}

/// The target's own wall clock, as RFC 3339 in UTC.
///
/// Boot time plus uptime. Formatted without a fractional part because `btime`
/// is only recorded to the second, and implying more precision than the source
/// has would be its own small lie.
fn wall_clock(btime: u64, uptime_seconds: f64) -> Option<String> {
    let epoch_seconds = i64::try_from(btime)
        .ok()?
        .checked_add(uptime_seconds as i64)?;
    Some(
        chrono::DateTime::from_timestamp(epoch_seconds, 0)?
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
    )
}

/// Fraction of CPU time spent doing work since boot.
///
/// `/proc/uptime`'s idle figure is summed across CPUs, so it has to be divided
/// by their number before it can be compared against wall-clock uptime. The
/// clamp guards the edges: the two figures come from different clocks and can
/// disagree slightly, which would otherwise produce a negative "busy".
fn busy_fraction(idle_seconds: f64, uptime_seconds: f64, cpu_count: u32) -> f64 {
    if uptime_seconds <= 0.0 {
        return 0.0;
    }
    let idle_per_cpu = idle_seconds / f64::from(cpu_count);
    (1.0 - idle_per_cpu / uptime_seconds).clamp(0.0, 1.0)
}

fn percent_of(part: u64, whole: u64) -> f64 {
    if whole == 0 {
        return 0.0;
    }
    part as f64 * 100.0 / whole as f64
}

/// Round a derived value, so arithmetic on kernel readings does not hand back
/// more apparent precision than the readings ever had.
fn round(value: f64, places: i32) -> f64 {
    let scale = 10f64.powi(places);
    (value * scale).round() / scale
}

/// Assemble the pressure block, or `None` if this kernel offers no PSI at all.
///
/// Every failure here — missing file or unreadable layout — collapses to an
/// absent field rather than failing the surrounding call, because a machine
/// without PSI still has a perfectly good answer for load and memory
/// (decision 18, rule 7).
///
/// The three resources are read independently. One unreadable file says
/// nothing about the other two, so it must not discard readings already in
/// hand, and it must not stop the remaining files from being read.
async fn read_pressure() -> Option<PressureSet> {
    let cpu = read_psi("/proc/pressure/cpu", parse_cpu_pressure).await;
    let memory = read_psi("/proc/pressure/memory", parse_pressure).await;
    let io = read_psi("/proc/pressure/io", parse_pressure).await;

    // All three absent is the ordinary "kernel has no PSI" answer, and an
    // empty object would say less than no object at all.
    if cpu.is_none() && memory.is_none() && io.is_none() {
        return None;
    }
    Some(PressureSet { cpu, memory, io })
}

/// Read and parse one pressure file, or `None` if it is missing or malformed.
///
/// A missing file is the expected shape of a kernel built without PSI and is
/// silent. A file that exists but does not parse is a real anomaly against a
/// stable kernel format, so it is logged for the operator — who can act on it,
/// unlike the model.
async fn read_psi<T>(path: &str, parse: fn(&str) -> Option<T>) -> Option<T> {
    let parsed = parse(&proc::read_optional(path).await?);
    if parsed.is_none() {
        eprintln!("ops-mcp: unexpected layout in {path}; omitting it");
    }
    parsed
}

fn parse_cpu_pressure(text: &str) -> Option<CpuPressure> {
    Some(CpuPressure {
        some_avg10: proc::psi_avg(text, "some ", "avg10=")?,
        some_avg60: proc::psi_avg(text, "some ", "avg60=")?,
        some_avg300: proc::psi_avg(text, "some ", "avg300=")?,
        some_total_us: proc::psi_total(text, "some ")?,
    })
}

fn parse_pressure(text: &str) -> Option<Pressure> {
    Some(Pressure {
        some_avg10: proc::psi_avg(text, "some ", "avg10=")?,
        full_avg10: proc::psi_avg(text, "full ", "avg10=")?,
        some_avg60: proc::psi_avg(text, "some ", "avg60=")?,
        full_avg60: proc::psi_avg(text, "full ", "avg60=")?,
        some_avg300: proc::psi_avg(text, "some ", "avg300=")?,
        full_avg300: proc::psi_avg(text, "full ", "avg300=")?,
        some_total_us: proc::psi_total(text, "some ")?,
        full_total_us: proc::psi_total(text, "full ")?,
    })
}
