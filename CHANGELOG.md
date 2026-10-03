# Changelog

All notable changes to this project are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Tray companion app for the standalone Rust binary (`--tray`): system tray
  icon (embedded TP-Link logo, no runtime files), on-click menu with battery,
  data usage/remaining, daily traffic, network, signal, devices and speed,
  plus D-Bus desktop notifications.
- Colored PNG icons generated at runtime for the menu (minimal dependency-free
  PNG encoder in `src/icons.rs`): battery bar, data usage bar and signal bars.
- "Powered off" look: the TP-Link logo is desaturated and darkened when the
  router is unreachable, and the menu switches to a minimal layout.
- Persistent panel toggles (`Percentuale batteria`, `Mostra GB`, `Notifiche`)
  saved in `~/.config/tplink-battery-monitor/prefs.json`.
- Text label next to the tray icon (Ayatana `XAyatanaLabel`) showing the
  battery percentage and, by default, the remaining GB; toggle with
  `--show-data-in-panel` / `--no-data-in-panel`.
- `--install-autostart` / `--uninstall-autostart` to register the companion at
  graphical login.
- Vendored `ksni` copy under `vendor/ksni` patched to expose the Ayatana
  `XAyatanaLabel` property (not supported upstream).
- Python tray prototype (`tplink_tray.py`).

## [0.1.0] - 2026-09-26

### Added

- Reverse-engineered client for the TP-Link MiFi JSON API (`/cgi-bin/auth_cgi`,
  `/cgi-bin/web_cgi`) handling the AES-128-CBC + chunked RSA-512 + MD5 "GDPR"
  authentication flow.
- Battery monitoring with a configurable threshold (default 20%).
- Low-battery notification, suppressed while charging.
- Charge-event notifications: charge started, charge complete (100%), charge
  interrupted (unplugged before 100%).
- Notification cooldown and persistent state across runs.
- Rust implementation producing a single binary, with native D-Bus
  notifications (`notify-rust`).
- Python implementation with `notify-send` notifications.
- systemd user `oneshot` service + 5-minute timer.
- Documentation: protocol write-up and architecture notes.

[Unreleased]: https://github.com/gioelerosana/tplink-battery-monitor/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/gioelerosana/tplink-battery-monitor/releases/tag/v0.1.0
