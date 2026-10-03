#!/usr/bin/env python3
"""Monitor della batteria e consumo dati del router TP-Link M7350 con notifiche desktop e tray app.

Esegue il polling dell'API del router e invia una notifica (notify-send)
quando la batteria scende sotto una soglia (default 20%) e non e' in carica.

Uso:
    python monitor.py                 # loop continuo da terminale / background
    python monitor.py --tray          # avvia l'app companion nella barra di sistema (tray / pannello)
    python monitor.py --data          # visualizza giga consumati e rimanenti ed esci
    python monitor.py --once          # un singolo controllo (per cron)
    python monitor.py --no-notify     # solo log, senza notifiche
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
import time
from datetime import datetime
from pathlib import Path

from tplink_mifi import MiFiClient, TpLinkError

STATE_FILE = Path(
    os.environ.get(
        "TP_BATTERY_STATE",
        Path.home() / ".cache" / "tplink-battery-monitor" / "state.json",
    )
)


def log(message: str) -> None:
    print(f"[{datetime.now():%Y-%m-%d %H:%M:%S}] {message}", flush=True)


def load_dotenv(path: Path) -> None:
    """Carica un file .env minimale nelle variabili d'ambiente."""
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
        log(f"(notify-send assente) {title}: {message}")
        return
    try:
        subprocess.run(
            ["notify-send", "-u", urgency, "-i", icon, title, message],
            check=True,
            timeout=10,
        )
    except (subprocess.SubprocessError, OSError) as exc:
        log(f"Invio notifica fallito: {exc}")


def check_once(
    client: MiFiClient,
    threshold: float,
    notify_enabled: bool,
    cooldown_minutes: float,
    notify_recovery: bool,
) -> None:
    try:
        battery = client.battery()
    except TpLinkError as exc:
        log(f"Router non raggiungibile: {exc}")
        return

    level = battery.get("level")
    charging = battery.get("charging")
    if level is None:
        log(f"Livello batteria non disponibile: {battery}")
        return

    log(
        f"Batteria {level}% "
        f"({'in carica' if charging else 'a batteria'}), modello {battery.get('model')}"
    )

    state = read_state()
    was_low = bool(state.get("low"))
    low = level < threshold and not charging
    now = time.time()

    # --- notifiche legate alla carica (transizioni di stato) --------------
    known_charging = state.get("charging")
    if isinstance(known_charging, bool):
        if not known_charging and charging:
            if notify_enabled:
                notify(
                    "Router M7350: carica avviata",
                    f"Batteria al {level}%. Caricabatterie collegato.",
                    urgency="normal",
                    icon="battery-charging",
                )
            log(f"Carica avviata ({level}%)")
            state["full_notified"] = False
        elif known_charging and charging:
            if level >= 100 and not state.get("full_notified"):
                if notify_enabled:
                    notify(
                        "Router M7350: carica completa",
                        "Batteria al 100%.",
                        urgency="normal",
                        icon="battery-full",
                    )
                log("Carica completa (100%)")
                state["full_notified"] = True
        elif known_charging and not charging:
            if level >= 100:
                log(f"Caricabatterie rimosso a carica completa ({level}%)")
            else:
                if notify_enabled:
                    notify(
                        "Router M7350: carica interrotta",
                        f"La carica si è fermata al {level}%.",
                        urgency="normal",
                        icon="battery-low",
                    )
                log(f"Carica interrotta ({level}%)")
            state["full_notified"] = False
    else:
        # primo avvio (stato senza "charging"): nessuna transizione da notificare
        state["full_notified"] = False

    # --- batteria bassa / recuperata --------------------------------------
    if low:
        last_notify = float(state.get("last_notify") or 0)
        due = (now - last_notify) >= cooldown_minutes * 60
        if not was_low or due:
            if notify_enabled:
                notify(
                    f"Router M7350: batteria al {level}%",
                    "Livello sotto la soglia. Collega il caricabatterie!",
                    urgency="critical",
                    icon="battery-caution",
                )
            log(f"NOTIFICA: batteria bassa ({level}% < {threshold:.0f}%)")
            state["last_notify"] = now
    elif was_low and not charging and level >= threshold:
        if notify_recovery and notify_enabled:
            notify(
                f"Router M7350: batteria al {level}%",
                "Livello tornato sopra la soglia.",
                urgency="normal",
                icon="battery-good",
            )
        log(f"Batteria recuperata ({level}%)")

    state["charging"] = charging
    state["low"] = low
    state["level"] = level
    write_state(state)


def print_data_overview(client: MiFiClient) -> None:
    try:
        summary = client.summary()
    except TpLinkError as exc:
        sys.exit(f"Errore connessione router: {exc}")

    batt = summary["battery"]
    data = summary["data"]
    charge_str = "in carica" if batt.get("charging") else "a batteria"

    print("══════════════════════════════════════════════════")
    print(f"  TP-Link {summary.get('model', 'M7350')} Status")
    print("══════════════════════════════════════════════════")
    print(f"  Batteria:       {batt.get('level')}% ({charge_str})")
    print("──────────────────────────────────────────────────")
    print(f"  Dati usati:     {data.get('total_formatted')} / {data.get('limit_formatted')}")
    if data.get("has_limit"):
        print(f"  Avanzamento:    {data.get('progress_bar')} usato")
        print(f"  Rimanenti:      {data.get('remaining_formatted')}")
    print(f"  Consumo oggi:   {data.get('daily_formatted')}")
    print("──────────────────────────────────────────────────")
    print(f"  Rete:           {data.get('operator')} ({data.get('network_type')})")
    print(f"  Segnale:        {data.get('signal_strength')}/4 ({data.get('signal_percent')}%)")
    print(f"  Dispositivi:    {summary.get('connected_devices')} connessi")
    print(f"  Velocità:       ↓ {data.get('rx_speed_formatted')}   ↑ {data.get('tx_speed_formatted')}")
    print("══════════════════════════════════════════════════")


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
        "--threshold",
        type=float,
        default=float(os.environ.get("BATTERY_THRESHOLD", "20")),
        help="soglia percentuale (default 20)",
    )
    parser.add_argument(
        "--interval",
        type=float,
        default=float(os.environ.get("CHECK_INTERVAL_MINUTES", "5")),
        help="minuti tra i controlli (default 5)",
    )
    parser.add_argument(
        "--cooldown",
        type=float,
        default=float(os.environ.get("NOTIFY_COOLDOWN_MINUTES", "60")),
        help="minuti minimi tra due notifiche di batteria bassa (default 60)",
    )
    parser.add_argument("--once", action="store_true", help="un solo controllo ed esci")
    parser.add_argument("--no-notify", action="store_true", help="non inviare notifiche")
    parser.add_argument(
        "--notify-recovery",
        action="store_true",
        help="notifica anche quando la batteria torna sopra la soglia",
    )
    parser.add_argument(
        "--data",
        action="store_true",
        help="mostra un riepilogo del consumo giga e della batteria nel terminale ed esci",
    )
    parser.add_argument(
        "--tray",
        action="store_true",
        help="avvia l'app companion con icona nella barra di sistema / background panel",
    )
    args = parser.parse_args()

    if not args.password:
        parser.error("specifica --password oppure imposta TPLINK_PASSWORD nel .env")

    client = MiFiClient(host=args.host, password=args.password)

    if args.data:
        print_data_overview(client)
        return 0

    if args.tray:
        from tplink_tray import MiFiTrayApp
        import gi
        from gi.repository import GLib, Gtk
        import signal

        signal.signal(signal.SIGINT, lambda *_: Gtk.main_quit())
        signal.signal(signal.SIGTERM, lambda *_: Gtk.main_quit())
        GLib.timeout_add(500, lambda: True)

        _app = MiFiTrayApp(
            client=client,
            poll_interval=args.interval * 60,
            battery_threshold=args.threshold,
            cooldown_minutes=args.cooldown,
            notifications_enabled=not args.no_notify,
        )
        log(f"Avvio tray monitor (intervallo: {args.interval:g}min)")
        Gtk.main()
        return 0

    notify_enabled = not args.no_notify

    if args.once:
        check_once(
            client, args.threshold, notify_enabled, args.cooldown, args.notify_recovery
        )
        return 0

    log(
        f"Monitor avviato: host={args.host} soglia={args.threshold:.0f}% "
        f"intervallo={args.interval:g}min"
    )
    while True:
        try:
            check_once(
                client, args.threshold, notify_enabled, args.cooldown, args.notify_recovery
            )
        except KeyboardInterrupt:
            log("Interrotto.")
            return 0
        except Exception as exc:  # noqa: BLE001
            log(f"Errore inatteso: {exc!r}")
        try:
            time.sleep(args.interval * 60)
        except KeyboardInterrupt:
            log("Interrotto.")
            return 0


if __name__ == "__main__":
    sys.exit(main())
