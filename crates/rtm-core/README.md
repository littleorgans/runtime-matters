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

The daemon, CLI, platform, launcher, and store crates remain private implementation details.
