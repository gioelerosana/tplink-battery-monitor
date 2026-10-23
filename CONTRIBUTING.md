# Contributing

Issues and pull requests are welcome.

## Ground rules

- **Never commit secrets.** The router password lives in `.env`, which is
  git-ignored. Do not hard-code credentials, even in tests or examples.
- Keep the API client (`src/tplink_mifi.rs`) free of notification/threshold
  logic, and the monitor (`src/main.rs`) free of crypto/HTTP details.
- No new runtime dependencies: the product is a single static binary.

## Build and run

```bash
cargo build --release
./target/release/tplink-battery-monitor --data

# AppImage (static musl build, output in dist/)
./packaging/build-appimage.sh
```

## Checks before opening a PR

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --release
```

## Releasing

Bump `version` in `Cargo.toml`, move the `Unreleased` changelog entries under
`## [X.Y.Z] - YYYY-MM-DD`, commit, then push a matching tag:

```bash
git tag v0.0.2
git push origin main v0.0.2
```

The `Release` workflow checks that the tag matches `Cargo.toml`, runs the
tests, builds the AppImage and publishes a GitHub release with it.

## Testing without hardware

The monitor's decision logic only needs a battery reading. You can simulate
charge transitions by editing the state file
(`~/.cache/tplink-battery-monitor/state.json`, or `TP_BATTERY_STATE`) and then
running with `--once --no-notify`. For example, set `"charging": true` while
the router is not charging, and the next run logs `Carica interrotta`.

## Reporting issues

Include the router model and firmware version, your distro and desktop, and
relevant log lines (`journalctl --user -u tplink-battery-monitor`). Do **not**
paste your admin password, the session token, or the contents of `.env`.
