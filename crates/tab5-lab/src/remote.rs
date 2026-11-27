//! The phone as terminal: the iPhone app (ios-lab/) over Bluetooth LE
//! (ble.rs, Nordic UART Service). It mirrors the screen and types into the
//! shell — the keyboard for a Tab5 without one.
//!
//! Text lines, `\n`-terminated, UTF-8; the first character says what:
//!
//! ```text
//! Tab5 → phone
//!   H <name> <version> <nonce>  hello on each connection; nonce: 32 hex digits
//!   O                           key accepted
//!   E <text>                    refused (wrong key, not logged in)
//!   L<tone> <text>              a screen line; tone: n normal, d dim, e echo,
//!                               o ok, r received, t sent, w warning, x error
//!   X                           screen cleared
//!   S <text>                    status bar
//!   P <text>                    prompt ("tab5> ", "lora> ", "host:port> ")
//!
//! phone → Tab5
//!   A <hmac>                    HMAC-SHA256(key, the nonce's 16 bytes), 64 hex digits
//!   C <line>                    a command line, as typed + Enter
//!   K <key>                     esc | ctrl-c | clear
//! ```
//!
//! After `O` the Tab5 sends the last lines of its screen, the status bar
//! and the prompt, then everything as it happens. The key is `LAB_KEY`,
//! compiled in (tmp/tab5-wifi.env, see scripts/build_tab5_lab.sh); it never
//! goes over the air.

use std::collections::VecDeque;
use std::sync::mpsc::{self, Sender};

use anyhow::Result;
use esp_idf_svc::sys;
use hmac::{Hmac, Mac};
use sha2::Sha256;

use crate::ble;
use crate::keyboard::Key;
use crate::shell::Shell;
use crate::terminal::{Mirror, Terminal, Tone};

/// The shared key; without one the phone can't log in.
const KEY: Option<&str> = option_env!("LAB_KEY");
/// Screen lines replayed to a phone that logs in.
const REPLAY: usize = 40;
/// A phone sending more than this without a newline is cut off.
const MAX_LINE: usize = 4096;

pub struct Remote {
    name: &'static str,
    frames: Sender<Vec<u8>>,
    /// The connection greeted last ([`ble::session`]).
    session: Option<u32>,
    authed: bool,
    nonce: [u8; 16],
    inbox: Vec<u8>,
    history: VecDeque<String>,
    /// As last sent to the phone.
    status: String,
    prompt: String,
}

impl Remote {
    /// Start Bluetooth (advertising as `name`) and the sending thread.
    pub fn start(name: &'static str) -> Result<Self> {
        if KEY.is_none() {
            log::warn!("remote: built without LAB_KEY — the phone can't log in");
        }
        ble::start(name)?;
        let (frames, queue) = mpsc::channel::<Vec<u8>>();
        // Sending can wait for the radio's buffers: never on the UI loop.
        std::thread::Builder::new()
            .name("ble-tx".into())
            .stack_size(6 * 1024)
            .spawn(move || {
                for frame in queue {
                    // Not connected (any more): the frame is dropped.
                    let _ = ble::send(&frame);
                }
            })?;
        Ok(Self {
            name,
            frames,
            session: None,
            authed: false,
            nonce: [0; 16],
            inbox: Vec::new(),
            history: VecDeque::new(),
            status: String::new(),
            prompt: String::new(),
        })
    }

    /// A phone is logged in.
    pub fn linked(&self) -> bool {
        self.authed
    }

    /// Every loop: the screen's news, new connections, what the phone sent.
    pub fn poll(&mut self, shell: &mut Shell, term: &mut Terminal) {
        self.mirror(term);

        let connected = ble::connected();
        if connected && self.session != Some(ble::session()) {
            self.greet();
        } else if !connected && self.session.is_some() {
            let was_authed = self.authed;
            self.session = None;
            self.authed = false;
            self.inbox.clear();
            if was_authed {
                term.push(Tone::Dim, "phone disconnected");
            }
        }

        let bytes = ble::read();
        if !bytes.is_empty() {
            self.inbox.extend_from_slice(&bytes);
            while let Some(pos) = self.inbox.iter().position(|b| *b == b'\n') {
                let raw: Vec<u8> = self.inbox.drain(..=pos).collect();
                let line = String::from_utf8_lossy(&raw)
                    .trim_end_matches(['\r', '\n'])
                    .to_string();
                self.handle(&line, shell, term);
            }
            if self.inbox.len() > MAX_LINE {
                self.inbox.clear();
            }
        }

        if self.authed {
            if term.status != self.status {
                self.status = term.status.clone();
                self.send(format!("S {}", self.status));
            }
            if term.prompt != self.prompt {
                self.prompt = term.prompt.clone();
                self.send(format!("P {}", self.prompt));
            }
        }
    }

    /// The screen's lines since the last loop: kept for the replay, sent
    /// live to a logged-in phone.
    fn mirror(&mut self, term: &mut Terminal) {
        for m in term.take_mirror() {
            let frame = match m {
                Mirror::Line(tone, text) => format!("L{} {text}", tone_code(tone)),
                Mirror::Clear => {
                    self.history.clear();
                    "X".to_string()
                }
            };
            if frame != "X" {
                self.history.push_back(frame.clone());
                while self.history.len() > REPLAY {
                    self.history.pop_front();
                }
            }
            if self.authed {
                self.send(frame);
            }
        }
    }

    /// A new connection: hello with a fresh challenge.
    fn greet(&mut self) {
        self.session = Some(ble::session());
        self.authed = false;
        self.inbox.clear();
        // SAFETY: fills the buffer (hardware RNG; the C6 link keeps RF on).
        unsafe { sys::esp_fill_random(self.nonce.as_mut_ptr().cast(), self.nonce.len()) };
        self.send(format!(
            "H {} {} {}",
            self.name,
            env!("CARGO_PKG_VERSION"),
            hex(&self.nonce)
        ));
        log::info!("remote: phone connected, waiting for its key");
    }

    fn handle(&mut self, line: &str, shell: &mut Shell, term: &mut Terminal) {
        let (kind, rest) = line.split_once(' ').unwrap_or((line, ""));
        match kind {
            "A" => self.login(rest, term),
            _ if !self.authed => self.send("E log in first".into()),
            "C" => shell.submit(rest.to_string(), term),
            "K" => match rest {
                "esc" => shell.key(Key::Esc, term),
                "ctrl-c" => shell.key(Key::Ctrl('c'), term),
                "clear" => shell.key(Key::Ctrl('l'), term),
                other => self.send(format!("E unknown key {other}")),
            },
            other => self.send(format!("E unknown message {other}")),
        }
    }

    fn login(&mut self, answer: &str, term: &mut Terminal) {
        let Some(key) = KEY else {
            self.send("E no key compiled into this Tab5 (LAB_KEY)".into());
            return;
        };
        let ok = unhex(answer).is_some_and(|mac| {
            let mut expected =
                <Hmac<Sha256>>::new_from_slice(key.as_bytes()).expect("any key length");
            expected.update(&self.nonce);
            expected.verify_slice(&mac).is_ok()
        });
        if !ok {
            log::warn!("remote: wrong key");
            self.send("E wrong key".into());
            return;
        }
        self.authed = true;
        self.send("O".into());
        for frame in self.history.clone() {
            self.send(frame);
        }
        self.status = term.status.clone();
        self.prompt = term.prompt.clone();
        self.send(format!("S {}", self.status));
        self.send(format!("P {}", self.prompt));
        term.push(Tone::Dim, "phone connected over Bluetooth");
    }

    fn send(&self, frame: String) {
        let mut bytes = frame.into_bytes();
        bytes.push(b'\n');
        let _ = self.frames.send(bytes);
    }
}

fn tone_code(tone: Tone) -> char {
    match tone {
        Tone::Normal => 'n',
        Tone::Dim => 'd',
        Tone::Echo => 'e',
        Tone::Ok => 'o',
        Tone::Rx => 'r',
        Tone::Tx => 't',
        Tone::Warn => 'w',
        Tone::Err => 'x',
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    let s = s.trim();
    if s.len() % 2 != 0 {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}
