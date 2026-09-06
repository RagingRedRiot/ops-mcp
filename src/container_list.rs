//! `container_list` — the containers on a target, with enough identity to pick one.
//!
//! Each entry carries its ID, the runtime that created it, and the process
//! names running inside it. No runtime-assigned name or image: decision 24
//! defers that to a separate future tool, so nothing here reads a runtime's
//! private state.
//!
//! Process names are included rather than left to `container_health` because
//! the alternative is calling that tool on every container to find the one you
//! want. On a host of a dozen containers that is a dozen calls returning full
//! metrics, against one listing of a few hundred bytes.

use rmcp::{ErrorData, schemars};
use serde::Serialize;

use crate::{cgroup, proc};

/// The containers found on a target.
#[derive(Serialize, schemars::JsonSchema)]
pub struct ContainerList {
    /// Echoes the requested target, so a response is self-describing when several targets are queried.
    target: String,
    /// Wall-clock time of this reading, RFC 3339 in UTC, to one-second resolution, as the *target's* own clock reports it. Use it to judge staleness and to difference two readings: the cumulative `*_total_us` figures are monotonic, so subtracting two readings gives exact stall time over the interval between these timestamps.
    collected_at: String,
    /// How many containers were found. Zero is an ordinary answer: the host may run none, or may use a runtime whose cgroup layout this server does not yet recognize.
    count: usize,
    containers: Vec<Container>,
}

/// One container, identified the only way that needs no runtime cooperation.
#[derive(Serialize, schemars::JsonSchema)]
pub struct Container {
    /// The container ID as the kernel knows it, taken from the cgroup directory name. This is what `container_health` takes. It is stable while the container lives and gone once it exits, so it should not be carried across a long gap or used to refer to a workload that an orchestrator may have replaced.
    id: String,
    /// Which runtime created the container, inferred from the cgroup naming convention: `docker`, `podman`, `containerd` or `cri-o`. It says nothing about health and is here because it tells the model which conventions apply.
    runtime: &'static str,
    /// Process names running in the container, from the kernel's own short name for each — `postgres`, `uvicorn`, `redis-server`. Deliberately not command lines: those routinely carry passwords and tokens, and this server never reads them.
    ///
    /// Enough to pick the container you want without calling `container_health` on each in turn. It says what a container is running, not which one it is: names repeat across containers, so the ID remains the identifier.
    processes: Vec<String>,
}

/// Collect the container list for the local machine.
pub async fn collect(target: String) -> Result<ContainerList, ErrorData> {
    let found = cgroup::discover().await;
    let collected_at = proc::collected_at().await?;
    let mut containers = Vec::with_capacity(found.len());
    for f in found {
        let (processes, _) = cgroup::identify(&f.path).await;
        containers.push(Container {
            id: f.id,
            runtime: f.runtime,
            processes,
        });
    }
    Ok(ContainerList {
        target,
        collected_at,
        count: containers.len(),
        containers,
    })
}
