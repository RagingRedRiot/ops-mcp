# ops-mcp — Current state

> **Read this first.** It answers: what are we building, what has actually been
> implemented, and what only *looks* implemented because it is described in
> `architecture.md`?
>
> Last updated: 2026-09-04 (`system_health` added; `/proc` and `/sys` adopted
> as the collection substrate; open question 5 settled; single-file
> implementation split into modules; `collected_at` added).
> Keep this file honest. If you implement something, update it here.

## What we are building

A local MCP server giving an AI assistant structured, read-only observability
into the user's Linux machine — and eventually remote machines via the user's
own OpenSSH — through narrow, named capabilities rather than shell access.
Full rationale in [`architecture.md`](architecture.md); the decisions and
their reasoning are in [`decisions.md`](decisions.md).

## Current milestone

**v0.1 — local, read-only.** Two tools exist.

## What actually exists

Everything, in full:

* A Rust binary crate (edition 2024) using `rmcp` 3.2.0.
* Four source files, ~580 lines including their documentation:
  * `src/main.rs` (~115 lines) — MCP **stdio** server plus the complete
    model-facing surface. Every tool is declared here and every body does only
    two things: validate the target and delegate. Kept that way on purpose, so
    decision 4's claim stays checkable by reading one short file. Also holds
    `TargetParams` (one shared parameter type, so both tools generate the same
    input schema) and `check_target`, which is the entire target mechanism —
    a string compared against one constant, deliberately not a type, trait or
    registry (decision 11).
  * `src/proc.rs` (~124 lines) — reading kernel virtual files and parsing the
    formats found there: `read`/`read_optional`, `parse_error`, `field`,
    `count_cpus`, `meminfo_bytes`, `psi_avg`/`psi_total`. It knows how the
    files are shaped and nothing about what any tool says about them, which is
    what will let the eventual per-container tool reuse the PSI parser —
    `/sys/fs/cgroup/*.pressure` is the same format — without inheriting
    `system_health`'s response shape. Reads go through `tokio::fs` so they land
    on the blocking pool (decision 16), and log the real `io::Error` to stderr
    while returning a flat message to the model (decision 9).
  * `src/system_info.rs` (~34 lines) — the `SystemInfo` shape and its
    collector: `target`, `hostname`, `kernel_release`, `os`, `arch`, from
    `/proc/sys/kernel/{hostname,osrelease}` and `std::env::consts`.
  * `src/system_health.rs` (~330 lines, mostly field documentation) — the
    `SystemHealth` shape and its collector: `collected_at` (RFC 3339 UTC, from
    `/proc/stat`'s `btime` plus uptime, so it is the target's own clock —
    decision 19), uptime, load with the CPU count
    needed to read it, memory availability and swap, and pressure-stall figures
    for CPU, memory and I/O, from `/proc/{uptime,loadavg,stat,meminfo}` and
    `/proc/pressure/*`. Normalized per decision 18: bytes rather than meminfo's
    mislabelled KiB, derived arithmetic alongside the inputs it came from, no
    verdict fields, and the interpretation in `///` comments that `schemars`
    lifts into the output schema. `pressure` is `Option` and simply absent on a
    kernel without PSI.
* Both tools take a required `target`; anything other than `"local"` is
  rejected with `invalid_params` (decision 15).
* Four dependencies: `rmcp`, `tokio`, `serde`, and `chrono` — the last only
  to format `collected_at`, and already present transitively before it was
  declared (decision 19).
* `scripts/smoke.sh` — drives the server over stdio with real JSON-RPC and
  asserts eleven things about the replies. Needs only `jq`; no MCP client, no
  Node, no test framework.
* This documentation.

Verified by `scripts/smoke.sh`: the handshake, both tools advertised,
`target` required on every tool, `target="local"` returning a hostname,
`target="nas"` rejected as `invalid_params`, and for `system_health` a
positive CPU count, memory in bytes rather than KiB, a `collected_at` within
two minutes of the shell's own clock, load shipped with its denominator, and no
verdict field anywhere in the payload. Builds clean under
`cargo clippy --all-targets`.

Not covered by anything: the missing-PSI path. `pressure` has only ever been
exercised on a kernel that has it.

## What is deliberately NOT implemented

Described in `architecture.md`, **none of it exists in code**:

* SSH of any kind — no connection, no config parsing, no `Tag` handling, no
  target discovery, no remote execution.
* Any notion of a target beyond a `String` compared against one constant.
  There is no target type, no trait, no registry, no dispatch, and no way to
  learn what targets exist — the model must already know that `"local"` is the
  answer.
* Most of the eventual capability list: disk/storage, processes, listening
  ports, systemd, Docker/containers, logs. `system_health` covers CPU and
  memory *contention*; it does not enumerate anything. Nothing reads
  per-process data or `/sys/fs/cgroup` yet, so per-container pressure and
  throttling — the reason decision 17 names `/sys` at all — is designed for
  and unbuilt.
* Mutations of any kind.
* Operator-facing logging beyond a single `eprintln!`.
* Configuration of any kind — no config file, no CLI flags, no env vars.
* Rust tests. `scripts/smoke.sh` is a black-box protocol check, not a test
  suite — there is no `#[test]` anywhere and no integration test using rmcp's
  client side. The parsing helpers added with `system_health` are the first
  code here with logic worth unit-testing against fixture strings; that is the
  argument for finally adding `#[test]`, and it has not been acted on.

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

5. ~~**How much of `/proc` is worth normalizing.**~~ **Settled 2026-09-04** —
   see decision 18. The split is by *kind* of transformation: mechanical unit
   conversion and plain arithmetic go in the payload, judgment does not go in
   the payload at all. Denominators always travel with their numerators, there
   are no verdict fields, and interpretation lives in the JSON Schema
   descriptions rather than the response.

6. **What `/proc/<pid>` may put into model context.** Not load-bearing yet —
   nothing reads per-process data — but it needs an answer before anything
   does. Decision 9 says information available to the `ops-mcp` process is not
   automatically model-facing, and it was written about SSH usernames and key
   paths. `/proc/<pid>/cmdline` is a larger and far more accidental leak:
   command lines routinely carry `--password=`, tokens and connection strings,
   and `/proc/<pid>/environ` almost always does. A process listing would emit
   those on every call, for every process, without anyone having decided to.
   Candidate answers: never read `environ`; return `comm`/`argv[0]` only; or
   redact `cmdline`. Decide before writing a `processes` tool, not during.

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
