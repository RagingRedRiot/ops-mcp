# stethoscope-mcp

A local [MCP](https://modelcontextprotocol.io) server giving an AI assistant
structured, read-only observability into a Linux machine — narrow, named
capabilities instead of shell access.

**Status: early. v0.1 in progress.** Four tools exist: `system_info`,
`system_health`, `container_list` and `container_health`.
This is a learning project, built deliberately over multiple sessions.

## Container runtime support

Container data comes from the kernel's cgroup v2 hierarchy rather than from any
container runtime's API, so the health, limit and pressure figures are the same
whatever created the container. Only *discovery* is runtime-specific, because
each runtime names its cgroups differently.

| runtime | discovery | status |
|---|---|---|
| Docker (systemd cgroup driver) | `system.slice/docker-<id>.scope` | **tested** |
| podman, rootless | `user.slice/…/user@<uid>.service/user.slice/libpod-<id>.scope` | **tested** |
| podman, rootful | `machine.slice/libpod-<id>.scope` | untested |
| Docker (cgroupfs driver) | `docker/<id>` | untested |
| containerd / CRI-O (Kubernetes) | `kubepods.slice/…/cri-containerd-<id>.scope` | **untested** |

Kubernetes support is a best-effort reading of the documented cgroup layout. No
cluster has been stood up to verify it, and none will be purely to claim
compatibility — so treat that row as unproven rather than broken. Kubernetes
also raises a question the other runtimes do not: whether the tools should
report pods, containers, or both.

**If you run Kubernetes (or rootful podman, or the cgroupfs driver) and would
like to test this, contributions are very welcome.** The useful report is what
`container_list` returns against a real cluster, and if it returns nothing, the
output of `find /sys/fs/cgroup -maxdepth 6 -name "*.scope" | head`. Discovery
lives in one function, `classify` in [`src/cgroup.rs`](src/cgroup.rs), and
adding a naming convention is usually a one-line change plus a test.

## Design

The guiding principle is *give the model capabilities, not credentials or
arbitrary machine access*. There is deliberately no `execute_shell` tool, and
there never will be.

Design documentation lives in [`docs/internal/`](docs/internal/) and is written
as working memory rather than polished docs:

* [`current-state.md`](docs/internal/current-state.md) — start here: what
  exists versus what is only planned
* [`architecture.md`](docs/internal/architecture.md) — the design and its
  trust boundaries
* [`decisions.md`](docs/internal/decisions.md) — what we decided and why

## Build and run

```sh
cargo build --release
```

`stethoscope-mcp` speaks MCP over stdio and is meant to be spawned by an MCP client.
It runs as the user who launches it and has exactly that user's permissions.
