# Changelog

## 0.1.0

Initial v1 substrate for runtime-matters.

- `rtm daemon` manages a per host runtime daemon over a Unix socket.
- `rtm-shim` supervises a single runtime process and reports lifecycle exits.
- Runtime launch dispatch supports Claude and Codex through a compile time launcher registry.
- Lifecycle state persists to sqlite and reconciles on startup and periodic probe sweeps.
- Tmux pane discovery and nudge support operator workflows.
- Admin MCP exposes `rtm_kill_by_pid`, `rtm_status`, `rtm_version`, and `rtm_watchers`.
- `rtm doctor` reports daemon, sqlite, watcher, launcher, tmux, sweep, and recent Lost health.
