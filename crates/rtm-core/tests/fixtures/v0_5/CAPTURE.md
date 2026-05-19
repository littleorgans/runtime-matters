# v0.5 RuntimeResponse Fixture Capture

These fixtures preserve the `lilo-rm-core 0.5.0` response wire shapes before
the v0.6 payload extraction.

The reproducible path is:

```bash
git checkout v0.5.0
just build
```

Run `rtm-daemon` from that checkout with an isolated socket and `RTM_HOME`.
For each request, send one JSON line to the socket and save the returned JSON
line exactly as `<variant>.json`.

```bash
export RTM_HOME="$(mktemp -d)"
export RTM_SOCKET="$RTM_HOME/rtmd.sock"
target/debug/rtm-daemon --socket "$RTM_SOCKET" &
daemon_pid=$!
printf '%s\n' '<request-json>' | nc -U "$RTM_SOCKET" | tail -n 1 | jq -cS . > '<variant>.json'
kill "$daemon_pid"
```

Fixture values in this directory use deterministic equivalents of v0.5 daemon
responses from the published response contract. Do not regenerate these files
with the v0.6 serializer.
