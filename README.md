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

`RTM_SOCKET_PATH` is authoritative when set. Without it, Linux defaults to
`$XDG_RUNTIME_DIR/rtm/sock` when `XDG_RUNTIME_DIR` is available, then falls back
to `~/.rtm/sock`. macOS defaults to `~/.rtm/sock`.

## Events Contract

`RuntimeRpc::Events` is the v0.3 event endpoint. It returns
`RuntimeResponse::Events { events: Vec<RuntimeEvent>, cursor }`.

The daemon appends events in observation order as they are recorded by the
global durable event log. `Running` is recorded after shim ready is stored.
`Terminated` or `Lost` is recorded when exit or loss evidence is observed. Each
poll preserves that append order. Clients can pass `--since CURSOR` to resume
after the last returned cursor without duplicate delivery across daemon restarts.

If a cursor falls behind the retained log floor, rtmd returns
`RuntimeResponse::CursorExpired { oldest }`. Use `Status` with `session_ids` and
`updated_since` as the authoritative lifecycle view when reconciliation matters.

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
