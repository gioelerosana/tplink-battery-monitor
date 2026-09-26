# Architecture

The project is built as **two independent layers**: a transport layer that
talks to the router, and an application layer that decides when to notify.
They live in separate files/modules, so the API client can be exercised without
triggering notifications, and the notification logic can be tested without a
router.

```
        ┌──────────────────────────────────────────────────────────┐
        │ monitor.py / src/main.rs   (application layer)           │
        │  - config (.env + args)                                   │
        │  - polling loop / oneshot                                 │
        │  - state machine: threshold, cooldown, charging events    │
        │  - persistent state on disk                               │
        │  - notification delivery                                  │
        └───────────────────────────┬──────────────────────────────┘
                                    │  battery() / get_status()
                                    ▼
        ┌──────────────────────────────────────────────────────────┐
        │ tplink_mifi.py / src/tplink_mifi.rs   (transport layer)  │
        │  - HTTP to 192.168.0.1                                    │
        │  - AES+RSA login, session token                           │
        │  - JSON envelope encode/decode                            │
        └───────────────────────────┬──────────────────────────────┘
                                    │  HTTP POST
                                    ▼
        ┌──────────────────────────────────────────────────────────┐
        │ Router: /cgi-bin/auth_cgi, /cgi-bin/web_cgi              │
        └──────────────────────────────────────────────────────────┘
```

The guiding rule: the client knows nothing about thresholds or notifications,
and the monitor knows nothing about AES/RSA. Each layer has a single
responsibility. The Rust and Python sources are kept as close to a 1:1
translation as practical (same function names, same control flow, same wire
format).

## Transport layer — `MiFiClient`

Reimplements the protocol the browser would use, without a browser:

1. **Handshake** (`_auth_load` / `auth_load`): fetch `nonce`, `rsaMod`,
   `rsaPubKey`, `seqNum`.
2. **Key material** (`login`): generate a random AES-128 key/IV, compute
   `hash = MD5("admin" + password)`.
3. **Sign & encrypt** (`_encrypt_payload` / `encrypt_payload`): AES-128-CBC
   (PKCS#7) the JSON, then produce the chunked RSA-512 signature. The signed
   string differs for login vs. normal calls.
4. **Tolerant decode** (`_decode` / `decode`): the router mixes plain JSON,
   base64-of-JSON and base64-of-AES; a single function normalizes all three.

The session `token` returned by login is stored on the client and re-injected
into every later request. If a request fails because the token expired,
`get_status` logs in again and retries once.

Public surface: `login`, `call(module, action, data)`, `get_status`,
`battery`, `reboot`. See [protocol.md](protocol.md) for the wire details.

## Application layer — monitor

This is not a bare `if battery < 20`. It is a **state machine** with several
concepts:

- **Threshold**: `level < BATTERY_THRESHOLD`.
- **Charging exception**: a low level while charging is not a problem, so the
  low-battery alert is suppressed (the level is still logged).
- **Charge events**: transitions of the `charging` flag drive dedicated
  notifications:
  - `false → true`: charge started,
  - `true → true` and `level == 100`: charge complete (once per cycle),
  - `true → false` with `level < 100`: charge interrupted,
  - `true → false` with `level == 100`: normal unplug, logged only.
- **Hysteresis + cooldown**: a low-battery alert fires on the state
  transition and then at most every `NOTIFY_COOLDOWN_MINUTES`, to avoid spam
  when the level oscillates around the threshold.

State (`low`, `last_notify`, `level`, `charging`, `full_notified`) is
persisted to JSON (`~/.cache/tplink-battery-monitor/state.json`) so a restart
or a fresh timer tick does not duplicate or lose notifications. On the very
first run the `charging` key is absent, so charge transitions are recorded
without emitting spurious notifications.

`check_once` is the core: read → evaluate → notify → persist. The `main`
function drives it either once (`--once`, used by the systemd timer) or in a
loop with `sleep(interval)`.

Notification delivery is isolated:

- Rust uses `notify-rust` (native D-Bus, `org.freedesktop.Notifications`).
- Python shells out to `notify-send`.

Both degrade gracefully (log only) when the notification service is
unavailable, which also makes `--no-notify` a natural test mode.

## Configuration and deployment

- **Config**: environment variables loaded from `.env`, with CLI args taking
  precedence. Search order: `$TP_ENV_FILE`, `./.env`,
  `~/.config/tplink-battery-monitor/.env`.
- **Secrets**: the password lives only in `.env`, which is git-ignored.
  The sources contain no credentials.
- **Deployment**: the Rust binary is installed to `~/.local/bin` and run by a
  systemd **user** `oneshot` service triggered by a 5-minute `timer`
  (`AccuracySec=30s` to coalesce wakeups). A user service is required so the
  D-Bus notification reaches the graphical session.

## Design decisions

| Decision | Reason |
| --- | --- |
| Two separate modules | Testability: exercise the API without notifying, and the threshold logic with `--no-notify`. |
| Duplicate implementations (Rust + Python) | A single self-contained binary for deployment; a dependency-free-ish script for inspection. |
| Chunked RSA-512 signature | A real firmware constraint, replicated faithfully rather than worked around. |
| Tolerant decode | The router mixes response encodings; one place normalizes them. |
| Persistent state | Correct cooldown/hysteresis across restarts and timer ticks. |
| Pluggable notification backend | The delivery channel can change without touching the logic. |
| Timer cadence decoupled from alert cadence | Battery changes slowly; the timer can stay at 5 minutes while cooldown controls notification frequency. |
