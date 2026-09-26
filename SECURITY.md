# Security

## Handling of secrets

The only secret this project uses is the router's admin password.

- It is read from the `TPLINK_PASSWORD` environment variable or from a `.env`
  file. `.env` is listed in `.gitignore` and must never be committed.
- It is never written to the state file, the logs, or the notification text.
- It is used to derive `MD5("admin" + password)` and, together with the
  per-session nonce, `MD5(password + ":" + nonce)` for the login. These are
  one-way hashes; the plaintext password is never transmitted.

## Network

- The router API is plain HTTP on the local network. Do not expose the router's
  web interface to the internet, and run the monitor on a machine connected to
  the router's LAN.
- No data is sent to any third-party service; notifications are delivered
  locally (D-Bus) or via `notify-send`.

## Reporting a vulnerability

If you find a security issue, please open a GitHub issue (or contact the
maintainer privately for anything sensitive). Do not include real passwords,
session tokens, or `.env` contents in reports.
