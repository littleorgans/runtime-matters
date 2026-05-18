# lilo-rm-core

Core protocol types for the Runtime Matters daemon.

`lilo-rm-core` owns the public JSON line contract shared by `rtm` clients and rtmd:

- spawn, status, nudge, kill, doctor, MCP bridge, and version request types
- runtime lifecycle and event types
- transport helpers for newline delimited JSON

The current runtime protocol version is `0.2`. `VersionInfo` advertises this
protocol version and these stable capability names:

| Capability | Contract |
| --- | --- |
| `structured_protocol_errors` | Error responses expose stable machine readable codes. |
| `headless_stdio_log_paths` | Headless spawn responses include stdout and stderr log paths. |
| `status_session_set_filter` | Status requests accept a set of session ids. |
| `status_updated_since_filter` | Status requests accept an updated time lower bound. |
| `typed_nudge_outcomes` | Nudge responses expose typed delivery outcomes. |
| `validate_target_preflight` | ValidateTarget checks a target string without spawning. |

## Doctor JSON Contract

`RuntimeResponse::Doctor` returns `DoctorResponse`. In protocol version `0.2`,
session-matters may treat these JSON field names and value kinds as stable:

- `version.version`, `version.git_sha`, `version.protocol_version`, and `version.capabilities`
- `socket_path` and `uptime_secs`
- `sqlite.applied`, `sqlite.total`, `sqlite.applied_descriptions`, and `sqlite.pending_descriptions`
- `lifecycles.forking`, `lifecycles.running`, `lifecycles.exited`, and `lifecycles.lost`
- `watchers.kqueue_watchers` and `watchers.shim_sockets`
- `launchers[].runtime`, `launchers[].command`, and `launchers[].error`
- `tmux.available`, `tmux.version`, and `tmux.error`
- `last_probe_sweep`
- `recent_lost[].session_id`, `recent_lost[].evidence`, and `recent_lost[].occurred_at`

Diagnostic values such as versions, git shas, paths, uptime, process counts,
watcher counts, launcher commands, tmux details, and timestamps vary by build
and host. Use `version.protocol_version` plus `version.capabilities` for
contract negotiation.

The daemon, CLI, platform, launcher, and store crates remain private implementation details.
