//! ops-mcp — a local stdio MCP server providing constrained, read-only
//! observability into the machine it runs on.
//!
//! See `docs/internal/` for the architecture and decision log. The single
//! most important rule: this server gives the model *capabilities*, never
//! arbitrary command execution.
//!
//! NOTE: stdout is the MCP transport. Never `println!` here. Anything
//! diagnostic must go to stderr.

use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::model::{Implementation, ServerCapabilities, ServerInfo};
use rmcp::transport::stdio;
use rmcp::{ErrorData, ServerHandler, ServiceExt, schemars, tool, tool_handler, tool_router};
use serde::{Deserialize, Serialize};

/// The only target that exists today. Remote aliases arrive in v0.2, sourced
/// from the user's SSH config rather than from anything hardcoded here.
const LOCAL_TARGET: &str = "local";

#[derive(Deserialize, schemars::JsonSchema)]
struct SystemInfoParams {
    /// Machine to describe. Only "local" is available today; remote targets
    /// arrive with v0.2 SSH support.
    target: String,
}

/// Normalized system identity for a target.
///
/// The shape is intended to be identical for local and remote targets — how
/// the values were collected is an implementation detail the model does not
/// see. The field set is deliberately small: it exists to prove the MCP
/// plumbing, not to be a complete inventory.
#[derive(Serialize, schemars::JsonSchema)]
struct SystemInfo {
    /// Echoes the requested target, so a response is self-describing when
    /// several targets are queried.
    target: String,
    hostname: String,
    kernel_release: String,
    os: &'static str,
    arch: &'static str,
}

/// The server itself holds no state. `#[tool_handler]` builds the tool router
/// on demand, so there is nothing to construct or carry.
struct OpsMcp;

#[tool_router]
impl OpsMcp {
    #[tool(
        name = "system_info",
        description = "Basic identity of a target machine: hostname, kernel release, OS and CPU architecture. Read-only. The only target currently available is \"local\"."
    )]
    async fn system_info(
        &self,
        Parameters(SystemInfoParams { target }): Parameters<SystemInfoParams>,
    ) -> Result<Json<SystemInfo>, ErrorData> {
        // Unknown alias is a malformed request, not an unreachable machine.
        // Reporting reachability is a separate question, still open, that
        // arrives with remote targets.
        if target != LOCAL_TARGET {
            return Err(ErrorData::invalid_params(
                format!("unknown target {target:?}; the only available target is \"local\""),
                None,
            ));
        }

        Ok(Json(SystemInfo {
            target,
            hostname: read_proc("/proc/sys/kernel/hostname").await?,
            kernel_release: read_proc("/proc/sys/kernel/osrelease").await?,
            os: std::env::consts::OS,
            arch: std::env::consts::ARCH,
        }))
    }
}

/// Read a single-line `/proc` value.
///
/// `tokio::fs` is not truly async — it hands the ordinary blocking read to
/// `spawn_blocking`. That is the point: the wait, however brief, lands on the
/// blocking pool instead of an async worker thread, so no tool can stall the
/// runtime for the requests running alongside it. Reads here are microseconds,
/// but the policy is what matters once tools do heavier I/O.
///
/// The underlying io::Error is intentionally not forwarded to the model: it
/// can carry local paths and details the model has no need for. Detail belongs
/// in operator-facing logs (not yet implemented), not in model context.
async fn read_proc(path: &str) -> Result<String, ErrorData> {
    tokio::fs::read_to_string(path)
        .await
        .map(|s| s.trim().to_owned())
        .map_err(|e| {
            eprintln!("ops-mcp: failed to read {path}: {e}");
            ErrorData::internal_error("failed to read system information", None)
        })
}

#[tool_handler]
impl ServerHandler for OpsMcp {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(
                Implementation::new("ops-mcp", env!("CARGO_PKG_VERSION")).with_title("ops-mcp"),
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
    let service = OpsMcp.serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
