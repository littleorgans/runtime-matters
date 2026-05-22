# TLDR

`runtime-matters` is the per host substrate that spawns and observes local
agent processes for littleorgans. One daemon owns lifecycle state. One CLI
binary talks to it. One shim wraps each runtime so exit and loss evidence
are captured even if the agent dies between polls.

## Mental Model

A spawn target is explicit. The caller asks for `headless` or for a concrete
tmux pane address. The daemon does not invent pane placement.

A runtime is the named agent the launcher knows how to start, for example
`claude` or `codex`. Launchers choose the command; the isolation backend
decides whether that command runs on the host or inside Docker.

A session id is a UUIDv7 issued by the caller before any process exists. It is
the join key for status, events, kill, and reconnection.

The durable event log is the source of truth for lifecycle. Transitions are
recorded in observation order and survive daemon restarts. Clients resume
with a cursor.

`rtm doctor` is the first command to run when something feels wrong. It
reports socket readiness, paths, Docker availability, and manifest
validation capability.

See [PROJECT.md](./PROJECT.md) for more.
