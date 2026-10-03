#!/usr/bin/env python3
"""Companion App / Tray Monitor per router mobile TP-Link (es. M7350).

Mostra un'icona e la percentuale della batteria nel pannello di sistema (system tray).
Al clic, visualizza un menu dettagliato con:
  - Percentuale batteria e stato di carica
  - Dati consumati, rimanenti e limite del piano dati (con barra di avanzamento)
  - Consumo dati odierno
  - Rete, segnale, velocità di trasmissione e dispositivi connessi
  - Notifiche desktop per batteria bassa / eventi di carica
  - Accesso rapido all'interfaccia web del router (192.168.0.1)
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import signal
import subprocess
import sys
import threading
import time
import webbrowser
from datetime import datetime
from pathlib import Path
from typing import Any

import gi

gi.require_version("Gtk", "3.0")
try:
    gi.require_version("AppIndicator3", "0.1")
    from gi.repository import AppIndicator3
except (ValueError, ImportError):
    try:
        gi.require_version("AyatanaAppIndicator3", "0.1")
        from gi.repository import AyatanaAppIndicator3 as AppIndicator3
    except (ValueError, ImportError) as exc:
        sys.exit(f"Errore: AppIndicator3 o AyatanaAppIndicator3 non disponibile: {exc}")

from gi.repository import GLib, Gtk

from tplink_mifi import MiFiClient, TpLinkError, format_bytes, format_speed

STATE_FILE = Path(
    os.environ.get(
        "TP_BATTERY_STATE",
        Path.home() / ".cache" / "tplink-battery-monitor" / "state.json",
    )
)

AUTOSTART_DIR = Path.home() / ".config" / "autostart"
AUTOSTART_FILE = AUTOSTART_DIR / "tplink-tray.desktop"


def log(msg: str) -> None:
    print(f"[{datetime.now():%Y-%m-%d %H:%M:%S}] [Tray] {msg}", flush=True)


def load_dotenv(path: Path) -> None:
    """Carica un file .env nelle variabili d'ambiente."""
    if not path.is_file():
        return
    for line in path.read_text().splitlines():
        line = line.strip()
        if not line or line.startswith("#") or "=" not in line:
            continue
        key, _, value = line.partition("=")
        key = key.strip()
        value = value.strip().strip("'\"")
        os.environ.setdefault(key, value)


def read_state() -> dict:
    try:
        return json.loads(STATE_FILE.read_text())
    except (OSError, ValueError):
        return {}


def write_state(state: dict) -> None:
    try:
        STATE_FILE.parent.mkdir(parents=True, exist_ok=True)
        STATE_FILE.write_text(json.dumps(state))
    except OSError as exc:
        log(f"Impossibile salvare lo stato: {exc}")


def notify(title: str, message: str, urgency: str = "normal", icon: str = "battery") -> None:
    if not shutil.which("notify-send"):
        log(f"(notify-send non disponibile) {title}: {message}")
        return
    try:
        subprocess.run(
            ["notify-send", "-u", urgency, "-i", icon, title, message],
            check=False,
            timeout=5,
        )
    except Exception as exc:  # noqa: BLE001
        log(f"Invio notifica fallito: {exc}")


def choose_battery_icon(level: int | None, charging: bool | None, connected: bool = True) -> str:
    if not connected or level is None:
        return "battery-missing-symbolic"

    if charging:
        if level >= 90:
            return "battery-full-charging-symbolic"
        if level >= 40:
            return "battery-good-charging-symbolic"
        if level >= 15:
            return "battery-low-charging-symbolic"
        return "battery-caution-charging-symbolic"

    if level >= 90:
        return "battery-full-symbolic"
    if level >= 40:
        return "battery-good-symbolic"
    if level >= 15:
        return "battery-low-symbolic"
    return "battery-caution-symbolic"


class MiFiTrayApp:
    def __init__(
        self,
        client: MiFiClient,
        poll_interval: float = 60.0,
        battery_threshold: float = 20.0,
        cooldown_minutes: float = 60.0,
        notifications_enabled: bool = True,
        show_data_in_panel: bool = True,
        show_battery_in_panel: bool = True,
    ) -> None:
        self.client = client
        self.poll_interval = max(5.0, poll_interval)
        self.battery_threshold = battery_threshold
        self.cooldown_minutes = cooldown_minutes
        self.notifications_enabled = notifications_enabled
        self.show_data_in_panel = show_data_in_panel
        self.show_battery_in_panel = show_battery_in_panel

        self._refresh_trigger = threading.Event()
        self._running = True
        self._last_summary: dict[str, Any] | None = None
        self._updating = False

        self.indicator = AppIndicator3.Indicator.new(
            "tplink-battery-monitor",
            "battery-missing-symbolic",
            AppIndicator3.IndicatorCategory.APPLICATION_STATUS,
        )
        self.indicator.set_status(AppIndicator3.IndicatorStatus.ACTIVE)
        self.indicator.set_title("TP-Link MiFi Monitor")
        self.indicator.set_label(" ...", " 100%")

        self.menu = Gtk.Menu()
        self._build_menu()
        self.indicator.set_menu(self.menu)

        self._worker_thread = threading.Thread(target=self._worker_loop, daemon=True)
        self._worker_thread.start()

    def _build_menu(self) -> None:
        # Header: modello e stato
        self.item_header = Gtk.MenuItem(label="TP-Link MiFi")
        self.item_header.set_sensitive(False)
        self.menu.append(self.item_header)

        self.item_status = Gtk.MenuItem(label="Connessione in corso...")
        self.item_status.set_sensitive(False)
        self.menu.append(self.item_status)

        self.menu.append(Gtk.SeparatorMenuItem())

        # Batteria
        self.item_battery = Gtk.MenuItem(label="Batteria: --")
        self.item_battery.set_sensitive(False)
        self.menu.append(self.item_battery)

        self.menu.append(Gtk.SeparatorMenuItem())

        # Dati e traffico
        self.item_data_used = Gtk.MenuItem(label="Dati consumati: --")
        self.item_data_used.set_sensitive(False)
        self.menu.append(self.item_data_used)

        self.item_data_progress = Gtk.MenuItem(label="   [------------] --%")
        self.item_data_progress.set_sensitive(False)
        self.menu.append(self.item_data_progress)

        self.item_data_remaining = Gtk.MenuItem(label="⏳ Dati rimanenti: --")
        self.item_data_remaining.set_sensitive(False)
        self.menu.append(self.item_data_remaining)

        self.item_data_daily = Gtk.MenuItem(label="Consumo oggi: --")
        self.item_data_daily.set_sensitive(False)
        self.menu.append(self.item_data_daily)

        self.menu.append(Gtk.SeparatorMenuItem())

        # Rete e stato WAN
        self.item_network = Gtk.MenuItem(label="Rete: --")
        self.item_network.set_sensitive(False)
        self.menu.append(self.item_network)

        self.item_signal = Gtk.MenuItem(label="Segnale: --")
        self.item_signal.set_sensitive(False)
        self.menu.append(self.item_signal)

        self.item_devices = Gtk.MenuItem(label="Dispositivi connessi: --")
        self.item_devices.set_sensitive(False)
        self.menu.append(self.item_devices)

        self.item_speed = Gtk.MenuItem(label="Velocità: --")
        self.item_speed.set_sensitive(False)
        self.menu.append(self.item_speed)

        self.menu.append(Gtk.SeparatorMenuItem())

        # Azioni e preferenze
        self.item_refresh = Gtk.MenuItem(label="Aggiorna ora")
        self.item_refresh.connect("activate", self._on_refresh_clicked)
        self.menu.append(self.item_refresh)

        self.item_open_web = Gtk.MenuItem(label="Apri interfaccia router")
        self.item_open_web.connect("activate", self._on_open_web_clicked)
        self.menu.append(self.item_open_web)

        self.item_toggle_battery = Gtk.CheckMenuItem(label="Percentuale batteria")
        self.item_toggle_battery.set_active(self.show_battery_in_panel)
        self.item_toggle_battery.connect("toggled", self._on_toggle_battery)
        self.menu.append(self.item_toggle_battery)

        self.item_toggle_panel_data = Gtk.CheckMenuItem(label="Mostra GB")
        self.item_toggle_panel_data.set_active(self.show_data_in_panel)
        self.item_toggle_panel_data.connect("toggled", self._on_toggle_panel_data)
        self.menu.append(self.item_toggle_panel_data)

        self.item_toggle_notify = Gtk.CheckMenuItem(label="Notifiche")
        self.item_toggle_notify.set_active(self.notifications_enabled)
        self.item_toggle_notify.connect("toggled", self._on_toggle_notify)
        self.menu.append(self.item_toggle_notify)

        self.menu.append(Gtk.SeparatorMenuItem())

        # Esci
        self.item_quit = Gtk.MenuItem(label="Esci")
        self.item_quit.connect("activate", self._on_quit_clicked)
        self.menu.append(self.item_quit)

        self.menu.show_all()

    def _worker_loop(self) -> None:
        """Loop in background per interrogare l'API senza bloccare la GUI."""
        while self._running:
            self._updating = True
            try:
                summary = self.client.summary()
                GLib.idle_add(self._apply_summary, summary)
            except Exception as exc:  # noqa: BLE001
                GLib.idle_add(self._apply_error, str(exc))
            finally:
                self._updating = False

            # Attende l'intervallo oppure un trigger manuale
            self._refresh_trigger.wait(timeout=self.poll_interval)
            self._refresh_trigger.clear()

    def _apply_summary(self, summary: dict[str, Any]) -> bool:
        self._last_summary = summary
        model = summary.get("model", "M7350")
        battery = summary.get("battery", {})
        data = summary.get("data", {})
        connected_devs = summary.get("connected_devices", 0)

        level = battery.get("level")
        charging = battery.get("charging", False)
        connected = battery.get("connected", True)

        # Aggiorna icona
        icon_name = choose_battery_icon(level, charging, connected)
        icon_desc = f"Batteria {level}%" if level is not None else "TP-Link"
        if hasattr(self.indicator, "set_icon_full"):
            self.indicator.set_icon_full(icon_name, icon_desc)
        else:
            self.indicator.set_icon(icon_name)

        # Aggiorna etichetta nel pannello (top bar)
        batt_str = f"{level}%" if level is not None else "--%"
        if charging:
            batt_str = f"{batt_str}+"

        data_str = ""
        if self.show_data_in_panel:
            if data.get("has_limit") and data.get("remaining_formatted"):
                data_str = f"{data.get('remaining_formatted')} rim."
            elif data.get("total_formatted"):
                data_str = data.get("total_formatted", "")

        parts = []
        if self.show_battery_in_panel:
            parts.append(batt_str)
        if data_str:
            parts.append(data_str)

        if parts:
            self.indicator.set_label(" " + " · ".join(parts), " 100% · 999.99 GB rim.")
        else:
            self.indicator.set_label("", " 100% · 999.99 GB rim.")

        # Aggiorna voci del menu
        op_name = data.get("operator") or "TP-Link"
        self.item_header.set_label(f"TP-Link {model}")
        now_time = datetime.now().strftime("%H:%M")
        self.item_status.set_label(f"Connesso • {op_name} (agg. {now_time})")

        charge_text = "in carica" if charging else "a batteria"
        self.item_battery.set_label(f"Batteria: {level}% ({charge_text})")

        # Dati consumati e rimanenti
        tot_fmt = data.get("total_formatted", "--")
        lim_fmt = data.get("limit_formatted", "--")
        has_limit = data.get("has_limit", False)
        if has_limit:
            pct_str = f"{data.get('usage_percent', 0):.1f}%"
            self.item_data_used.set_label(f"Dati usati: {tot_fmt} / {lim_fmt} ({pct_str})")
            pbar = data.get("progress_bar", "")
            self.item_data_progress.set_label(f"   {pbar} usato")
            self.item_data_progress.set_visible(True)
            rem_fmt = data.get("remaining_formatted", "--")
            self.item_data_remaining.set_label(f"Dati rimanenti: {rem_fmt}")
            self.item_data_remaining.set_visible(True)
        else:
            self.item_data_used.set_label(f"Dati usati: {tot_fmt}")
            self.item_data_progress.set_visible(False)
            self.item_data_remaining.set_visible(False)

        daily_fmt = data.get("daily_formatted", "--")
        self.item_data_daily.set_label(f"Consumo oggi: {daily_fmt}")

        # Info Rete e Segnale
        net_type = data.get("network_type", "--")
        self.item_network.set_label(f"Rete: {op_name} ({net_type})")

        sig_val = data.get("signal_strength", 0)
        sig_pct = data.get("signal_percent", 0)
        self.item_signal.set_label(f"Segnale: {sig_val}/4 ({sig_pct}%)")

        self.item_devices.set_label(f"Dispositivi connessi: {connected_devs}")

        rx = data.get("rx_speed_formatted", "--")
        tx = data.get("tx_speed_formatted", "--")
        self.item_speed.set_label(f"Velocità: ↓ {rx}   ↑ {tx}")

        # Controlla notifiche batteria
        if self.notifications_enabled and level is not None:
            self._handle_battery_notifications(level, charging)

        return False

    def _apply_error(self, err_msg: str) -> bool:
        if hasattr(self.indicator, "set_icon_full"):
            self.indicator.set_icon_full("battery-missing-symbolic", "Router non raggiungibile")
        else:
            self.indicator.set_icon("battery-missing-symbolic")
        self.indicator.set_label(" ?", " 100%")
        self.item_status.set_label("Router non raggiungibile")
        log(f"Errore connessione router: {err_msg}")
        return False

    def _handle_battery_notifications(self, level: int, charging: bool) -> None:
        state = read_state()
        was_low = bool(state.get("low"))
        low = (level < self.battery_threshold) and not charging
        now = time.time()

        known_charging = state.get("charging")
        if isinstance(known_charging, bool):
            if not known_charging and charging:
                notify(
                    "Router: carica avviata",
                    f"Batteria al {level}%. Caricabatterie collegato.",
                    urgency="normal",
                    icon="battery-charging",
                )
                state["full_notified"] = False
            elif known_charging and charging:
                if level >= 100 and not state.get("full_notified"):
                    notify(
                        "Router: carica completa",
                        "Batteria al 100%.",
                        urgency="normal",
                        icon="battery-full",
                    )
                    state["full_notified"] = True
            elif known_charging and not charging:
                if level < 100:
                    notify(
                        "Router: carica interrotta",
                        f"La carica si è fermata al {level}%.",
                        urgency="normal",
                        icon="battery-low",
                    )
                state["full_notified"] = False
        else:
            state["full_notified"] = False

        if low:
            last_notify = float(state.get("last_notify") or 0)
            due = (now - last_notify) >= self.cooldown_minutes * 60
            if not was_low or due:
                notify(
                    f"Router: batteria al {level}%",
                    f"Livello sotto la soglia ({self.battery_threshold:.0f}%). Collega il caricabatterie!",
                    urgency="critical",
                    icon="battery-caution",
                )
                state["last_notify"] = now
        elif was_low and not charging and level >= self.battery_threshold:
            notify(
                f"Router: batteria al {level}%",
                "Livello tornato sopra la soglia di sicurezza.",
                urgency="normal",
                icon="battery-good",
            )

        state["charging"] = charging
        state["low"] = low
        state["level"] = level
        write_state(state)

    def _on_refresh_clicked(self, _item: Gtk.MenuItem) -> None:
        log("Aggiornamento manuale richiesto...")
        self.item_status.set_label("Aggiornamento in corso...")
        self._refresh_trigger.set()

    def _on_open_web_clicked(self, _item: Gtk.MenuItem) -> None:
        url = self.client.host
        log(f"Apertura interfaccia web: {url}")
        webbrowser.open(url)

    def _on_toggle_battery(self, item: Gtk.CheckMenuItem) -> None:
        self.show_battery_in_panel = item.get_active()
        if self._last_summary:
            self._apply_summary(self._last_summary)

    def _on_toggle_panel_data(self, item: Gtk.CheckMenuItem) -> None:
        self.show_data_in_panel = item.get_active()
        if self._last_summary:
            self._apply_summary(self._last_summary)

    def _on_toggle_notify(self, item: Gtk.CheckMenuItem) -> None:
        self.notifications_enabled = item.get_active()
        log(f"Notifiche desktop impostate a: {self.notifications_enabled}")

    def _on_quit_clicked(self, _item: Gtk.MenuItem) -> None:
        log("Chiusura applicazione.")
        self._running = False
        self._refresh_trigger.set()
        Gtk.main_quit()


def install_autostart() -> None:
    AUTOSTART_DIR.mkdir(parents=True, exist_ok=True)
    script_path = Path(__file__).resolve()
    desktop_content = f"""[Desktop Entry]
Type=Application
Name=TP-Link MiFi Tray Monitor
Comment=Monitor della batteria e consumo dati del router TP-Link M7350
Exec={sys.executable} {script_path}
Icon=battery-good-symbolic
Terminal=false
Categories=Utility;System;
StartupNotify=false
X-GNOME-Autostart-enabled=true
"""
    AUTOSTART_FILE.write_text(desktop_content)
    AUTOSTART_FILE.chmod(0o755)
    print(f"File di avvio automatico creato con successo in: {AUTOSTART_FILE}")


def uninstall_autostart() -> None:
    if AUTOSTART_FILE.is_file():
        AUTOSTART_FILE.unlink()
        print(f"File di avvio automatico rimosso da: {AUTOSTART_FILE}")
    else:
        print("Nessun file di avvio automatico presente.")


def main() -> int:
    load_dotenv(Path(__file__).with_name(".env"))

    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--host", default=os.environ.get("TPLINK_HOST", "192.168.0.1"))
    parser.add_argument(
        "--password",
        default=os.environ.get("TPLINK_PASSWORD", ""),
        help="password admin del router (o variabile TPLINK_PASSWORD)",
    )
    parser.add_argument(
        "--interval",
        type=float,
        default=float(os.environ.get("CHECK_INTERVAL_SECONDS", "60")),
        help="secondi tra gli aggiornamenti automatici (default 60)",
    )
    parser.add_argument(
        "--threshold",
        type=float,
        default=float(os.environ.get("BATTERY_THRESHOLD", "20")),
        help="soglia percentuale batteria per notifiche (default 20)",
    )
    parser.add_argument(
        "--cooldown",
        type=float,
        default=float(os.environ.get("NOTIFY_COOLDOWN_MINUTES", "60")),
        help="minuti minimi tra notifiche di batteria bassa (default 60)",
    )
    parser.add_argument(
        "--show-data-in-panel",
        dest="show_data_in_panel",
        action="store_true",
        help="mostra i giga accanto all'icona nel pannello (default)",
    )
    parser.add_argument(
        "--no-data-in-panel",
        dest="show_data_in_panel",
        action="store_false",
        help="mostra solo la percentuale accanto all'icona",
    )
    parser.set_defaults(
        show_data_in_panel=os.environ.get("SHOW_DATA_IN_PANEL", "1").strip().lower()
        not in ("0", "false", "no", "off", ""),
    )
    parser.add_argument(
        "--show-battery-in-panel",
        dest="show_battery_in_panel",
        action="store_true",
        help="mostra la percentuale batteria accanto all'icona (default)",
    )
    parser.add_argument(
        "--no-battery-in-panel",
        dest="show_battery_in_panel",
        action="store_false",
        help="nasconde la percentuale batteria accanto all'icona",
    )
    parser.set_defaults(
        show_battery_in_panel=os.environ.get("SHOW_BATTERY_IN_PANEL", "1")
        .strip()
        .lower()
        not in ("0", "false", "no", "off", ""),
    )
    parser.add_argument(
        "--no-notify",
        action="store_true",
        help="disabilita le notifiche desktop",
    )
    parser.add_argument(
        "--install-autostart",
        action="store_true",
        help="installa l'avvio automatico al login di sistema (~/.config/autostart)",
    )
    parser.add_argument(
        "--uninstall-autostart",
        action="store_true",
        help="rimuove l'avvio automatico al login di sistema",
    )

    args = parser.parse_args()

    if args.install_autostart:
        install_autostart()
        return 0

    if args.uninstall_autostart:
        uninstall_autostart()
        return 0

    if not args.password:
        parser.error("specifica --password oppure imposta TPLINK_PASSWORD nel .env")

    client = MiFiClient(host=args.host, password=args.password)

    # Gestione segnali Unix per terminare pulitamente (Ctrl+C)
    signal.signal(signal.SIGINT, lambda *_: Gtk.main_quit())
    signal.signal(signal.SIGTERM, lambda *_: Gtk.main_quit())
    GLib.timeout_add(500, lambda: True)  # permette a Python di intercettare SIGINT

    app = MiFiTrayApp(
        client=client,
        poll_interval=args.interval,
        battery_threshold=args.threshold,
        cooldown_minutes=args.cooldown,
        notifications_enabled=not args.no_notify,
        show_data_in_panel=args.show_data_in_panel,
        show_battery_in_panel=args.show_battery_in_panel,
    )

    log(f"Avvio tray monitor per {args.host} (intervallo: {args.interval:g}s)")
    Gtk.main()
    return 0


if __name__ == "__main__":
    sys.exit(main())
