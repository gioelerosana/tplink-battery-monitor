//! Client minimale per l'API web dei router mobili TP-Link (es. M7350).
//!
//! Reverse engineered dall'interfaccia
//! web del router (`js/tpweb.min.js`, `js/libs.min.js`). L'API e' JSON su due
//! endpoint:
//!
//!   POST /cgi-bin/auth_cgi   modulo "authenticator" (login)
//!   POST /cgi-bin/web_cgi    tutti gli altri moduli (es. "status")
//!
//! Formato delle richieste:
//!
//! ```text
//! - moduli senza cifratura:   {"data": "<base64 del JSON>"}
//! - richieste cifrate (GDPR): {"data": "<AES-128-CBC base64>",
//!                              "sign": "<RSA-512 PKCS#1 v1.5 hex>"}
//! ```
//!
//! La firma RSA contiene:
//!
//! ```text
//! login -> "key=<aesKey>&iv=<aesIv>&h=<md5(admin+pwd)>&s=<seq+len(data)>"
//! resto -> "h=<md5(admin+pwd)>&s=<seq+len(data)>"
//! ```
//!
//! Le risposte cifrate sono base64 di AES-128-CBC con la stessa key/iv.

use std::fmt;
use std::time::Duration;

use aes::Aes128;
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use cbc::cipher::{block_padding::Pkcs7, BlockDecryptMut, BlockEncryptMut, KeyIvInit};
use chrono::{Datelike, NaiveDate};
use md5::{Digest, Md5};
use rand::distributions::Alphanumeric;
use rand::Rng;
use rsa::pkcs1v15::Pkcs1v15Encrypt;
use rsa::traits::PublicKeyParts;
use rsa::{BigUint, RsaPublicKey};
use serde_json::{json, Value};

const AUTH_CGI: &str = "/cgi-bin/auth_cgi";
const WEB_CGI: &str = "/cgi-bin/web_cgi";

type AesEnc = cbc::Encryptor<Aes128>;
type AesDec = cbc::Decryptor<Aes128>;

#[derive(Debug)]
pub enum TpLinkError {
    Http(String),
    Auth(String),
    WrongPassword,
    Protocol(String),
}

impl fmt::Display for TpLinkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TpLinkError::Http(m) => write!(f, "HTTP: {m}"),
            TpLinkError::Auth(m) => write!(f, "Autenticazione: {m}"),
            TpLinkError::WrongPassword => write!(f, "Autenticazione: password del router errata"),
            TpLinkError::Protocol(m) => write!(f, "Protocollo: {m}"),
        }
    }
}

impl std::error::Error for TpLinkError {}

pub type Result<T> = std::result::Result<T, TpLinkError>;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BatteryInfo {
    pub connected: bool,
    pub charging: bool,
    pub level: i64,
    pub voltage: i64,
    pub model: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DataUsage {
    pub total_bytes: f64,
    pub total_formatted: String,
    pub daily_bytes: f64,
    pub daily_formatted: String,
    pub limit_bytes: f64,
    pub limit_formatted: String,
    pub has_limit: bool,
    pub remaining_bytes: Option<f64>,
    pub remaining_formatted: Option<String>,
    pub usage_percent: Option<f64>,
    pub progress_bar: String,
    pub operator: String,
    pub network_type_code: i64,
    pub network_type: String,
    pub signal_strength: i64,
    pub signal_percent: i64,
    pub rx_speed: f64,
    pub rx_speed_formatted: String,
    pub tx_speed: f64,
    pub tx_speed_formatted: String,
}

/// Budget giornaliero di traffico: quanto si puo' consumare oggi per arrivare
/// al rinnovo del piano senza esaurire la quota.
///
/// Il budget e' fissato a inizio giornata (dati rimasti a mezzanotte divisi
/// per i giorni al rinnovo), quindi non cala mentre si consuma: se ieri si e'
/// consumato meno del previsto, oggi il budget cresce, e viceversa.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DailyBudget {
    pub budget_bytes: f64,
    pub used_bytes: f64,
    /// Consumo di oggi rispetto al budget; oltre 100 se superato.
    pub percent: f64,
    /// Budget di domani se da qui a mezzanotte non si consuma altro
    /// (`None` l'ultimo giorno prima del rinnovo).
    pub tomorrow_bytes: Option<f64>,
    /// Giorni al rinnovo, oggi compreso.
    pub days_left: i64,
    /// Data del prossimo rinnovo (`gg/mm`).
    pub renewal_formatted: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DeviceSummary {
    pub model: String,
    pub firmware: String,
    pub battery: BatteryInfo,
    pub data: DataUsage,
    /// Presente solo con limite dati e rinnovo mensile attivi sul router.
    pub daily_budget: Option<DailyBudget>,
    pub connected_devices: i64,
}

pub fn format_bytes(bytes: f64) -> String {
    let mut val = bytes;
    for unit in ["B", "KB", "MB", "GB", "TB"] {
        if val.abs() < 1024.0 || unit == "TB" {
            return format!("{val:.2} {unit}");
        }
        val /= 1024.0;
    }
    format!("{val:.2} GB")
}

pub fn format_speed(bps: f64) -> String {
    if bps >= 1024.0 * 1024.0 {
        format!("{:.1} MB/s", bps / (1024.0 * 1024.0))
    } else {
        format!("{:.1} KB/s", bps / 1024.0)
    }
}

pub fn format_progress_bar(percent: f64, length: usize) -> String {
    let pct = percent.clamp(0.0, 100.0);
    let filled = ((pct / 100.0) * length as f64).round() as usize;
    let empty = length.saturating_sub(filled);
    format!("[{}{}] {:.1}%", "█".repeat(filled), "░".repeat(empty), pct)
}

/// Data di rinnovo nel mese indicato. Il router accetta giorni 1-31: nei mesi
/// piu' corti il rinnovo cade sull'ultimo giorno del mese.
fn renewal_in_month(year: i32, month: u32, payment_day: u32) -> Option<NaiveDate> {
    (1..=payment_day.clamp(1, 31))
        .rev()
        .find_map(|day| NaiveDate::from_ymd_opt(year, month, day))
}

/// Prossimo rinnovo del piano dati dopo `today` (se il rinnovo e' oggi, conta
/// quello del mese successivo: il contatore e' gia' stato azzerato).
pub fn next_renewal(payment_day: u32, today: NaiveDate) -> Option<NaiveDate> {
    let this_month = renewal_in_month(today.year(), today.month(), payment_day)?;
    if this_month > today {
        return Some(this_month);
    }
    let (year, month) = if today.month() == 12 {
        (today.year() + 1, 1)
    } else {
        (today.year(), today.month() + 1)
    };
    renewal_in_month(year, month, payment_day)
}

/// Calcola il budget giornaliero dalla quota del piano, dal traffico del
/// ciclo corrente (`total_bytes`, oggi compreso) e da quello di oggi.
pub fn daily_budget(
    limit_bytes: f64,
    total_bytes: f64,
    daily_bytes: f64,
    payment_day: u32,
    today: NaiveDate,
) -> Option<DailyBudget> {
    if limit_bytes <= 0.0 {
        return None;
    }
    let renewal = next_renewal(payment_day, today)?;
    let days_left = (renewal - today).num_days().max(1);
    let used_before_today = (total_bytes - daily_bytes).max(0.0);
    let budget_bytes = (limit_bytes - used_before_today).max(0.0) / days_left as f64;
    let percent = if budget_bytes > 0.0 {
        daily_bytes / budget_bytes * 100.0
    } else if daily_bytes > 0.0 {
        100.0
    } else {
        0.0
    };
    let tomorrow_bytes =
        (days_left > 1).then(|| (limit_bytes - total_bytes).max(0.0) / (days_left - 1) as f64);
    Some(DailyBudget {
        budget_bytes,
        used_bytes: daily_bytes,
        percent,
        tomorrow_bytes,
        days_left,
        renewal_formatted: renewal.format("%d/%m").to_string(),
    })
}

pub fn network_type_name(code: i64) -> &'static str {
    match code {
        0 => "Nessun servizio",
        1 => "2G (GSM)",
        2 => "3G (WCDMA)",
        3 => "4G (LTE)",
        4 => "3G (TD-SCDMA)",
        5 => "CDMA 1x",
        6 => "CDMA EVDO",
        7 => "4G+ (LTE+)",
        _ => "Sconosciuto",
    }
}

pub struct MiFiClient {
    host: String,
    username: String,
    password: String,
    timeout: Duration,
    token: Option<String>,
    seq: u64,
    aes_key: String,
    aes_iv: String,
    rsa_nn: String,
    rsa_ee: String,
    hash: String,
}

impl MiFiClient {
    pub fn new(host: &str, password: &str) -> Self {
        let host = if host.starts_with("http://") || host.starts_with("https://") {
            host.to_string()
        } else {
            format!("http://{host}")
        };
        Self {
            host: host.trim_end_matches('/').to_string(),
            username: "admin".to_string(),
            password: password.to_string(),
            timeout: Duration::from_secs(8),
            token: None,
            seq: 0,
            aes_key: String::new(),
            aes_iv: String::new(),
            rsa_nn: String::new(),
            rsa_ee: String::new(),
            hash: String::new(),
        }
    }

    pub fn host(&self) -> &str {
        &self.host
    }

    // ------------------------------------------------------------------ HTTP
    fn post(&self, path: &str, payload: &Value) -> Result<String> {
        let url = format!("{}{}", self.host, path);
        let body =
            serde_json::to_string(payload).map_err(|e| TpLinkError::Protocol(e.to_string()))?;
        let response = ureq::post(&url)
            .timeout(self.timeout)
            .set("Content-Type", "application/json")
            .set("Referer", &format!("{}/", self.host))
            .set("Origin", &self.host)
            .send_string(&body);
        match response {
            Ok(r) => r
                .into_string()
                .map_err(|e| TpLinkError::Http(e.to_string())),
            Err(ureq::Error::Status(code, _)) => Err(TpLinkError::Http(format!("HTTP {code}"))),
            Err(e) => Err(TpLinkError::Http(format!(
                "Impossibile contattare {}: {e}",
                self.host
            ))),
        }
    }

    // -------------------------------------------------------------- cifratura
    fn aes_encrypt(&self, plaintext: &str) -> Result<String> {
        let enc = AesEnc::new_from_slices(self.aes_key.as_bytes(), self.aes_iv.as_bytes())
            .map_err(|e| TpLinkError::Protocol(e.to_string()))?;
        let mut buf = vec![0u8; plaintext.len() + 16];
        let ct = enc
            .encrypt_padded_b2b_mut::<Pkcs7>(plaintext.as_bytes(), &mut buf)
            .map_err(|e| TpLinkError::Protocol(e.to_string()))?;
        Ok(B64.encode(ct))
    }

    fn aes_decrypt(&self, ciphertext_b64: &str) -> Result<String> {
        let raw = B64
            .decode(ciphertext_b64.trim())
            .map_err(|e| TpLinkError::Protocol(e.to_string()))?;
        let dec = AesDec::new_from_slices(self.aes_key.as_bytes(), self.aes_iv.as_bytes())
            .map_err(|e| TpLinkError::Protocol(e.to_string()))?;
        let pt = dec
            .decrypt_padded_vec_mut::<Pkcs7>(&raw)
            .map_err(|e| TpLinkError::Protocol(e.to_string()))?;
        Ok(String::from_utf8_lossy(&pt).into_owned())
    }

    /// Firma il messaggio con RSA-512 a blocchi (max 53 byte ciascuno).
    ///
    /// L'encryptor del firmware (`lb`) spezza la stringa in chunk di `e-11`
    /// byte e concatena le cifrature esadecimali, perche' una singola chiave
    /// a 512 bit non puo' cifrare piu' di 53 byte.
    fn rsa_sign(&self, message: &str) -> Result<String> {
        let n = BigUint::from_bytes_be(&hex_decode(&self.rsa_nn)?);
        let e = BigUint::from_bytes_be(&hex_decode(&self.rsa_ee)?);
        let public =
            RsaPublicKey::new(n, e).map_err(|e| TpLinkError::Protocol(format!("RSA key: {e}")))?;
        let chunk_size = public.size().saturating_sub(11);
        let mut rng = rand::thread_rng();
        let mut out = String::new();
        for chunk in message.as_bytes().chunks(chunk_size) {
            let ct = public
                .encrypt(&mut rng, Pkcs1v15Encrypt, chunk)
                .map_err(|e| TpLinkError::Protocol(format!("RSA encrypt: {e}")))?;
            out.push_str(&hex_encode(&ct));
        }
        Ok(out)
    }

    fn sign(&self, data_b64: &str, is_login: bool) -> Result<String> {
        let seq = self.seq + data_b64.len() as u64;
        let message = if is_login {
            format!(
                "key={}&iv={}&h={}&s={}",
                self.aes_key, self.aes_iv, self.hash, seq
            )
        } else {
            format!("h={}&s={}", self.hash, seq)
        };
        self.rsa_sign(&message)
    }

    fn encrypt_payload(&self, obj: &Value, is_login: bool) -> Result<String> {
        let data = self.aes_encrypt(&serde_json::to_string(obj).unwrap())?;
        let sign = self.sign(&data, is_login)?;
        Ok(json!({ "data": data, "sign": sign }).to_string())
    }

    // ---------------------------------------------------------------- parsing
    /// Decodifica una risposta: JSON in chiaro, AES (GDPR) o base64+JSON.
    fn decode(&self, text: &str) -> Result<Value> {
        let text = text.trim();
        if let Ok(value) = serde_json::from_str::<Value>(text) {
            return Ok(value);
        }
        if !self.aes_key.is_empty() {
            if let Ok(decrypted) = self.aes_decrypt(text) {
                if let Ok(value) = serde_json::from_str::<Value>(&decrypted) {
                    return Ok(value);
                }
            }
        }
        if text.is_empty() {
            return Err(TpLinkError::Protocol(
                "Risposta vuota dal router".to_string(),
            ));
        }
        let raw = B64.decode(text).map_err(|e| {
            TpLinkError::Protocol(format!("Decodifica base64 fallita: {e} (testo: {text})"))
        })?;
        serde_json::from_slice(&raw)
            .map_err(|e| TpLinkError::Protocol(format!("Decodifica JSON payload fallita: {e}")))
    }

    // ------------------------------------------------------------------ login
    fn auth_load(&self) -> Result<Value> {
        let inner = json!({ "module": "authenticator", "action": 0 });
        let encoded = B64.encode(serde_json::to_vec(&inner).unwrap());
        self.decode(&self.post(AUTH_CGI, &json!({ "data": encoded }))?)
    }

    pub fn login(&mut self) -> Result<String> {
        let info = self.auth_load()?;
        let result = info.get("result").and_then(Value::as_i64).unwrap_or(-999);
        if result != 0 && result != 1 {
            return Err(TpLinkError::Auth(format!("auth_cgi load fallito: {info}")));
        }
        let nonce = info
            .get("nonce")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        self.rsa_nn = info
            .get("rsaMod")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        self.rsa_ee = info
            .get("rsaPubKey")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        self.seq = info.get("seqNum").and_then(Value::as_u64).unwrap_or(0);

        self.aes_key = random_token(16);
        self.aes_iv = random_token(16);
        self.hash = md5_hex(&format!("{}{}", self.username, self.password));
        let digest = md5_hex(&format!("{}:{}", self.password, nonce));

        let payload = self.encrypt_payload(
            &json!({ "module": "authenticator", "action": 1, "digest": digest }),
            true,
        )?;
        let resp = self.decode(&self.post(AUTH_CGI, &serde_json::from_str(&payload).unwrap())?)?;
        let result = resp.get("result").and_then(Value::as_i64).unwrap_or(-999);
        if result == 1 {
            return Err(TpLinkError::WrongPassword);
        }
        if result != 0 {
            return Err(TpLinkError::Auth(format!(
                "Login fallito (result={result}): {resp}"
            )));
        }
        let token = resp
            .get("token")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        if token.is_empty() {
            return Err(TpLinkError::Auth(format!("Login senza token: {resp}")));
        }
        self.token = Some(token.clone());
        Ok(token)
    }

    // ---------------------------------------------------------------- richieste
    fn execute_call(
        &self,
        module: &str,
        action: i64,
        data: Option<&Value>,
        token: Option<&str>,
    ) -> Result<Value> {
        let mut obj = serde_json::Map::new();
        obj.insert("module".to_string(), json!(module));
        obj.insert("action".to_string(), json!(action));
        if let Some(Value::Object(map)) = data {
            for (key, value) in map {
                obj.insert(key.clone(), value.clone());
            }
        }
        if let Some(t) = token {
            obj.insert("token".to_string(), json!(t));
        }
        let payload = self.encrypt_payload(&Value::Object(obj), false)?;
        let path = if module == "authenticator" {
            AUTH_CGI
        } else {
            WEB_CGI
        };
        let raw = self.post(path, &serde_json::from_str(&payload).unwrap())?;
        self.decode(&raw)
    }

    pub fn call(&mut self, module: &str, action: i64, data: Option<&Value>) -> Result<Value> {
        if module != "authenticator" && self.token.is_none() {
            self.login()?;
        }

        let res = self.execute_call(module, action, data, self.token.as_deref());
        match res {
            Ok(v) if v.get("result").and_then(Value::as_i64) == Some(0) => Ok(v),
            Ok(_v) if module != "authenticator" => {
                // Token scaduto o invalido: riprova login
                self.token = None;
                self.login()?;
                self.execute_call(module, action, data, self.token.as_deref())
            }
            Err(_) if module != "authenticator" => {
                // Sessione o cifratura compromessa: re-autenticati
                self.token = None;
                self.login()?;
                self.execute_call(module, action, data, self.token.as_deref())
            }
            other => other,
        }
    }

    // ---------------------------------------------------------------- comandi
    pub fn get_status(&mut self) -> Result<Value> {
        let mut resp = self.call("status", 0, None)?;
        if resp.get("result").and_then(Value::as_i64) != Some(0) {
            // token scaduto: riprova una volta dopo nuovo login
            self.token = None;
            self.login()?;
            resp = self.call("status", 0, None)?;
        }
        Ok(resp)
    }

    /// Livello batteria.
    ///
    /// Nota: nel firmware il campo si chiama "voltage" ma contiene gia' la
    /// percentuale (0-100); l'interfaccia web lo usa direttamente per
    /// scegliere l'icona. Lo esponiamo anche come "level".
    pub fn battery(&mut self) -> Result<Value> {
        let status = self.get_status()?;
        let battery = status.get("battery").cloned().unwrap_or_else(|| json!({}));
        let model = status
            .pointer("/deviceInfo/model")
            .cloned()
            .unwrap_or(Value::Null);
        let voltage = battery.get("voltage").cloned().unwrap_or(Value::Null);
        Ok(json!({
            "connected": battery.get("connected").cloned().unwrap_or(Value::Null),
            "charging": battery.get("charging").cloned().unwrap_or(Value::Null),
            "level": voltage,
            "voltage": voltage,
            "model": model,
        }))
    }

    /// Statistiche sul consumo dati (giga consumati, totale piano e rimanenti).
    pub fn data_usage(&mut self, status: Option<&Value>) -> Result<DataUsage> {
        let fetched_status;
        let s = match status {
            Some(val) => val,
            None => {
                fetched_status = self.get_status()?;
                &fetched_status
            }
        };

        let wan = s.get("wan");
        let parse_f64 = |key: &str| -> f64 {
            wan.and_then(|w| w.get(key))
                .and_then(|v| {
                    if let Some(num) = v.as_f64() {
                        Some(num)
                    } else if let Some(st) = v.as_str() {
                        st.parse::<f64>().ok()
                    } else {
                        None
                    }
                })
                .unwrap_or(0.0)
        };

        let total_bytes = parse_f64("totalStatistics");
        let daily_bytes = parse_f64("dailyStatistics");
        let limit_bytes = parse_f64("limitation");
        let has_limit = wan
            .and_then(|w| w.get("enableDataLimit"))
            .and_then(Value::as_bool)
            .unwrap_or(false)
            && limit_bytes > 0.0;

        let remaining_bytes = if has_limit {
            Some((limit_bytes - total_bytes).max(0.0))
        } else {
            None
        };

        let usage_percent = if has_limit && limit_bytes > 0.0 {
            Some((total_bytes / limit_bytes * 100.0).clamp(0.0, 100.0))
        } else {
            None
        };

        let progress_bar = match usage_percent {
            Some(pct) => format_progress_bar(pct, 12),
            None => String::new(),
        };

        let operator = wan
            .and_then(|w| w.get("operatorName"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();

        let network_type_code = wan
            .and_then(|w| w.get("networkType"))
            .and_then(Value::as_i64)
            .unwrap_or(-1);

        let signal_strength = wan
            .and_then(|w| w.get("signalStrength"))
            .and_then(Value::as_i64)
            .unwrap_or(0);

        let rx_speed = parse_f64("rxSpeed");
        let tx_speed = parse_f64("txSpeed");

        Ok(DataUsage {
            total_bytes,
            total_formatted: format_bytes(total_bytes),
            daily_bytes,
            daily_formatted: format_bytes(daily_bytes),
            limit_bytes,
            limit_formatted: if has_limit {
                format_bytes(limit_bytes)
            } else {
                "Nessun limite".to_string()
            },
            has_limit,
            remaining_bytes,
            remaining_formatted: remaining_bytes.map(format_bytes),
            usage_percent,
            progress_bar,
            operator,
            network_type_code,
            network_type: network_type_name(network_type_code).to_string(),
            signal_strength,
            signal_percent: (signal_strength as f64 / 4.0 * 100.0) as i64,
            rx_speed,
            rx_speed_formatted: format_speed(rx_speed),
            tx_speed,
            tx_speed_formatted: format_speed(tx_speed),
        })
    }

    /// Riepilogo unificato (modello, batteria, dati, dispositivi connessi).
    pub fn summary(&mut self) -> Result<DeviceSummary> {
        let status = self.get_status()?;
        let model = status
            .pointer("/deviceInfo/model")
            .and_then(Value::as_str)
            .unwrap_or("M7350")
            .to_string();
        let firmware = status
            .pointer("/deviceInfo/firmwareVersion")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();

        let battery = status.get("battery");
        let level = battery
            .and_then(|b| b.get("voltage"))
            .and_then(Value::as_i64)
            .unwrap_or(0);
        let charging = battery
            .and_then(|b| b.get("charging"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let connected = battery
            .and_then(|b| b.get("connected"))
            .and_then(Value::as_bool)
            .unwrap_or(true);

        let battery_info = BatteryInfo {
            connected,
            charging,
            level,
            voltage: level,
            model: model.clone(),
        };

        let data = self.data_usage(Some(&status))?;
        // Il giorno di rinnovo sta solo nelle impostazioni del piano: senza
        // budget il resto del riepilogo resta valido, quindi niente errore.
        let daily_budget = if data.has_limit {
            self.payment_day().ok().flatten().and_then(|day| {
                daily_budget(
                    data.limit_bytes,
                    data.total_bytes,
                    data.daily_bytes,
                    day,
                    chrono::Local::now().date_naive(),
                )
            })
        } else {
            None
        };
        let connected_devices = status
            .pointer("/connectedDevices/number")
            .and_then(Value::as_i64)
            .unwrap_or(0);

        Ok(DeviceSummary {
            model,
            firmware,
            battery: battery_info,
            data,
            daily_budget,
            connected_devices,
        })
    }

    /// Giorno del mese (1-31) in cui il router azzera il contatore dati, letto
    /// dalle impostazioni del piano (modulo `flowstat`). `None` se il rinnovo
    /// mensile e' disattivato.
    pub fn payment_day(&mut self) -> Result<Option<u32>> {
        let config = self.call("flowstat", 0, None)?;
        let settings = config.get("settings");
        let enabled = settings
            .and_then(|s| s.get("enablePaymentDay"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let day = settings
            .and_then(|s| s.get("paymentDay"))
            .and_then(Value::as_u64)
            .filter(|day| (1..=31).contains(day));
        Ok(day.filter(|_| enabled).map(|day| day as u32))
    }

    pub fn reboot(&mut self) -> Result<Value> {
        self.call("reboot", 0, None)
    }

    /// Configurazione del risparmio energetico (`power_save`).
    pub fn power_save(&mut self) -> Result<Value> {
        self.call("power_save", 0, None)
    }

    /// Vero se il risparmio energetico e' attivo.
    pub fn power_save_enabled(&mut self) -> Result<bool> {
        let config = self.power_save()?;
        Ok(config.get("enable").and_then(Value::as_i64).unwrap_or(0) != 0)
    }

    /// Attiva o disattiva il risparmio energetico preservando gli altri campi
    /// di configurazione letti dal router.
    pub fn set_power_save(&mut self, enable: bool) -> Result<Value> {
        let current = self.power_save()?;
        let mut data = serde_json::Map::new();
        for key in ["powerLevel", "autoDisableTime", "wlanOnOff"] {
            if let Some(value) = current.get(key) {
                data.insert(key.to_string(), value.clone());
            }
        }
        data.insert("enable".to_string(), json!(if enable { 1 } else { 0 }));
        self.call("power_save", 1, Some(&Value::Object(data)))
    }
}

fn random_token(len: usize) -> String {
    rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(len)
        .map(char::from)
        .collect()
}

fn md5_hex(input: &str) -> String {
    let mut hasher = Md5::new();
    hasher.update(input.as_bytes());
    hex_encode(&hasher.finalize())
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

fn hex_decode(input: &str) -> Result<Vec<u8>> {
    if !input.len().is_multiple_of(2) {
        return Err(TpLinkError::Protocol(
            "hex di lunghezza dispari".to_string(),
        ));
    }
    (0..input.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&input[i..i + 2], 16)
                .map_err(|e| TpLinkError::Protocol(e.to_string()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const GB: f64 = 1024.0 * 1024.0 * 1024.0;

    fn date(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).unwrap()
    }

    #[test]
    fn renewal_later_this_month() {
        assert_eq!(
            next_renewal(22, date(2026, 10, 5)),
            Some(date(2026, 10, 22))
        );
    }

    #[test]
    fn renewal_day_counts_next_month() {
        assert_eq!(
            next_renewal(22, date(2026, 10, 22)),
            Some(date(2026, 11, 22))
        );
        assert_eq!(
            next_renewal(22, date(2026, 10, 23)),
            Some(date(2026, 11, 22))
        );
    }

    #[test]
    fn renewal_wraps_year() {
        assert_eq!(next_renewal(5, date(2026, 12, 20)), Some(date(2027, 1, 5)));
    }

    #[test]
    fn renewal_clamps_to_short_months() {
        assert_eq!(next_renewal(31, date(2027, 2, 10)), Some(date(2027, 2, 28)));
        assert_eq!(next_renewal(31, date(2027, 2, 28)), Some(date(2027, 3, 31)));
        assert_eq!(next_renewal(30, date(2028, 1, 30)), Some(date(2028, 2, 29)));
    }

    #[test]
    fn budget_splits_remaining_quota_over_days_left() {
        // 400 GB, 100 GB gia' usati prima di oggi, 30 giorni al rinnovo.
        let budget = daily_budget(400.0 * GB, 100.0 * GB, 0.0, 22, date(2026, 10, 23)).unwrap();
        assert_eq!(budget.days_left, 30);
        assert!((budget.budget_bytes - 10.0 * GB).abs() < 1.0);
        assert_eq!(budget.percent, 0.0);
        assert_eq!(budget.renewal_formatted, "22/11");
    }

    #[test]
    fn budget_is_fixed_during_the_day() {
        let morning = daily_budget(400.0 * GB, 100.0 * GB, 0.0, 22, date(2026, 10, 23)).unwrap();
        let evening =
            daily_budget(400.0 * GB, 105.0 * GB, 5.0 * GB, 22, date(2026, 10, 23)).unwrap();
        assert_eq!(morning.budget_bytes, evening.budget_bytes);
        assert!((evening.percent - 50.0).abs() < 1e-9);
    }

    #[test]
    fn saved_data_raises_tomorrow_budget() {
        // Oggi 2 GB su 10: domani restano 298 GB su 29 giorni.
        let budget =
            daily_budget(400.0 * GB, 102.0 * GB, 2.0 * GB, 22, date(2026, 10, 23)).unwrap();
        let tomorrow = budget.tomorrow_bytes.unwrap();
        assert!(tomorrow > budget.budget_bytes);
        assert!((tomorrow - 298.0 / 29.0 * GB).abs() < 1.0);
    }

    #[test]
    fn exceeded_budget_goes_over_100() {
        let budget =
            daily_budget(400.0 * GB, 115.0 * GB, 15.0 * GB, 22, date(2026, 10, 23)).unwrap();
        assert!((budget.percent - 150.0).abs() < 1e-9);
        assert!(budget.tomorrow_bytes.unwrap() < budget.budget_bytes);
    }

    #[test]
    fn last_day_gets_whole_remaining_quota() {
        let budget = daily_budget(400.0 * GB, 390.0 * GB, 0.0, 22, date(2026, 11, 21)).unwrap();
        assert_eq!(budget.days_left, 1);
        assert!((budget.budget_bytes - 10.0 * GB).abs() < 1.0);
        assert_eq!(budget.tomorrow_bytes, None);
    }

    #[test]
    fn exhausted_quota() {
        let budget = daily_budget(400.0 * GB, 401.0 * GB, 1.0 * GB, 22, date(2026, 11, 1)).unwrap();
        assert_eq!(budget.budget_bytes, 0.0);
        assert_eq!(budget.percent, 100.0);
        assert!(daily_budget(0.0, 0.0, 0.0, 22, date(2026, 11, 1)).is_none());
    }
}
