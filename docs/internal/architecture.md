# ops-mcp — Architecture

> Internal working document. This is design memory for the humans and Claude
> sessions working on this project, not public project documentation.

## Purpose

`ops-mcp` is a local MCP server that gives an AI assistant structured,
constrained observability into the user's machines — the local Linux machine
first, and eventually remote Linux machines reachable over SSH.

The motivating use case:

> "Why isn't this service/container/application working?"

Today answering that means the user repeatedly explains their environment and
hand-runs commands on the model's behalf, pasting output back. `ops-mcp` lets
the assistant gather that operational information directly, through narrowly
defined tools that return normalized, structured results.

## The central principle

> **Give the model capabilities, not credentials or arbitrary machine access.**

`ops-mcp` is a security boundary between the model and the operating
environment. The model asks for a *named operation on a named target*:

```text
disk_usage(target = "nas")
```

It never receives a generic primitive:

```text
execute_shell(command)          # rejected
execute_ssh_command(target, cmd) # rejected
```

`ops-mcp` may internally shell out to OS utilities or OpenSSH. The distinction
that matters is **who chooses the command**: trusted `ops-mcp` code constructs
it, never the model.

### Why arbitrary shell is explicitly rejected

An `execute_shell` tool would collapse every boundary below into one: the
model would hold the user's full OS and SSH authority, gated only by the
model's own judgment and whatever the MCP client asks the user to approve.
It also produces unstructured text the model has to parse and guess at.

Refusing it is what makes the rest of the design meaningful. A narrow tool is
auditable: you can read `ops-mcp`'s source and enumerate exactly what it can
do. That property is worth more than the convenience of a general escape
hatch, and it is lost permanently the moment one is added.

## Local execution model (v0.1)

```text
Claude / MCP client
        |
       MCP (stdio)
        |
        v
     ops-mcp
        |
        v
 local operating system
```

Launched by a local MCP client, `ops-mcp` runs as the OS user who launched it,
and inherits exactly that user's permissions. This is intentional. v0.1
deliberately does **not** introduce:

* root privileges
* a dedicated `ops-mcp` system account
* a daemon or privileged service
* containers
* any additional credential storage

If the user cannot read something, neither can `ops-mcp`, and that is the
correct behavior — not a limitation to engineer around.

v0.1 is local, read-only inspection only. No mutations. No SSH.

### Practical consequence: stdout is the protocol

On a stdio MCP server, stdout belongs to the JSON-RPC transport. Nothing may
print to it. Diagnostics go to stderr (and eventually to an operator-facing
log). This is a real footgun; see the note at the top of `src/main.rs`.

## Remote architecture (v0.2, not built)

```text
Claude
   |
   | MCP
   v
ops-mcp
   |
   | constrained operation
   v
system OpenSSH client
   |
   | user's existing SSH configuration/authentication
   v
remote host
```

### OpenSSH delegation

`ops-mcp` delegates SSH connectivity and authentication to the system OpenSSH
client. It does **not** become an SSH credential manager, and does not store
SSH passwords, private-key passphrases, or equivalent secrets.

The user keeps managing SSH the normal way: `~/.ssh/config`, `~/.ssh/known_hosts`,
encrypted private keys, `ssh-agent`, hardware-backed keys, askpass/interactive
authentication, `ProxyJump`, `Include`, and so on.

We specifically do **not** require passwordless SSH keys. If a user's normal
SSH setup demands a passphrase, an agent, a touch on a hardware key, or an
interactive prompt, the design should preserve that behavior rather than
pressure the user into weakening it. (How interactive auth surfaces through an
MCP server is an open question — see `current-state.md`.)

The division of responsibility:

> **OpenSSH owns connection and authentication.**
> **`ops-mcp` owns model-facing authorization and capabilities.**

### SSH config as target inventory

There is deliberately no `ops-mcp` target configuration file. The user's
existing OpenSSH configuration is the eventual canonical remote inventory —
it already exists, the user already maintains it, and duplicating it would
create a second source of truth that drifts.

But **SSH access to a host does not imply the model may inspect it.** Opt-in
is explicit, via an OpenSSH `Tag`:

```sshconfig
Host nas
    HostName 192.168.1.50
    User some-user
    IdentityFile ~/.ssh/some-key
    Tag ops-mcp
```

If discovery is implemented, it stays deliberately narrow: find literal `Host`
aliases whose applicable entry is tagged `ops-mcp`, and leave every other bit
of SSH semantics to OpenSSH. We are not writing an SSH configuration
implementation — OpenSSH already owns those semantics, including wildcards,
match blocks, and precedence rules. Do not prematurely solve them.

`Include` may eventually matter for discovery. It is not a bootstrap concern.

## The model-visible information boundary

A target may internally resolve to:

```text
alias:     nas
hostname:  192.168.1.50
username:  example-user
identity:  ~/.ssh/example-key
jump host: example-bastion
```

The model needs to see only:

```json
{
  "target": "nas",
  "platform": "linux",
  "capabilities": ["system", "storage", "processes", "containers"]
}
```

Usernames, addresses, ports, key paths, and ProxyJump topology stay
implementation-private. OpenSSH resolves them when `ops-mcp` eventually
invokes the equivalent of `ssh nas <trusted operation>`.

Errors respect the same boundary. Prefer:

```json
{ "target": "nas", "error": "authentication_failed" }
```

over an SSH diagnostic dump containing the username, address, and key path.
Detailed diagnostics belong in local logs intended for the human operator.

The general rule:

> **Information available to the `ops-mcp` process is not automatically
> information that should enter model context.**

This already applies locally: `read_proc` in `src/main.rs` logs the underlying
`io::Error` to stderr and returns a flat message to the model.

## Authorization model

The effective permission model is a conjunction — defense in depth:

```text
  User's OS permissions
    AND user's SSH permissions
    AND target explicitly opted into ops-mcp
    AND operation explicitly implemented/allowed by ops-mcp
  = capability available to the AI
```

Possession of SSH access alone does not imply AI authorization. Opting a
machine into `ops-mcp` does not imply arbitrary command execution on it.

## Local/remote normalization

Local and remote targets should eventually expose the same model-facing
operations returning the same response shape:

```text
disk_usage(target = "local")
disk_usage(target = "nas")
```

```text
                   Target
                     |
             +-------+-------+
             |               |
          Local            Remote
                             |
                          OpenSSH
```

How the information was collected is an implementation detail.

Tools are therefore named for the *operation*, not the transport:
`system_info(target)`, never `local_system_info` (decision 15). The parameter
is required — an ops tool pointed at the wrong machine is a real hazard, so
the model always states which machine it means.

**This is a design intention, not a code structure.** A Rust trait for targets
will probably make sense eventually. It does not exist and should not be
created until a second implementation actually pushes on it — one
implementation cannot tell you where the seam belongs. Today `target` is a
`String` compared against one constant; that is the whole mechanism, and it is
deliberately not a target type, trait, or registry.

## Milestone boundaries

| Milestone | Scope | Status |
|---|---|---|
| v0.1 | Local, read-only inspection | in progress — one tool exists |
| v0.2 | Remote targets via user's OpenSSH; `Tag ops-mcp` discovery | not started |
| v0.3 | Narrowly scoped mutations, after an explicit security design discussion | not started, deliberately |

v0.3 (restart an allowlisted container or service) must not begin as a
casual feature addition. It requires deciding authorization, confirmation,
auditing, allowlisting, and MCP safety semantics first.
