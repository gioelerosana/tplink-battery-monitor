# Linee Guida di Sviluppo per Agenti (AGENTS.md)

## Architettura e Flusso di Lavoro

Questo repository (`tplink-battery-monitor`) implementa strumenti di monitoraggio e companion app per router mobile TP-Link (es. M7350, serie M7000/M7200).

### Regola Fondamentale del Progetto

1. **Solo Rust**:
   - Il codice Python di prototipazione e' stato rimosso. Ogni funzionalita' va implementata direttamente in Rust (`src/tplink_mifi.rs`, `src/main.rs`, `src/icons.rs`, `src/install.rs`).

2. **Il Prodotto Finale e' un'AppImage con un unico binario Rust statico**:
   - Il binario `tplink-battery-monitor` e' compilato per `x86_64-unknown-linux-musl` (statico, nessuna dipendenza runtime: niente interpreti, runtime pesanti o GUI toolkit) e impacchettato in AppImage da `packaging/build-appimage.sh` (output in `dist/`).
   - Avviata senza argomenti (doppio clic), l'AppImage mostra un installer grafico (`src/dialog.rs`: zenity o kdialog, testo da terminale, notifiche come ultima risorsa) con benvenuto, verifica della password sul router, finestra di avanzamento e riepilogo finale; se gia' installata propone Aggiorna / Disinstalla. L'installazione (`src/install.rs`) si copia in `~/.local/bin/tplink-battery-monitor`, chiede la password del router se manca (terminale, zenity o kdialog) e registra il servizio utente systemd `tplink-battery-monitor.service` (`graphical-session.target`, `--tray`). Senza systemd utente ripiega su `~/.config/autostart`. Non deve comparire nel menu applicazioni.
   - Profilo release con `lto = true`, `opt-level = 3`, `strip = true`.

3. **Capacità del Binario Standalone Rust (`tplink-battery-monitor`)**:
   - `--once`: controllo singolo (es. per timer systemd utente).
   - `--data`: panoramica rapida a terminale su consumo dati, limiti, batteria e stato rete.
   - `--tray`: applicazione companion residente in background nel pannello di sistema / system tray (KStatusNotifierItem / SNI / DBusMenu), con:
     - icona del logo TP-Link incorporata nel binario (`assets/tplink-*.argb` generati da `assets/tplink.svg`, nessun file a runtime); quando il router non risponde l'icona viene mostrata "spenta" (desaturata e piu' scura);
     - etichetta testuale accanto all'icona (Ayatana `XAyatanaLabel`): percentuale batteria e, di default, i GB rimanenti;
     - menu compatto al clic: barre testuali a tutta larghezza su una riga dedicata sotto le voci batteria, dati e consumo di oggi (texture `█░`, `▓▒` e `▆▁` per distinguerle) con indicatore colore generato a runtime (verde/rosso batteria, blu dati, viola budget giornaliero) e tacche segnale, tramite un encoder PNG minimale senza dipendenze (`src/icons.rs`); dettagli rete in un sottomenu;
     - budget giornaliero (barra viola): GB rimasti a inizio giornata divisi per i giorni al rinnovo, con quota e giorno di rinnovo letti dal modulo router `flowstat` (`paymentDay`); compare solo con limite dati e rinnovo mensile attivi;
     - toggle `Risparmio energetico` che chiama il modulo router `power_save` (getConfig/setConfig), con retry in caso di caduta di rete transitoria;
     - toggle persistenti (`Percentuale batteria`, `Mostra GB`, `Notifiche`) salvati in `~/.config/tplink-battery-monitor/prefs.json`;
     - notifiche desktop D-Bus per batteria bassa, eventi di carica e budget giornaliero (vicino alla soglia e superato, al massimo una volta al giorno ciascuna).
   - `--show-data-in-panel` / `--no-data-in-panel`: mostra i GB accanto all'icona (default: mostrati i GB).
   - `--show-battery-in-panel` / `--no-battery-in-panel`: mostra/nasconde la percentuale batteria accanto all'icona (default: mostrata).
   - `--threshold`, `--interval`, `--cooldown`, `--no-notify`, `--notify-recovery`: opzioni di controllo soglia e cadenza.
   - `--daily-warning <N>` (o `DAILY_BUDGET_WARNING`): percentuale del budget giornaliero oltre la quale avvisare (default 80).
   - `--install` / `--uninstall`: installazione come servizio utente al login grafico (alias storici `--install-autostart` / `--uninstall-autostart`).

4. **Dipendenza `ksni` vendorizzata**:
   - La libreria `ksni` e' copiata in `vendor/ksni` e riscritta localmente (`[patch.crates-io]` in `Cargo.toml`) per esporre la proprieta' Ayatana `XAyatanaLabel`, non ancora supportata a monte.
   - Non introdurre dipendenze runtime: la patch resta comunque compilata staticamente nel singolo binario.

5. **Verifica**:
   - Eseguire sempre `cargo check`, `cargo test` e `cargo build --release` prima di rilasciare o chiudere l'attivita'.
   - Per le modifiche a installazione o packaging, ricostruire l'AppImage con `./packaging/build-appimage.sh`.

6. **Rilascio (tag di versione)**:
   - Schema di versione `AA.MINOR.PATCH`: il primo numero e' l'anno a due cifre (26 = 2026) e cambia solo con l'anno; durante l'anno si incrementano MINOR (nuove funzionalita') e PATCH (correzioni). La prima release e' `26.0.1`. Esempio di tag: `v26.0.1`.
   - Una release si pubblica solo con un tag: il push di `vX.Y.Z` (o `X.Y.Z`) avvia `.github/workflows/release.yml`, che verifica che il tag coincida con `version` in `Cargo.toml`, esegue i test, costruisce l'AppImage e crea la release GitHub con l'AppImage, il suo `.sha256` e la sezione `## [X.Y.Z]` del CHANGELOG come note.
   - Procedura: aggiornare `version` in `Cargo.toml` (e `Cargo.lock` con `cargo check`), spostare le voci di `## [Unreleased]` in `## [X.Y.Z] - AAAA-MM-GG` nel CHANGELOG aggiornando i link in fondo, fare commit, poi `git tag vX.Y.Z` e `git push origin main vX.Y.Z`.
   - Il nome dell'asset resta fisso (`tplink-battery-monitor-x86_64.AppImage`), cosi' il link `releases/latest/download/...` funziona sempre.
   - `.github/workflows/ci.yml` esegue fmt, clippy e test su ogni push a `main` e sulle pull request.

### Stile del Codice

- **Nessuna emoji** in codice, output CLI, etichette di menu, notifiche, README o CHANGELOG. Usare testo semplice.
- I caratteri tipografici strutturali (barre `█░`, frecce `↓↑`, separatori `·•`, righe `═─`) sono ammessi per la leggibilità dell'output.
