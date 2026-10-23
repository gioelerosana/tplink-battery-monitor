<p align="center">
  <img src="assets/icon.png" width="96" alt="TP-Link logo">
</p>

<h1 align="center">TP-Link MiFi Monitor</h1>

<p align="center">
  Battery and data usage of your TP-Link mobile router, always visible in the system tray.<br>
  <a href="README.it.md">Italiano</a> · <a href="LICENSE">MIT</a>
</p>

---

A small background app for Linux that shows the router's **battery percentage**
and **remaining GB** next to the TP-Link icon in your panel. Click it for data
usage, signal, connected devices, speed and a power-saving toggle. You also get
desktop notifications for low battery and charging events.

It talks directly to the router over your Wi-Fi. No cloud, no account.

Tested on the **M7350 v9** (firmware 9.0.5). Other models using the same web API
(M7000, M7200) should work.

## Install

1. Download [`tplink-battery-monitor-x86_64.AppImage`](https://github.com/gioelerosana/tplink-battery-monitor/releases/latest/download/tplink-battery-monitor-x86_64.AppImage)
   from the [latest release](https://github.com/gioelerosana/tplink-battery-monitor/releases/latest).
2. Make it executable (file properties, or `chmod +x`) and double-click it.
3. Click **Installa** and enter the router admin password.

A progress window shows each step: the app copies itself to `~/.local/bin`,
checks the password with the router, registers a background service that
starts at every login and puts the icon in the tray. A final window tells you
where everything went. Nothing is added to the application menu, and you can
delete the downloaded file afterwards.

To update, open the new AppImage the same way and click **Aggiorna**. To
remove it, open the AppImage again and click **Disinstalla** (or run
`~/.local/bin/tplink-battery-monitor --uninstall`).

The dialogs use `zenity` (GNOME) or `kdialog` (KDE). If neither is installed
you get desktop notifications instead, and from a terminal everything is text.

> GNOME needs the [AppIndicator extension](https://extensions.gnome.org/extension/615/appindicator-support/)
> to show tray icons. KDE, Cinnamon, XFCE and most other desktops work out of the box.

## Configuration

Settings live in `~/.config/tplink-battery-monitor/.env`
(see [.env.example](.env.example) for all options):

```ini
TPLINK_HOST=192.168.0.1
TPLINK_PASSWORD=your_admin_password
BATTERY_THRESHOLD=20
```

After editing, restart the service:
`systemctl --user restart tplink-battery-monitor`.

## Terminal use

```bash
tplink-battery-monitor --data      # one-shot summary of battery, data and network
tplink-battery-monitor --help      # all options
```

## Build from source

```bash
cargo build --release              # plain binary in target/release/
./packaging/build-appimage.sh      # static AppImage in dist/
```

## More

- [How the router API works](docs/protocol.md)
- [Architecture](docs/architecture.md)
- [Contributing](CONTRIBUTING.md) · [Changelog](CHANGELOG.md)

Not affiliated with TP-Link. The logo belongs to its owner.
