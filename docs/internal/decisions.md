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

---

## Session decisions (2026-09-04)

### 17. `/proc` and `/sys` are the primary local collection substrate — accepted

Local data comes from the kernel's virtual filesystems by preference, not from
running OS utilities and parsing their output.

Three reasons, in order of weight:

* **It reinforces decision 4 mechanically.** A `/proc` read constructs no
  command at all. There is no argument, no quoting, no `PATH` lookup — so
  there is nothing for a later change to accidentally let the model influence.
  Wrapping `ps` or `df` means a command string exists in the code, and a
  command string is a thing that can grow a parameter.
* **It is the same everywhere.** `df`, `ps`, `ss` and `free` differ in flags
  and output format across distributions, and busybox differs from all of
  them. `/proc/meminfo` does not. This is what makes decision 11 mostly fall
  out for free: the parser written against a local read works byte-identically
  against a remote `cat` of the same path, so local and remote normalization
  is not an engineering problem for anything `/proc` covers.
* **It needs no dependency.** `tokio::fs` is sufficient, which keeps
  decision 14 intact.

`/sys` — specifically `/sys/fs/cgroup` under cgroup v2 — is treated as the
same substrate: same properties, different mount point. It is where
per-container PSI, `cpu.stat` throttling counters and `memory.events` OOM
counts live, and it is the *only* place a starved or quota-throttled container
is visible. Host-level PSI is structurally blind to it, because CPU `full`
pressure is always zero at the system level and only becomes meaningful
per-cgroup.

Recorded limits, so a future session does not rediscover them mid-implementation:

* **No free-space data anywhere in `/proc`.** `disk_usage` needs `statfs(2)`.
  That is `architecture.md`'s own worked example, and it is the one early
  capability this decision does not serve.
* **`/proc/net/tcp` yields socket inodes, not PIDs.** Mapping a listening port
  to a process means scanning `/proc/*/fd`, which only succeeds for processes
  the user owns. A partial answer is the correct answer here; the response
  shape should be able to say "listener exists, owner unknown".
* **systemd unit state, journal logs, and Docker container names/images are
  not in `/proc`** in any usable form. They are D-Bus, the journal, and the
  Docker socket respectively.
* **Container cgroups identify containers by ID, never by name.** Everything
  about a container's resource behaviour is readable without Docker; the name
  is not.

Where a capability genuinely requires a different mechanism, that is a reason
to use that mechanism, not a reason to reach for a general command tool.
Decision 4 is unaffected.

### 18. Normalized values in the payload, interpretation in the schema, no verdicts — accepted

Settles open question 5 ("how much of `/proc` is worth normalizing"). The
answer is neither "raw" nor "cooked" but a split by *kind* of transformation:
mechanical conversion and arithmetic go in the payload, judgment does not go
in the payload at all.

1. **Normalize units at the boundary.** `meminfo`'s `kB` is really KiB and PSI
   totals are microseconds. Convert to bytes; name microsecond fields `_us`.
   Lossless and mechanical, so there is no argument for passing the footgun
   downstream where each consumer re-learns it and one of them gets it wrong.

2. **Never return a number without its denominator.** Load average without
   `cpu.count` is uninterpretable; `available_bytes` without `total_bytes` is
   uninterpretable. This is the rule to hold hardest, because breaking it
   produces confident wrong conclusions rather than visible errors.

3. **Compute derived values where the computation is arithmetic, not
   judgment.** `load_1m_per_cpu`, `available_percent`, `swap_used_bytes`,
   busy fraction. The model would otherwise derive these itself and can get
   them wrong — dividing load by the wrong core count is a realistic failure.
   Doing it in trusted code is deterministic, and nothing is lost because the
   inputs ship alongside.

4. **No verdicts in the payload.** No `"status": "critical"`, no
   `"health": "degraded"`. Thresholds are workload-dependent — a batch host at
   load 30 is working as designed, a latency-sensitive service at load 3 may
   not be — so a fixed threshold is wrong for half its uses. And a label is
   lossy in precisely the wrong direction: it discards the number needed to
   reason about the *specific* failure. Collecting is this server's job;
   judging is not.

5. **Interpretation belongs in the JSON Schema, not the response.** Tool and
   field descriptions are model-facing prose sent once at `tools/list` time,
   not per call. That is the right home for "MemFree is deliberately absent",
   "load counts I/O-blocked tasks as well as CPU-waiting ones", and
   "`full_avg10` above 10 means all runnable work was stalled". `schemars`
   lifts `///` doc comments into the schema, so the comment that documents a
   field for a human reviewer *is* the model's reading instruction — one text,
   both audiences, zero per-response cost.

6. **Omission is a design act, and a documented one.** `MemFree`,
   `Active`/`Inactive`, `DirectMap*` and `Vmalloc*` are left out: every field
   returned is a field the model will try to use, and `MemFree` in particular
   invites the most common misreading of `meminfo`. Note notable omissions in
   the schema so they read as deliberate rather than as a bug.

7. **A field whose source may not exist is optional, not an error.** PSI needs
   kernel 4.20+ and can be compiled out or disabled; an older remote host may
   lack it. The absence of one file must not fail the whole call. This is
   decision 11's normalization pressure arriving early, and it is far cheaper
   to build in now than to retrofit.

### 19. `system_health` carries `collected_at`, from the target's own clock — accepted

The response includes a wall-clock timestamp, RFC 3339 in UTC, derived from
`/proc/stat`'s `btime` plus `/proc/uptime`.

**Not for measuring intervals.** The obvious motivation — letting the model
difference two readings to turn the PSI `*_total_us` counters into rates — is
already served by `uptime_seconds`, and served better: it is monotonic, so it
is unaffected by NTP steps, DST, or a clock that is simply wrong. The field
documentation says so explicitly, because handing the model two ways to
compute elapsed time invites it to pick the one that breaks.

What the timestamp actually answers:

* **Staleness.** Model context is summarized, cached and persisted; a reading
  from forty minutes ago looks exactly like a fresh one unless it is dated.
* **Correlation across targets.** The v0.2 case, and the reason not to defer
  this: once `local` and `nas` readings sit side by side, "were these taken at
  the same moment?" is a real question and uptime cannot answer it — two
  machines have unrelated uptimes.
* **Correlation with external evidence**, such as a log line's timestamp.

**Boot time plus uptime, not `SystemTime::now()`.** The value must be the
*target's* clock, not `ops-mcp`'s, or it is a claim about a machine whose clock
was never consulted. `btime` is in `/proc/stat`, which `system_health` already
reads, so this costs no extra read and stays a pure file parse that will work
unchanged over SSH (decision 17). Verified exact against `date` on the
development machine. Second resolution, since `btime` is only recorded to the
second, and the timestamp is formatted without a fractional part rather than
implying precision it does not have.

A wrong clock on the target produces a wrong `collected_at`. That is
information about the target, not a defect, and the field documentation says
so — skew between hosts is itself an operational problem worth seeing.

**Rejected: having `ops-mcp` compute the delta itself.** A "stall since your
last call" field would require the server to remember the previous reading.
That makes it stateful, the state would be per-session so two clients would get
different answers to the same question, and it would silently choose a
measurement window the caller never asked for. Return the inputs; let the
caller difference two responses.

**`chrono` promoted to a direct dependency.** It was already in the tree
transitively, via both `rmcp` and `schemars`, so declaring it adds no build
time and no supply-chain surface — `Cargo.lock` gained one line and no new
crates. It is recorded here because decision 14 states the dependency set is
deliberate, and a fourth entry should not appear in `Cargo.toml` without a
reason written down. Declared `default-features = false`, matching the other
three: only formatting is needed, not `chrono`'s clock.

---

## Session decisions (2026-09-05)

### 20. The remote batch is plaintext and length-framed, never encoded — accepted

One SSH invocation returns every file a tool needs. The response is a sequence
of frames: a header line, then exactly the bytes it announces.

```
FILE /proc/meminfo 1614
MemTotal:        8192000 kB
...
MISSING /proc/pressure/cpu
```

Three reasons, in order of weight:

* **The command stays legible to whoever audits it.** A `base64` in a remote
  command is an exfiltration signature, and an operator reading `auditd` should
  see `cat /proc/meminfo`, not an encoder. The genuine indicator is an encoded
  *command* — `echo <blob> | base64 -d | sh` — and encoding only the output is
  benign by comparison, but telling the two apart requires reading the
  direction of a pipe, and a reviewer should not have to. This extends
  decision 4 one step: there is not only no command string the model can
  influence, there is nothing that *looks* like there might be.
* **A byte count is stronger than any delimiter.** A delimiter has to be a
  string the content cannot contain, which is an assumption about content a
  future kernel is free to break. A length prefix assumes nothing: the reader
  counts, it never scans. Verified against a hostile payload — a file whose
  content contains a literal `FILE /proc/evil 999` line frames and recovers
  intact, and the fake header never reaches the map.
* **It is smaller.** 5,670 bytes against 7,343 for the same seven files;
  base64 inflates by a third.

Measured against a remote host over a relayed link of roughly 47 ms
round-trip latency:

| | |
|---|---|
| seven separate round trips | 3,761–4,063 ms |
| one batched round trip | 530–556 ms (**7.2x**) |
| cold connect | ~750 ms |
| `ControlMaster` reuse | ~560 ms |

Batching is not an optimization here, it is the difference between a usable
tool and an unusable one — and note that connection pooling recovers far less
than batching does, so it is not a substitute.

**Decision 17 verified, not merely asserted.** Every parser in `proc.rs`, run
unmodified against bytes pulled from a remote host, produced values matching
that host's own tools exactly — memory and swap against `free -b`, CPU count
against `nproc`, uptime against `uptime -p`, and `collected_at` tracking
`date -u`. The claim that a local parser works byte-identically on a remote
`cat` is now measured.

Recorded limits, so a future session does not rediscover them:

* **Never build the frame through a pipe.** `base64 <"$f" | tr -d '\n'` and
  every piped variant return the *pipeline's* exit status, not the redirect's,
  so a missing file reads as a successful empty read — silently collapsing the
  required-versus-optional distinction `read_pressure` depends on. Guard with
  `[ -r "$f" ]` or use no pipe at all.
* **`/proc` files report size 0.** `tar` recovers zero bytes from them and any
  `stat`-based length scheme is dead on arrival. The count has to come from
  content that has already been read.
* **Count the variable, not the file.** `wc -c` applied to the file would read
  it a second time, sampling a different instant for anything volatile.
* **The frame does not preserve trailing whitespace.** `$(...)` strips trailing
  newlines, so a framed file is one byte shorter than a direct read. Harmless
  because `proc::read` already trims and the trimmed content is byte-identical
  (verified on non-volatile files), but a consumer needing exact trailing bytes
  would need a different frame.
* **Root is not required.** All seven files read cleanly as uid 65534, which
  keeps decision 8's least-privilege posture available.

**Rejected: `tar`.** Zero bytes, per the `/proc` size-0 limit above.

**Rejected: bare `cat a b c`.** Concatenation with no way to know where one
file ends.

### 21. Frames are self-labeling; retrieval order is not load-bearing — accepted

Every frame carries its own path, and `read_all` returns a map keyed by path
rather than a positional `Vec`. The caller asks for `/proc/meminfo` by name and
never does index arithmetic.

Retrieval order *is* in fact deterministic — a shell `for` loop returned an
identical sequence across five runs. This decision is not that order is
unreliable; it is that depending on it buys nothing and costs the failure modes
below.

* **Local and remote must not diverge in their contract.** This is the real
  argument. `futures::join_all` preserves input order in its output regardless
  of completion order, so a positional design works perfectly well locally,
  forever, and breaks the first time a remote host prints a banner. That is the
  worst available failure shape: an asymmetry local testing can never surface.
  Keying both implementations by path is decision 11 applied one level below
  the response — normalize the *retrieval* contract, not only the output.
* **`ssh host cmd` stdout is only as clean as the remote account's rc files.**
  It was verified clean on the test host, where `echo MARKER` returned exactly
  six bytes with no motd or banner, but that is a property of that host's
  configuration rather than of SSH. A path-keyed reader skips lines it does not
  recognize; a positional reader silently misattributes every file after the
  noise. Verified: with junk injected before and after the payload, all files
  were still recovered and correctly attributed.
* **It degrades gracefully.** A truncated response loses only the frames it
  lost, and every recovered file is still correct. A duplicate path collapses
  harmlessly instead of shifting everything after it.

What input order *does* control is the order files are read, and therefore
sampling adjacency: `/proc/stat` and `/proc/uptime` belong next to each other
because `collected_at` is `btime` plus uptime. Within one batch all seven land
inside a single remote loop of about a millisecond, against roughly four
seconds spread across seven round trips — so batching narrows skew as well as
latency.
