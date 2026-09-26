//! Monitor della batteria del router TP-Link M7350 con notifiche desktop.
//!
//! Traduzione 1:1 di `monitor.py`. Esegue il polling dell'API del router e
//! invia una notifica D-Bus quando la batteria scende sotto una soglia
//! (default 20%) e non e' in carica.
//!
//! Uso:
//!     tplink-battery-monitor              # loop continuo
//!     tplink-battery-monitor --once       # un singolo controllo (per systemd)
//!     tplink-battery-monitor --no-notify  # solo log, senza notifiche

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::exit;
use std::thread::sleep;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use notify_rust::{Notification, Urgency};
use serde_json::{json, Value};

use tplink_mifi::MiFiClient;

struct Config {
    host: String,
    password: String,
    threshold: f64,
    interval_minutes: f64,
    cooldown_minutes: f64,
    once: bool,
    no_notify: bool,
    notify_recovery: bool,
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

fn print_help() {
    println!(
        "Uso: tplink-battery-monitor [--host HOST] [--password PWD] \
         [--threshold N] [--interval MIN] [--cooldown MIN] \
         [--once] [--no-notify] [--notify-recovery]"
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
        no_notify: false,
        notify_recovery: false,
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
            "--no-notify" => config.no_notify = true,
            "--notify-recovery" => config.notify_recovery = true,
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

fn check_once(
    client: &mut MiFiClient,
    threshold: f64,
    notify_enabled: bool,
    cooldown_minutes: f64,
    notify_recovery: bool,
) {
    let battery = match client.battery() {
        Ok(battery) => battery,
        Err(err) => {
            log(&format!("Router non raggiungibile: {err}"));
            return;
        }
    };

    let level = battery.get("level").and_then(Value::as_i64);
    let charging = battery
        .get("charging")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let model = battery.get("model").and_then(Value::as_str).unwrap_or("?");
    let Some(level) = level else {
        log(&format!("Livello batteria non disponibile: {battery}"));
        return;
    };

    log(&format!(
        "Batteria {level}% ({}), modello {model}",
        if charging { "in carica" } else { "a batteria" }
    ));

    let mut state = read_state();
    let was_low = state.get("low").and_then(Value::as_bool).unwrap_or(false);
    let low = (level as f64) < threshold && !charging;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0);

    // --- notifiche legate alla carica (transizioni di stato) --------------
    match state.get("charging").and_then(Value::as_bool) {
        Some(known_charging) => {
            if !known_charging && charging {
                if notify_enabled {
                    notify(
                        "🔌 Router M7350: carica avviata",
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
                            "✅ Router M7350: carica completa",
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
                            "⚠️ Router M7350: carica interrotta",
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
            // primo avvio (stato senza "charging"): nessuna transizione da notificare
            state["full_notified"] = json!(false);
        }
    }

    // --- batteria bassa / recuperata --------------------------------------
    if low {
        let last_notify = state
            .get("last_notify")
            .and_then(Value::as_f64)
            .unwrap_or(0.0);
        let due = (now - last_notify) >= cooldown_minutes * 60.0;
        if !was_low || due {
            if notify_enabled {
                notify(
                    &format!("⚠️ Router M7350: batteria al {level}%"),
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
                &format!("🔋 Router M7350: batteria al {level}%"),
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

fn main() {
    // Ordine di ricerca del file di configurazione.
    if let Ok(path) = env::var("TP_ENV_FILE") {
        load_dotenv(&PathBuf::from(path));
    }
    load_dotenv(&PathBuf::from(".env"));
    load_dotenv(&home_dir().join(".config/tplink-battery-monitor/.env"));

    let config = parse_args();
    if config.password.is_empty() {
        eprintln!("specifica --password oppure imposta TPLINK_PASSWORD nel .env");
        exit(2);
    }

    let mut client = MiFiClient::new(&config.host, &config.password);
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
