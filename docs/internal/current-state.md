# ops-mcp — Current state

> **Read this first.** It answers: what are we building, what has actually been
> implemented, and what only *looks* implemented because it is described in
> `architecture.md`?
>
> Last updated: 2026-09-03 (bootstrap session; tool naming settled; smoke
> script added; tool I/O moved to tokio).
> Keep this file honest. If you implement something, update it here.

## What we are building

A local MCP server giving an AI assistant structured, read-only observability
into the user's Linux machine — and eventually remote machines via the user's
own OpenSSH — through narrow, named capabilities rather than shell access.
Full rationale in [`architecture.md`](architecture.md); the decisions and
their reasoning are in [`decisions.md`](decisions.md).

## Current milestone

**v0.1 — local, read-only.** Just bootstrapped. One proof-of-life tool exists.

## What actually exists

Everything, in full:

* A Rust binary crate (edition 2024) using `rmcp` 3.2.0.
* `src/main.rs`, ~120 lines, the entire implementation:
  * an MCP **stdio** server that initializes and advertises tool capability;
  * one read-only tool, `system_info(target)`, returning `target`, `hostname`,
    `kernel_release`, `os`, `arch`, read from
    `/proc/sys/kernel/{hostname,osrelease}` and `std::env::consts`. `target` is
    required; anything other than `"local"` is rejected with `invalid_params`
    (decision 15);
  * `read_proc`, an async helper using `tokio::fs` so the read lands on the
    blocking pool rather than an async worker (decision 16). It logs the real
    `io::Error` to stderr and returns a generic message to the model —
    decision 9, applied locally.
* Three dependencies: `rmcp`, `tokio`, `serde`.
* `scripts/smoke.sh` — drives the server over stdio with real JSON-RPC and
  asserts six things about the replies. Needs only `jq`; no MCP client, no
  Node, no test framework.
* This documentation.

Verified by `scripts/smoke.sh`: the handshake, tool advertisement, `target`
being required, `target="local"` returning a hostname, and `target="nas"`
being rejected as `invalid_params`. Builds clean under
`cargo clippy --all-targets`.

## What is deliberately NOT implemented

Described in `architecture.md`, **none of it exists in code**:

* SSH of any kind — no connection, no config parsing, no `Tag` handling, no
  target discovery, no remote execution.
* Any notion of a target beyond a `String` compared against one constant.
  There is no target type, no trait, no registry, no dispatch, and no way to
  learn what targets exist — the model must already know that `"local"` is the
  answer.
* Every capability in the eventual list: CPU, memory, disk/storage, processes,
  listening ports, systemd, Docker/containers, logs. `system_info` is the only
  tool.
* Mutations of any kind.
* Operator-facing logging beyond a single `eprintln!`.
* Configuration of any kind — no config file, no CLI flags, no env vars.
* Rust tests. `scripts/smoke.sh` is a black-box protocol check, not a test
  suite — there is no `#[test]` anywhere and no integration test using rmcp's
  client side. Fine while there is one tool; revisit when there are several.

## Open questions

Unresolved; do not assume an answer has been chosen.

1. **Interactive SSH authentication through a stdio MCP server.** If `ssh nas`
   needs a passphrase, an agent confirmation, or a hardware-key touch, where
   does that prompt go? The MCP client owns the terminal, and `ops-mcp`'s
   stdio is the protocol channel. Options not yet evaluated: rely on a
   pre-warmed `ssh-agent`, `SSH_ASKPASS`, ControlMaster sockets the user opens
   out-of-band, or return a distinct "authentication required" result and let
   the user act. This is the biggest unknown in v0.2 and it is a design
   question, not an implementation detail. It is not grounds to reverse
   decision 6.

   Two leads, both from thinking about the stdio process lifecycle
   (2026-09-03). Neither is a solution; both are starting points.

   * **Authenticate once per session, not once per call.** A stdio MCP server
     is a child process the client spawns once and keeps alive until the
     session ends, so it can hold warm state across tool calls. A
     ControlMaster socket opened on first use would make a passphrase prompt
     or key touch a once-per-session event rather than a per-call one. That
     turns "prompt on every call" (unusable) into "get through auth once at
     first use" (tractable), and it is probably the shape the answer takes.
     Note the process is per client session, not a shared daemon — two
     concurrent sessions are two processes with no coordination between them,
     so anything cached here is per-session.
   * **MCP elicitation may be the missing channel.** MCP lets a server request
     structured input from the user *through the client*, and `rmcp` ships an
     `elicitation` feature flag. That is plausibly how "target nas needs a
     passphrase" reaches the user without `ops-mcp` ever handling a
     credential. Unverified in two respects: whether the clients we care about
     actually support elicitation, and — the harder problem — that `ssh`
     reads a passphrase from its controlling terminal, not stdin, so bridging
     elicited input into OpenSSH likely still needs `SSH_ASKPASS` or an agent.
     Investigate before assuming this works.

2. **Failure semantics for a well-formed request to an unreachable target.**
   MCP tool error vs. a successful result carrying a normalized error payload.
   Decision 9 constrains the *content*; it does not settle the *channel*.
   Still open — decision 15 settled only the *unknown alias* case, which is
   request validation rather than reachability.

   One data point for whenever this is decided: rmcp is not itself consistent
   here. A missing parameter comes back as a *successful* JSON-RPC response
   carrying `isError: true` in the tool result, while our explicit
   `invalid_params` comes back as a JSON-RPC error object. Both are legal MCP;
   worth picking one deliberately rather than inheriting the split.

3. **Where operator-facing logs go.** Decision 9 implies a real log, but a
   stdio subprocess has no obvious place to put one. Deferred until something
   actually needs it.

4. ~~**Tool naming and granularity.**~~ **Settled 2026-09-03** — see
   decision 15. Tools are named for the operation and take a required
   `target`: `system_info(target)`. Numbering of the remaining questions is
   left alone so existing references stay valid.

5. **How much of `/proc` is worth normalizing.** Raw kernel values are precise
   but need interpretation; normalized values are model-friendly but lossy.
   No general policy chosen yet.

## Working agreements for future sessions

* When you find yourself writing "we'll probably need this eventually" —
  document it here, do not build it.
* Do not introduce a target abstraction until a real remote implementation
  demands one (decision 11).
* Do not add an arbitrary-command tool. If an operation is missing, implement
  that operation (decision 4).
* Update this file when you change what exists. A stale current-state file is
  worse than none, because the next session will believe it.
* New tools follow decision 15: name the operation, take a required `target`,
  return the same shape regardless of how the data was collected.
