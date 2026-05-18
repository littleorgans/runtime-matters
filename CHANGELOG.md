# Changelog

## [0.1.5](https://github.com/littleorgans/runtime-matters/compare/v0.1.4...v0.1.5) (2026-05-18)


### Bug Fixes

* forward caller env and cwd to spawned runtimes ([#11](https://github.com/littleorgans/runtime-matters/issues/11)) ([5a254c9](https://github.com/littleorgans/runtime-matters/commit/5a254c92eb8ea6dd31c7a09c77d333b7e2424dc6))

## [0.1.4](https://github.com/littleorgans/runtime-matters/compare/v0.1.3...v0.1.4) (2026-05-18)


### Features

* introduce strict deterministic spawn API ([#9](https://github.com/littleorgans/runtime-matters/issues/9)) ([ba7058a](https://github.com/littleorgans/runtime-matters/commit/ba7058a04d6182d226f0d6dfef19a106f624d452))

## [0.1.3](https://github.com/littleorgans/runtime-matters/compare/v0.1.2...v0.1.3) (2026-05-17)


### Bug Fixes

* defer GitHub Release creation to release-please ([#7](https://github.com/littleorgans/runtime-matters/issues/7)) ([e9d338b](https://github.com/littleorgans/runtime-matters/commit/e9d338b62b27bf778806ab4a1c057e3ca9d5f0da))

## [0.1.2](https://github.com/littleorgans/runtime-matters/compare/v0.1.1...v0.1.2) (2026-05-17)


### Bug Fixes

* define [profile.dist] and upgrade cargo-dist to 0.31.0 ([#5](https://github.com/littleorgans/runtime-matters/issues/5)) ([da660eb](https://github.com/littleorgans/runtime-matters/commit/da660eb1df488b333507f448f305ee32c7909031))

## [0.1.1](https://github.com/littleorgans/runtime-matters/compare/v0.1.0...v0.1.1) (2026-05-17)


### Features

* ship runtime-matters v1 (per-host kubelet + container runtime) ([#2](https://github.com/littleorgans/runtime-matters/issues/2)) ([28babac](https://github.com/littleorgans/runtime-matters/commit/28babacd727f0c468434cf21858c0932f6b5d00f))


### Bug Fixes

* redact binary version in mcp_responses snapshot ([#4](https://github.com/littleorgans/runtime-matters/issues/4)) ([4b83716](https://github.com/littleorgans/runtime-matters/commit/4b8371613891712294b52b6293230c02dab0cab2))

## 0.1.0

Initial v1 substrate for runtime-matters.

- `rtm daemon` manages a per host runtime daemon over a Unix socket.
- `rtm-shim` supervises a single runtime process and reports lifecycle exits.
- Runtime launch dispatch supports Claude and Codex through a compile time launcher registry.
- Lifecycle state persists to sqlite and reconciles on startup and periodic probe sweeps.
- Tmux pane discovery and nudge support operator workflows.
- Admin MCP exposes `rtm_kill_by_pid`, `rtm_status`, `rtm_version`, and `rtm_watchers`.
- `rtm doctor` reports daemon, sqlite, watcher, launcher, tmux, sweep, and recent Lost health.
