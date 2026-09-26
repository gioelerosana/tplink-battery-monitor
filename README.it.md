# TP-Link M7350 Battery Monitor

[![CI](https://github.com/JoelShepard/tplink-battery-monitor/actions/workflows/ci.yml/badge.svg)](https://github.com/JoelShepard/tplink-battery-monitor/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

Notifica desktop quando la batteria del router mobile TP-Link scende sotto una
soglia (default **20%**), più notifiche per gli eventi di carica (carica
avviata / completa / interrotta).

> 🇬🇧 [Read the English README](README.md)

Il firmware del router espone un'API JSON non documentata, usata dalla sua
stessa interfaccia web. Questo progetto fa reverse engineering di quell'API,
reimplementa il flusso di autenticazione AES + RSA + MD5 e fornisce **due
implementazioni equivalenti (1:1)**:

| Implementazione | File | Backend notifiche |
| --- | --- | --- |
| **Rust** (consigliata, binario singolo) | `src/tplink_mifi.rs`, `src/main.rs` | D-Bus nativo (`notify-rust`) |
| **Python** | `tplink_mifi.py`, `monitor.py` | `notify-send` |

La versione Rust produce un **singolo binario**, pensato per essere avviato
periodicamente da un **timer systemd utente** (service oneshot). La versione
Python resta comoda per ispezione ed esperimenti senza compilare.

Testato su **TP-Link M7350 (EU) v9.0**, firmware `9.0.5`. Dovrebbe funzionare
su altri modelli mobile Wi-Fi TP-Link che espongono l'API `/cgi-bin/web_cgi`
(es. famiglia M7000 / M7200).

## Funzionalità

- Monitoraggio del livello batteria con soglia configurabile (default 20%).
- Allarme batteria bassa soppresso mentre è in carica.
- Notifiche degli eventi di carica:
  - 🔌 **carica avviata** (da batteria → in carica),
  - ✅ **carica completa** (raggiunge il 100%),
  - ⚠️ **carica interrotta** (spina rimossa prima del 100%).
- Cooldown delle notifiche + isteresi di stato (niente spam).
- Stato persistente tra un avvio e l'altro, così il cooldown sopravvive alla
  cadenza del timer.
- Nessun cloud, nessun browser: parla direttamente col router sulla tua LAN.

## Requisiti

- **Rust** (toolchain stabile, `cargo`) per la versione binaria.
- **Python 3.9+** e `cryptography` per la versione Python.
- Una sessione grafica con D-Bus / `notify-send` per le notifiche desktop.
- Una macchina connessa alla rete del router.

## Configurazione

Copia `.env.example` in `.env` e compila:

```ini
TPLINK_HOST=192.168.0.1
TPLINK_PASSWORD=la_tua_password_admin
BATTERY_THRESHOLD=20
CHECK_INTERVAL_MINUTES=5
NOTIFY_COOLDOWN_MINUTES=60
```

Il binario Rust cerca la configurazione in quest'ordine: `$TP_ENV_FILE`,
`./.env`, `~/.config/tplink-battery-monitor/.env`. Le variabili d'ambiente
reali hanno sempre la precedenza. `.env` è ignorato da git: **non committare
mai la tua password**.

## Rust: build, install e avvio

```bash
cargo build --release
# binario in target/release/tplink-battery-monitor

# controllo singolo, senza notifiche (per verificare che funzioni)
./target/release/tplink-battery-monitor --once --no-notify

# loop continuo
./target/release/tplink-battery-monitor

# controllo singolo forzando la notifica (test)
./target/release/tplink-battery-monitor --once --threshold 100
```

Opzioni: `--host`, `--password`, `--threshold`, `--interval`, `--cooldown`,
`--once`, `--no-notify`, `--notify-recovery`.

Lo stato è salvato in `~/.cache/tplink-battery-monitor/state.json`
(sovrascrivibile con `TP_BATTERY_STATE`).

### Timer systemd utente

I file in `systemd/` installano il binario e un **timer utente da 5 minuti**:

```bash
mkdir -p ~/.local/bin ~/.config/tplink-battery-monitor ~/.config/systemd/user
cargo build --release
install -Dm755 target/release/tplink-battery-monitor ~/.local/bin/tplink-battery-monitor
cp .env ~/.config/tplink-battery-monitor/.env && chmod 600 ~/.config/tplink-battery-monitor/.env
cp systemd/tplink-battery-monitor.service systemd/tplink-battery-monitor.timer ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now tplink-battery-monitor.timer

systemctl --user list-timers tplink-battery-monitor.timer
journalctl --user -u tplink-battery-monitor.service -f
```

Il `service` è `Type=oneshot` e viene attivato dal `timer`
(`OnUnitActiveSec=5min`, `AccuracySec=30s`). Non serve `DISPLAY`: la notifica
viaggia su D-Bus nella sessione utente.

## Python: avvio

```bash
# controllo singolo, senza notifiche
python3 monitor.py --once --no-notify

# loop continuo (legge .env)
python3 monitor.py

# controllo singolo forzando la notifica
python3 monitor.py --once --threshold 100

# client API a basso livello
python3 tplink_mifi.py --password '...' battery
python3 tplink_mifi.py --password '...' status
```

### cron (alternativa al timer systemd)

```cron
*/5 * * * * cd /percorso/tplink-battery-monitor && /usr/bin/python3 monitor.py --once >> monitor.log 2>&1
```

Con cron le notifiche desktop richiedono `DISPLAY` e
`DBUS_SESSION_BUS_ADDRESS` nell'ambiente; per questo è preferibile systemd.

## La trappola del campo `battery`

Il firmware riporta:

```json
"battery": { "connected": true, "charging": false, "voltage": 98 }
```

Nonostante il nome, **`voltage` contiene già la percentuale (0–100)** —
l'interfaccia web lo usa direttamente per scegliere l'icona. Quindi
`voltage: 98` significa batteria al 98%. Entrambe le implementazioni lo
espongono anche come `level`.

## Come funziona

Il router usa un'API JSON su due endpoint e un livello di cifratura "GDPR"
(AES-128-CBC per il payload, RSA-512 per la firma, MD5 per l'hash della
password). La descrizione completa del protocollo è in
[docs/protocol.md](docs/protocol.md); l'architettura del software (layer di
trasporto vs. applicativo, macchina a stati, deployment) è in
[docs/architecture.md](docs/architecture.md).

## Documentazione

- [docs/protocol.md](docs/protocol.md) — API HTTP ricostruita.
- [docs/architecture.md](docs/architecture.md) — architettura del software.
- [CONTRIBUTING.md](CONTRIBUTING.md) — build, test e note per contribuire.
- [CHANGELOG.md](CHANGELOG.md) — storico delle release.
- [SECURITY.md](SECURITY.md) — come vengono gestiti i segreti.

## Risoluzione problemi

- **"Router non raggiungibile"** — assicurati di eseguire lo strumento sulla
  LAN del router e che `TPLINK_HOST` sia corretto (default `192.168.0.1`).
- **"Password del router errata"** — la password admin in `.env` è sbagliata.
- **Nessuna notifica** — verifica che `notify-send` / la sessione D-Bus
  funzionino nella tua sessione grafica; da systemd usa un servizio *utente*
  (non di sistema).

## Licenza

[MIT](LICENSE) © 2026 Gioele Rosana
