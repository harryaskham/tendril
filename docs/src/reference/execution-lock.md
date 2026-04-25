# Execution lock and queue

`tendril run` is protected by a host-local execution lock by default. The lock is intentionally simple: every `run` command creates a queue ticket, waits until its ticket is first, creates the lock directory atomically, runs the input sequence, and removes the lock when the process exits.

This prevents concurrent agents on the same host/session from interleaving clicks, keystrokes, focus changes, and waits.

## Scope

The default lock covers `tendril run`, including MCP `run` tool calls. That is the highest-risk command because it mutates the live desktop.

Read-oriented commands such as `list` and `capture` do not take this lock by default. They can still observe desktop state changing while another process is running input, so callers that need a fully serialized inspect-act flow should run their own higher-level workflow serialization or opt a future Tendril command into the same lock.

## Default lock location

Unless overridden, the lock root is created under the OS temp directory and namespaced by user plus desktop session:

```text
$TMPDIR/tendril-execution-lock-<user>-<session>/
```

The session component is resolved from `TENDRIL_LOCK_SESSION`, `WAYLAND_DISPLAY`, `DISPLAY`, or `XDG_SESSION_ID`, falling back to `default-session`. The lock is host-local and works across separate Tendril processes and separate Cacophony agents on the same machine.

## Queue and stale-lock behavior

Each waiting process writes a JSON ticket under `queue/`. The current owner writes JSON metadata under `held/owner.json` and refreshes a heartbeat while the command is running.

Defaults:

- queue wait timeout: `60000` ms
- stale heartbeat threshold: `30000` ms

If the owner process crashes, its heartbeat stops. A later waiter removes the stale `held/` directory after the stale threshold and continues the queue. Stale queue tickets are also reaped so abandoned waiters do not block the queue permanently.

## JSON metadata

Successful `run` results include execution-lock metadata under `data.execution_lock`:

```json
{
  "enabled": true,
  "acquired": true,
  "lock_path": "/tmp/tendril-execution-lock-alice-wayland-1",
  "timeout_ms": 60000,
  "stale_ms": 30000,
  "wait_ms": 151,
  "queue_position_at_join": 2,
  "queue_depth_at_join": 2,
  "stale_locks_reaped": 0,
  "stale_tickets_reaped": 0,
  "owner_pid": 12345,
  "token": "12345-1770000000000-1"
}
```

If a process times out while waiting, JSON error details include `execution_lock`, `holder`, `queue_position`, and `queue_depth` so agents can diagnose who held the lock and how long they waited.

## CLI controls

```bash
# Default: wait for the host-local execution lock
tendril --window <id> run 'send("hello")'

# Opt out for an advanced workflow that already serializes desktop control
tendril --window <id> run --no-lock 'send("hello")'

# Wait at most five seconds for the lock
tendril --window <id> run --lock-timeout-ms 5000 'send("hello")'

# Use a custom lock root, useful for tests or isolated sandboxes
tendril --window <id> run --lock-path /tmp/my-tendril-lock 'send("hello")'
```

`--lock-stale-ms <ms>` overrides the stale heartbeat threshold for the invocation. Use values larger than normal command latency; very small values are intended only for tests.

## Environment controls

- `TENDRIL_NO_LOCK=true` disables the lock for `run`.
- `TENDRIL_LOCK_TIMEOUT_MS=<ms>` sets the queue wait timeout.
- `TENDRIL_LOCK_STALE_MS=<ms>` sets the stale heartbeat threshold.
- `TENDRIL_LOCK_PATH=<path>` overrides the lock root.
- `TENDRIL_LOCK_SESSION=<name>` overrides the session component of the default path.

CLI flags take precedence over environment values, and environment values take precedence over config defaults.

## Config defaults

```yaml
execution_lock:
  enabled: true
  timeout_ms: 60000
  stale_ms: 30000
  path: null
```

Set `path` only when you intentionally want to share or isolate lock state differently from the default user/session namespace.
