# Contributing

Thanks for your interest! This is a small project; issues and pull requests are
welcome.

## Ground rules

- **Never commit secrets.** The router password lives in `.env`, which is
  git-ignored. Do not hard-code credentials, even in tests or examples.
- Keep the **Rust and Python implementations in sync**. They are intended to be
  a 1:1 translation of the same behaviour and wire format. If you change the
  protocol handling or the notification state machine in one, change the other
  too.
- Keep the API client (`tplink_mifi`) free of notification/threshold logic, and
  the monitor free of crypto/HTTP details.

## Prerequisites

- Rust stable (`cargo`)
- Python 3.9+ with `cryptography`

## Build and run

```bash
# Rust
cargo build --release
./target/release/tplink-battery-monitor --once --no-notify

# Python
pip install -r requirements.txt
python3 monitor.py --once --no-notify
```

## Checks before opening a PR

```bash
# Rust
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo build --release

# Python
python3 -m py_compile tplink_mifi.py monitor.py
```

The CI workflow (`.github/workflows/ci.yml`) runs the same checks.

## Testing without hardware

The monitor's decision logic only needs a battery reading. You can simulate
charge transitions by editing the state file directly
(`~/.cache/tplink-battery-monitor/state.json`, or `TP_BATTERY_STATE`) and then
running with `--once --no-notify` to observe the logs. For example, set
`"charging": true` while the router is not charging, and the next run will log
`Carica interrotta`.

## Reporting issues

When opening an issue, please include:

- router model and firmware version (the `status` response has
  `deviceInfo.model` / `deviceInfo.firmwareVer`),
- the OS and how you run the tool (systemd timer, cron, manual),
- relevant log lines.

Do **not** paste your admin password, the session token, or the contents of
`.env`.
