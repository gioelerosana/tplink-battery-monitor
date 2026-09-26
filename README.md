# TP-Link M7350 Battery Monitor

[![CI](https://github.com/JoelShepard/tplink-battery-monitor/actions/workflows/ci.yml/badge.svg)](https://github.com/JoelShepard/tplink-battery-monitor/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

Get a desktop notification when the battery of a portable TP-Link MiFi router
drops below a threshold (default **20%**), plus notifications for charging
events (charge started / complete / interrupted).

> 🇮🇹 [Leggi il README in italiano](README.it.md)

The router firmware exposes an undocumented JSON API used by its own web UI.
This project reverse-engineers that API, reimplements the AES + RSA + MD5
authentication flow, and ships **two equivalent (1:1) implementations**:

| Implementation | Files | Notification backend |
| --- | --- | --- |
| **Rust** (recommended, single binary) | `src/tplink_mifi.rs`, `src/main.rs` | native D-Bus (`notify-rust`) |
| **Python** | `tplink_mifi.py`, `monitor.py` | `notify-send` |

The Rust version builds to a single binary designed to be run periodically by
a **systemd user timer** (oneshot service). The Python version is handy for
inspection and experimentation without compiling.

Tested on **TP-Link M7350 (EU) v9.0**, firmware `9.0.5`. It should work on
other TP-Link mobile Wi-Fi models that expose the `/cgi-bin/web_cgi` API
(e.g. M7000 / M7200 family).

## Features

- Battery level monitoring with a configurable threshold (default 20%).
- Low-battery alert suppressed while charging.
- Charge-event notifications:
  - 🔌 **charge started** (battery → charging),
  - ✅ **charge complete** (reaches 100%),
  - ⚠️ **charge interrupted** (unplugged before 100%).
- Notification cooldown + state hysteresis (no notification spam).
- Persistent state across runs, so the cooldown survives the timer cadence.
- No cloud, no browser: it talks directly to the router on your LAN.

## Requirements

- **Rust** (stable toolchain, `cargo`) for the binary version.
- **Python 3.9+** and `cryptography` for the Python version.
- A graphical session with D-Bus / `notify-send` for desktop notifications.
- A machine connected to the router's LAN.

## Configuration

Copy `.env.example` to `.env` and fill it in:

```ini
TPLINK_HOST=192.168.0.1
TPLINK_PASSWORD=your_admin_password
BATTERY_THRESHOLD=20
CHECK_INTERVAL_MINUTES=5
NOTIFY_COOLDOWN_MINUTES=60
```

The Rust binary looks for the config in this order: `$TP_ENV_FILE`, `./.env`,
`~/.config/tplink-battery-monitor/.env`. Real environment variables always
take precedence. `.env` is git-ignored: **never commit your password**.

## Rust: build, install, run

```bash
cargo build --release
# binary at target/release/tplink-battery-monitor

# single check, no notifications (just verify it works)
./target/release/tplink-battery-monitor --once --no-notify

# continuous loop
./target/release/tplink-battery-monitor

# single check forcing a notification (test)
./target/release/tplink-battery-monitor --once --threshold 100
```

Options: `--host`, `--password`, `--threshold`, `--interval`, `--cooldown`,
`--once`, `--no-notify`, `--notify-recovery`.

State is stored in `~/.cache/tplink-battery-monitor/state.json`
(override with `TP_BATTERY_STATE`).

### systemd user timer

The files in `systemd/` install the binary and a **5-minute user timer**:

```bash
mkdir -p ~/.local/bin ~/.config/tplink-battery-monitor ~/.config/systemd/user
cargo build --release
install -Dm755 target/release/tplink-battery-monitor ~/.local/bin/tplink-battery-monitor
cp .env ~/.config/tplink-battery-monitor/.env && chmod 600 ~/.config/tplink-battery-monitor/.env
cp systemd/tplink-battery-monitor.service systemd/tplink-battery-monitor.timer ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now tplink-battery-monitor.timer

systemctl --user list-timers tplink-battery-monitor.timer
journalctl --user -u tplink-battery-monitor.service -f
```

The `service` is `Type=oneshot` and is triggered by the `timer`
(`OnUnitActiveSec=5min`, `AccuracySec=30s`). No `DISPLAY` is needed: the
notification travels over D-Bus in the user session.

## Python: run

```bash
# single check, no notifications
python3 monitor.py --once --no-notify

# continuous loop (reads .env)
python3 monitor.py

# single check forcing a notification
python3 monitor.py --once --threshold 100

# low-level API client
python3 tplink_mifi.py --password '...' battery
python3 tplink_mifi.py --password '...' status
```

### cron (alternative to the systemd timer)

```cron
*/5 * * * * cd /path/to/tplink-battery-monitor && /usr/bin/python3 monitor.py --once >> monitor.log 2>&1
```

With cron, desktop notifications need `DISPLAY` and
`DBUS_SESSION_BUS_ADDRESS` in the environment; systemd is preferred.

## The `battery` field gotcha

The firmware reports:

```json
"battery": { "connected": true, "charging": false, "voltage": 98 }
```

Despite the name, **`voltage` already contains the percentage (0–100)** — the
web UI uses it directly to pick the battery icon. So `voltage: 98` means
98% battery. Both implementations expose it as `level` as well.

## How it works

The router uses a JSON API with two endpoints and a "GDPR" encryption layer
(AES-128-CBC for the payload, RSA-512 for the signature, MD5 for the
password hash). A complete write-up of the protocol is in
[docs/protocol.md](docs/protocol.md), and the software architecture
(transport vs. application layer, state machine, deployment) is in
[docs/architecture.md](docs/architecture.md).

## Documentation

- [docs/protocol.md](docs/protocol.md) — reverse-engineered HTTP API.
- [docs/architecture.md](docs/architecture.md) — software architecture.
- [CONTRIBUTING.md](CONTRIBUTING.md) — build, test and contribution notes.
- [CHANGELOG.md](CHANGELOG.md) — release history.
- [SECURITY.md](SECURITY.md) — how secrets are handled.

## Troubleshooting

- **"Router non raggiungibile"** — make sure you run the tool on the router's
  LAN and that `TPLINK_HOST` is correct (default `192.168.0.1`).
- **"Password del router errata"** — the admin password in `.env` is wrong.
- **No notification appears** — check that `notify-send` / the D-Bus session
  works in your graphical session; from a systemd service, use a *user*
  service (not a system service).

## License

[MIT](LICENSE) © 2026 Gioele Rosana
