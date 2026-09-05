# stethoscope-mcp

A local [MCP](https://modelcontextprotocol.io) server giving an AI assistant
structured, read-only observability into a Linux machine — narrow, named
capabilities instead of shell access.

**Status: early. v0.1 in progress.** Two tools exist: `system_info` and
`system_health`.
This is a learning project, built deliberately over multiple sessions.

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
