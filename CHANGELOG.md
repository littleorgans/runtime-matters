# Changelog

## Unreleased

### Features

* add Linux cargo-dist release artifact support for `x86_64-unknown-linux-gnu`
* document macOS and Linux host support, including the Linux runtime socket path
* start the v0.4 release notes with the public process exit observation rename

## [0.2.2](https://github.com/littleorgans/runtime-matters/compare/v0.2.1...v0.2.2) (2026-05-19)


### Bug Fixes

* make rm client publishable ([7172b7c](https://github.com/littleorgans/runtime-matters/commit/7172b7c2c766b3bd1681f8442952150baf9ef3f3))

## [0.2.1](https://github.com/littleorgans/runtime-matters/compare/v0.2.0...v0.2.1) (2026-05-19)


### Features

* runtime-matters v0.6 client ergonomics ([#33](https://github.com/littleorgans/runtime-matters/issues/33)) ([86fce89](https://github.com/littleorgans/runtime-matters/commit/86fce89f419b990fde046597440585f32c244987))

## [0.2.0](https://github.com/littleorgans/runtime-matters/compare/v0.1.11...v0.2.0) (2026-05-19)


### ⚠ BREAKING CHANGES

* **rtm-cli:** clients on protocol 0.3 will see a version mismatch against rtmd. Pair with fec94dd, which switched operator commands to positional session id; both ship together as the v0.4 surface.

### Features

* **rtm-cli:** make session id positional for operator commands ([#30](https://github.com/littleorgans/runtime-matters/issues/30)) ([526314c](https://github.com/littleorgans/runtime-matters/commit/526314c90b178cc1325c25a92d7efa92d309c4d2))

## [0.1.11](https://github.com/littleorgans/runtime-matters/compare/v0.1.10...v0.1.11) (2026-05-19)


### Features

* Linux runtime v0.4 support ([#27](https://github.com/littleorgans/runtime-matters/issues/27)) ([76e0027](https://github.com/littleorgans/runtime-matters/commit/76e00278e84e11e887841356c700c14631bc5f4d))

## [0.1.10](https://github.com/littleorgans/runtime-matters/compare/v0.1.9...v0.1.10) (2026-05-18)


### Features

* runtime contract v0.3 for session-matters integration ([#23](https://github.com/littleorgans/runtime-matters/issues/23)) ([7c269da](https://github.com/littleorgans/runtime-matters/commit/7c269daa293485b6050f568c6b1e7a7ec57a04da))

## [0.1.9](https://github.com/littleorgans/runtime-matters/compare/v0.1.8...v0.1.9) (2026-05-18)


### Bug Fixes

* restore cargo-dist rtm release ([#20](https://github.com/littleorgans/runtime-matters/issues/20)) ([1016fd5](https://github.com/littleorgans/runtime-matters/commit/1016fd588df4762bb954e7b2a2de864ba814ad3d))

## [0.1.8](https://github.com/littleorgans/runtime-matters/compare/v0.1.7...v0.1.8) (2026-05-18)


### Features

* add session-matters runtime contract ([#17](https://github.com/littleorgans/runtime-matters/issues/17)) ([0f57833](https://github.com/littleorgans/runtime-matters/commit/0f57833b721bcabc919165e8e7ed0a3069d28c0e))

## [0.1.7](https://github.com/littleorgans/runtime-matters/compare/v0.1.6...v0.1.7) (2026-05-18)


### Bug Fixes

* make install switch back to release ([#15](https://github.com/littleorgans/runtime-matters/issues/15)) ([3c9eccc](https://github.com/littleorgans/runtime-matters/commit/3c9eccc9bfebf23f3dcfa8b940088e87f4ea745d))

## [0.1.6](https://github.com/littleorgans/runtime-matters/compare/v0.1.5...v0.1.6) (2026-05-18)


### Bug Fixes

* add install version switching ([#13](https://github.com/littleorgans/runtime-matters/issues/13)) ([ca26989](https://github.com/littleorgans/runtime-matters/commit/ca26989082c927e0d62b535986fa7905ac3d86d3))

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
