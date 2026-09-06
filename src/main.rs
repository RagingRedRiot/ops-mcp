//! stethoscope-mcp — a local stdio MCP server providing constrained, read-only observability.
//!
//! See `docs/internal/` for the architecture and decision log. The single
//! most important rule: this server gives the model *capabilities*, never
//! arbitrary command execution.
//!
//! This file is deliberately the whole model-facing surface: every tool the
//! server exposes is declared here, and the bodies do nothing but validate the
//! target and delegate. You can read one short file and enumerate exactly what
//! this server can do.
//!
//! NOTE: stdout is the MCP transport. Never `println!` here. Anything
//! diagnostic must go to stderr.

mod cgroup;
mod container_health;
mod container_list;
mod guard;
mod proc;
mod system_health;
mod system_info;

use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::model::{Implementation, ServerCapabilities, ServerInfo};
use rmcp::transport::stdio;
use rmcp::{ErrorData, ServerHandler, ServiceExt, schemars, tool, tool_handler, tool_router};
use serde::Deserialize;

use container_health::ContainerHealth;
use container_list::ContainerList;
use system_health::SystemHealth;
use system_info::SystemInfo;

/// The only target that exists today.
const LOCAL_TARGET: &str = "local";

/// Every tool takes the same required `target`, so they share
/// one parameter type and therefore one input schema.
#[derive(Deserialize, schemars::JsonSchema)]
struct TargetParams {
    /// Machine to act on. Only "local" is available today.
    target: String,
}

/// Tools that act on a single container take its ID alongside the target.
#[derive(Deserialize, schemars::JsonSchema)]
struct ContainerParams {
    /// Machine to act on. Only "local" is available today.
    target: String,
    /// Container ID, as returned by `container_list`. IDs are stable while a container lives and gone once it exits.
    container: String,
}

/// Reject an alias that names no target we know.
///
/// An unknown alias is a malformed request, not an unreachable machine, so it
/// is `invalid_params`.
///
/// This function is the entire target mechanism: a string compared against one
/// constant. It is deliberately not a target type, trait, or registry — where
/// that seam belongs cannot be judged until a real remote implementation
/// pushes on it. It is nonetheless the natural home for dispatch once there is
/// more than one target to route to; today there is nothing to route, only
/// something to reject.
fn check_target(target: &str) -> Result<(), ErrorData> {
    if target == LOCAL_TARGET {
        return Ok(());
    }
    Err(ErrorData::invalid_params(
        format!("unknown target {target:?}; the only available target is \"local\""),
        None,
    ))
}

/// The server itself holds no state. `#[tool_handler]` builds the tool router
/// on demand, so there is nothing to construct or carry.
struct StethoscopeMcp;

#[tool_router]
impl StethoscopeMcp {
    #[tool(
        name = "system_info",
        description = "Basic identity of a target machine: hostname, kernel release, OS and CPU architecture."
    )]
    async fn system_info(
        &self,
        Parameters(TargetParams { target }): Parameters<TargetParams>,
    ) -> Result<Json<SystemInfo>, ErrorData> {
        check_target(&target)?;
        Ok(Json(system_info::collect(target).await?))
    }

    #[tool(
        name = "container_list",
        description = "The containers on a target machine, by ID, with the runtime that created each. Reads the kernel's cgroup hierarchy rather than asking a container runtime, so it works the same for Docker, podman and Kubernetes and needs no daemon to be running. Returns no container names or images: call container_health for what a container is running."
    )]
    async fn container_list(
        &self,
        Parameters(TargetParams { target }): Parameters<TargetParams>,
    ) -> Result<Json<ContainerList>, ErrorData> {
        check_target(&target)?;
        Ok(Json(container_list::collect(target).await?))
    }

    #[tool(
        name = "container_health",
        description = "Resource contention and configured limits for one container: CPU use and throttling, memory use against its limit, OOM kills, process count, and kernel pressure-stall figures. Also reports which limits are set at all — an unlimited container can consume the whole machine, and one limited too tightly is throttled or killed while the host itself looks healthy. Raw numbers only, with no thresholds or verdicts applied; the output schema describes how to read them."
    )]
    async fn container_health(
        &self,
        Parameters(ContainerParams { target, container }): Parameters<ContainerParams>,
    ) -> Result<Json<ContainerHealth>, ErrorData> {
        check_target(&target)?;
        Ok(Json(container_health::collect(target, container).await?))
    }

    #[tool(
        name = "system_health",
        description = "Resource-contention health for a target machine: load average with CPU count, memory availability, swap, and kernel pressure-stall figures for CPU, memory and I/O. Raw numbers only, with no thresholds or verdicts applied; the output schema describes how to read them."
    )]
    async fn system_health(
        &self,
        Parameters(TargetParams { target }): Parameters<TargetParams>,
    ) -> Result<Json<SystemHealth>, ErrorData> {
        check_target(&target)?;
        Ok(Json(system_health::collect(target).await?))
    }
}

#[tool_handler]
impl ServerHandler for StethoscopeMcp {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(
                Implementation::new("stethoscope-mcp", env!("CARGO_PKG_VERSION"))
                    .with_title("stethoscope-mcp"),
            )
            .with_instructions(
                "Read-only observability for Linux machines. Tools take a target \
                 and return normalized, structured results; the only target \
                 currently available is \"local\". There is deliberately no \
                 arbitrary shell or command-execution tool.",
            )
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let service = StethoscopeMcp.serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
