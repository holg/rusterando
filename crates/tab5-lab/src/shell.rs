//! The command line: line editing and history on the input row, the
//! commands, and two modes where typed lines go elsewhere — LoRa chat over
//! the RAK3172 (`rak client`) and a TCP session (`tcp host:port`). Esc or
//! Ctrl+C leaves them.

use crate::keyboard::Key;
use crate::lan::{self, Lan, Tcp};
use crate::rak::{Cmd as RakCmd, Mode as RakMode, Rak};
use crate::terminal::{Terminal, Tone};
use crate::{say, Out};

const HISTORY: usize = 50;

/// Commands the host firmware adds (the kitchen firmware: `wifi`): gets the
/// words of a line, returns `false` for a line it doesn't know.
pub type HostCommands = Box<dyn FnMut(&[&str], &mut Terminal) -> bool + Send>;

const HELP: &str = "\
Commands (Esc leaves chat/tcp, Ctrl+L clears the screen, Up/Down history):
  info                      device, modules found on the M5-Bus, keyboard
  lan                       LAN status: link, IP, gateway, DNS
  lan restart               restart the LAN driver
  lan reset                 reset the W5500 itself (as at boot)
  ping <host>               ICMP ping over the LAN
  get <url>                 HTTP(S) GET over the LAN
  tcp <host:port>           line terminal over TCP, like nc
  rak                       LoRaWAN module (RAK3172): mode, P2P settings
  rak listen                P2P receive only (sniffer)
  rak server                answer every message (ack) and ping (pong)
  rak client  (or chat)     chat: each line you type is sent and acked
  rak stop                  receive off
  rak send <text>           one message, waits for its ack
  rak ping [n]              n pings (default 5) to a server
  at+<cmd>                  AT command to it, answers show as rak: …
                            at+ver=?  at+p2p=?  at+nwm=0 (P2P) | 1 (LoRaWAN)
                            at+p2p=869525000:7:125:0:8:14  Hz:SF:BW:CR:pre:dBm
  flip                      turn the screen the other way round
  clear  reboot";

enum Mode {
    Command,
    Chat,
    Tcp(Tcp),
}

pub struct Shell {
    out: Out,
    lan: Option<Lan>,
    rak: Option<Rak>,
    info: Vec<String>,
    mode: Mode,
    history: Vec<String>,
    browsing: Option<usize>,
    turned: bool,
    host: Option<(&'static str, HostCommands)>,
}

impl Shell {
    pub fn new(out: Out, lan: Option<Lan>, rak: Option<Rak>, info: Vec<String>) -> Self {
        Self {
            out,
            lan,
            rak,
            info,
            mode: Mode::Command,
            history: Vec::new(),
            browsing: None,
            turned: false,
            host: None,
        }
    }

    /// Add the host firmware's commands, and their lines for `help`.
    #[allow(dead_code)]
    pub fn with_host_commands(mut self, help: &'static str, commands: HostCommands) -> Self {
        self.host = Some((help, commands));
        self
    }

    /// `flip`: the screen turned the other way round than it would be.
    pub fn turned(&self) -> bool {
        self.turned
    }

    fn prompt(&self) -> String {
        match &self.mode {
            Mode::Command => "tab5> ".into(),
            Mode::Chat => "lora> ".into(),
            Mode::Tcp(t) => format!("{}> ", t.peer),
        }
    }

    /// Housekeeping every loop: a TCP session closed by the other end ends
    /// the mode; the prompt follows the mode.
    pub fn tick(&mut self, term: &mut Terminal) {
        if matches!(&self.mode, Mode::Tcp(t) if !t.is_open()) {
            self.mode = Mode::Command;
        }
        let prompt = self.prompt();
        if term.prompt != prompt {
            term.prompt = prompt;
            term.touch();
        }
    }

    pub fn key(&mut self, key: Key, term: &mut Terminal) {
        match key {
            Key::Char(c) | Key::Alt(c) => {
                let at = byte_at(&term.input, term.cursor);
                term.input.insert(at, c);
                term.cursor += 1;
            }
            Key::Backspace => {
                if term.cursor > 0 {
                    term.cursor -= 1;
                    let at = byte_at(&term.input, term.cursor);
                    term.input.remove(at);
                }
            }
            Key::Delete => {
                if term.cursor < term.input.chars().count() {
                    let at = byte_at(&term.input, term.cursor);
                    term.input.remove(at);
                }
            }
            Key::Left => term.cursor = term.cursor.saturating_sub(1),
            Key::Right => term.cursor = (term.cursor + 1).min(term.input.chars().count()),
            Key::Up => self.browse(term, true),
            Key::Down => self.browse(term, false),
            Key::Tab => {}
            Key::Ctrl('l') => term.clear(),
            Key::Ctrl('a') => term.cursor = 0,
            Key::Ctrl('e') => term.cursor = term.input.chars().count(),
            Key::Ctrl('u') => {
                term.input.clear();
                term.cursor = 0;
            }
            Key::Esc | Key::Ctrl('c') => self.escape(term),
            Key::Ctrl(_) => {}
            Key::Enter => {
                let line = std::mem::take(&mut term.input);
                term.cursor = 0;
                self.browsing = None;
                self.submit(line, term);
            }
        }
        term.touch();
    }

    fn escape(&mut self, term: &mut Terminal) {
        match std::mem::replace(&mut self.mode, Mode::Command) {
            Mode::Command => {
                term.input.clear();
                term.cursor = 0;
            }
            Mode::Chat => {
                if let Some(rak) = &self.rak {
                    rak.send(RakCmd::Mode(RakMode::Idle));
                }
            }
            // Dropping the session closes the connection.
            Mode::Tcp(_) => {}
        }
    }

    fn browse(&mut self, term: &mut Terminal, back: bool) {
        if self.history.is_empty() {
            return;
        }
        let last = self.history.len() - 1;
        let next = match (self.browsing, back) {
            (None, true) => Some(last),
            (None, false) => None,
            (Some(i), true) => Some(i.saturating_sub(1)),
            (Some(i), false) if i < last => Some(i + 1),
            (Some(_), false) => None,
        };
        self.browsing = next;
        term.input = next.map(|i| self.history[i].clone()).unwrap_or_default();
        term.cursor = term.input.chars().count();
    }

    pub fn submit(&mut self, line: String, term: &mut Terminal) {
        let echo = format!("{}{line}", self.prompt());
        match &self.mode {
            Mode::Chat => {
                if line.trim().is_empty() {
                    return;
                }
                if let Some(rak) = &self.rak {
                    rak.send(RakCmd::Send(line));
                }
            }
            Mode::Tcp(t) => {
                term.push(Tone::Echo, &echo);
                t.send(line);
            }
            Mode::Command => {
                let line = line.trim().to_string();
                term.push(Tone::Echo, &echo);
                if line.is_empty() {
                    return;
                }
                if self.history.last() != Some(&line) {
                    self.history.push(line.clone());
                    if self.history.len() > HISTORY {
                        self.history.remove(0);
                    }
                }
                self.run(&line, term);
            }
        }
    }

    fn run(&mut self, line: &str, term: &mut Terminal) {
        if let Some(cmd) = at_command(line) {
            self.with_rak(|r| r.send(RakCmd::At(cmd)));
            return;
        }
        let words: Vec<&str> = line.split_whitespace().collect();
        let out = self.out.clone();
        match words.as_slice() {
            ["rak"] => self.with_rak(|r| {
                term.push(Tone::Normal, &r.describe());
                r.send(RakCmd::Status);
            }),
            ["rak", "listen"] => self.with_rak(|r| r.send(RakCmd::Mode(RakMode::Listen))),
            ["rak", "server"] => self.with_rak(|r| r.send(RakCmd::Mode(RakMode::Server))),
            // Chat: answering (so the other end gets its acks), every typed
            // line sent.
            ["rak", "client"] | ["chat"] => match &self.rak {
                Some(rak) => {
                    rak.send(RakCmd::Mode(RakMode::Server));
                    say(
                        &out,
                        Tone::Ok,
                        "LoRa chat: type a line to send it, Esc to leave".into(),
                    );
                    self.mode = Mode::Chat;
                }
                None => no_rak(&out),
            },
            ["rak", "stop"] | ["rak", "idle"] => {
                self.with_rak(|r| r.send(RakCmd::Mode(RakMode::Idle)))
            }
            ["rak", "ping"] => self.with_rak(|r| r.send(RakCmd::Ping(5))),
            ["rak", "ping", n] => match n.parse() {
                Ok(n) => self.with_rak(|r| r.send(RakCmd::Ping(n))),
                Err(_) => term.push(Tone::Warn, "rak ping: bad count — try help"),
            },
            ["rak", "send", ..] if words.len() > 2 => {
                // Everything after "send", spacing kept.
                let text = line
                    .splitn(3, char::is_whitespace)
                    .nth(2)
                    .unwrap_or("")
                    .trim()
                    .to_string();
                self.with_rak(|r| r.send(RakCmd::Send(text)));
            }
            ["help"] | ["?"] => {
                term.push(Tone::Normal, HELP);
                if let Some((help, _)) = &self.host {
                    term.push(Tone::Normal, help);
                }
            }
            ["clear"] => term.clear(),
            ["flip"] => self.turned = !self.turned,
            ["info"] => {
                for l in &self.info {
                    term.push(Tone::Normal, l);
                }
                // SAFETY: read-only heap statistics.
                let (free, internal) = unsafe {
                    (
                        esp_idf_svc::sys::esp_get_free_heap_size() / 1024,
                        esp_idf_svc::sys::heap_caps_get_free_size(
                            esp_idf_svc::sys::MALLOC_CAP_INTERNAL,
                        ) / 1024,
                    )
                };
                term.push(
                    Tone::Dim,
                    &format!("free heap {free} KB, internal {internal} KB"),
                );
            }
            ["reboot"] => {
                say(&out, Tone::Warn, "restarting …".into());
                std::thread::sleep(std::time::Duration::from_millis(200));
                // SAFETY: plain restart.
                unsafe { esp_idf_svc::sys::esp_restart() };
            }
            ["lan"] => match &self.lan {
                Some(lan) => lan.print_status(&out),
                None => no_lan(&out),
            },
            ["lan", "restart"] => match &self.lan {
                Some(lan) => lan.restart(),
                None => no_lan(&out),
            },
            ["lan", "reset"] => match &self.lan {
                Some(lan) => lan.reset(),
                None => no_lan(&out),
            },
            ["lan", "ping", host] | ["ping", host] => {
                self.with_lan(|| lan::ping(host.to_string(), out.clone()))
            }
            ["lan", "get", url] | ["get", url] => {
                self.with_lan(|| lan::get(url.to_string(), out.clone()))
            }
            ["lan", "tcp", target] | ["tcp", target] => self.tcp(target.to_string()),
            ["lan", "tcp", host, port] | ["tcp", host, port] => self.tcp(format!("{host}:{port}")),
            words => {
                let known = match &mut self.host {
                    Some((_, commands)) => commands(words, term),
                    None => false,
                };
                if !known {
                    term.push(Tone::Warn, &format!("unknown: {line} — try help"));
                }
            }
        }
    }

    fn with_rak(&self, f: impl FnOnce(&Rak)) {
        match &self.rak {
            Some(rak) => f(rak),
            None => no_rak(&self.out),
        }
    }

    fn with_lan(&self, f: impl FnOnce()) {
        if self.lan.is_some() {
            f();
        } else {
            no_lan(&self.out);
        }
    }

    fn tcp(&mut self, target: String) {
        if self.lan.is_none() {
            return no_lan(&self.out);
        }
        self.mode = Mode::Tcp(Tcp::connect(target, self.out.clone()));
    }
}

fn no_lan(out: &Out) {
    say(out, Tone::Warn, "no LAN module found (see info)".into());
}

fn no_rak(out: &Out) {
    say(
        out,
        Tone::Warn,
        "no LoRaWAN module (RAK3172) found (see info)".into(),
    );
}

/// `at`, `at+ver=?`, `AT +NWM=0` … → the command for the module: the name
/// (up to `=` or `?`) in capitals, the parameters as typed (hex payloads,
/// keys).
fn at_command(line: &str) -> Option<String> {
    let head = line.get(..2)?;
    let rest = line[2..].trim_start();
    if !head.eq_ignore_ascii_case("at") || !(rest.is_empty() || rest.starts_with('+')) {
        return None;
    }
    let split = rest.find(['=', '?']).unwrap_or(rest.len());
    Some(format!(
        "AT{}{}",
        rest[..split].to_ascii_uppercase(),
        &rest[split..]
    ))
}

/// Byte offset of the `n`-th char.
fn byte_at(s: &str, n: usize) -> usize {
    s.char_indices().nth(n).map(|(i, _)| i).unwrap_or(s.len())
}
