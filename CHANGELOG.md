# Changelog

All notable changes to this project are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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

[Unreleased]: https://github.com/JoelShepard/tplink-battery-monitor/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/JoelShepard/tplink-battery-monitor/releases/tag/v0.1.0
