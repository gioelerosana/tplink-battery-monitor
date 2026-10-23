//! Interfaccia minima per l'installazione, senza GUI toolkit nel binario.
//!
//! Avviata con un doppio clic l'AppImage non ha un terminale: i dialoghi
//! vengono delegati a `zenity` (GNOME e GTK) o `kdialog` (KDE) se installati.
//! Da terminale si usa il testo; senza nessuno dei due si ripiega sulle
//! notifiche desktop.

use std::env;
use std::io::{self, BufRead, IsTerminal, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::thread::sleep;
use std::time::Duration;

use notify_rust::Notification;

const TITLE: &str = "TP-Link MiFi Monitor";
pub const ICON_NAME: &str = "tplink-battery-monitor";

/// Pausa tra i passaggi della barra di avanzamento: l'installazione dura
/// pochi millisecondi e senza pausa la finestra lampeggerebbe illeggibile.
const STEP_PAUSE: Duration = Duration::from_millis(450);

pub enum Choice {
    Install,
    Uninstall,
    Cancel,
}

pub enum Ui {
    Terminal,
    Zenity,
    Kdialog,
    Notifications,
}

impl Ui {
    pub fn detect() -> Self {
        if io::stdin().is_terminal() {
            Ui::Terminal
        } else if in_path("zenity") {
            Ui::Zenity
        } else if in_path("kdialog") {
            Ui::Kdialog
        } else {
            Ui::Notifications
        }
    }

    /// Dialogo di benvenuto. Se l'app e' gia' installata propone anche la
    /// disinstallazione, l'unico modo per farla senza terminale.
    pub fn welcome(&self, text: &str, installed: bool) -> Choice {
        let action = if installed { "Aggiorna" } else { "Installa" };
        match self {
            Ui::Terminal => {
                println!("{TITLE}\n\n{text}\n");
                let options = if installed {
                    "[A]ggiorna, [d]isinstalla, [n] annulla"
                } else {
                    "[I]nstalla, [n] annulla"
                };
                match read_line(&format!("{options}: "))
                    .trim()
                    .to_lowercase()
                    .as_str()
                {
                    "" | "a" | "i" | "s" | "y" => Choice::Install,
                    "d" if installed => Choice::Uninstall,
                    _ => Choice::Cancel,
                }
            }
            Ui::Zenity => {
                let mut args = vec![
                    "--question".to_string(),
                    format!("--text={}", markup(text)),
                    format!("--ok-label={action}"),
                    "--cancel-label=Annulla".to_string(),
                ];
                if installed {
                    args.push("--extra-button=Disinstalla".to_string());
                }
                match zenity(&args) {
                    (true, _) => Choice::Install,
                    (false, out) if out.trim() == "Disinstalla" => Choice::Uninstall,
                    _ => Choice::Cancel,
                }
            }
            Ui::Kdialog => {
                let mode = if installed {
                    "--yesnocancel"
                } else {
                    "--yesno"
                };
                let mut args = vec![mode, text, "--yes-label", action];
                if installed {
                    args.extend(["--no-label", "Disinstalla", "--cancel-label", "Annulla"]);
                } else {
                    args.extend(["--no-label", "Annulla"]);
                }
                match kdialog(&args).0 {
                    Some(0) => Choice::Install,
                    Some(1) if installed => Choice::Uninstall,
                    _ => Choice::Cancel,
                }
            }
            // Senza dialoghi non si puo' chiedere conferma: il doppio clic
            // stesso vale come consenso.
            Ui::Notifications => Choice::Install,
        }
    }

    pub fn confirm(&self, text: &str, ok_label: &str) -> bool {
        match self {
            Ui::Terminal => {
                println!("{text}");
                matches!(
                    read_line("Confermi? [s/N]: ")
                        .trim()
                        .to_lowercase()
                        .as_str(),
                    "s" | "y"
                )
            }
            Ui::Zenity => {
                zenity(&[
                    "--question".to_string(),
                    format!("--text={}", markup(text)),
                    format!("--ok-label={ok_label}"),
                    "--cancel-label=Annulla".to_string(),
                    "--default-cancel".to_string(),
                ])
                .0
            }
            Ui::Kdialog => {
                kdialog(&[
                    "--warningyesno",
                    text,
                    "--yes-label",
                    ok_label,
                    "--no-label",
                    "Annulla",
                ])
                .0 == Some(0)
            }
            Ui::Notifications => false,
        }
    }

    /// Chiede la password. `None` se l'utente annulla o non c'e' modo di
    /// chiederla.
    pub fn password(&self, text: &str) -> Option<String> {
        let value = match self {
            Ui::Terminal => {
                println!("{text}");
                let _ = Command::new("stty").arg("-echo").status();
                let line = read_line("Password: ");
                let _ = Command::new("stty").arg("echo").status();
                println!();
                Some(line)
            }
            Ui::Zenity => match zenity(&[
                "--entry".to_string(),
                "--hide-text".to_string(),
                format!("--text={text}"),
                "--ok-label=Continua".to_string(),
                "--cancel-label=Annulla".to_string(),
            ]) {
                (true, out) => Some(out),
                _ => None,
            },
            Ui::Kdialog => match kdialog(&["--password", text]) {
                (Some(0), out) => Some(out),
                _ => None,
            },
            Ui::Notifications => None,
        }?;
        let value = value.trim().to_string();
        (!value.is_empty()).then_some(value)
    }

    pub fn info(&self, text: &str) {
        match self {
            Ui::Terminal => println!("\n{text}"),
            Ui::Zenity => {
                zenity(&["--info".to_string(), format!("--text={}", markup(text))]);
            }
            Ui::Kdialog => {
                kdialog(&["--msgbox", text]);
            }
            Ui::Notifications => notification(text),
        }
    }

    pub fn error(&self, text: &str) {
        match self {
            Ui::Terminal => eprintln!("\nErrore: {text}"),
            Ui::Zenity => {
                zenity(&["--error".to_string(), format!("--text={}", markup(text))]);
            }
            Ui::Kdialog => {
                kdialog(&["--error", text]);
            }
            Ui::Notifications => notification(&format!("Errore: {text}")),
        }
    }

    pub fn progress(&self, text: &str) -> Progress {
        let mut window = match self {
            Ui::Zenity => Command::new("zenity")
                .args(common_args())
                .args([
                    "--progress",
                    "--auto-close",
                    "--no-cancel",
                    "--percentage=0",
                    &format!("--text={}", markup(text)),
                ])
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .ok(),
            Ui::Kdialog => {
                let _ = Command::new("kdialog")
                    .args(["--title", TITLE, "--passivepopup", text, "3"])
                    .stderr(Stdio::null())
                    .spawn();
                None
            }
            Ui::Terminal => {
                println!("\n{text}");
                None
            }
            Ui::Notifications => {
                notification(text);
                None
            }
        };
        let stdin = window.as_mut().and_then(|child| child.stdin.take());
        Progress {
            window,
            stdin,
            terminal: matches!(self, Ui::Terminal),
        }
    }
}

/// Finestra di avanzamento di zenity; negli altri casi i passaggi vengono
/// stampati a terminale o ignorati.
pub struct Progress {
    window: Option<Child>,
    stdin: Option<ChildStdin>,
    terminal: bool,
}

impl Progress {
    pub fn step(&mut self, percent: u8, text: &str) {
        if self.terminal {
            println!("  - {text}");
        }
        if let Some(stdin) = self.stdin.as_mut() {
            let _ = writeln!(stdin, "{percent}\n# {}", markup(text));
            let _ = stdin.flush();
            sleep(STEP_PAUSE);
        }
    }

    pub fn finish(mut self) {
        self.close();
    }

    fn close(&mut self) {
        if let Some(mut stdin) = self.stdin.take() {
            let _ = writeln!(stdin, "100");
        }
        if let Some(mut child) = self.window.take() {
            let _ = child.wait();
        }
    }
}

impl Drop for Progress {
    fn drop(&mut self) {
        self.close();
    }
}

fn common_args() -> Vec<String> {
    vec![
        format!("--title={TITLE}"),
        format!("--icon={ICON_NAME}"),
        "--width=440".to_string(),
    ]
}

/// Esegue zenity e restituisce (OK premuto, stdout).
fn zenity(args: &[String]) -> (bool, String) {
    let output = Command::new("zenity")
        .args(common_args())
        .args(args)
        .stderr(Stdio::null())
        .output();
    match output {
        Ok(out) => (
            out.status.success(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
        ),
        Err(_) => (false, String::new()),
    }
}

/// Esegue kdialog e restituisce (codice di uscita, stdout).
fn kdialog(args: &[&str]) -> (Option<i32>, String) {
    let output = Command::new("kdialog")
        .args(["--title", TITLE, "--icon", ICON_NAME])
        .args(args)
        .stderr(Stdio::null())
        .output();
    match output {
        Ok(out) => (
            out.status.code(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
        ),
        Err(_) => (None, String::new()),
    }
}

fn notification(text: &str) {
    let _ = Notification::new()
        .appname(ICON_NAME)
        .summary(TITLE)
        .body(text)
        .icon(ICON_NAME)
        .show();
}

fn read_line(prompt: &str) -> String {
    print!("{prompt}");
    let _ = io::stdout().flush();
    let mut line = String::new();
    let _ = io::stdin().lock().read_line(&mut line);
    line
}

/// zenity interpreta il testo come markup Pango.
fn markup(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn in_path(program: &str) -> bool {
    env::var_os("PATH")
        .map(|paths| env::split_paths(&paths).any(|dir| Path::new(&dir).join(program).is_file()))
        .unwrap_or(false)
}
