# runtime-matters

Per host runtime substrate for the Helioy agent platform.

## Install

runtime-matters ships one binary, `rtm`. The daemon, CLI, MCP bridge, and shim are subcommands of that binary.

```bash
cargo install --path crates/rtm-cli
rtm daemon start
```

v0.1.0 release artifacts are configured through cargo-dist for macOS:

- `aarch64-apple-darwin`
- `x86_64-apple-darwin`

Linux is intentionally out of v1. The `pidfd` platform module is stubbed for v2.

## Development

```bash
just check
just build
just test
just insta-test
```

## Strict Spawn Quickstart

Build the local `rtm` binary and start the daemon:

```bash
cargo build -p rtm-cli
rtm_bin="$PWD/target/debug/rtm"
"$rtm_bin" daemon start &
rtm_daemon_pid=$!
until "$rtm_bin" status >/dev/null 2>&1; do sleep 0.1; done
```

From inside an existing tmux session, create the pane outside rtm and pass that exact target:

```bash
target="$(tmux split-window -P -F '#S:#I.#P' -d)"
cargo run -p rtm-cli --example test_spawn -- --target "tmux:${target}" --runtime claude --session-id "$(uuidgen)"
"$rtm_bin" status
```

For a headless runtime, pass the target explicitly. The spawn response prints the session log directory.

```bash
cargo run -p rtm-cli --example test_spawn -- --target headless --runtime claude --session-id "$(uuidgen)"
"$rtm_bin" status
```

Stop the daemon:

```bash
"$rtm_bin" daemon stop
wait "$rtm_daemon_pid"
```

## Hardening Gates

```bash
just dist-plan
just insta-test
just bench-spawn
just bench-status
just load-test
```

Current local perf targets and results from this pass:

| Surface | Target | Local result |
| --- | --- | --- |
| Spawn latency | p50 under 200 ms | 9.991 ms |
| Status query | p50 under 5 ms | 0.065 ms |
| 50 session load | rtmd plus 50 shims under 90 MiB app footprint | 83.67 MiB |

The load gate also prints raw RSS and OS footprint for diagnostics. The assertion uses application footprint because raw RSS double counts the shared `rtm` image across 50 shim processes.

## Release

Release Please owns changelog and version bumps through `.release-please-manifest.json` and `release-please-config.json`. cargo-dist owns binary artifact planning and tagged release builds. This pass does not create a release tag.

## Roadmap

- Linux parity through `pidfd`.
- Runtime plugin loading beyond the v1 compile time registry.
- Optional system level daemon when shared host infrastructure needs it.

<!-- rtm-admin-tools:start -->
## Admin MCP Tools

| Tool | Purpose |
| --- | --- |
| `rtm_kill_by_pid` | Admin escape hatch that signals a runtime process by pid, waits for the grace period, then sends SIGKILL if the process remains alive. |
| `rtm_status` | Return rtmd Lifecycle rows, optionally filtered by session id, runtime, and lifecycle state. |
| `rtm_version` | Return the rtmd package version and build git sha. |
| `rtm_watchers` | Return rtmd operator visibility counters for kqueue watchers and pending shim socket waiters. |
<!-- rtm-admin-tools:end -->
