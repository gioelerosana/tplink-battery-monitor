# TP-Link M7350 Battery & Data Monitor

[![CI](https://github.com/gioelerosana/tplink-battery-monitor/actions/workflows/ci.yml/badge.svg)](https://github.com/gioelerosana/tplink-battery-monitor/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

Companion app with system tray icon (background panel) and desktop notification monitor for portable TP-Link routers (e.g. M7350).
Provides at a glance:
- **Battery percentage** and charging status
- **Data consumption** vs **remaining data** and total plan limit (with visual progress bar)
- **Today's data usage**
- **Mobile network status** (operator, 4G LTE signal, tx/rx speed and connected devices)

> [Leggi il README in italiano](README.it.md)

The router firmware exposes an undocumented JSON API used by its own web UI. This project reverse-engineers that API, reimplements the AES + RSA + MD5 authentication flow, and ships both a **Tray Companion App** and CLI tools in Python and Rust:

| Component | Files | Description |
| --- | --- | --- |
| **Tray Companion App** | `tplink_tray.py` | Battery icon & percentage in the system tray, detailed drop-down menu on click |
| **Python CLI Monitor** | `monitor.py` | Background polling, desktop notifications (`notify-send`), terminal `--data` view |
| **Python API Client** | `tplink_mifi.py` | Reusable API library (battery, data usage, status, reboot) |
| **Rust Binary Monitor** | `src/tplink_mifi.rs`, `src/main.rs` | Final standalone binary: tray companion (`--tray`), systemd timer, native D-Bus notifications |

Tested on **TP-Link M7350 (EU) v9.0**, firmware `9.0.5`. Compatible with TP-Link mobile Wi-Fi models using the `/cgi-bin/web_cgi` JSON API (e.g. M7000 / M7200 / M7350).

## Features

- **Battery Monitor**: exact percentage displayed in the system panel, plus desktop notifications when below threshold (default 20%).
- **Data Usage & Remaining Giga**: immediate visual glance at consumed data, remaining data, plan limit and daily traffic.
- **Smart Charging Events**:
  - Charge started,
  - Charge completed (100%),
  - Charge interrupted (unplugged before 100%).
- **Connection Info**: cellular operator, network type (4G/LTE), signal strength (0-4), real-time speed and connected clients.
- **Quick Web Access**: one-click button to open the router web interface (192.168.0.1).
- **Zero Cloud**: direct LAN/Wi-Fi communication with the router, credentials stored only in your local `.env`.

## Configuration

Copy `.env.example` to `.env` and set your router admin password:

```ini
TPLINK_HOST=192.168.0.1
TPLINK_PASSWORD=your_admin_password
BATTERY_THRESHOLD=20
CHECK_INTERVAL_SECONDS=60
NOTIFY_COOLDOWN_MINUTES=60
```

The `.env` file is git-ignored: **never commit your password**.

---

## Tray Companion App (Background Panel)

The final, distributable product is the standalone optimized Rust binary. It sits in your system tray (GNOME top bar indicator area via the AppIndicator extension, KDE system tray, etc.) and displays the TP-Link logo icon (embedded in the binary, no extra files) with a text label next to it: the battery percentage and, by default, the remaining GB. When the router is unreachable the logo is shown "powered off" (desaturated and darker) and the menu becomes minimal.

### Running the Rust binary (final product)

```bash
cargo build --release
./target/release/tplink-battery-monitor --tray

# Hide the battery percentage next to the icon (data only):
./target/release/tplink-battery-monitor --tray --no-battery-in-panel

# Show only the battery percentage next to the icon:
./target/release/tplink-battery-monitor --tray --no-data-in-panel
```

### On-Click Dropdown Menu (compact, with colored icons):
- **Header**: device model
- **Connection**: `WEB CoopVoce · 4G (LTE)`
- **Battery**: `47% · a batteria` followed by a full-width text bar with a green (or red below 20%) color dot
- **Data**: `134.6 / 400 GB · 33%` followed by a full-width text bar with a blue color dot
- **Today**: `Oggi 3.9 GB`
- **Network submenu**: signal bars (colored), connected devices and live speed
- **Actions**: `Aggiorna`, `Apri router`, `Risparmio energetico` (router power-save toggle, retried across the expected link drop)
- **Toggles** (persisted in `~/.config/tplink-battery-monitor/prefs.json`): `Percentuale batteria`, `Mostra GB`, `Notifiche`

> Note: GNOME menu entries cannot color text, so the big bars are monochrome and the color is shown by a small dot on the same row. The router drops the link briefly when toggling power saving (expected).

### Python prototype

`tplink_tray.py` is the reference prototype used during development; the Rust binary is the supported product.

```bash
python3 tplink_tray.py
python3 monitor.py --tray
```

### Autostart on login

```bash
# Standalone Rust binary
./target/release/tplink-battery-monitor --install-autostart
./target/release/tplink-battery-monitor --uninstall-autostart
```

Alternatively, a systemd user service unit is provided in `systemd/tplink-tray.service`.

---

## Terminal Overview (`--data`)

Both the Python and Rust implementations support the `--data` flag for a quick terminal glance:

```bash
python3 monitor.py --data
# or with the compiled Rust binary:
./target/release/tplink-battery-monitor --data
```

Example output:
```text
══════════════════════════════════════════════════
  TP-Link M7350 Status
══════════════════════════════════════════════════
  Batteria:       47% (a batteria)
──────────────────────────────────────────────────
  Dati usati:     133.86 GB / 400.00 GB
  Avanzamento:    [████░░░░░░░░] 33.5% usato
  Rimanenti:      266.14 GB
  Consumo oggi:   3.19 GB
──────────────────────────────────────────────────
  Rete:           WEB CoopVoce (4G (LTE))
  Segnale:        3/4 (75%)
  Dispositivi:    3 connessi
  Velocità:       ↓ 22.5 KB/s   ↑ 283.1 KB/s
══════════════════════════════════════════════════
```

---

## Rust: build and systemd user timer

```bash
cargo build --release
# binary at target/release/tplink-battery-monitor

# single check, no notifications (test)
./target/release/tplink-battery-monitor --once --no-notify

# data overview
./target/release/tplink-battery-monitor --data
```

### systemd user timer

```bash
mkdir -p ~/.local/bin ~/.config/tplink-battery-monitor ~/.config/systemd/user
cargo build --release
install -Dm755 target/release/tplink-battery-monitor ~/.local/bin/tplink-battery-monitor
cp .env ~/.config/tplink-battery-monitor/.env && chmod 600 ~/.config/tplink-battery-monitor/.env
cp systemd/tplink-battery-monitor.service systemd/tplink-battery-monitor.timer ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now tplink-battery-monitor.timer
```

---

## The `battery` field quirk

The router reports:
```json
"battery": { "connected": true, "charging": false, "voltage": 48 }
```
Despite the name `voltage`, the value is already a percentage (0–100). The web UI uses it directly to choose the battery icon, and both implementations expose it as `level`.

Data limits and usage (`limitation`, `totalStatistics`) are calculated in binary gigabytes (1024³ = 1 GiB), matching the router's web portal.

## Documentation

- [docs/protocol.md](docs/protocol.md) — HTTP API details.
- [docs/architecture.md](docs/architecture.md) — Software architecture.
- [CONTRIBUTING.md](CONTRIBUTING.md) — Development & contribution guide.
- [CHANGELOG.md](CHANGELOG.md) — Release notes.

## License

[MIT](LICENSE) © 2026 Gioele Rosana
