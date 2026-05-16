# runtime-matters

Per host runtime substrate for the Helioy agent platform.

## Development

```bash
just check
just build
just test
```

## Pass 1 tracer

Start the daemon:

```bash
rtm daemon start
```

Spawn a Claude runtime:

```bash
cargo run -p rtm-cli --example test_spawn -- --runtime claude --session-id "$(uuidgen)"
```

Check status:

```bash
rtm status
```

Stop the daemon:

```bash
rtm daemon stop
```
