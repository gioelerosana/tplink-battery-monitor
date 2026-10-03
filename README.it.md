# TP-Link M7350 Battery & Data Monitor

[![CI](https://github.com/gioelerosana/tplink-battery-monitor/actions/workflows/ci.yml/badge.svg)](https://github.com/gioelerosana/tplink-battery-monitor/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

Companion app con icona nella barra di sistema (system tray / pannello in background) e monitor con notifiche desktop per router mobile TP-Link (es. M7350).
Fornisce a colpo d'occhio:
- **Percentuale batteria** e stato di ricarica
- **Giga consumati** in relazione ai **giga rimanenti** e al piano mensile (con barra di avanzamento)
- **Consumo dati odierno**
- **Stato rete mobile** (operatore, segnale LTE, velocità tx/rx e dispositivi connessi)

> [Read the English README](README.md)

Il firmware del router espone un'API JSON non documentata, usata dalla sua stessa interfaccia web. Questo progetto fa reverse engineering di quell'API, reimplementa il flusso di autenticazione AES + RSA + MD5 e fornisce sia una **Tray Companion App** grafica sia strumenti CLI in Python e Rust:

| Componente | File | Descrizione |
| --- | --- | --- |
| **Tray Companion App** | `tplink_tray.py` | Icona e percentuale sempre visibili nel pannello, menu dettagliato al clic |
| **Monitor CLI Python** | `monitor.py` | Polling e notifiche desktop (`notify-send`), comando rapido `--data` |
| **Client API Python** | `tplink_mifi.py` | Libreria API riutilizzabile (batteria, dati, stato, reboot) |
| **Monitor Binario Rust** | `src/tplink_mifi.rs`, `src/main.rs` | Binario finale standalone: companion tray (`--tray`), timer systemd, notifiche D-Bus native |

Testato su **TP-Link M7350 (EU) v9.0**, firmware `9.0.5`. Compatibile con i modelli MiFi TP-Link basati su `/cgi-bin/web_cgi` (es. famiglia M7000 / M7200 / M7350).

## Funzionalità

- **Monitoraggio Batteria**: percentuale esatta nel pannello di sistema ed allarmi sonori/visivi sotto soglia (default 20%).
- **Monitoraggio Dati & Traffico**: visualizzazione immediata dei giga usati, giga rimanenti, limite del piano e consumo del giorno.
- **Eventi di Carica Intelligenti**:
  - Carica avviata,
  - Carica completata (100%),
  - Carica interrotta (cavo rimosso prima del 100%).
- **Info Connessione**: operatore mobile, tipo di rete (4G/LTE), potenza segnale (0-4), velocità attuale e numero dispositivi connessi.
- **Accesso Rapido**: apertura diretta del pannello di amministrazione (192.168.0.1) dal menu.
- **Zero Cloud**: comunicazione diretta LAN/Wi-Fi con il router, credenziali solo nel tuo `.env` locale.

## Configurazione

Copia `.env.example` in `.env` e imposta la password di amministrazione:

```ini
TPLINK_HOST=192.168.0.1
TPLINK_PASSWORD=la_tua_password_admin
BATTERY_THRESHOLD=20
CHECK_INTERVAL_SECONDS=60
NOTIFY_COOLDOWN_MINUTES=60
```

Il file `.env` è escluso da git: **non committare mai la password del router**.

---

## Tray Companion App (Pannello in Background)

Il prodotto finale e distribuito e' il binario Rust standalone ottimizzato. Si posiziona nella barra di sistema (area indicatori / quick settings di GNOME tramite l'estensione AppIndicator, system tray di KDE, ecc.) ed espone l'icona del logo TP-Link (incorporata nel binario, nessun file esterno) con un'etichetta testuale accanto: percentuale della batteria e, di default, i giga rimanenti. Quando il router non e' raggiungibile il logo viene mostrato "spento" (desaturato e piu' scuro) e il menu diventa minimale.

### Avvio del binario Rust (prodotto finale)

```bash
cargo build --release
./target/release/tplink-battery-monitor --tray

# Nasconde la percentuale batteria accanto all'icona (solo dati):
./target/release/tplink-battery-monitor --tray --no-battery-in-panel

# Mostra solo la percentuale accanto all'icona:
./target/release/tplink-battery-monitor --tray --no-data-in-panel
```

### Funzionalità del Menu al Clic (compatto, con icone colorate):
- **Intestazione**: modello del router
- **Connessione**: `WEB CoopVoce · 4G (LTE)`
- **Batteria**: `47% · a batteria` con barra batteria grande verde (rossa sotto il 20%)
- **Dati**: `134.6 / 400 GB · 33%` con barra di avanzamento grande blu
- **Oggi**: `Oggi 3.9 GB`
- **Sottomenu Rete**: tacche segnale (colorate), dispositivi connessi e velocità live
- **Azioni**: `Aggiorna`, `Apri router`
- **Toggle** (persistenti in `~/.config/tplink-battery-monitor/prefs.json`): `Percentuale batteria`, `Mostra GB`, `Notifiche`

### Prototipo Python

`tplink_tray.py` e' il prototipo di riferimento usato in sviluppo; il binario Rust e' il prodotto supportato.

```bash
python3 tplink_tray.py
python3 monitor.py --tray
```

### Avvio automatico al login

```bash
# Binario Rust standalone
./target/release/tplink-battery-monitor --install-autostart
./target/release/tplink-battery-monitor --uninstall-autostart
```

In alternativa e' disponibile un unit file systemd utente in `systemd/tplink-tray.service`.

---

## Riepilogo Rapido da Terminale (`--data`)

Sia la versione Python che quella Rust supportano il flag `--data` per ottenere un colpo d'occhio immediato da terminale senza avviare la GUI:

```bash
python3 monitor.py --data
# oppure con il binario compilato:
./target/release/tplink-battery-monitor --data
```

Output di esempio:
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

## Rust: build e monitor systemd utente

```bash
cargo build --release
# binario in target/release/tplink-battery-monitor

# controllo singolo, senza notifiche (per test)
./target/release/tplink-battery-monitor --once --no-notify

# visualizzazione dati
./target/release/tplink-battery-monitor --data
```

### Timer systemd utente

I file in `systemd/` configurano un timer utente periodico per controllare la batteria in background senza occupare memoria continua:

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

## La particolarità del campo `battery`

Nel firmware del router:
```json
"battery": { "connected": true, "charging": false, "voltage": 48 }
```
Nonostante il nome `voltage`, il valore rappresenta direttamente la **percentuale (0-100)**. L'interfaccia web e i nostri moduli lo espongono coerentemente come `level`.

Per quanto riguarda i dati (`limitation` e `totalStatistics`), il router calcola le dimensioni in base binaria (1024³ = 1 GiB), quindi una quota di 400 GB impostata nel pannello web corrisponde a `429496729600` byte, formattati esattamente in GB.

## Documentazione

- [docs/protocol.md](docs/protocol.md) — Dettagli dell'API HTTP.
- [docs/architecture.md](docs/architecture.md) — Architettura del software.
- [CONTRIBUTING.md](CONTRIBUTING.md) — Guida allo sviluppo.
- [CHANGELOG.md](CHANGELOG.md) — Note di rilascio.

## Licenza

[MIT](LICENSE) © 2026 Gioele Rosana
