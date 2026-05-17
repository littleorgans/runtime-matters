# runtime-matters

`runtime-matters` is the per host runtime substrate for Helioy agent processes.

Pass 1 implements the tracer slice:

- `rtm daemon start` listens on the runtime socket.
- A spawn request forks `rtm __shim`.
- The shim starts the Claude runtime and sends `ShimReady`.
- The daemon records a Running lifecycle in memory and exposes status.

