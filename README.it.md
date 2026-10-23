<p align="center">
  <img src="assets/icon.png" width="96" alt="Logo TP-Link">
</p>

<h1 align="center">TP-Link MiFi Monitor</h1>

<p align="center">
  Batteria e consumo dati del router mobile TP-Link, sempre visibili nella barra di sistema.<br>
  <a href="README.md">English</a> · <a href="LICENSE">MIT</a>
</p>

---

Una piccola app in background per Linux che mostra la **percentuale di batteria**
e i **GB rimanenti** del router accanto all'icona TP-Link nel pannello. Con un
clic vedi consumo dati, segnale, dispositivi connessi, velocita' e un toggle per
il risparmio energetico. Ricevi anche notifiche per batteria bassa e carica.

Comunica direttamente con il router via Wi-Fi. Niente cloud, niente account.

Testato su **M7350 v9** (firmware 9.0.5). Gli altri modelli con la stessa API web
(M7000, M7200) dovrebbero funzionare.

## Installazione

1. Scarica [`tplink-battery-monitor-x86_64.AppImage`](https://github.com/gioelerosana/tplink-battery-monitor/releases/latest/download/tplink-battery-monitor-x86_64.AppImage)
   dall'[ultima release](https://github.com/gioelerosana/tplink-battery-monitor/releases/latest).
2. Rendila eseguibile (proprieta' del file, oppure `chmod +x`) e aprila con un doppio clic.
3. Premi **Installa** e inserisci la password admin del router.

Una finestra di avanzamento mostra ogni passaggio: l'app si copia in
`~/.local/bin`, verifica la password sul router, registra un servizio in
background che parte a ogni accesso e mette l'icona nella barra. Una finestra
finale riepiloga dove e' stato installato tutto. Non aggiunge nulla al menu
delle applicazioni e il file scaricato si puo' cancellare.

Per aggiornare, apri la nuova AppImage allo stesso modo e premi **Aggiorna**.
Per rimuoverla, riapri l'AppImage e premi **Disinstalla** (oppure
`~/.local/bin/tplink-battery-monitor --uninstall`).

I dialoghi usano `zenity` (GNOME) o `kdialog` (KDE). Se mancano entrambi
ricevi notifiche desktop, e da terminale tutto avviene in modalita' testo.

> Su GNOME serve l'[estensione AppIndicator](https://extensions.gnome.org/extension/615/appindicator-support/)
> per vedere le icone nella barra. KDE, Cinnamon, XFCE e gli altri desktop funzionano subito.

## Configurazione

Le impostazioni sono in `~/.config/tplink-battery-monitor/.env`
(tutte le opzioni in [.env.example](.env.example)):

```ini
TPLINK_HOST=192.168.0.1
TPLINK_PASSWORD=la_tua_password_admin
BATTERY_THRESHOLD=20
```

Dopo una modifica riavvia il servizio:
`systemctl --user restart tplink-battery-monitor`.

## Da terminale

```bash
tplink-battery-monitor --data      # riepilogo di batteria, dati e rete
tplink-battery-monitor --help      # tutte le opzioni
```

## Compilare dai sorgenti

```bash
cargo build --release              # binario in target/release/
./packaging/build-appimage.sh      # AppImage statica in dist/
```

## Altro

- [Come funziona l'API del router](docs/protocol.md)
- [Architettura](docs/architecture.md)
- [Contribuire](CONTRIBUTING.md) · [Changelog](CHANGELOG.md)

Progetto non affiliato a TP-Link. Il logo appartiene ai rispettivi proprietari.
