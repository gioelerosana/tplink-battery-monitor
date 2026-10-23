//! Installazione come servizio utente in background.
//!
//! Avviando l'AppImage senza argomenti (doppio clic) viene mostrato un
//! dialogo di benvenuto, poi `install()` copia l'eseguibile in
//! `~/.local/bin`, chiede e verifica la password del router se manca e
//! registra un servizio systemd utente che avvia `--tray` a ogni login
//! grafico. Ogni passaggio e' visibile in una finestra di avanzamento e alla
//! fine un riepilogo indica dove e' stata installata l'app. Nessuna voce viene
//! aggiunta al menu delle applicazioni.
//!
//! Se systemd utente non e' disponibile (o la sessione grafica non attiva
//! `graphical-session.target`) si ripiega su una voce XDG in
//! `~/.config/autostart`.

use std::env;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{exit, Command, Stdio};
use std::thread::sleep;
use std::time::Duration;

use tplink_mifi::{MiFiClient, TpLinkError};

use crate::dialog::{Choice, Ui, ICON_NAME};
use crate::home_dir;

const SERVICE_NAME: &str = "tplink-battery-monitor.service";
const APP_NAME: &str = "tplink-battery-monitor";
const VERSION: &str = env!("CARGO_PKG_VERSION");
const ICON_PNG: &[u8] = include_bytes!("../assets/icon.png");

/// Tempo concesso al processo della tray per partire prima di controllare
/// che sia ancora vivo.
const STARTUP_GRACE: Duration = Duration::from_millis(1500);

enum Startup {
    SystemdService,
    XdgAutostart,
}

enum PasswordCheck {
    Verified,
    RouterUnreachable,
    AlreadyConfigured,
}

fn installed_binary() -> PathBuf {
    home_dir().join(".local/bin").join(APP_NAME)
}

fn config_file() -> PathBuf {
    home_dir().join(".config/tplink-battery-monitor/.env")
}

fn service_file() -> PathBuf {
    home_dir().join(".config/systemd/user").join(SERVICE_NAME)
}

/// Icona "non a tema" nella radice di `~/.local/share/icons`: GTK la trova
/// sempre, mentre in `hicolor/` verrebbe ignorata se un'altra app ha lasciato
/// un `icon-theme.cache` non aggiornato.
fn icon_file() -> PathBuf {
    home_dir()
        .join(".local/share/icons")
        .join(format!("{ICON_NAME}.png"))
}

/// Posizione usata dalla prima versione dell'installer.
fn legacy_icon_file() -> PathBuf {
    home_dir()
        .join(".local/share/icons/hicolor/256x256/apps")
        .join(format!("{ICON_NAME}.png"))
}

fn autostart_files() -> [PathBuf; 2] {
    let dir = home_dir().join(".config/autostart");
    [
        dir.join("tplink-battery-monitor.desktop"),
        // Nome usato dalle versioni precedenti.
        dir.join("tplink-tray.desktop"),
    ]
}

/// Percorso con `~` al posto della home, per i messaggi all'utente.
fn display(path: &Path) -> String {
    match path.strip_prefix(home_dir()) {
        Ok(relative) => format!("~/{}", relative.display()),
        Err(_) => path.display().to_string(),
    }
}

/// Percorso reale dell'eseguibile da installare: dentro un'AppImage
/// `current_exe()` punta al mount temporaneo, quindi si usa `$APPIMAGE`.
fn source_executable() -> Result<PathBuf, String> {
    if let Some(path) = env::var_os("APPIMAGE") {
        return Ok(PathBuf::from(path));
    }
    env::current_exe().map_err(|err| format!("eseguibile non trovato: {err}"))
}

/// Installa o aggiorna. Con `ask_first` mostra prima il dialogo di benvenuto
/// (avvio dell'AppImage con doppio clic).
pub fn install(ask_first: bool) {
    let ui = Ui::detect();
    // Prima di tutto, cosi' anche i dialoghi mostrano il logo TP-Link.
    let _ = install_icon();
    let installed = installed_binary().is_file();

    if ask_first {
        match ui.welcome(&welcome_text(installed), installed) {
            Choice::Install => {}
            Choice::Uninstall => {
                uninstall_with(&ui, true);
                return;
            }
            Choice::Cancel => {
                if !installed {
                    remove(&icon_file());
                }
                return;
            }
        }
    }

    let host = env::var("TPLINK_HOST").unwrap_or_else(|_| "192.168.0.1".to_string());
    let (password, check) = match configured_password() {
        // Password gia' nota (config esistente o variabile d'ambiente): la si
        // salva solo se il file di configurazione utente non esiste ancora.
        Some(password) => (
            (!config_file().is_file()).then_some(password),
            PasswordCheck::AlreadyConfigured,
        ),
        None => match ask_valid_password(&ui, &host) {
            Some((password, check)) => (Some(password), check),
            None => {
                let _ = write_config(&host, "");
                ui.error(&format!(
                    "Installazione annullata: serve la password admin del router.\n\n\
                     Puoi anche scriverla in {} (TPLINK_PASSWORD=...) e avviare di nuovo l'AppImage.",
                    display(&config_file())
                ));
                exit(1);
            }
        },
    };

    match run_steps(&ui, installed, &host, password.as_deref()) {
        Ok(startup) => ui.info(&summary(installed, &startup, &check, &host)),
        Err(message) => {
            ui.error(&format!("Installazione non riuscita.\n\n{message}"));
            exit(1);
        }
    }
}

fn welcome_text(installed: bool) -> String {
    let target = display(&installed_binary());
    if installed {
        let current = installed_version().unwrap_or_else(|| "precedente".to_string());
        format!(
            "TP-Link MiFi Monitor e' gia' installato in {target}.\n\n\
             Versione installata: {current}\n\
             Questa versione: {VERSION}\n\n\
             Aggiorna sostituisce l'app installata con questa versione e la riavvia."
        )
    } else {
        format!(
            "TP-Link MiFi Monitor {VERSION}\n\n\
             Mostra batteria e consumo dati del router TP-Link nella barra di sistema.\n\n\
             L'installazione:\n\
             \u{2022} copia l'app in {target}\n\
             \u{2022} la avvia automaticamente a ogni accesso, in background\n\
             \u{2022} non aggiunge voci al menu delle applicazioni"
        )
    }
}

/// Versione dell'app gia' installata (le versioni senza `--version` danno `None`).
fn installed_version() -> Option<String> {
    let output = Command::new(installed_binary())
        .arg("--version")
        .env_remove("APPIMAGE")
        .stderr(Stdio::null())
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    let version = text.trim().strip_prefix(APP_NAME)?.trim();
    (output.status.success() && !version.is_empty()).then(|| version.to_string())
}

fn configured_password() -> Option<String> {
    env::var("TPLINK_PASSWORD").ok().filter(|p| !p.is_empty())
}

/// Chiede la password finche' il router non la accetta. Se il router non e'
/// raggiungibile in quel momento la password viene accettata senza verifica.
fn ask_valid_password(ui: &Ui, host: &str) -> Option<(String, PasswordCheck)> {
    let mut note = "";
    loop {
        let text = format!(
            "{note}Password di amministrazione del router TP-Link.\n\
             E' quella che usi per accedere a http://{host}"
        );
        let password = ui.password(&text)?;

        let mut progress = ui.progress("Verifica della password sul router...");
        progress.step(50, &format!("Connessione a {host}"));
        let result = MiFiClient::new(host, &password).login();
        progress.finish();

        match result {
            Ok(_) => return Some((password, PasswordCheck::Verified)),
            Err(TpLinkError::WrongPassword) => note = "Password errata, riprova.\n\n",
            Err(_) => return Some((password, PasswordCheck::RouterUnreachable)),
        }
    }
}

fn run_steps(
    ui: &Ui,
    installed: bool,
    host: &str,
    password: Option<&str>,
) -> Result<Startup, String> {
    let target = installed_binary();
    let mut progress = ui.progress(if installed {
        "Aggiornamento di TP-Link MiFi Monitor..."
    } else {
        "Installazione di TP-Link MiFi Monitor..."
    });

    progress.step(10, &format!("Copia dell'app in {}", display(&target)));
    copy_executable(&source_executable()?, &target)?;

    if let Some(password) = password {
        progress.step(30, "Salvataggio della configurazione");
        write_config(host, password)?;
    }

    progress.step(50, "Registrazione dell'avvio automatico al login");
    // Un'eventuale istanza precedente (anche avviata via autostart) va chiusa
    // prima di avviare la nuova versione.
    stop_running_tray(&target);
    let startup = if register_systemd_service(&target) {
        remove_autostart_entries();
        Startup::SystemdService
    } else {
        install_autostart_entry(&target)?;
        Startup::XdgAutostart
    };

    progress.step(75, "Avvio dell'icona nella barra di sistema");
    start_tray(&target, &startup)?;

    progress.step(100, "Completato");
    progress.finish();
    Ok(startup)
}

fn summary(installed: bool, startup: &Startup, check: &PasswordCheck, host: &str) -> String {
    let headline = if installed {
        "TP-Link MiFi Monitor e' stato aggiornato."
    } else {
        "TP-Link MiFi Monitor e' installato."
    };
    let startup = match startup {
        Startup::SystemdService => format!("servizio utente {SERVICE_NAME}"),
        Startup::XdgAutostart => display(&autostart_files()[0]),
    };
    let mut text = format!(
        "{headline}\n\n\
         App: {}\n\
         Avvio automatico: {startup}\n\
         Configurazione: {}\n\n\
         L'icona e' ora nella barra di sistema e partira' a ogni accesso.\n\
         Il file AppImage scaricato si puo' eliminare.",
        display(&installed_binary()),
        display(&config_file()),
    );
    if matches!(check, PasswordCheck::RouterUnreachable) {
        text.push_str(&format!(
            "\n\nAttenzione: il router ({host}) non e' raggiungibile ora, quindi la password \
             non e' stata verificata. L'icona resta spenta finche' non ti colleghi al suo Wi-Fi."
        ));
    }
    if !tray_host_available() {
        text.push_str(
            "\n\nAttenzione: il desktop non sembra mostrare icone nella barra di sistema. \
             Su GNOME installa l'estensione \"AppIndicator and KStatusNotifierItem Support\".",
        );
    }
    text.push_str("\n\nPer disinstallare avvia di nuovo l'AppImage e scegli Disinstalla.");
    text
}

/// `--uninstall` da riga di comando.
pub fn uninstall() {
    uninstall_with(&Ui::detect(), false);
}

fn uninstall_with(ui: &Ui, ask_first: bool) {
    let question = format!(
        "Rimuovere TP-Link MiFi Monitor?\n\n\
         Verranno rimossi l'app e l'avvio automatico. La configurazione in {} resta.",
        display(&config_file())
    );
    if ask_first && !ui.confirm(&question, "Disinstalla") {
        return;
    }

    let mut progress = ui.progress("Disinstallazione di TP-Link MiFi Monitor...");
    progress.step(30, "Arresto e rimozione dell'avvio automatico");
    if service_file().is_file() {
        let _ = systemctl(&["disable", "--now", SERVICE_NAME]);
        remove(&service_file());
        let _ = systemctl(&["daemon-reload"]);
    }
    remove_autostart_entries();
    stop_running_tray(&installed_binary());

    progress.step(70, "Rimozione dell'app");
    remove(&installed_binary());
    remove(&icon_file());
    remove(&legacy_icon_file());
    progress.step(100, "Completato");
    progress.finish();

    ui.info(&format!(
        "TP-Link MiFi Monitor e' stato disinstallato.\n\n\
         La configurazione resta in {}: puoi eliminarla a mano.",
        display(&config_file())
    ));
}

/// Copia atomica (file temporaneo + rename) cosi' da poter aggiornare anche
/// mentre il servizio sta eseguendo la versione precedente.
fn copy_executable(source: &Path, target: &Path) -> Result<(), String> {
    if fs::canonicalize(source).ok() == fs::canonicalize(target).ok() {
        return Ok(());
    }
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    let staging = target.with_extension("new");
    fs::copy(source, &staging)
        .and_then(|_| fs::set_permissions(&staging, fs::Permissions::from_mode(0o755)))
        .and_then(|_| fs::rename(&staging, target))
        .map_err(|err| {
            let _ = fs::remove_file(&staging);
            format!("Copia in {} fallita: {err}", display(target))
        })
}

fn write_config(host: &str, password: &str) -> Result<(), String> {
    let path = config_file();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let content = format!("TPLINK_HOST={host}\nTPLINK_PASSWORD={password}\n");
    fs::write(&path, content)
        .and_then(|_| fs::set_permissions(&path, fs::Permissions::from_mode(0o600)))
        .map_err(|err| format!("Scrittura di {} fallita: {err}", display(&path)))
}

fn install_icon() -> std::io::Result<()> {
    remove(&legacy_icon_file());
    let path = icon_file();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, ICON_PNG)
}

fn systemctl(args: &[&str]) -> bool {
    Command::new("systemctl")
        .arg("--user")
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// Registra il servizio. Restituisce `false` se systemd utente non e'
/// utilizzabile o la sessione non attiva `graphical-session.target`, nel qual
/// caso il servizio non partirebbe al login.
fn register_systemd_service(binary: &Path) -> bool {
    if !systemctl(&["is-active", "--quiet", "graphical-session.target"]) {
        return false;
    }
    let unit = format!(
        "[Unit]\n\
         Description=TP-Link MiFi tray monitor (batteria e dati)\n\
         Documentation=https://github.com/gioelerosana/tplink-battery-monitor\n\
         After=graphical-session.target\n\
         PartOf=graphical-session.target\n\
         \n\
         [Service]\n\
         ExecStart=\"{}\" --tray\n\
         Restart=on-failure\n\
         RestartSec=10\n\
         \n\
         [Install]\n\
         WantedBy=graphical-session.target\n",
        binary.display()
    );
    let path = service_file();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    fs::write(&path, unit).is_ok()
        && systemctl(&["daemon-reload"])
        && systemctl(&["enable", SERVICE_NAME])
}

/// Avvia la tray e controlla che sia ancora in esecuzione dopo qualche istante.
fn start_tray(binary: &Path, startup: &Startup) -> Result<(), String> {
    match startup {
        Startup::SystemdService => {
            if !systemctl(&["restart", SERVICE_NAME]) {
                return Err(format!("Avvio del servizio {SERVICE_NAME} fallito."));
            }
            sleep(STARTUP_GRACE);
            if systemctl(&["is-active", "--quiet", SERVICE_NAME]) {
                Ok(())
            } else {
                Err(format!(
                    "Il servizio {SERVICE_NAME} si e' fermato subito dopo l'avvio.\n\
                     Dettagli: journalctl --user -u {SERVICE_NAME}"
                ))
            }
        }
        Startup::XdgAutostart => {
            let mut child = Command::new(binary)
                .arg("--tray")
                .env_remove("APPIMAGE")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|err| format!("Avvio dell'app fallito: {err}"))?;
            sleep(STARTUP_GRACE);
            match child.try_wait() {
                Ok(None) => Ok(()),
                _ => Err("L'app si e' chiusa subito dopo l'avvio.".to_string()),
            }
        }
    }
}

/// Un'istanza avviata tramite autostart non e' gestita da systemd.
fn stop_running_tray(binary: &Path) {
    let _ = Command::new("pkill")
        .args(["-f", &format!("{} --tray", binary.display())])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// Verifica se il desktop espone un host per le icone di stato (SNI). Se lo
/// strumento di controllo manca si presume di si', per non dare falsi avvisi.
fn tray_host_available() -> bool {
    Command::new("busctl")
        .args(["--user", "status", "org.kde.StatusNotifierWatcher"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(true)
}

fn install_autostart_entry(binary: &Path) -> Result<(), String> {
    remove_autostart_entries();
    let target = &autostart_files()[0];
    if let Some(parent) = target.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let entry = format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=TP-Link MiFi Monitor\n\
         Comment=Batteria e consumo dati del router TP-Link\n\
         Exec=\"{}\" --tray\n\
         Icon={ICON_NAME}\n\
         Terminal=false\n\
         NoDisplay=true\n\
         X-GNOME-Autostart-enabled=true\n",
        binary.display()
    );
    fs::write(target, entry)
        .map_err(|err| format!("Scrittura di {} fallita: {err}", display(target)))
}

fn remove_autostart_entries() {
    for entry in autostart_files() {
        remove(&entry);
    }
}

fn remove(path: &Path) {
    let _ = fs::remove_file(path);
}
