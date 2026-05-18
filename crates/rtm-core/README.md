# lilo-rm-core

Core protocol types for the Runtime Matters daemon.

`lilo-rm-core` owns the public JSON line contract shared by `rtm` clients and rtmd:

- spawn, status, nudge, kill, doctor, MCP bridge, and version request types
- runtime lifecycle and event types
- transport helpers for newline delimited JSON

The daemon, CLI, platform, launcher, and store crates remain private implementation details.
