# Changelog

All notable changes to this project are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.0.1] - 2026-10-23

First public release.

### Added

- AppImage with a graphical installer: opening it copies the app to
  `~/.local/bin`, asks for the router password and checks it against the
  router, registers a systemd user service started at every login (XDG
  autostart as fallback) and shows each step in a progress window, then a
  summary. Opening it again offers Aggiorna / Disinstalla. No application
  menu entry is created. Dialogs use zenity or kdialog, text in a terminal.
- System tray app (`--tray`) with the TP-Link logo and a label showing the
  battery percentage and remaining GB. The click menu shows battery and data
  bars, daily traffic, network, signal, devices, speed and a router
  power-saving toggle. The logo turns grey when the router is unreachable.
- Desktop notifications for low battery and for charge started, completed and
  interrupted, with cooldown and state kept across restarts.
- Terminal summary (`--data`), single check (`--once`), `--version`.
- Client for the router's undocumented JSON API (AES-128-CBC + chunked RSA-512
  + MD5 login), shipped as a single static binary with no runtime
  dependencies.

[Unreleased]: https://github.com/gioelerosana/tplink-battery-monitor/compare/v0.0.1...HEAD
[0.0.1]: https://github.com/gioelerosana/tplink-battery-monitor/releases/tag/v0.0.1
