//! `container_health` — resource contention and limits for one container.
//!
//! The container counterpart to [`crate::system_health`], and deliberately not
//! a copy of it. A container has no load average, its CPU accounting is
//! cumulative microseconds rather than jiffies, and it has two signals the host
//! has no analogue for: quota throttling and OOM kills. It also has limits,
//! which is the point — an unlimited container can take the whole machine, and
//! one limited too tightly throttles or gets killed without the host looking
//! unhealthy at all.

use rmcp::{ErrorData, schemars};
use serde::Serialize;

use crate::{cgroup, proc};

/// Resource contention and configured limits for one container.
#[derive(Serialize, schemars::JsonSchema)]
pub struct ContainerHealth {
    /// Echoes the requested target.
    target: String,
    /// Echoes the requested container ID.
    container: String,
    /// Which runtime created the container, inferred from the cgroup naming convention.
    runtime: &'static str,
    /// Wall-clock time of this reading, RFC 3339 in UTC, to one-second resolution, as the *target's* own clock reports it. Use it to judge staleness and to difference two readings: the cumulative `*_total_us` figures are monotonic, so subtracting two readings gives exact stall time over the interval between these timestamps.
    collected_at: String,
    /// Seconds this container has been running, from the creation time of its cgroup. The cgroup is created when the container's processes start and deleted when they exit, so a stop and start resets this — it is the length of the *current* run, not the container's age since creation.
    ///
    /// This is the denominator for every `*_total_us` figure below. Those counters live in the same cgroup and reset with it, so they always cover exactly this span: `full_total_us` divided by this is the fraction of the run spent fully stalled. Without it a large total is unreadable, since it may be seconds of stall over a month or over a minute. Absent if the cgroup's timestamp could not be read.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(extend("type" = "integer"))]
    uptime_seconds: Option<i64>,
    /// Process names running in the container, from the kernel's own short name for each. Deliberately not command lines: those routinely carry passwords and tokens, and this server never reads them. Names repeat across containers, so this identifies what a container *is* — `postgres`, `uvicorn` — not which one it is.
    processes: Vec<String>,
    /// Processes in the container, kernel threads included. Climbing across readings with a `pids.max` in sight is how a container hits its process limit.
    process_count: usize,
    /// Absent when the cpu controller is not delegated to this cgroup.
    #[serde(skip_serializing_if = "Option::is_none")]
    cpu: Option<CpuHealth>,
    /// Absent when the memory controller is not delegated to this cgroup.
    #[serde(skip_serializing_if = "Option::is_none")]
    memory: Option<MemoryHealth>,
    /// Absent when the pids controller is not delegated to this cgroup.
    #[serde(skip_serializing_if = "Option::is_none")]
    pids: Option<PidsHealth>,
    /// Absent when the kernel supplies no pressure-stall information for this cgroup.
    #[serde(skip_serializing_if = "Option::is_none")]
    pressure: Option<PressureSet>,
}

/// CPU use and the quota, if any, that bounds it.
#[derive(Serialize, schemars::JsonSchema)]
pub struct CpuHealth {
    /// Cumulative CPU microseconds consumed since the container started. Difference two readings to get use over an interval; the absolute figure only says how much work it has ever done.
    usage_usec: u64,
    user_usec: u64,
    system_usec: u64,
    /// CPUs this container may use, derived from its quota and period. Absent when no CPU limit is set — the container may then use the whole machine, which is normal on a single-tenant host and often unintended on a shared one.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(extend("type" = "number"))]
    limit_cpus: Option<f64>,
    /// The raw quota and period behind `limit_cpus`, in microseconds. Absent when no limit is set.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(extend("type" = "integer"))]
    quota_usec: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(extend("type" = "integer"))]
    period_usec: Option<u64>,
    /// Relative share of CPU when the machine is contended, default 100. Unlike a quota this never caps an idle machine, so a low weight only bites under contention.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(extend("type" = "integer"))]
    weight: Option<u64>,
    /// Enforcement periods elapsed. Zero when no quota is set, which is why the throttling figures below read zero on an unlimited container rather than meaning "not throttled".
    nr_periods: u64,
    /// Periods in which the container was throttled for exhausting its quota. Rising across readings means the CPU limit is too low for the work, and it is the clearest signal a container is being starved by configuration rather than by the host.
    nr_throttled: u64,
    /// Cumulative microseconds spent throttled. Monotonic, so the difference between two readings is exact stall time.
    throttled_usec: u64,
}

/// Memory use, the limits around it, and what those limits have done.
#[derive(Serialize, schemars::JsonSchema)]
pub struct MemoryHealth {
    /// Memory currently charged to the container, page cache included.
    current_bytes: u64,
    /// Hard limit. Exceeding it kills a process in the container. Absent when no limit is set, meaning the container can consume the host's memory.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(extend("type" = "integer"))]
    limit_bytes: Option<u64>,
    /// `current_bytes` as a percentage of `limit_bytes`. Absent when no limit is set. Sustained near 100 with a rising `oom_kills` is a limit set too low.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(extend("type" = "number"))]
    limit_used_percent: Option<f64>,
    /// Throttling threshold. Above it the kernel reclaims aggressively rather than killing, so a container can be slow here without ever being OOM-killed. Absent when unset.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(extend("type" = "integer"))]
    high_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(extend("type" = "integer"))]
    swap_current_bytes: Option<u64>,
    /// Times allocation was blocked at the hard limit.
    limit_hits: u64,
    /// Times the container was throttled at `high_bytes`.
    high_hits: u64,
    /// Times a process here was killed for exceeding the limit. Any non-zero value is a container that has already failed, and the host's own memory figures will not show it.
    oom_kills: u64,
    /// Times the container ran out of memory, which may resolve by reclaim without a kill.
    oom_events: u64,
}

/// Process count against the process limit.
#[derive(Serialize, schemars::JsonSchema)]
pub struct PidsHealth {
    current: u64,
    /// Maximum processes. Often a large kernel default rather than a deliberate limit, so a high value here usually means unset rather than generous. Absent when explicitly unlimited.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(extend("type" = "integer"))]
    limit: Option<u64>,
}

/// Pressure-stall information for this container's three contended resources.
///
/// Each average is a percentage of wall-clock time over the trailing 10, 60 or 300 seconds, already normalized. Unlike the host's figures, `full` is meaningful for CPU here: a container can be wholly starved while the machine as a whole is busy and healthy.
#[derive(Serialize, schemars::JsonSchema)]
pub struct PressureSet {
    #[serde(skip_serializing_if = "Option::is_none")]
    cpu: Option<Pressure>,
    #[serde(skip_serializing_if = "Option::is_none")]
    memory: Option<Pressure>,
    /// Absent when the io controller is not delegated, which is usual for rootless containers.
    #[serde(skip_serializing_if = "Option::is_none")]
    io: Option<Pressure>,
}

/// Stall time for one resource.
///
/// `some` is the share of time at least one task was stalled; `full` the share in which every task was, so the container did no useful work at all.
#[derive(Serialize, schemars::JsonSchema)]
pub struct Pressure {
    some_avg10: f64,
    full_avg10: f64,
    some_avg60: f64,
    full_avg60: f64,
    some_avg300: f64,
    full_avg300: f64,
    some_total_us: u64,
    full_total_us: u64,
}

/// Collect health for one container on the local machine.
pub async fn collect(target: String, container: String) -> Result<ContainerHealth, ErrorData> {
    let found = cgroup::discover()
        .await
        .into_iter()
        .find(|f| f.id == container)
        .ok_or_else(|| {
            ErrorData::invalid_params(
                format!(
                    "no container {container:?} on target {target:?}; \
                     call container_list for the containers that exist now"
                ),
                None,
            )
        })?;

    let (processes, process_count) = cgroup::identify(&found.path).await;
    // The target's own clock, differenced against the cgroup's timestamp from
    // the same machine — never this server's clock, which would be wrong the
    // moment the reads happen over SSH.
    let now = proc::target_now().await?;
    let uptime_seconds = proc::dir_mtime(&found.path)
        .await
        .map(|created| (now - created).max(0));
    Ok(ContainerHealth {
        target,
        container: found.id,
        runtime: found.runtime,
        collected_at: proc::wall_clock_from(now),
        uptime_seconds,
        processes,
        process_count,
        cpu: cpu_health(&found.path).await,
        memory: memory_health(&found.path).await,
        pids: pids_health(&found.path).await,
        pressure: pressure_set(&found.path).await,
    })
}

async fn cpu_health(path: &str) -> Option<CpuHealth> {
    let stat = proc::read_optional(&format!("{path}/cpu.stat")).await?;
    let max = proc::read_optional(&format!("{path}/cpu.max")).await;
    let quota = max.as_deref().and_then(cgroup::cpu_max);
    Some(CpuHealth {
        usage_usec: cgroup::keyed(&stat, "usage_usec")?,
        user_usec: cgroup::keyed(&stat, "user_usec").unwrap_or_default(),
        system_usec: cgroup::keyed(&stat, "system_usec").unwrap_or_default(),
        limit_cpus: quota.map(|(q, p)| round(q as f64 / p as f64, 3)),
        quota_usec: quota.map(|(q, _)| q),
        period_usec: quota.map(|(_, p)| p),
        weight: proc::read_optional(&format!("{path}/cpu.weight"))
            .await
            .as_deref()
            .and_then(cgroup::single),
        nr_periods: cgroup::keyed(&stat, "nr_periods").unwrap_or_default(),
        nr_throttled: cgroup::keyed(&stat, "nr_throttled").unwrap_or_default(),
        throttled_usec: cgroup::keyed(&stat, "throttled_usec").unwrap_or_default(),
    })
}

async fn memory_health(path: &str) -> Option<MemoryHealth> {
    let current = proc::read_optional(&format!("{path}/memory.current")).await?;
    let current_bytes = cgroup::single(&current)?;
    let limit_bytes = proc::read_optional(&format!("{path}/memory.max"))
        .await
        .as_deref()
        .and_then(cgroup::limit);
    let events = proc::read_optional(&format!("{path}/memory.events"))
        .await
        .unwrap_or_default();
    Some(MemoryHealth {
        current_bytes,
        limit_bytes,
        limit_used_percent: limit_bytes
            .filter(|l| *l > 0)
            .map(|l| round(current_bytes as f64 * 100.0 / l as f64, 1)),
        high_bytes: proc::read_optional(&format!("{path}/memory.high"))
            .await
            .as_deref()
            .and_then(cgroup::limit),
        swap_current_bytes: proc::read_optional(&format!("{path}/memory.swap.current"))
            .await
            .as_deref()
            .and_then(cgroup::single),
        limit_hits: cgroup::keyed(&events, "max").unwrap_or_default(),
        high_hits: cgroup::keyed(&events, "high").unwrap_or_default(),
        oom_kills: cgroup::keyed(&events, "oom_kill").unwrap_or_default(),
        oom_events: cgroup::keyed(&events, "oom").unwrap_or_default(),
    })
}

async fn pids_health(path: &str) -> Option<PidsHealth> {
    let current = proc::read_optional(&format!("{path}/pids.current")).await?;
    Some(PidsHealth {
        current: cgroup::single(&current)?,
        limit: proc::read_optional(&format!("{path}/pids.max"))
            .await
            .as_deref()
            .and_then(cgroup::limit),
    })
}

/// Assemble the pressure block, or `None` if this cgroup offers no PSI at all.
///
/// The three resources are read independently for the reason decision 20 gives:
/// one unreadable file says nothing about the other two, so it must not discard
/// readings already in hand.
async fn pressure_set(path: &str) -> Option<PressureSet> {
    let cpu = read_pressure(&format!("{path}/cpu.pressure")).await;
    let memory = read_pressure(&format!("{path}/memory.pressure")).await;
    let io = read_pressure(&format!("{path}/io.pressure")).await;
    if cpu.is_none() && memory.is_none() && io.is_none() {
        return None;
    }
    Some(PressureSet { cpu, memory, io })
}

async fn read_pressure(path: &str) -> Option<Pressure> {
    let text = proc::read_optional(path).await?;
    let parsed = Pressure {
        some_avg10: proc::psi_avg(&text, "some ", "avg10=")?,
        full_avg10: proc::psi_avg(&text, "full ", "avg10=")?,
        some_avg60: proc::psi_avg(&text, "some ", "avg60=")?,
        full_avg60: proc::psi_avg(&text, "full ", "avg60=")?,
        some_avg300: proc::psi_avg(&text, "some ", "avg300=")?,
        full_avg300: proc::psi_avg(&text, "full ", "avg300=")?,
        some_total_us: proc::psi_total(&text, "some ")?,
        full_total_us: proc::psi_total(&text, "full ")?,
    };
    Some(parsed)
}

/// Round a derived value, so arithmetic on kernel readings does not hand back
/// more apparent precision than the readings ever had.
fn round(value: f64, places: i32) -> f64 {
    let scale = 10f64.powi(places);
    (value * scale).round() / scale
}
