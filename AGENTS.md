# Linee Guida di Sviluppo per Agenti (AGENTS.md)

## Architettura e Flusso di Lavoro

Questo repository (`tplink-battery-monitor`) implementa strumenti di monitoraggio e companion app per router mobile TP-Link (es. M7350, serie M7000/M7200).

### Regola Fondamentale del Progetto

1. **Doppia Implementazione (Python + Rust)**:
   - **Python** (`tplink_mifi.py`, `monitor.py`, `tplink_tray.py`): viene utilizzato per reverse engineering rapido, prototipazione veloce, ispezione e script di utilità.
   - **Rust** (`src/tplink_mifi.rs`, `src/main.rs`): costituisce il **prodotto finale**.

2. **Il Prodotto Finale è Esclusivamente il Binario Rust Standalone**:
   - Qualsiasi nuova funzionalità introdotta o esplorata in Python **deve essere tradotta e integrata in Rust**.
   - Il prodotto finale distribuito ed eseguito deve essere **un unico binario super ottimizzato in Rust** (`tplink-battery-monitor`), privo di dipendenze runtime esterne (niente interprete Python, niente runtime pesanti, niente GUI toolkit).
   - Profilo release con `lto = true`, `opt-level = 3`, `strip = true`.

3. **Capacità del Binario Standalone Rust (`tplink-battery-monitor`)**:
   - `--once`: controllo singolo (es. per timer systemd utente).
   - `--data`: panoramica rapida a terminale su consumo dati, limiti, batteria e stato rete.
   - `--tray`: applicazione companion residente in background nel pannello di sistema / system tray (KStatusNotifierItem / SNI / DBusMenu), con:
     - icona del logo TP-Link incorporata nel binario (`assets/tplink-*.argb` generati da `assets/tplink.svg`, nessun file a runtime); quando il router non risponde l'icona viene mostrata "spenta" (desaturata e piu' scura);
     - etichetta testuale accanto all'icona (Ayatana `XAyatanaLabel`): percentuale batteria e, di default, i GB rimanenti;
     - menu compatto al clic: barre testuali a tutta larghezza su una riga dedicata sotto le voci batteria e dati (texture `█░` e `▓▒` per distinguerle) con indicatore colore generato a runtime (verde/rosso batteria, blu dati) e tacche segnale, tramite un encoder PNG minimale senza dipendenze (`src/icons.rs`); dettagli rete in un sottomenu;
     - toggle `Risparmio energetico` che chiama il modulo router `power_save` (getConfig/setConfig), con retry in caso di caduta di rete transitoria;
     - toggle persistenti (`Percentuale batteria`, `Mostra GB`, `Notifiche`) salvati in `~/.config/tplink-battery-monitor/prefs.json`;
     - notifiche desktop D-Bus per batteria bassa ed eventi di carica.
   - `--show-data-in-panel` / `--no-data-in-panel`: mostra i GB accanto all'icona (default: bollati i GB).
   - `--show-battery-in-panel` / `--no-battery-in-panel`: mostra/nasconde la percentuale batteria accanto all'icona (default: mostrata).
   - `--threshold`, `--interval`, `--cooldown`, `--no-notify`, `--notify-recovery`: opzioni di controllo soglia e cadenza.
   - `--install-autostart` / `--uninstall-autostart`: avvio automatico al login grafico.

4. **Dipendenza `ksni` vendorizzata**:
   - La libreria `ksni` e' copiata in `vendor/ksni` e riscritta localmente (`[patch.crates-io]` in `Cargo.toml`) per esporre la proprieta' Ayatana `XAyatanaLabel`, non ancora supportata a monte.
   - Non introdurre dipendenze runtime: la patch resta comunque compilata staticamente nel singolo binario.

5. **Sincronizzazione e Verifica**:
   - Quando si implementa una modifica o una nuova feature, verificare e mantenere allineate entrambe le codebase (Python per test rapido, Rust per il binario di produzione).
   - Eseguire sempre `cargo check`, `cargo test` e `cargo build --release` prima di rilasciare o chiudere l'attività.
   - Verificare la sintassi Python con `python3 -m py_compile monitor.py tplink_tray.py tplink_mifi.py`.

### Stile del Codice

- **Nessuna emoji** in codice, output CLI, etichette di menu, notifiche, README o CHANGELOG. Usare testo semplice.
- I caratteri tipografici strutturali (barre `█░`, frecce `↓↑`, separatori `·•`, righe `═─`) sono ammessi per la leggibilità dell'output.
