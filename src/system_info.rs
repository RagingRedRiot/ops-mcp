//! `system_info` — basic identity of a target.

use rmcp::{ErrorData, schemars};
use serde::Serialize;

use crate::proc;

/// Normalized system identity for a target.
///
/// The shape is intended to be identical for local and remote targets — how
/// the values were collected is an implementation detail the model does not
/// see. The field set is deliberately small: it exists to prove the MCP
/// plumbing, not to be a complete inventory.
#[derive(Serialize, schemars::JsonSchema)]
pub struct SystemInfo {
    /// Echoes the requested target, so a response is self-describing when several targets are queried.
    target: String,
    hostname: String,
    kernel_release: String,
    os: &'static str,
    arch: &'static str,
}

/// Collect identity for the local machine.
///
/// `target` is carried only so the response can echo it; nothing here branches
/// on it. Validation happens once, in `check_target`, which is where dispatch
/// will land when there is something to dispatch to. See [`proc::read`] for
/// why no target abstraction exists yet.
pub async fn collect(target: String) -> Result<SystemInfo, ErrorData> {
    Ok(SystemInfo {
        target,
        hostname: proc::read("/proc/sys/kernel/hostname").await?,
        kernel_release: proc::read("/proc/sys/kernel/osrelease").await?,
        os: std::env::consts::OS,
        arch: std::env::consts::ARCH,
    })
}
