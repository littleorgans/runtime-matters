# runtime-matters

Per host runtime substrate for Helioy agents.

Runtime Matters runs and observes local agent processes. It gives callers one
stable control surface for spawning runtimes, tracking lifecycle state, reading
durable events, and nudging tmux sessions.

## Install

Runtime Matters ships one binary, `rtm`. The daemon, CLI, MCP bridge, and shim
are subcommands of that binary.

```bash
cargo install --path crates/rtm-cli
rtm daemon start
```

Release artifacts are built through cargo-dist for supported native hosts:

- `aarch64-apple-darwin`
- `x86_64-apple-darwin`
- `x86_64-unknown-linux-gnu`

| Host | Support |
| --- | --- |
| macOS | Supported for headless and tmux runtimes. |
| Linux | Supported for headless runtimes. Tmux support is optional and depends on the host tmux capability. |
| Windows | Native Windows is out of scope. Use a supported Unix host. |

## Development

```bash
just check
just build
just test
just insta-test
```

## Runtime Model

Spawn targets are explicit. A caller passes either `headless` or a concrete tmux
pane address.

```bash
cargo run -p rtm-cli --example test_spawn -- --target headless --runtime claude --session-id "$(uuidgen)"
```

For tmux runtimes, create the pane outside `rtm` and pass the exact pane target:

```bash
target="$(tmux split-window -P -F '#S:#I.#P' -d)"
cargo run -p rtm-cli --example test_spawn -- --target "tmux:${target}" --runtime claude --session-id "$(uuidgen)"
```

Runtime filesystem locations are modeled as paths, while daemon connection
targets are modeled as endpoints. The current functional endpoint is a Unix
socket. The `rtm-paths` crate owns this policy so callers do not treat future
transport targets as filesystem paths.

`RTM_SOCKET_PATH` is authoritative when set. Without it, Linux defaults to
`$XDG_RUNTIME_DIR/rtm/sock` when `XDG_RUNTIME_DIR` is available, then falls back
to `~/.rtm/sock`. macOS defaults to `~/.rtm/sock`. `RTM_DB_PATH`,
`RTM_HOME`, and `RTM_SHIM_PATH` keep their existing behavior for database,
runtime home, logs, event log placement, and shim bootstrap.

## Docker Isolation

Host execution is the default. Docker isolation is an experimental execution
policy selected per spawn with `--isolation docker`. Runtime kind remains
unchanged. Launchers still choose the agent command. The backend decides whether
that command runs on the host or inside Docker.

Headless Docker spawns run as foreground `docker run` processes owned by the
host shim. Tmux Docker spawns use Pattern A: `rtm` starts a detached container
and attaches the existing host tmux pane to it. Closing the pane ends the attach
path, while the container remains managed by the Docker backend until runtime
exit or an explicit kill. Manual detach and reconnect UX are out of scope.

`/workspace` is the canonical default workspace path for Docker images and
operator examples. The current backend bind mounts the requested spawn cwd at
the same path inside the container and sets it as the container workdir.

The Docker image policy is Option A: `rtm` is image agnostic. A practical
starter base is `mcr.microsoft.com/devcontainers/base:ubuntu` because it already
fits interactive coding agent expectations. Distroless and Alpine/musl images
are discouraged starters. They commonly lack the shell, libc, and
troubleshooting surface expected by interactive coding agents.

Interactive images must provide `/bin/sh`; `/bin/bash` is recommended where
practical. Starter images should include or inherit `git`. Images should run as
a non-root user. The daemon enforces non-root image metadata by default, rejects
missing root metadata as root, and requires an explicit opt in for root images.
On arm64 hosts the daemon validates image manifest metadata by default and fails
early when an arm64 manifest is known absent or cannot be checked.

The backend adds Docker init by default. Images that provide their own init can
use the `docker:own-init` isolation profile. Capability changes are opt in.
Privileged execution is rejected, and aggressive capability hardening is
deferred.

`rtm` does not automatically mount host credential directories. Credential pass
through is explicit. Operators can pass environment variables through the spawn
request and can add explicit bind mounts in their own image or daemon profile
configuration when a deployment owns that risk. Named credential volume
management is deferred.

```bash
SESSION_ID="$(uuidgen)"
rtm spawn \
  --session-id "$SESSION_ID" \
  --runtime claude \
  --target headless \
  --isolation docker \
  --image runtime-matters-claude:local \
  --env CLAUDE_CODE_OAUTH_TOKEN
```

Prefer `--image` for ad hoc Docker spawns. `RTM_DOCKER_IMAGE` is a daemon
startup environment default used only when a Docker spawn omits `--image`.

```toml
# Example operator profile fragment.
[docker.credentials]
env = ["ANTHROPIC_API_KEY"]
mounts = [
  { source = "/secure/agent-credentials", target = "/run/agent-credentials", readonly = true },
]
```

`rtm doctor` reports Docker CLI readiness, daemon readiness, manifest validation
capability, Docker isolation support, and the unsupported Pattern E boundary.
Docker can be unavailable and host spawning remains supported.

Pattern D, Pattern E, Kubernetes, SandboxClaim, `rtm` injected sidecars,
reconnecting PTY servers, first class firewall UX, named credential volume
management, and aggressive capability hardening are not part of this
experimental surface.

## Dockerfile Contract

The in-repo example at `examples/dockerfiles/claude.Dockerfile` demonstrates the
contract for the Claude runtime and serves as the repository end-to-end Docker
verification target. Treat it as a contract example, not a supported recipe
matrix.

Docker images used with `rtm` must satisfy this contract:

- Base image: any Debian or Ubuntu compatible base is acceptable. The suggested
  starter is `mcr.microsoft.com/devcontainers/base:ubuntu`.
- User: the final image should declare a non-root `USER`. Root requires an
  explicit operator escape hatch.
- Entrypoint: do not replace the runtime command with a long running wrapper.
  Leave `ENTRYPOINT` empty or use a pass through entrypoint that execs the
  command supplied by `docker run`.
- Environment: runtime credentials are explicit pass through. Do not assume host
  credential directories are mounted.
- Runtime binary: install the runtime executable on `PATH`. `rtm` passes the
  runtime command directly, for example `claude`, and does not inject it into
  the image.
- Workspace: create `/workspace`, make it writable by the runtime user, and
  expect `rtm` to set the workdir from the spawn cwd.
- Tools: include `git`; include `/bin/sh`; prefer `/bin/bash`.
- Exit codes: the runtime command exit code is the container exit code. Do not
  mask failures in shell wrappers.
- Init: rely on the default Docker init added by `rtm`, unless the image owns
  init and the spawn uses `docker:own-init`.

## Events Contract

`RuntimeRpc::Events` is the v0.4 event endpoint. It returns
`RuntimeResponse::Events { events: Vec<RuntimeEvent>, cursor }`.

The daemon appends events in observation order as they are recorded by the
global durable event log. `Running` is recorded after shim ready is stored.
`Terminated` or `Lost` is recorded when exit or loss evidence is observed. Each
poll preserves that append order. Clients can pass `--since CURSOR` to resume
after the last returned cursor without duplicate delivery across daemon restarts.

If a cursor falls behind the retained log floor, rtmd returns
`RuntimeResponse::CursorExpired { oldest }`. Use `Status` with `session_ids` and
`updated_since` as the authoritative lifecycle view when reconciliation matters.
For the CLI, `rtm events --format json` emits `{ "cursor_expired": true,
"latest_cursor": N }`. `rtm events --format human` writes
`cursor expired (latest_cursor: N)` to stderr and exits with code 2.

## Release

Release Please owns the `rtm` binary changelog and workspace version bump.
cargo-dist owns binary artifact planning and tagged binary release builds.

The public crates.io contract is limited to `lilo-rm-core` and `lilo-rm-client`.
release-plz owns their crate release PRs, GitHub Releases, and crates.io
publishing. The daemon, CLI, platform, launchers, and store crates are private
implementation details.

v0.4 starts with the public process exit observation rename from kqueue specific
names to platform neutral watcher names. Linux runtime support is complete when
the Linux cargo-dist artifact and this host support documentation are released.

<!-- rtm-admin-tools:start -->
## Admin MCP Tools

| Tool | Purpose |
| --- | --- |
| `rtm_kill_by_pid` | Admin escape hatch that signals a runtime process by pid, waits for the grace period, then sends SIGKILL if the process remains alive. |
| `rtm_status` | Return rtmd Lifecycle rows, optionally filtered by session id, session set, runtime, lifecycle state, and updated time. |
| `rtm_version` | Return the rtmd package version, build git sha, protocol version, and advertised capabilities. |
| `rtm_watchers` | Return rtmd operator visibility counters for process exit watchers, pending shim socket waiters, and Events long poll waiters. |
<!-- rtm-admin-tools:end -->
