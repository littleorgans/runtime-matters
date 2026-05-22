# Project

`runtime-matters` exists to give littleorgans one honest answer to the question
"what is actually running on this host, and is it still alive". Everything in
the repo is shaped around that question.

The working assumption is that local agent processes are unreliable to
observe through their own stdout. A coding agent can wedge, crash between
polls, or be killed by an unrelated tmux command. Callers above this layer
need a substrate that records evidence rather than guesses.

## System Model

A spawn request names a runtime, a target, an isolation policy, and a session
id. The daemon validates the request, the launcher resolves the runtime
command, the platform layer starts a shim, and the shim wraps the actual
agent process. The shim reports ready, holds the child, and is the source of
the exit signal the daemon trusts.

The daemon writes lifecycle transitions into a durable event log. `Running`
is appended after shim ready is stored. `Terminated` or `Lost` is appended
when exit evidence or loss evidence is observed. The log is append only and
cursor addressable, so any client can resume from the last cursor it saw
without coordination.

Spawn targets are explicit. Host execution is the default. Docker isolation
is a backend selected per spawn. The runtime kind does not change when
isolation changes. Launchers still choose the command. The backend chooses
where that command runs.

## Main Workflows

`rtm daemon start` brings up `rtmd` on the local Unix socket and prepares the
event log, status store, and watcher pool.

`rtm doctor` runs the local integrity checks. It reports socket readiness,
configured paths, runtime launcher availability, Docker CLI readiness, Docker
daemon readiness, manifest validation capability, and tmux capability.

`rtm spawn` is the canonical control surface. The caller passes a session id,
a runtime, a target, and an optional isolation policy. The daemon owns
lifecycle bookkeeping from that point until exit or loss.

`rtm status` returns Lifecycle rows for the requested session ids or filters.
It is the authoritative reconciliation view when an events cursor falls
behind the retained log floor.

`rtm events` returns durable lifecycle events. Clients pass `--since CURSOR`
to resume after the last cursor they saw. An expired cursor returns a typed
expiry response and the latest cursor, so the client can switch to status
reconciliation without ambiguity.

`rtm kill` and the admin MCP tools provide explicit operator escape hatches
for processes that need to be ended out of band.

## Repository Boundaries

The workspace is intentionally narrow. Each crate owns one responsibility and
is sized for that responsibility.

`rtm-core` owns the public protocol: `RuntimeRpc`,
`RuntimeResponse`, the JSON line wire contract, the lifecycle and event
types, and the cursor representation. It is published to crates.io because
external callers, including session-matters, compile against it.

`rtm-client` owns the public client shell: connection,
framing, retries, and typed RPC helpers over the rtmd Unix socket. It is the
only sanctioned way to talk to rtmd from outside this workspace.

`rtm-paths` owns the path and endpoint policy. Filesystem locations are
modeled as paths. Daemon connection targets are modeled as endpoints. The
crate exists so callers do not treat future transport targets as filesystem
paths. `RTM_SOCKET_PATH`, `RTM_DB_PATH`, `RTM_HOME`, and `RTM_SHIM_PATH` are
resolved here.

`rtm-platform` owns the host facing pieces that vary by operating system:
process exit watchers, tmux pane interaction, signal delivery, and the host
side of the shim handshake. macOS and Linux differences live here so the
daemon stays portable.

`rtm-launchers` owns the per runtime command resolution. The Claude launcher
knows how to start the Claude runtime; the Codex launcher knows how to start
the Codex runtime. Launchers build commands and hold no policy.

`rtm-store` owns the durable state: the SQLite database, the event log
schema, migrations, lifecycle reads and writes, and cursor allocation.

`rtm-daemon` owns the daemon process: socket listener, request dispatch,
lifecycle bookkeeping, watcher coordination, isolation backends, and the
durable event append path. `rtmd` is a binary inside this crate.

`rtm-cli` owns the user facing CLI and the embedded MCP bridge. The `rtm`
binary lives here. The CLI never reaches into the store or watchers; it
talks to rtmd through `rtm-client`.

The boundary that matters most is between rtm-core and everything else.
External integrators compile against the public protocol; internal crates
may evolve freely as long as the wire contract holds.

## Endpoints And Paths

Daemon endpoints are not filesystem paths even when the current transport is
a Unix socket. The `rtm-paths` distinction is deliberate, because future
transports may be different and callers that treat the endpoint as a file
will break in subtle ways.

`RTM_SOCKET_PATH` is authoritative when set. Without it, Linux prefers
`$XDG_RUNTIME_DIR/rtm/sock` when available, then falls back to
`~/.rtm/sock`. macOS defaults to `~/.rtm/sock`.

`RTM_DB_PATH` selects the SQLite database location. `RTM_HOME` selects the
runtime home root, including logs and event log on disk state.
`RTM_SHIM_PATH` selects the shim binary used for bootstrap.

## Isolation Backends

Host execution is the default backend. The shim is launched as a child of
rtmd and owns the runtime process directly.

Docker isolation is the alternate backend. Spawns take two shapes. A
headless Docker spawn runs as a foreground `docker run` process owned by
the host shim. A tmux Docker spawn starts a detached container and
attaches the existing host tmux pane to it. Closing the pane ends the
attach path; the container stays managed by the Docker backend until
runtime exit or an explicit kill.

`/workspace` is the canonical workspace path inside the container. The
backend bind mounts the spawn cwd at `/workspace` and sets it as the
container workdir.

### Image Contract

`rtm` is image agnostic. The image is chosen by the operator or the spawn
caller. A practical starter base is
`mcr.microsoft.com/devcontainers/base:ubuntu` because it already fits
interactive coding agent expectations. Distroless and Alpine or musl images
are discouraged starters; they commonly lack the shell, libc, and
troubleshooting surface expected by interactive coding agents.

Images used with `rtm` must satisfy this contract:

- Base: any Debian or Ubuntu compatible base is acceptable.
- User: declare a non-root `USER`. Root requires an explicit operator opt
  in. The daemon enforces non-root image metadata by default and rejects
  missing root metadata as root.
- Manifest: on arm64 hosts the daemon validates image manifest metadata by
  default and fails early when an arm64 manifest is known absent or cannot
  be checked.
- Entrypoint: do not replace the runtime command with a long running
  wrapper. Leave `ENTRYPOINT` empty or use a pass through entrypoint that
  execs the command supplied by `docker run`.
- Runtime binary: install the runtime executable on `PATH`. `rtm` passes
  the runtime command directly (for example `claude`) and does not inject
  it into the image.
- Workspace: create `/workspace` and make it writable by the runtime user.
- Tools: include `git` and `/bin/sh`; prefer `/bin/bash`.
- Exit codes: the runtime command exit code is the container exit code. Do
  not mask failures in shell wrappers.
- Init: rely on the default Docker init added by `rtm`, unless the image
  owns init and the spawn uses the `docker:own-init` isolation profile.

Privileged execution is rejected. Capability changes are opt in. The
in-repo example at `examples/dockerfiles/claude.Dockerfile` is the
repository end-to-end Docker verification target. Treat it as a contract
example. Do not treat it as a supported recipe matrix.

### Image Selection

`--image` is preferred for ad hoc spawns. `RTM_DOCKER_IMAGE` is a daemon
startup environment default used only when a Docker spawn omits `--image`.

### Credentials

Credential pass through is explicit. The daemon does not mount host
credential directories. Operators pass environment variables through the
spawn request and add explicit bind mounts in their own image or daemon
profile configuration when a deployment owns that risk.

```toml
# Example operator profile fragment
[docker.credentials]
env = ["ANTHROPIC_API_KEY"]
mounts = [
  { source = "/secure/agent-credentials", target = "/run/agent-credentials", readonly = true },
]
```

## Events Contract

`RuntimeRpc::Events` returns `RuntimeResponse::Events { events, cursor }`.
Events are appended in observation order by the durable event log. `Running`
is appended after the shim ready record is stored. `Terminated` or `Lost` is
appended on exit or loss evidence.

Each poll preserves append order. `--since CURSOR` resumes without duplicate
delivery across daemon restarts. If a cursor falls behind the retained log
floor, the daemon returns `RuntimeResponse::CursorExpired { oldest }` and the
client is expected to use `Status` with `session_ids` and `updated_since` as
the authoritative reconciliation view. The CLI surfaces expiry as a typed
JSON object on stdout and a typed message on stderr with exit code 2.

<!-- rtm-admin-tools:start -->
## Admin MCP Tools

| Tool | Purpose |
| --- | --- |
| `rtm_kill_by_pid` | Admin escape hatch that signals a runtime process by pid, waits for the grace period, then sends SIGKILL if the process remains alive. |
| `rtm_status` | Return rtmd Lifecycle rows, optionally filtered by session id, session set, runtime, lifecycle state, and updated time. |
| `rtm_version` | Return the rtmd package version, build git sha, protocol version, and advertised capabilities. |
| `rtm_watchers` | Return rtmd operator visibility counters for process exit watchers, pending shim socket waiters, and Events long poll waiters. |
<!-- rtm-admin-tools:end -->

## Release And Distribution

The `rtm` binary changelog and workspace version bump are owned by Release
Please. cargo-dist owns binary artifact planning and tagged binary release
builds. Supported targets are the macOS and Linux gnu and musl targets
listed in the workspace `Cargo.toml`.

The public crates.io surface is two crates: `lilo-rm-core` and
`lilo-rm-client`. release-plz owns those crate release PRs, GitHub Releases,
and crates.io publishing. `rtm-daemon`, `rtm-cli`, `rtm-platform`,
`rtm-launchers`, `rtm-store`, and `rtm-paths` are private implementation
crates and are not published.

The `lilo-` prefix marks the consumer facing crates in this family. The
`helioy-` prefix is reserved for enterprise distribution and is not used by
this repo.

## Cross Repo Contracts

session-matters is the primary upstream consumer. It authorizes spawns
through identity-matters and then delegates execution to rtmd over the local
Unix socket. The runtime protocol version is gated by `rtm-core`. `smd`
requires `rtmd` at the compatible minor.

Session ids are UUIDv7 and are issued by session-matters at spawn time,
before any runtime process exists. runtime-matters treats them as opaque
identifiers and does not generate them on the host.

transport-matters observes wire traffic and is orthogonal to this layer. It
does not require coordination from rtmd.

## Engineering Standards

Keep the protocol crate small and stable. Put filesystem and process work in
the appropriate platform or daemon crate. Keep the CLI thin and force domain
decisions down to the daemon. Launchers build commands and stay clear of
policy.

For CLI shape, think `kubectl`: stable verbs, positional targets, and flags
for modifiers.

Spawn targets stay explicit. Pane placement belongs to the caller as policy.
The daemon owns mechanism only. Silent recreation of state defeats deliberate
operator action and is rejected by design.

The normal quality gate is:

```bash
just check
just build
just test
```

Snapshot tests run through `just insta-test`. The toolchain is pinned in
`rust-toolchain.toml`.
