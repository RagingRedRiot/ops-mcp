# ops-mcp — Decision log

> Lightweight ADR log. The point is to stop a future session — or a future us —
> from casually reversing a decision without understanding why it was made.
>
> If you want to reverse one of these, that is allowed. Read the rationale
> first, then record the reversal here with its own reasoning. Do not silently
> drop one.

Status legend: **accepted** · **superseded** · **open**

---

### 1. Implement in Rust — accepted

This is a learning project and Rust is the thing being learned. Beyond that it
suits the job: a single static binary an MCP client can spawn, no runtime to
install, and a type system that makes "what exactly can this tool do" legible.

### 2. Local stdio MCP server first — accepted

The MCP client spawns `ops-mcp` as a subprocess and talks JSON-RPC over
stdin/stdout. No network listener, no ports, no HTTP server, no auth layer.
The process runs as the invoking user and inherits their permissions.

An HTTP transport is explicitly *not* wanted: it would turn a
runs-as-you subprocess into a service with its own identity, lifetime, and
access-control problem.

Consequence: stdout is reserved for the protocol. Diagnostics go to stderr.

### 3. v0.1 is read-only — accepted

Inspection only. No mutations of any kind. Getting the read-only architecture
and its boundaries right is the whole point of v0.1; adding writes early would
mean designing the security model under pressure from a feature.

### 4. No arbitrary shell tool — accepted, load-bearing

No `execute_shell(command)`, no `execute_ssh_command(target, command)`, no
tool that accepts a command, command fragment, or shell string from the model.

`ops-mcp` may run OS utilities internally, but *trusted code selects and
constructs the command*. This is the decision the entire project rests on: it
is what makes `ops-mcp` a boundary rather than a transport. Adding a general
escape hatch would nullify decisions 8, 9 and 12 at the same time, and the
narrow tools would become pointless overhead around it.

If a needed operation is not implemented, the answer is to implement that
operation, not to add a general one.

### 5. Use the system OpenSSH client, not an in-process SSH library — accepted

When remote support arrives, `ops-mcp` will invoke the user's OpenSSH client
rather than link an SSH implementation.

OpenSSH already correctly implements config resolution, host-key verification,
agent protocols, PKCS#11/FIDO tokens, `ProxyJump`, and multiplexing. An
embedded library means reimplementing that, diverging from the user's actual
SSH behavior, and taking on a credential-handling role we do not want.

### 6. Do not require passwordless SSH keys — accepted

A tempting shortcut is "just set up a passwordless key for ops-mcp". Rejected:
it degrades the user's security posture to make our implementation easier, and
it creates exactly the standing unattended credential we are trying not to
have.

Whatever authentication the user's normal SSH requires — passphrase, agent,
hardware token, interactive prompt — the design should preserve. See the open
question in `current-state.md` about how interactive auth surfaces through a
stdio MCP server; it is a real problem and it is not a reason to reverse this.

### 7. The user's existing SSH config is the eventual remote inventory — accepted

Remote targets come from `~/.ssh/config`, which the user already maintains.
No separate host list to keep in sync, nothing to duplicate, nothing to drift.

### 8. `Tag ops-mcp` is the explicit opt-in — accepted

Having SSH access to a host does not mean the model may inspect it. Only hosts
whose applicable config entry carries `Tag ops-mcp` are eligible.

Opt-in, not opt-out: adding a host to `ops-mcp`'s reach must be a deliberate
act by the user, and it lives in the same file they already edit. Reading the
whole config and exposing everything in it would silently hand the model the
user's entire infrastructure the first time they run the server.

### 9. Do not expose SSH usernames, addresses, or key paths to the model — accepted

The `ops-mcp` process can see these; that does not make them model-facing
data. The model gets the target alias and what it can do. Errors are
normalized (`"authentication_failed"`), not raw SSH diagnostics.

Detail belongs in operator-facing logs. Rationale: model context is
copied, summarized, and persisted in places the user did not choose; the
information boundary should be drawn at the point of production, not left to
downstream handling.

### 10. No `ops-mcp` target config file unless a concrete requirement appears — accepted

We are not creating a config format on speculation. If a real requirement
emerges that SSH config genuinely cannot express, revisit *then*, with the
requirement in hand to shape the design.

### 11. Local and remote must eventually produce normalized results — accepted (intent, not code)

`disk_usage(target="local")` and `disk_usage(target="nas")` should return the
same shape; collection method is an implementation detail.

Explicitly **not** a license to build a target abstraction now. With one
implementation you cannot tell where the seam goes, and a trait invented ahead
of its second implementer usually ends up wrong. Introduce it when a real
remote implementation pushes on it.

### 12. Mutations deferred to a security-focused milestone — accepted

Restarting a container or service is v0.3 at the earliest, and it starts with
a design discussion — authorization, explicit allowlists, user confirmation,
audit trail, MCP safety semantics — not with an implementation.

The read-only architecture must be mature first. A mutation added
opportunistically to a read-only design is how this project would go wrong.

---

## Bootstrap-session decisions (2026-09-03)

### 13. `rmcp` 3.2.0 as the MCP SDK — accepted

The official Rust MCP SDK (`modelcontextprotocol/rust-sdk`), verified current
at bootstrap time. 3.x is a major break from 2.x, so **treat pre-3.x examples
found online as stale** — the breaks are concentrated in HTTP/OAuth/tasks, but
the API surface moved.

Two things verified against the 3.2.0 source, both easy to get wrong:

* `Implementation::from_build_env()` resolves `CARGO_CRATE_NAME` inside *rmcp*,
  so a server using it reports its name as `rmcp`. We set identity explicitly.
* `#[tool_handler]` defaults to a router expression of `Self::tool_router()`,
  so it never reads a stored `tool_router` field. The struct is therefore a
  stateless unit struct; the usual `tool_router` field + constructor is dead
  weight unless you opt in with `#[tool_handler(router = self.tool_router)]`.

### 14. Minimal dependency set — accepted

`rmcp` (no default features), `tokio` (no default features), `serde`. That is
all. Notably absent and deliberately so:

* `sysinfo` / `procfs` — `local_system_info` reads two `/proc` files with
  `std::fs`. Add a crate when the data actually needs one, not before.
* `tracing` / `tracing-subscriber` — decision 9 implies operator-facing logs
  eventually, but a single `eprintln!` covers today's needs. See the open
  question in `current-state.md`.
* `anyhow` / `thiserror` — three error sites do not justify an error crate.

### 16. Blocking work goes to the blocking pool, not an async worker — accepted

Tools are `async` and do their I/O through `tokio` (`tokio::fs` today), even
where the underlying operation is not genuinely asynchronous.

`tokio::fs` is not async file I/O — it wraps the ordinary blocking `std::fs`
call in `spawn_blocking`. That is precisely what we want: rmcp `tokio::spawn`s
each incoming request onto the runtime, so a tool that blocks inline occupies
an async worker thread and stalls every request running beside it. Routing the
wait to the blocking pool keeps the workers free.

For `/proc` this is close to pure ceremony — those reads are kernel-generated
and return in microseconds, and the `spawn_blocking` hop costs more than the
wait it avoids. It is adopted as a *policy* rather than an optimization,
because the cost of applying it uniformly is negligible and the cost of
forgetting it exactly once is a frozen server.

The case that will genuinely need it is v0.2: an `ssh` subprocess can hang for
30 seconds on a dead host, or indefinitely on a passphrase prompt. Use
`tokio::process` there — note that `tokio::fs` would not have helped with a
subprocess at all.

Cost: tokio's `fs` feature, which is the only change to the dependency set in
decision 14.

### 15. Tools are named for the operation and take a `target` — accepted

Settles open question 4. The tool is `system_info(target)`, not
`local_system_info` / `remote_system_info`. Decision 11 says local and remote
must eventually return the same shape; a tool named for its transport
contradicts that before the second implementation even exists. Settled now
because renaming tools later breaks anything referencing them, and this is the
cheapest moment — there is one tool.

Three sub-decisions, all reversible but worth stating:

* **`target` is required, not optional-defaulting-to-`"local"`.** Ops tooling
  aimed at the wrong machine is a real hazard, so the model should always say
  which machine it means. It also keeps the signature stable across v0.2:
  making a required parameter optional later (or the reverse) silently changes
  how the model calls it.
* **`target` is a `String`, not an enum.** Remote targets will be
  runtime-discovered SSH aliases, not compile-time variants, so `String` is
  the correct long-term shape. Validation is a runtime comparison against
  `LOCAL_TARGET`. Note this is *not* a target abstraction — decision 11 still
  holds; there is no target type, trait, or registry.
* **An unknown alias is `invalid_params`.** This does not settle open
  question 2, which is about a *known but unreachable* target. Unknown-alias
  is request validation; reachability is a remote concern that does not exist
  yet.
