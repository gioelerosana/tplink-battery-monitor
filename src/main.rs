//! Monitor della batteria e del traffico dati del router TP-Link M7350.
//!
//! Esegue il polling dell'API del router, invia notifiche desktop D-Bus
//! e fornisce un'icona e menu interattivo per la barra di sistema (system tray).
//!
//! Uso:
//!     tplink-battery-monitor                       # loop continuo a terminale
//!     tplink-battery-monitor --tray                # companion app per la system tray
//!     tplink-battery-monitor --tray --no-data-in-panel  # solo percentuale accanto all'icona
//!     tplink-battery-monitor --data                # statistiche consumo dati ed esci
//!     tplink-battery-monitor --once                # un singolo controllo (per systemd timer)
//!     tplink-battery-monitor --install-autostart   # configura l'avvio automatico al login
//!     tplink-battery-monitor --uninstall-autostart # rimuove l'avvio automatico

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::exit;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::Arc;
use std::thread::sleep;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ksni::blocking::TrayMethods;
use notify_rust::{Notification, Urgency};
use serde_json::{json, Value};

use tplink_mifi::MiFiClient;

mod icons;

struct Config {
    host: String,
    password: String,
    threshold: f64,
    interval_minutes: f64,
    cooldown_minutes: f64,
    once: bool,
    data: bool,
    tray: bool,
    install_autostart: bool,
    uninstall_autostart: bool,
    no_notify: bool,
    notify_recovery: bool,
    show_data_in_panel: bool,
    show_battery_in_panel: bool,
}

fn log(message: &str) {
    println!(
        "[{}] {}",
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
        message
    );
}

fn home_dir() -> PathBuf {
    PathBuf::from(env::var("HOME").unwrap_or_else(|_| ".".to_string()))
}

fn state_file() -> PathBuf {
    if let Ok(path) = env::var("TP_BATTERY_STATE") {
        return PathBuf::from(path);
    }
    home_dir().join(".cache/tplink-battery-monitor/state.json")
}

fn prefs_file() -> PathBuf {
    home_dir().join(".config/tplink-battery-monitor/prefs.json")
}

/// Preferenze dell'utente (toggle del menu) persistenti tra i riavvii.
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct Prefs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    battery_in_panel: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    data_in_panel: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    notifications: Option<bool>,
}

fn load_prefs() -> Prefs {
    fs::read_to_string(prefs_file())
        .ok()
        .and_then(|content| serde_json::from_str(&content).ok())
        .unwrap_or_default()
}

fn save_prefs(prefs: &Prefs) {
    let path = prefs_file();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(text) = serde_json::to_string_pretty(prefs) {
        let _ = fs::write(path, text);
    }
}

fn autostart_files() -> (PathBuf, PathBuf) {
    let autostart_dir = home_dir().join(".config/autostart");
    (
        autostart_dir.join("tplink-battery-monitor.desktop"),
        autostart_dir.join("tplink-tray.desktop"),
    )
}

fn install_autostart() {
    let (target, legacy) = autostart_files();
    if let Some(parent) = target.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if legacy.is_file() {
        let _ = fs::remove_file(&legacy);
    }

    let exe = env::current_exe()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| "tplink-battery-monitor".to_string());

    let content = format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=TP-Link MiFi Monitor\n\
         Comment=Monitor della batteria e consumo dati del router TP-Link M7350\n\
         Exec={exe} --tray\n\
         Icon=battery-good-symbolic\n\
         Terminal=false\n\
         Categories=Utility;System;\n\
         StartupNotify=false\n\
         X-GNOME-Autostart-enabled=true\n"
    );

    match fs::write(&target, content) {
        Ok(_) => {
            println!("File di avvio automatico installato con successo:");
            println!("  {}", target.display());
            println!("L'applicazione verra' avviata automaticamente al prossimo login.");
        }
        Err(e) => {
            eprintln!(
                "Errore creazione file autostart in {}: {e}",
                target.display()
            );
            exit(1);
        }
    }
}

fn uninstall_autostart() {
    let (target, legacy) = autostart_files();
    let mut removed = false;
    if target.is_file() {
        let _ = fs::remove_file(&target);
        println!("Rimosso: {}", target.display());
        removed = true;
    }
    if legacy.is_file() {
        let _ = fs::remove_file(&legacy);
        println!("Rimosso: {}", legacy.display());
        removed = true;
    }
    if !removed {
        println!("Nessun file di avvio automatico trovato.");
    }
}

/// Carica un file .env minimale nelle variabili d'ambiente (senza
/// sovrascrivere quelle gia' presenti).
fn load_dotenv(path: &PathBuf) {
    let Ok(content) = fs::read_to_string(path) else {
        return;
    };
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim().trim_matches(|c| c == '\'' || c == '"');
        if env::var_os(key).is_none() {
            env::set_var(key, value);
        }
    }
}

fn env_f64(key: &str, default: f64) -> f64 {
    env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn env_bool(key: &str, default: bool) -> bool {
    match env::var(key) {
        Ok(value) => !matches!(value.trim(), "0" | "false" | "False" | "no" | "off" | ""),
        Err(_) => default,
    }
}

fn print_help() {
    print!(
        "Uso: tplink-battery-monitor [OPZIONI]

Opzioni principali:
  --tray                  Avvia l'app companion nella system tray / background
  --data                  Mostra il riepilogo dati e batteria ed esci
  --once                  Esegui un singolo controllo (per systemd timer)
  --install-autostart     Installa l'avvio automatico al login grafico
  --uninstall-autostart   Rimuove l'avvio automatico al login grafico

Parametri di connessione e soglia:
  --host <HOST>           Indirizzo IP router (default: 192.168.0.1 o TPLINK_HOST)
  --password <PWD>        Password router (o TPLINK_PASSWORD)
  --threshold <N>         Soglia percentuale batteria (default: 20%)
  --interval <MIN>        Intervallo controllo in minuti (default: 5)
  --cooldown <MIN>        Cooldown notifiche in minuti (default: 60)
  --no-notify             Disabilita le notifiche desktop
  --notify-recovery       Notifica quando la batteria torna sopra la soglia
  --show-data-in-panel    Mostra i GB nel testo accanto all'icona (default)
  --no-data-in-panel      Mostra solo la percentuale accanto all'icona
  --show-battery-in-panel Mostra la percentuale batteria accanto all'icona (default)
  --no-battery-in-panel   Nasconde la percentuale batteria accanto all'icona
  -h, --help              Mostra questo messaggio di aiuto
"
    );
}

fn parse_args() -> Config {
    let mut config = Config {
        host: env::var("TPLINK_HOST").unwrap_or_else(|_| "192.168.0.1".to_string()),
        password: env::var("TPLINK_PASSWORD").unwrap_or_default(),
        threshold: env_f64("BATTERY_THRESHOLD", 20.0),
        interval_minutes: env_f64("CHECK_INTERVAL_MINUTES", 5.0),
        cooldown_minutes: env_f64("NOTIFY_COOLDOWN_MINUTES", 60.0),
        once: false,
        data: false,
        tray: false,
        install_autostart: false,
        uninstall_autostart: false,
        no_notify: false,
        notify_recovery: false,
        show_data_in_panel: env_bool("SHOW_DATA_IN_PANEL", true),
        show_battery_in_panel: env_bool("SHOW_BATTERY_IN_PANEL", true),
    };

    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut take = |name: &str| match args.next() {
            Some(value) => value,
            None => {
                eprintln!("Manca il valore per {name}");
                exit(2);
            }
        };
        match arg.as_str() {
            "--host" => config.host = take("--host"),
            "--password" => config.password = take("--password"),
            "--threshold" => config.threshold = take("--threshold").parse().unwrap_or(20.0),
            "--interval" => config.interval_minutes = take("--interval").parse().unwrap_or(5.0),
            "--cooldown" => config.cooldown_minutes = take("--cooldown").parse().unwrap_or(60.0),
            "--once" => config.once = true,
            "--data" => config.data = true,
            "--tray" => config.tray = true,
            "--install-autostart" => config.install_autostart = true,
            "--uninstall-autostart" => config.uninstall_autostart = true,
            "--no-notify" => config.no_notify = true,
            "--notify-recovery" => config.notify_recovery = true,
            "--show-data-in-panel" => config.show_data_in_panel = true,
            "--no-data-in-panel" => config.show_data_in_panel = false,
            "--show-battery-in-panel" => config.show_battery_in_panel = true,
            "--no-battery-in-panel" => config.show_battery_in_panel = false,
            "-h" | "--help" => {
                print_help();
                exit(0);
            }
            other => {
                eprintln!("Argomento sconosciuto: {other}");
                print_help();
                exit(2);
            }
        }
    }
    config
}

fn read_state() -> Value {
    let value = fs::read_to_string(state_file())
        .ok()
        .and_then(|content| serde_json::from_str::<Value>(&content).ok())
        .unwrap_or_else(|| json!({}));
    if value.is_object() {
        value
    } else {
        json!({})
    }
}

fn write_state(state: &Value) {
    let path = state_file();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Err(err) = fs::write(&path, state.to_string()) {
        log(&format!("Impossibile salvare lo stato: {err}"));
    }
}

fn notify(title: &str, message: &str, urgency: Urgency, icon: &str) {
    let result = Notification::new()
        .appname("tplink-battery-monitor")
        .summary(title)
        .body(message)
        .urgency(urgency)
        .icon(icon)
        .show();
    if let Err(err) = result {
        log(&format!("Invio notifica fallito: {err}"));
    }
}

/// Icona TP-Link incorporata nel binario (nessun file a runtime).
///
/// I dati ARGB32 sono pre-renderizzati dall'SVG in `assets/tplink.svg` alle
/// dimensioni usate dai pannelli. Per rigenerarli:
///
/// ```text
/// rsvg-convert -w N -h N -o tplink-N.png assets/tplink.svg
/// # poi convertire RGBA -> ARGB32 (A,R,G,B) e salvare assets/tplink-N.argb
/// ```
fn tplink_icon_pixmap(dimmed: bool) -> Vec<ksni::Icon> {
    const SIZES: &[i32] = &[16, 22, 24, 32, 48, 64];
    const DATA: &[&[u8]] = &[
        include_bytes!("../assets/tplink-16.argb"),
        include_bytes!("../assets/tplink-22.argb"),
        include_bytes!("../assets/tplink-24.argb"),
        include_bytes!("../assets/tplink-32.argb"),
        include_bytes!("../assets/tplink-48.argb"),
        include_bytes!("../assets/tplink-64.argb"),
    ];
    SIZES
        .iter()
        .zip(DATA.iter())
        .map(|(size, data)| ksni::Icon {
            width: *size,
            height: *size,
            data: if dimmed {
                icons::argb_powered_off(data)
            } else {
                data.to_vec()
            },
        })
        .collect()
}

/// Compone l'etichetta testuale mostrata accanto all'icona nel pannello.
///
/// Esempi: ` 47%`, ` 47% · 266.14 GB rim.`, ` 266.14 GB rim.` (solo dati)
/// oppure stringa vuota se entrambi i valori sono nascosti o il router non
/// risponde (in quel caso parla l'icona scura).
///
/// `show` e' la coppia `(percentuale_batteria, giga)`.
fn format_panel_label(
    connected: bool,
    level: Option<i64>,
    charging: bool,
    show: (bool, bool),
    has_limit: bool,
    remaining: Option<&str>,
    total: &str,
) -> String {
    let (show_battery, show_data) = show;
    if !connected {
        // Router spento: nessun testo, basta l'icona scura.
        return String::new();
    }

    let battery = match level {
        Some(lvl) if charging => format!("{lvl}%+"),
        Some(lvl) => format!("{lvl}%"),
        None => "--%".to_string(),
    };

    let data = if show_data {
        if has_limit {
            Some(format!("{} rim.", remaining.unwrap_or("--")))
        } else if total.is_empty() {
            None
        } else {
            Some(total.to_string())
        }
    } else {
        None
    };

    match (show_battery, data) {
        (true, Some(data)) => format!(" {battery} · {data}"),
        (true, None) => format!(" {battery}"),
        (false, Some(data)) => format!(" {data}"),
        (false, None) => String::new(),
    }
}

/// Barra testuale a tutta larghezza usata nel menu (una riga dedicata sotto
/// la voce), per un colpo d'occhio immediato su batteria e consumo.
///
/// Il colore non e' disponibile nelle voci di menu GNOME, quindi batteria e
/// dati usano due texture diverse (`filled`/`empty`) per distinguerle.
fn menu_bar(percent: f64, length: usize, filled: char, empty: char) -> String {
    let pct = percent.clamp(0.0, 100.0);
    let count = ((pct / 100.0) * length as f64).round() as usize;
    format!(
        "[{}{}]",
        filled.to_string().repeat(count),
        empty.to_string().repeat(length - count)
    )
}

fn process_battery_notifications(
    level: i64,
    charging: bool,
    threshold: f64,
    notify_enabled: bool,
    cooldown_minutes: f64,
    notify_recovery: bool,
) {
    let mut state = read_state();
    let was_low = state.get("low").and_then(Value::as_bool).unwrap_or(false);
    let low = (level as f64) < threshold && !charging;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0);

    // --- transizioni di carica -----------------------------------------
    match state.get("charging").and_then(Value::as_bool) {
        Some(known_charging) => {
            if !known_charging && charging {
                if notify_enabled {
                    notify(
                        "Router M7350: carica avviata",
                        &format!("Batteria al {level}%. Caricabatterie collegato."),
                        Urgency::Normal,
                        "battery-charging",
                    );
                }
                log(&format!("Carica avviata ({level}%)"));
                state["full_notified"] = json!(false);
            } else if known_charging && charging {
                let full_notified = state
                    .get("full_notified")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                if level >= 100 && !full_notified {
                    if notify_enabled {
                        notify(
                            "Router M7350: carica completa",
                            "Batteria al 100%.",
                            Urgency::Normal,
                            "battery-full",
                        );
                    }
                    log("Carica completa (100%)");
                    state["full_notified"] = json!(true);
                }
            } else if known_charging && !charging {
                if level >= 100 {
                    log(&format!(
                        "Caricabatterie rimosso a carica completa ({level}%)"
                    ));
                } else {
                    if notify_enabled {
                        notify(
                            "Router M7350: carica interrotta",
                            &format!("La carica si è fermata al {level}%."),
                            Urgency::Normal,
                            "battery-low",
                        );
                    }
                    log(&format!("Carica interrotta ({level}%)"));
                }
                state["full_notified"] = json!(false);
            }
        }
        None => {
            state["full_notified"] = json!(false);
        }
    }

    // --- soglia batteria ------------------------------------------------
    if low {
        let last_notify = state
            .get("last_notify")
            .and_then(Value::as_f64)
            .unwrap_or(0.0);
        let due = (now - last_notify) >= cooldown_minutes * 60.0;
        if !was_low || due {
            if notify_enabled {
                notify(
                    &format!("Router M7350: batteria al {level}%"),
                    "Livello sotto la soglia. Collega il caricabatterie!",
                    Urgency::Critical,
                    "battery-caution",
                );
            }
            log(&format!(
                "NOTIFICA: batteria bassa ({level}% < {threshold:.0}%)"
            ));
            state["last_notify"] = json!(now);
        }
    } else if was_low && !charging && (level as f64) >= threshold {
        if notify_recovery && notify_enabled {
            notify(
                &format!("Router M7350: batteria al {level}%"),
                "Livello tornato sopra la soglia.",
                Urgency::Normal,
                "battery-good",
            );
        }
        log(&format!("Batteria recuperata ({level}%)"));
    }

    state["charging"] = json!(charging);
    state["low"] = json!(low);
    state["level"] = json!(level);
    write_state(&state);
}

fn print_data_overview(client: &mut MiFiClient) {
    let summary = match client.summary() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Errore connessione router: {e}");
            exit(1);
        }
    };

    let charge_str = if summary.battery.charging {
        "in carica"
    } else {
        "a batteria"
    };

    println!("══════════════════════════════════════════════════");
    println!("  TP-Link {} Status", summary.model);
    println!("══════════════════════════════════════════════════");
    println!(
        "  Batteria:       {}% ({charge_str})",
        summary.battery.level
    );
    println!("──────────────────────────────────────────────────");
    println!(
        "  Dati usati:     {} / {}",
        summary.data.total_formatted, summary.data.limit_formatted
    );
    if summary.data.has_limit {
        println!("  Avanzamento:    {} usato", summary.data.progress_bar);
        println!(
            "  Rimanenti:      {}",
            summary.data.remaining_formatted.as_deref().unwrap_or("N/A")
        );
    }
    println!("  Consumo oggi:   {}", summary.data.daily_formatted);
    println!("──────────────────────────────────────────────────");
    let net_op = if !summary.data.operator.is_empty() {
        summary.data.operator.as_str()
    } else {
        "TP-Link"
    };
    println!("  Rete:           {net_op} ({})", summary.data.network_type);
    println!(
        "  Segnale:        {}/4 ({}%)",
        summary.data.signal_strength, summary.data.signal_percent
    );
    println!("  Dispositivi:    {} connessi", summary.connected_devices);
    println!(
        "  Velocità:       ↓ {}   ↑ {}",
        summary.data.rx_speed_formatted, summary.data.tx_speed_formatted
    );
    println!("══════════════════════════════════════════════════");
}

fn check_once(
    client: &mut MiFiClient,
    threshold: f64,
    notify_enabled: bool,
    cooldown_minutes: f64,
    notify_recovery: bool,
) {
    let summary = match client.summary() {
        Ok(s) => s,
        Err(err) => {
            log(&format!("Router non raggiungibile: {err}"));
            return;
        }
    };

    let level = summary.battery.level;
    let charging = summary.battery.charging;
    let model = summary.model;

    log(&format!(
        "Batteria {level}% ({}), modello {model}",
        if charging { "in carica" } else { "a batteria" }
    ));

    process_battery_notifications(
        level,
        charging,
        threshold,
        notify_enabled,
        cooldown_minutes,
        notify_recovery,
    );
}

// ----------------------------------------------------------------------- Tray
struct MiFiTray {
    host: String,
    model: String,
    connected: bool,
    last_update: Option<String>,
    battery_level: Option<i64>,
    battery_charging: bool,
    data_total: String,
    data_limit: String,
    data_remaining: Option<String>,
    data_daily: String,
    usage_percent: Option<f64>,
    has_limit: bool,
    operator: String,
    network_type: String,
    signal_strength: i64,
    signal_percent: i64,
    connected_devices: i64,
    rx_speed: String,
    tx_speed: String,
    show_data_in_panel: bool,
    show_battery_in_panel: bool,
    power_save_enabled: Option<bool>,
    notifications_enabled: Arc<AtomicBool>,
    refresh_requested: Arc<AtomicBool>,
    power_save_request: Arc<AtomicI32>,
}

impl MiFiTray {
    /// Salva le preferenze correnti del menu su disco.
    fn persist_prefs(&self) {
        save_prefs(&Prefs {
            battery_in_panel: Some(self.show_battery_in_panel),
            data_in_panel: Some(self.show_data_in_panel),
            notifications: Some(self.notifications_enabled.load(Ordering::SeqCst)),
        });
    }
}

impl ksni::Tray for MiFiTray {
    const MENU_ON_ACTIVATE: bool = true;

    fn id(&self) -> String {
        "tplink-battery-monitor".into()
    }

    fn watcher_offline(&self, _reason: ksni::OfflineReason) -> bool {
        // Ritorna true per mantenere il servizio tray vivo anche se GNOME Shell
        // o StatusNotifierWatcher si riavvia / ricarica
        true
    }

    fn watcher_online(&self) {
        log("StatusNotifierWatcher tornato online (GNOME Shell / tray ricollegato)");
    }

    fn title(&self) -> String {
        if let Some(lvl) = self.battery_level {
            format!("{lvl}% - TP-Link {}", self.model)
        } else {
            format!("TP-Link {}", self.model)
        }
    }

    /// Testo mostrato accanto all'icona nella barra di sistema (Ayatana
    /// `XAyatanaLabel`): percentuale batteria ed eventualmente i giga.
    fn label(&self) -> String {
        format_panel_label(
            self.connected,
            self.battery_level,
            self.battery_charging,
            (self.show_battery_in_panel, self.show_data_in_panel),
            self.has_limit,
            self.data_remaining.as_deref(),
            &self.data_total,
        )
    }

    fn icon_name(&self) -> String {
        // Vuoto: l'host usa l'IconPixmap con il logo TP-Link incorporato.
        String::new()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        // Quando il router non risponde, l'icona viene mostrata "spenta"
        // (desaturata e piu' scura).
        tplink_icon_pixmap(!self.connected)
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        let title = if let Some(lvl) = self.battery_level {
            format!("TP-Link {} ({}%)", self.model, lvl)
        } else {
            format!("TP-Link {}", self.model)
        };
        let description = if self.connected {
            let charge_info = if self.battery_charging {
                "in carica"
            } else {
                "a batteria"
            };
            if self.has_limit {
                format!(
                    "Batteria: {}% ({charge_info})\nDati: {} / {}\nRimanenti: {}\nOggi: {}",
                    self.battery_level.unwrap_or(0),
                    self.data_total,
                    self.data_limit,
                    self.data_remaining.as_deref().unwrap_or("N/A"),
                    self.data_daily
                )
            } else {
                format!(
                    "Batteria: {}% ({charge_info})\nDati: {}\nOggi: {}",
                    self.battery_level.unwrap_or(0),
                    self.data_total,
                    self.data_daily
                )
            }
        } else {
            "Router non raggiungibile".to_string()
        };

        ksni::ToolTip {
            title,
            description,
            icon_name: String::new(),
            icon_pixmap: tplink_icon_pixmap(!self.connected),
        }
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::*;

        let mut items: Vec<ksni::MenuItem<Self>> = Vec::new();

        // Intestazione: modello.
        items.push(
            StandardItem {
                label: self.model.clone(),
                enabled: false,
                ..Default::default()
            }
            .into(),
        );

        if self.connected {
            // Connessione: operatore e tipo di rete.
            let conn = if self.operator.is_empty() {
                self.network_type.clone()
            } else {
                format!("{} · {}", self.operator, self.network_type)
            };
            items.push(
                StandardItem {
                    label: conn,
                    icon_name: "network-wireless-symbolic".into(),
                    enabled: false,
                    ..Default::default()
                }
                .into(),
            );

            items.push(MenuItem::Separator);

            // Batteria: valore, poi barra a tutta larghezza sotto la voce.
            const BAR_LEN: usize = 22;
            let batt_label = match self.battery_level {
                Some(lvl) if self.battery_charging => format!("Batteria {lvl}% · in carica"),
                Some(lvl) => format!("Batteria {lvl}% · a batteria"),
                None => "Batteria n/d".to_string(),
            };
            items.push(
                StandardItem {
                    label: batt_label,
                    enabled: false,
                    ..Default::default()
                }
                .into(),
            );
            if let Some(lvl) = self.battery_level {
                items.push(
                    StandardItem {
                        label: menu_bar(lvl as f64, BAR_LEN, '█', '░'),
                        icon_data: icons::battery_png(self.battery_level),
                        enabled: false,
                        ..Default::default()
                    }
                    .into(),
                );
            }

            // Dati: valore, poi barra a tutta larghezza sotto la voce.
            let data_label = if self.has_limit {
                format!(
                    "Dati {} / {} · {:.0}%",
                    self.data_total,
                    self.data_limit,
                    self.usage_percent.unwrap_or(0.0)
                )
            } else {
                format!("Dati {}", self.data_total)
            };
            items.push(
                StandardItem {
                    label: data_label,
                    enabled: false,
                    ..Default::default()
                }
                .into(),
            );
            if self.has_limit {
                items.push(
                    StandardItem {
                        label: menu_bar(self.usage_percent.unwrap_or(0.0), BAR_LEN, '▓', '▒'),
                        icon_data: icons::data_bar_png(self.usage_percent.unwrap_or(0.0)),
                        enabled: false,
                        ..Default::default()
                    }
                    .into(),
                );
            }

            items.push(
                StandardItem {
                    label: format!("Oggi {}", self.data_daily),
                    icon_name: "x-office-calendar-symbolic".into(),
                    enabled: false,
                    ..Default::default()
                }
                .into(),
            );

            items.push(MenuItem::Separator);

            // Dettagli rete in un sottomenu per non affollare il menu.
            let net_details: Vec<ksni::MenuItem<Self>> = vec![
                StandardItem {
                    label: format!(
                        "Segnale {}/4 · {}%",
                        self.signal_strength, self.signal_percent
                    ),
                    icon_data: icons::signal_png(self.signal_strength),
                    enabled: false,
                    ..Default::default()
                }
                .into(),
                StandardItem {
                    label: format!("{} dispositivi", self.connected_devices),
                    icon_name: "computer-symbolic".into(),
                    enabled: false,
                    ..Default::default()
                }
                .into(),
                StandardItem {
                    label: format!("↓ {}   ↑ {}", self.rx_speed, self.tx_speed),
                    icon_name: "network-transmit-receive-symbolic".into(),
                    enabled: false,
                    ..Default::default()
                }
                .into(),
            ];
            items.push(
                SubMenu {
                    label: "Rete".into(),
                    submenu: net_details,
                    ..Default::default()
                }
                .into(),
            );
        } else {
            // Router spento: menu minimale.
            items.push(
                StandardItem {
                    label: "Router spento".into(),
                    icon_name: "network-offline-symbolic".into(),
                    enabled: false,
                    ..Default::default()
                }
                .into(),
            );
        }

        items.push(MenuItem::Separator);

        // Azioni.
        items.push(
            StandardItem {
                label: "Aggiorna".into(),
                icon_name: "view-refresh-symbolic".into(),
                activate: Box::new(|this: &mut Self| {
                    this.refresh_requested.store(true, Ordering::SeqCst);
                }),
                ..Default::default()
            }
            .into(),
        );

        let host_for_open = self.host.clone();
        items.push(
            StandardItem {
                label: "Apri router".into(),
                icon_name: "web-browser-symbolic".into(),
                activate: Box::new(move |_| {
                    let _ = std::process::Command::new("xdg-open")
                        .arg(format!("http://{host_for_open}"))
                        .spawn();
                }),
                ..Default::default()
            }
            .into(),
        );

        if self.connected {
            let ps_enabled = self.power_save_enabled.unwrap_or(false);
            items.push(
                CheckmarkItem {
                    label: "Risparmio energetico".into(),
                    checked: ps_enabled,
                    activate: Box::new(|this: &mut Self| {
                        let target = !this.power_save_enabled.unwrap_or(false);
                        this.power_save_enabled = Some(target);
                        this.power_save_request
                            .store(if target { 1 } else { -1 }, Ordering::SeqCst);
                        // Sveglia il worker per applicare subito il cambio.
                        this.refresh_requested.store(true, Ordering::SeqCst);
                    }),
                    ..Default::default()
                }
                .into(),
            );
        }

        items.push(MenuItem::Separator);

        // Toggle aspetto pannello e notifiche (persistenti tra i riavvii).
        items.push(
            CheckmarkItem {
                label: "Percentuale batteria".into(),
                checked: self.show_battery_in_panel,
                activate: Box::new(|this: &mut Self| {
                    this.show_battery_in_panel = !this.show_battery_in_panel;
                    this.persist_prefs();
                }),
                ..Default::default()
            }
            .into(),
        );

        items.push(
            CheckmarkItem {
                label: "Mostra GB".into(),
                checked: self.show_data_in_panel,
                activate: Box::new(|this: &mut Self| {
                    this.show_data_in_panel = !this.show_data_in_panel;
                    this.persist_prefs();
                }),
                ..Default::default()
            }
            .into(),
        );

        let is_checked = self.notifications_enabled.load(Ordering::SeqCst);
        items.push(
            CheckmarkItem {
                label: "Notifiche".into(),
                checked: is_checked,
                activate: Box::new(|this: &mut Self| {
                    let prev = this.notifications_enabled.load(Ordering::SeqCst);
                    this.notifications_enabled.store(!prev, Ordering::SeqCst);
                    this.persist_prefs();
                }),
                ..Default::default()
            }
            .into(),
        );

        items.push(MenuItem::Separator);

        items.push(
            StandardItem {
                label: "Esci".into(),
                icon_name: "application-exit-symbolic".into(),
                activate: Box::new(|_| {
                    std::process::exit(0);
                }),
                ..Default::default()
            }
            .into(),
        );

        items
    }
}

/// Opzioni di avvio della companion app.
struct TrayOptions {
    threshold: f64,
    interval_seconds: u64,
    cooldown_minutes: f64,
    notify_recovery: bool,
    show_data_in_panel: bool,
    show_battery_in_panel: bool,
    notify_enabled: bool,
}

fn run_tray(mut client: MiFiClient, options: TrayOptions) {
    let TrayOptions {
        threshold,
        interval_seconds,
        cooldown_minutes,
        notify_recovery,
        show_data_in_panel,
        show_battery_in_panel,
        notify_enabled,
    } = options;

    // Le preferenze salvate (toggle del menu) hanno la precedenza sui default.
    let prefs = load_prefs();
    let show_data_in_panel = prefs.data_in_panel.unwrap_or(show_data_in_panel);
    let show_battery_in_panel = prefs.battery_in_panel.unwrap_or(show_battery_in_panel);
    let notify_enabled = prefs.notifications.unwrap_or(notify_enabled);

    let refresh_requested = Arc::new(AtomicBool::new(false));
    let power_save_request = Arc::new(AtomicI32::new(0));
    let notifications_enabled = Arc::new(AtomicBool::new(notify_enabled));

    let host_cleaned = client
        .host()
        .trim_start_matches("http://")
        .trim_start_matches("https://")
        .to_string();

    let tray = MiFiTray {
        host: host_cleaned,
        model: "M7350".to_string(),
        connected: false,
        last_update: None,
        battery_level: None,
        battery_charging: false,
        data_total: "0.00 B".to_string(),
        data_limit: "Nessun limite".to_string(),
        data_remaining: None,
        data_daily: "0.00 B".to_string(),
        usage_percent: None,
        has_limit: false,
        operator: String::new(),
        network_type: "In attesa...".to_string(),
        signal_strength: 0,
        signal_percent: 0,
        connected_devices: 0,
        rx_speed: "0.0 KB/s".to_string(),
        tx_speed: "0.0 KB/s".to_string(),
        show_data_in_panel,
        show_battery_in_panel,
        power_save_enabled: None,
        notifications_enabled: notifications_enabled.clone(),
        refresh_requested: refresh_requested.clone(),
        power_save_request: power_save_request.clone(),
    };

    let handle = match tray.spawn() {
        Ok(h) => h,
        Err(e) => {
            eprintln!("Errore inizializzazione Tray StatusNotifierItem: {e}");
            exit(1);
        }
    };

    log("TP-Link Companion Tray avviata in background (D-Bus StatusNotifierItem)");

    let worker_handle = handle.clone();
    let worker_refresh = refresh_requested.clone();
    let worker_notif = notifications_enabled.clone();
    let worker_power = power_save_request.clone();

    let _worker = std::thread::spawn(move || loop {
        // Applica un'eventuale richiesta di risparmio energetico dal menu,
        // ritentando in caso di caduta di rete transitoria.
        let mut power_request = worker_power.swap(0, Ordering::SeqCst);
        let mut attempts = 0;
        while power_request != 0 && attempts < 3 {
            let enable = power_request > 0;
            match client.set_power_save(enable) {
                Ok(_) => {
                    log(&format!(
                        "Risparmio energetico {}",
                        if enable { "attivato" } else { "disattivato" }
                    ));
                    power_request = 0;
                }
                Err(err) => {
                    attempts += 1;
                    log(&format!(
                        "Errore risparmio energetico (tentativo {attempts}/3): {err}"
                    ));
                    sleep(Duration::from_secs(2));
                }
            }
        }
        if power_request != 0 {
            // Rimetti in coda e risveglia subito il worker per ritentare.
            worker_power.store(power_request, Ordering::SeqCst);
            worker_refresh.store(true, Ordering::SeqCst);
        }

        match client.summary() {
            Ok(summary) => {
                let now_str = chrono::Local::now().format("%H:%M").to_string();
                let level = summary.battery.level;
                let charging = summary.battery.charging;

                worker_handle.update(|t| {
                    t.connected = true;
                    t.model = summary.model.clone();
                    t.last_update = Some(now_str);
                    t.battery_level = Some(level);
                    t.battery_charging = charging;
                    t.data_total = summary.data.total_formatted.clone();
                    t.data_limit = summary.data.limit_formatted.clone();
                    t.data_remaining = summary.data.remaining_formatted.clone();
                    t.data_daily = summary.data.daily_formatted.clone();
                    t.usage_percent = summary.data.usage_percent;
                    t.has_limit = summary.data.has_limit;
                    t.operator = summary.data.operator.clone();
                    t.network_type = summary.data.network_type.clone();
                    t.signal_strength = summary.data.signal_strength;
                    t.signal_percent = summary.data.signal_percent;
                    t.connected_devices = summary.connected_devices;
                    t.rx_speed = summary.data.rx_speed_formatted.clone();
                    t.tx_speed = summary.data.tx_speed_formatted.clone();
                });

                let notify_active = worker_notif.load(Ordering::SeqCst);
                process_battery_notifications(
                    level,
                    charging,
                    threshold,
                    notify_active,
                    cooldown_minutes,
                    notify_recovery,
                );
            }
            Err(err) => {
                log(&format!("Errore lettura dati router: {err}"));
                worker_handle.update(|t| {
                    t.connected = false;
                });
            }
        }

        // Stato del risparmio energetico, per la spunta nel menu.
        if let Ok(config) = client.power_save() {
            let enabled = config.get("enable").and_then(Value::as_i64).unwrap_or(0) != 0;
            worker_handle.update(|t| t.power_save_enabled = Some(enabled));
        }

        let poll_ticks = (interval_seconds * 10).max(10);
        for _ in 0..poll_ticks {
            if worker_refresh.swap(false, Ordering::SeqCst) {
                break;
            }
            sleep(Duration::from_millis(100));
        }
    });

    // Mantieni il processo attivo
    loop {
        sleep(Duration::from_secs(3600));
    }
}

fn main() {
    // Ordine di ricerca configurazione:
    // 1. $TP_ENV_FILE
    // 2. .env nella directory corrente
    // 3. ~/.config/tplink-battery-monitor/.env
    if let Ok(path) = env::var("TP_ENV_FILE") {
        load_dotenv(&PathBuf::from(path));
    }
    load_dotenv(&PathBuf::from(".env"));
    load_dotenv(&home_dir().join(".config/tplink-battery-monitor/.env"));

    let config = parse_args();

    if config.install_autostart {
        install_autostart();
        return;
    }

    if config.uninstall_autostart {
        uninstall_autostart();
        return;
    }

    if config.password.is_empty() {
        eprintln!("specifica --password oppure imposta TPLINK_PASSWORD nel .env");
        exit(2);
    }

    let mut client = MiFiClient::new(&config.host, &config.password);

    if config.data {
        print_data_overview(&mut client);
        return;
    }

    if config.tray {
        let interval_sec = (config.interval_minutes * 60.0).max(10.0) as u64;
        run_tray(
            client,
            TrayOptions {
                threshold: config.threshold,
                interval_seconds: interval_sec,
                cooldown_minutes: config.cooldown_minutes,
                notify_recovery: config.notify_recovery,
                show_data_in_panel: config.show_data_in_panel,
                show_battery_in_panel: config.show_battery_in_panel,
                notify_enabled: !config.no_notify,
            },
        );
        return;
    }

    let notify_enabled = !config.no_notify;

    if config.once {
        check_once(
            &mut client,
            config.threshold,
            notify_enabled,
            config.cooldown_minutes,
            config.notify_recovery,
        );
        return;
    }

    log(&format!(
        "Monitor avviato: host={} soglia={:.0}% intervallo={}min",
        config.host, config.threshold, config.interval_minutes
    ));
    loop {
        check_once(
            &mut client,
            config.threshold,
            notify_enabled,
            config.cooldown_minutes,
            config.notify_recovery,
        );
        sleep(Duration::from_secs_f64(config.interval_minutes * 60.0));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panel_label_disconnected() {
        assert_eq!(
            format_panel_label(
                false,
                Some(50),
                false,
                (true, false),
                false,
                None,
                "1.00 GB"
            ),
            ""
        );
    }

    #[test]
    fn panel_label_battery_only() {
        assert_eq!(
            format_panel_label(true, Some(47), false, (true, false), false, None, "1.00 GB"),
            " 47%"
        );
    }

    #[test]
    fn panel_label_charging() {
        assert_eq!(
            format_panel_label(true, Some(47), true, (true, false), false, None, ""),
            " 47%+"
        );
    }

    #[test]
    fn panel_label_unknown_level() {
        assert_eq!(
            format_panel_label(true, None, false, (true, false), false, None, ""),
            " --%"
        );
    }

    #[test]
    fn panel_label_with_limit() {
        assert_eq!(
            format_panel_label(
                true,
                Some(47),
                false,
                (true, true),
                true,
                Some("266.14 GB"),
                "400.00 GB"
            ),
            " 47% \u{b7} 266.14 GB rim."
        );
    }

    #[test]
    fn panel_label_without_limit() {
        assert_eq!(
            format_panel_label(
                true,
                Some(47),
                false,
                (true, true),
                false,
                None,
                "133.86 GB"
            ),
            " 47% \u{b7} 133.86 GB"
        );
    }

    #[test]
    fn panel_label_data_only() {
        assert_eq!(
            format_panel_label(
                true,
                Some(47),
                false,
                (false, true),
                true,
                Some("266.14 GB"),
                "400.00 GB"
            ),
            " 266.14 GB rim."
        );
    }

    #[test]
    fn panel_label_empty_when_all_hidden() {
        assert_eq!(
            format_panel_label(
                true,
                Some(47),
                false,
                (false, false),
                true,
                Some("266.14 GB"),
                "400.00 GB"
            ),
            ""
        );
    }
}
