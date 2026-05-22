# runtime-matters

Per host runtime substrate for littleorgans agents.

`rtm` is the layer that owns the question every caller above it needs
answered: "is this process still alive, and if not, what happened?" When
a coding agent wedges, crashes between polls, or is killed by an
unrelated tmux command, stdout cannot be trusted. The kernel can. A
shim wrapping the process can. `rtm` composes those signals into one
stable surface.

## The substrate

Three load bearing components.

**rtmd**, the daemon, owns canonical state. Every spawn request flows
through it. Every lifecycle transition is appended to a durable event
log before any other observer sees it. Restart the daemon and the
state survives, because the source of truth is on disk, not in process
memory.

**The shim** wraps each runtime process. The daemon trusts the shim's
exit signal because nothing else on the host is in a reliable position
to deliver it.

**Launchers** know how to start each named runtime. The Claude launcher
knows how to start `claude`. The Codex launcher knows how to start
`codex`. Launchers build the command and hold no policy.

## The boundary

The interface is intentionally narrow. Spawn targets are explicit: the
caller asks for `headless` or names a concrete tmux pane address. The
daemon does not invent pane placement. Isolation policy is per spawn:
host execution by default, Docker as the alternate backend. The runtime
kind never changes when isolation changes. Launchers still choose the
command. The backend chooses where the command runs.

## The composition

A session id, UUIDv7 minted above this layer, is the join key for
status, events, kill, and reconnection across every component in the
stack. Clients consume the durable event log with a cursor.
Reconciliation is the status endpoint. `rtm doctor` is the diagnostic
surface. Each piece does one thing. The seams between them are
explicit.

## Install

```bash
cargo install --path crates/rtm-cli
rtm daemon start
```

| Host | Support |
| --- | --- |
| macOS | Headless and tmux runtimes. |
| Linux | Headless runtimes. Tmux support depends on host tmux capability. |

## Quickstart

```bash
rtm doctor
rtm spawn --session-id "$(uuidgen)" --runtime claude --target headless
rtm status
rtm events
```

Tmux runtime:

```bash
target="$(tmux split-window -P -F '#S:#I.#P' -d)"
rtm spawn --session-id "$(uuidgen)" --runtime claude --target "tmux:${target}"
```

Docker isolation:

```bash
rtm spawn --session-id "$(uuidgen)" --runtime claude --target headless \
  --isolation docker --image runtime-matters-claude:local
```

<!-- rtm-admin-tools:start -->
## Admin MCP Tools

| Tool | Purpose |
| --- | --- |
| `rtm_kill_by_pid` | Admin escape hatch that signals a runtime process by pid, waits for the grace period, then sends SIGKILL if the process remains alive. |
| `rtm_status` | Return rtmd Lifecycle rows, optionally filtered by session id, session set, runtime, lifecycle state, and updated time. |
| `rtm_version` | Return the rtmd package version, build git sha, protocol version, and advertised capabilities. |
| `rtm_watchers` | Return rtmd operator visibility counters for process exit watchers, pending shim socket waiters, and Events long poll waiters. |
<!-- rtm-admin-tools:end -->

See [PROJECT.md](./PROJECT.md) for more.
