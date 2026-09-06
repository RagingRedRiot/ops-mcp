# stethoscope-mcp — Decision log

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

The MCP client spawns `stethoscope-mcp` as a subprocess and talks JSON-RPC over
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

`stethoscope-mcp` may run OS utilities internally, but *trusted code selects and
constructs the command*. This is the decision the entire project rests on: it
is what makes `stethoscope-mcp` a boundary rather than a transport. Adding a general
escape hatch would nullify decisions 8, 9 and 12 at the same time, and the
narrow tools would become pointless overhead around it.

If a needed operation is not implemented, the answer is to implement that
operation, not to add a general one.

### 5. Use the system OpenSSH client, not an in-process SSH library — accepted

When remote support arrives, `stethoscope-mcp` will invoke the user's OpenSSH client
rather than link an SSH implementation.

OpenSSH already correctly implements config resolution, host-key verification,
agent protocols, PKCS#11/FIDO tokens, `ProxyJump`, and multiplexing. An
embedded library means reimplementing that, diverging from the user's actual
SSH behavior, and taking on a credential-handling role we do not want.

### 6. Do not require passwordless SSH keys — accepted

A tempting shortcut is "just set up a passwordless key for stethoscope-mcp". Rejected:
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

### 8. `Tag stethoscope-mcp` is the explicit opt-in — accepted

Having SSH access to a host does not mean the model may inspect it. Only hosts
whose applicable config entry carries `Tag stethoscope-mcp` are eligible.

Opt-in, not opt-out: adding a host to `stethoscope-mcp`'s reach must be a deliberate
act by the user, and it lives in the same file they already edit. Reading the
whole config and exposing everything in it would silently hand the model the
user's entire infrastructure the first time they run the server.

### 9. Do not expose SSH usernames, addresses, or key paths to the model — accepted

The `stethoscope-mcp` process can see these; that does not make them model-facing
data. The model gets the target alias and what it can do. Errors are
normalized (`"authentication_failed"`), not raw SSH diagnostics.

Detail belongs in operator-facing logs. Rationale: model context is
copied, summarized, and persisted in places the user did not choose; the
information boundary should be drawn at the point of production, not left to
downstream handling.

### 10. No `stethoscope-mcp` target config file unless a concrete requirement appears — accepted

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
*target's* clock, not `stethoscope-mcp`'s, or it is a claim about a machine whose clock
was never consulted. `btime` is in `/proc/stat`, which `system_health` already
reads, so this costs no extra read and stays a pure file parse that will work
unchanged over SSH (decision 17). Verified exact against `date` on the
development machine. Second resolution, since `btime` is only recorded to the
second, and the timestamp is formatted without a fractional part rather than
implying precision it does not have.

A wrong clock on the target produces a wrong `collected_at`. That is
information about the target, not a defect, and the field documentation says
so — skew between hosts is itself an operational problem worth seeing.

**Rejected: having `stethoscope-mcp` compute the delta itself.** A "stall since your
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

### 22. Container tools read cgroups; runtime knowledge is confined to enumeration — accepted

`container_health` and `container_list` read cgroup v2 files. Health, limits
and pressure are universal across Docker, podman and Kubernetes.

Three reasons, in order of weight:

* **cgroups are the enforcement layer, not a reporting layer.** `docker run
  -m 512m --cpus=2`, a Kubernetes `resources.limits` block and `podman
  --memory` all do the same thing: write cgroup files. Reading them back
  therefore needs no runtime knowledge at all. Verified against a live
  container — `cpu.max`, `cpu.weight`, `memory.max`, `memory.high`,
  `memory.low`, `memory.min`, `memory.swap.max`, `pids.max` and `io.max` are
  all present and readable.
* **The health files are not container concepts.** Verified identical on
  cgroups that have nothing to do with containers — an ordinary systemd
  service, a user slice, `init.scope`. The runtime determines the *path* and
  nothing else.
* **PSI in cgroups is byte-identical to `/proc/pressure/*`.** `psi_avg` and
  `psi_total` parse it unmodified, which is what `proc.rs`'s module doc
  predicted before the use case existed.

**What is not universal: enumeration.** Path layout differs by runtime *and* by
cgroup driver.

| runtime | cgroup path | status |
|---|---|---|
| Docker, systemd driver | `system.slice/docker-<id>.scope` | tested |
| podman rootless | `user.slice/user-<uid>.slice/user@<uid>.service/user.slice/libpod-<id>.scope` | tested |
| podman rootful | `machine.slice/libpod-<id>.scope` | untested |
| Docker, cgroupfs driver | `/sys/fs/cgroup/docker/<id>` | untested |
| Kubernetes, systemd driver | `kubepods.slice/kubepods-<qos>.slice/kubepods-<qos>-pod<uid>.slice/cri-containerd-<id>.scope` | untested |

**Kubernetes will not be verified here, and that is a deliberate choice.**
Standing up a cluster purely to claim compatibility is more work than the claim
is worth, and an unverified claim is worse than an honest gap. The layout above
is a good-faith reading of the documented convention; it stays marked untested
until somebody runs it against a real cluster. The README invites exactly that.
Kubernetes also adds a structural level the others lack — a pod slice containing
several container scopes — so whether the tools report pods, containers or both
is **open** and should be settled by someone holding a real cluster rather than
guessed at here.

**What the podman test established**, against a rootless container on a host
already running Docker:

* The `libpod-` naming and the deep `user.slice` nesting are both real. The
  container sat five levels below `/sys/fs/cgroup`, which is why the discovery
  walk bounds at six rather than the four Docker alone would suggest.
* The io-delegation caveat below is measured, not inferred: `io.stat` and
  `io.max` were absent while `io.pressure` was present.
* **A prefix match alone is not enough.** podman gives each container a sibling
  `libpod-conmon-<id>.scope` for its monitor process, which strips to
  `conmon-<id>` and was reported as a second, non-existent container — a
  phantom in every podman listing, and invisible to any amount of Docker
  testing. `classify` now requires a hex ID after the prefix, which rejects that
  scope structurally rather than by special-casing its name, and so should
  reject comparable infrastructure cgroups from runtimes not yet seen.

Recorded limits, so a future session does not rediscover them:

* **File availability follows controller delegation, not runtime.** Under
  `user.slice`, where rootless podman lives, `io.stat` and `io.max` are absent
  because the `io` controller is not delegated — while `io.pressure` is present,
  because PSI is not controller-gated. Decision 18's rule 7 already covers this:
  the field is simply absent, which is an answer rather than a failure.
* **Container CPU pressure has a meaningful `full` line** where the host's is
  permanently zero. Verified on both. `container_health` therefore reuses
  `Pressure` for CPU where `system_health` uses `CpuPressure` — the type split
  made in decision 18 turns out to be exactly the one the container case needs.
* **`cpu.stat` is cumulative `usage_usec`, not jiffies**, so
  `busy_fraction_since_boot` has no direct analogue.
* **There is no per-cgroup load average.** `/proc/loadavg` is host-wide; that
  field has no container version and should not be invented.
* **Limits are string sentinels.** `memory.max` reads `max` and `cpu.max` reads
  `max 100000` when unset. Normalizing that is decision 18's first rule applied
  again — and unset is the common case, not the edge case.
* **Throttling and OOM counters are the highest-value fields and are inert
  without limits.** `nr_throttled` requires a CPU quota to move; `memory.events`
  requires a memory limit. On a host that sets no limits the tool degrades to
  memory consumption and pressure, which is still useful but not diagnostic.

### 23. Identity is `comm`; process arguments and environment are never read — accepted

A container's identity is the set of distinct `/proc/<pid>/comm` values in its
cgroup. `stethoscope-mcp` does not read `/proc/<pid>/cmdline`, and does not read
`/proc/<pid>/environ`.

**The prohibition is server-wide, not container-scoped.** It is recorded here
because containers are where the question first arose, but `/proc/<pid>` is the
same interface for every process on the machine — a container process is a host
process in a namespace, and nothing about the namespace restricts what a reader
on the host can see. No current tool reads per-process data at all, so the whole
value of this decision is the constraint it places on future ones: a
`process_list`-shaped tool answering "what is using the CPU" is exactly where a
later session would reach for `cmdline` or `environ` as convenient identity.
Measured on a real host, credential-shaped environment variables appeared in
both container and non-container processes; the container ones held actual
database passwords and API keys while the host ones happened to hold only
socket and directory paths, but that is a fact about how that machine was
configured and not a property to rely on. A systemd unit with an
`Environment=` line puts a live secret in a host process immediately.

* **Command lines carry credentials in practice, not in theory.** A scan of a
  running container host found credential-shaped arguments on nine of its 156
  processes, including a literal `--password` flag on a process inside a
  container. Returning command lines would have moved a live credential into
  model context on the first call.
* **`argv[0]` is not a safe subset.** Processes rewrite it: Redis publishes
  `redis-server *:6379` and PostgreSQL `postgres: io worker 0`. Anything a
  process can write there, it can write a secret into.
* **`comm` is structurally incapable of carrying a command line** — fifteen
  characters, kernel-managed. It also proved *better* identity in practice,
  reporting `uvicorn` where `argv[0]` reported `python3.12`.
* **`environ` is prohibited outright, not merely unused.** It is strictly worse
  than a command line, being where database passwords and API keys actually
  live. Writing it down as a prohibition stops a future tool reaching for it as
  a convenient identity source.

**This generalizes decision 9.** That decision refuses SSH usernames, addresses
and key paths. The rule underneath it is: `stethoscope-mcp` returns *operational
measurements*, never *process arguments or environment*. Stated that way it
covers the cases nobody has enumerated yet.

**Rejected: redacting known-sensitive flags.** A blocklist has to be complete to
be safe and cannot be. `-p` means password to `mysql` and port to half a dozen
other tools, and every new application invents its own spelling.

### 24. Container name resolution is deferred to a separate tool — accepted

`container_list` and `container_health` identify a container by cgroup path and
`comm` set. Mapping that to a runtime-assigned name and image is a future tool,
called only for containers already identified as worth investigating.

* **It quarantines the only fragile part.** Name resolution is runtime-specific,
  format-unstable and privilege-requiring: Docker keeps it in an undocumented
  `config.v2.json`, podman in a BoltDB, and Kubernetes exposes only a pod UID in
  the cgroup path with the human name behind the kubelet or API server.
  Isolating it means the health tools never break when any of those change.
* **Lazy resolution scales with faults, not inventory.** Resolving every name on
  a large node is work proportional to the node; resolving the ones that need
  attention is work proportional to the problem.
* **Nothing about health needs a name.** Contention, throttling and OOM counts
  are facts about a cgroup, and `comm` already answers "which one is the
  database".

Recorded limits:

* **Container IDs are ephemeral.** An ID is stable while the container lives and
  gone afterwards, so a name resolved in a later call may name something that no
  longer exists — especially under an orchestrator that replaces workloads. The
  schema must say so, the way `collected_at` says not to use it for interval
  arithmetic.
* **No verdict fields.** "Needs attention" is the model's conclusion drawn from
  `throttled_usec`, `oom_kill` and pressure — not a boolean in the payload.
  Decision 18 continues to hold here.

**Rejected: resolving names eagerly in `container_list`.** It couples the
universal tool to the fragile one and pays the cost on every call, for
information that is only wanted about the exceptions.

### 25. The read guard is an allowlist at the single read chokepoint — accepted

`src/guard.rs` decides which paths this server may open. `proc::read` and
`proc::read_optional` ask it first, and nothing else in the crate opens a file.
Decision 23 said what must never be read; this is the mechanism that makes it so
rather than a promise that it is.

**An allowlist, not a denylist.** A denylist has to enumerate every dangerous
file forever, and it loses outright to renaming: a symlink at `/tmp/x` pointing
at `/proc/1/environ` has the basename `x` and passes any check on basenames. An
allowlist rejects `/tmp/x` because it is not a shape this server reads. The
first draft of this guard demonstrated the failure mode on itself — a basename
denylist containing `io`, to catch `/proc/<pid>/io`, also blocked the legitimate
`/proc/pressure/io`, and its own tests caught it. A narrow `FORBIDDEN` list
remains underneath the allowlist as a backstop against careless widening.

**Lexical normalization, not `canonicalize`.** `std::fs::canonicalize` resolves
symlinks and `..` correctly, but requires the file to exist — verified — which
would turn every legitimately absent optional file into a resolution failure
instead of the "missing" answer decision 18 rule 7 depends on. Rejecting empty,
`.` and `..` components is sufficient, because the server's paths never contain
them, so their appearance is either a bug or an attempt to get around the
function. That leaves symlinks unresolved and does not need to resolve them: a
symlink can only reach a forbidden file by way of a path that is not in the
allowlist.

**Under `/proc/<pid>/`, only `comm`.** `status`, `stat` and `limits` are denied
by omission — harmless, but unused, and the narrower rule is the one worth
keeping.

**Four layers, because a denial is always a programming error.** No path is
model-controlled, so the compile-time and CI layers matter as much as the
runtime check:

| layer | catches |
|---|---|
| runtime allowlist | a read of any non-approved path, including future dynamic cgroup paths |
| backstop denylist | a careless widening of the allowlist |
| guard unit tests | regressions in the guard itself, including traversal and renaming evasions |
| source-scan test | a forbidden path named as a literal in any other module |

**CI flags edits to the guard.** `detect-guard-modifications` raises a PR notice
whenever existing guard code or guard tests are edited or removed, adapted from
a pattern in a sibling project. Pure additions do not trigger it, so adding
tests stays frictionless. Verified against three simulated pull requests: adding
a test posted no notice, deleting a denial assertion was flagged, and widening
the allowlist was flagged.

**Defense in depth, demonstrated rather than claimed.** Adding `/proc/1/environ`
to the allowlist leaves it denied at runtime by the backstop, *and* fails the
guard's own tests because they detect the contradiction, *and* raises the CI
notice on the diff. Three independent layers have to be defeated together.

Recorded limits:

* **This guards programmer error, not an attacker — for now.** Every path in the
  crate is a string literal today. That changes with container enumeration,
  where cgroup directory listings produce the first constructed paths and
  `cgroup.procs` produces the first paths built from file *contents*. The guard
  is deliberately landed before that work rather than after it.
* **Tests are inline `#[cfg(test)]`, not in `tests/`.** The crate is a binary
  with no lib target, so integration tests cannot reach a private function, and
  exposing the guard publicly to enable them would be a worse trade. Verified
  that `#[cfg(test)]` costs nothing at runtime: release binaries built with and
  without a test module are byte-identical.

**Rejected: a `SafePath` newtype with a private constructor.** More machinery
than two chokepoint functions justify. Worth revisiting if the read surface
grows beyond them.

### 26. A container is addressed by ID alongside its target — accepted

`container_list(target)` returns the containers on a machine: an ID, the runtime
that created each, and the process names running inside it.
`container_health(target, container)` returns one container's contention,
configured limits and uptime.

**Identity belongs in the listing, not only in the health tool.** Withholding it
sounds tidier — the listing enumerates, the health tool describes — but it
forces a model wanting one specific container to call `container_health` on
every candidate until it finds the right one. Measured on a host of thirteen
containers, that is thirteen calls returning full metrics against a single
listing of about 1,700 bytes, and thirteen SSH round trips instead of two once
the target is remote. The extra reads cost 54 file reads and a few hundred
microseconds; the names cost roughly 40 bytes per container.

**A second parameter, not a second kind of target.** Decision 7 makes the user's
SSH config the target inventory, and containers are not in it. A container is a
thing *on* a target rather than a target of its own, so `target` keeps meaning
"machine" and remote plus container stays a pair of coordinates instead of a
flattened namespace.

**Limits are reported by presence.** An absent `limit_bytes` or `limit_cpus`
means no limit is configured — the fact worth surfacing, since an unlimited
container can consume the whole machine. A limit that exists but is too low
shows up instead as `nr_throttled` climbing or `oom_kills` non-zero, so both
failure modes are visible without the payload judging either. Verified against a
host whose twelve containers were first entirely unlimited and then limited.

**Enumeration needed a new capability.** Discovery walks the cgroup tree, which
is the first time this server builds a path it did not have as a literal — the
case decision 25 anticipated. Listing is guarded separately from reading and
confined to the cgroup hierarchy, and discovering a directory does not make its
files readable: every file the walk turns up still goes through the read
allowlist.

Recorded limits:

* **`pids.max` is usually a large kernel default rather than a deliberate
  limit**, so a high value there means unset rather than generous.
* **Container CPU `full` pressure is real**, unlike the host's. Measured at 78%
  on a quota-throttled container while the host's own `full` figure stayed at a
  permanent zero — which is why `container_health` reports a full `Pressure` for
  CPU where `system_health` reports only `some`.
* **Identity is the distinct `comm` set** in the cgroup, per decision 23. It
  says what a container is running — `postgres`, `uvicorn`, `redis-server` — not
  which one it is, and repeats across containers by design.

### 27. Container uptime comes from the cgroup directory's timestamp — accepted

`container_health` reports `uptime_seconds`, derived from the modification time
of the container's cgroup directory subtracted from the target's own clock.

**It exists because the cumulative figures were unreadable without it.** Every
`*_total_us` in the payload is monotonic since the cgroup was created, and a
model shown `full_total_us: 709537` cannot tell whether that is a rounding error
over four hours or a crisis over four minutes. The averages decay to zero after
a problem passes, so the totals are the only record that it happened — and a
record without a denominator is not evidence.

**The timestamp means the current run, not the container's age.** Verified on
both Docker and podman: stopping a container *deletes* its cgroup directory, and
starting it again recreates it with the same container ID and a fresh timestamp.
A stopped container therefore has no cgroup at all, which is also why
`container_list` never reports one — correct for a health tool.

**The denominator and the numerators share an origin, which is what makes the
ratio honest.** The PSI totals and `cpu.stat` live inside that same directory
and reset when it is recreated — measured on both runtimes, where a restarted
container came back with totals at zero. So `full_total_us` over
`uptime_seconds` always covers exactly the same span, with no skew to correct
for.

**Derived from the target's clock, not this server's.** `proc::target_now` is
boot time plus uptime, differenced against a file timestamp from the same
machine. Using the local clock would be wrong the moment the reads happen over
SSH, for the reason decision 19 already gives.

**Guarded by `check_dir` rather than a rule of its own.** Reading a directory's
metadata reveals strictly less than enumerating it, and the server may already
enumerate every path this can reach. A third guard rule would have admitted
exactly the same set while implying the two capabilities could diverge.

Verified against fifteen containers across two hosts and both runtimes: on the
Docker host every cgroup timestamp matched the runtime's own `StartedAt` to the
second, and a freshly created rootless podman container reported three seconds.
