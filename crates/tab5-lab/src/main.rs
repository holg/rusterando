//! tab5-lab: sample firmware for the M5Stack Tab5 add-ons.
//!
//! - **Tab5 Keyboard** (EXT header, I2C) → a text terminal on the screen
//!   (80×24, turns with the device like the kitchen screen).
//! - **Module13.2 LAN** (M5-Bus SPI, W5500) → LAN client: DHCP, ping,
//!   HTTP(S) GET, TCP line terminal.
//! - **Module LoRaWAN-EU868** (M5-Bus UART, RAK3172 with AT firmware) →
//!   LoRa P2P client/server (`rak listen|server|send|ping`, signal strength
//!   both ways) and `at…` commands typed in the shell.
//!
//! The modules' jumper/DIP positions are detected at start (bus.rs,
//! rak.rs). `help` on the device lists the commands.
//!
//! Board setup (I/O expanders, battery charging) and the screen-turning
//! accelerometer come from the kitchen firmware (crates/kds-tab5) as they
//! are. Build with `LAB_AUTORUN="rak server"` (commands separated by `;`)
//! to run commands at start — e.g. a second Tab5 without keyboard as the
//! LoRa server.

// Shared with the kitchen firmware: `cargo fmt` here must not reformat them.
#[allow(dead_code)]
#[rustfmt::skip]
#[path = "../../kds-tab5/src/board.rs"]
mod board;
#[allow(dead_code)]
#[rustfmt::skip]
#[path = "../../kds-tab5/src/imu.rs"]
mod imu;

mod battery;
mod ble;
mod bus;
mod console;
mod keyboard;
mod lan;
mod rak;
mod remote;
mod report;
mod shell;
mod terminal;

use std::sync::mpsc::{self, Sender};
use std::time::{Duration, Instant};

use anyhow::Result;
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::hal::peripherals::Peripherals;
use esp_idf_svc::sys;

use console::Screen;
use terminal::{Terminal, Tone};

/// Output lines for the screen, from any thread.
pub type Out = Sender<(Tone, String)>;

/// Show a line on the screen and in the serial log.
pub fn say(out: &Out, tone: Tone, text: String) {
    log::info!("{text}");
    let _ = out.send((tone, text));
}

const BRIGHTNESS: i32 = 60;
/// At most this often a screen update (output bursts are drawn together).
const FRAME: Duration = Duration::from_millis(30);
const TICK: Duration = Duration::from_millis(10);
/// The screen position (`Screen::set_flipped`) that faces the Tab5
/// Keyboard: it attaches at the edge the kitchen screen has at the top.
const KEYBOARD_SIDE: bool = true;

fn main() -> Result<()> {
    sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();
    // A USB flash leaves no update pending, but say so in any case: with
    // rollback enabled an unconfirmed image would be undone on reset.
    // SAFETY: plain OTA bookkeeping call.
    unsafe { sys::esp_ota_mark_app_valid_cancel_rollback() };

    board::init()?;
    let mut screen = Screen::new(BRIGHTNESS)?;
    let mut term = Terminal::default();
    let (out, lines) = mpsc::channel::<(Tone, String)>();
    let name = board::device_name();
    say(
        &out,
        Tone::Ok,
        format!(
            "tab5-lab {} on {name} — type help",
            env!("CARGO_PKG_VERSION")
        ),
    );
    let mut info = vec![format!("{name}: tab5-lab {}", env!("CARGO_PKG_VERSION"))];

    // The phone terminal over Bluetooth LE, through the C6 (powered by
    // board::init).
    let mut remote = remote::Remote::start(name)
        .map_err(|e| say(&out, Tone::Err, format!("bluetooth: {e:#}")))
        .ok();

    if let Err(e) = bus::ext5v_on() {
        say(&out, Tone::Err, format!("bus 5 V: {e:#}"));
    }

    let orientation = imu::Orientation::default();
    if let Err(e) = imu::start(orientation.clone()) {
        say(
            &out,
            Tone::Warn,
            format!("imu: {e:#} — screen stays in one position"),
        );
    }

    // Found later too: it boots when the bus 5 V comes on, and may be
    // plugged in at any time.
    let mut keyboard = keyboard::Keyboard::open(out.clone())
        .map_err(|e| say(&out, Tone::Err, format!("keyboard bus: {e:#}")))
        .ok();
    info.push("keyboard: Tab5 Keyboard on the EXT header, I2C 0x6D, GPIO0/1, INT GPIO50".into());

    let peripherals = Peripherals::take()?;
    let sysloop = EspSystemEventLoop::take()?;
    let pins = peripherals.pins;
    let spi = bus::spi(peripherals.spi2, pins.gpio5, pins.gpio18, pins.gpio19)?;
    let mut found = bus::probe(spi)?;
    // The UART module after the LAN module, keeping off its pins.
    let busy: Vec<i32> = found
        .w5500
        .iter()
        .flat_map(|c| [Some(c.cs), c.rst])
        .flatten()
        .collect();
    let rak_found = rak::probe(&busy, &mut found.notes, &mut found.raw);
    for note in &found.notes {
        say(&out, Tone::Warn, format!("bus: {note}"));
    }
    match found.w5500 {
        Some(c) => info.push(format!("LAN: W5500 (v{}) — {}", c.version, c.describe())),
        None => info.push("LAN: no W5500 on the M5-Bus".into()),
    }
    match &rak_found {
        Some(r) => info.push(r.describe()),
        None => info.push("LoRaWAN: no RAK3172 on the M5-Bus".into()),
    }
    for line in &info[1..] {
        say(&out, Tone::Dim, line.clone());
    }

    let lan = found.w5500.and_then(|chip| {
        lan::start(spi, chip, sysloop.clone(), out.clone())
            .map_err(|e| say(&out, Tone::Err, format!("LAN: {e:#}")))
            .ok()
    });
    // Every LoRa event also goes to the shop server's admin page (LoRa
    // tab), which can send messages back through this Tab5.
    let (lora_log, lora_events) = report::channel();
    let rak = rak_found.and_then(|found| {
        rak::start(found, name, out.clone(), lora_log)
            .map_err(|e| say(&out, Tone::Err, format!("RAK3172: {e:#}")))
            .ok()
    });
    if let Some(rak) = &rak {
        if let Err(e) = report::start(name, rak.clone(), out.clone(), lora_events) {
            say(&out, Tone::Err, format!("LoRa log: {e:#}"));
        }
    }

    let battery = match battery::start(out.clone()) {
        Ok(b) => Some(b),
        Err(e) => {
            say(&out, Tone::Warn, format!("battery: {e:#}"));
            None
        }
    };

    let boot_report: Vec<String> = info
        .iter()
        .chain(&found.notes)
        .chain(&found.raw)
        .cloned()
        .collect();
    let mut shell = shell::Shell::new(out.clone(), lan.clone(), rak.clone(), info);
    // LoRa answers from the start: a Tab5 without keyboard is a server
    // right away, and pings and chat reach every Tab5.
    if rak.is_some() {
        shell.submit("rak server".into(), &mut term);
    }
    if let Some(script) = option_env!("LAB_AUTORUN") {
        for command in script.split(';').map(str::trim).filter(|c| !c.is_empty()) {
            shell.submit(command.to_string(), &mut term);
        }
    }

    let started = Instant::now();
    let mut last_frame = Instant::now() - FRAME;
    let mut last_status = Instant::now() - Duration::from_secs(1);
    let mut last_log = Instant::now();
    let mut first_log = true;
    let mut ble_was_ready = false;
    loop {
        if ble::ready() && !ble_was_ready {
            ble_was_ready = true;
            say(
                &out,
                Tone::Dim,
                format!(
                    "bluetooth: the phone finds this Tab5 as {name} ({})",
                    ble::address()
                ),
            );
        }
        if let Some(kb) = keyboard.as_mut() {
            for key in kb.poll() {
                shell.key(key, &mut term);
            }
        }
        while let Ok((tone, text)) = lines.try_recv() {
            term.push(tone, &text);
        }
        if let Some(r) = remote.as_mut() {
            r.poll(&mut shell, &mut term);
        }
        shell.tick(&mut term);
        if last_status.elapsed() >= Duration::from_secs(1) {
            last_status = Instant::now();
            let status = status_line(
                name,
                &lan,
                &rak,
                &battery,
                keyboard.as_ref().is_some_and(|k| k.present()),
                remote.as_ref().is_some_and(|r| r.linked()),
                started.elapsed(),
            );
            if last_log.elapsed() >= Duration::from_secs(10) {
                // The USB console misses the start now and then: repeat
                // what was found once, then the status every 10 s.
                if std::mem::take(&mut first_log) {
                    for line in &boot_report {
                        log::info!("found: {line}");
                    }
                }
                last_log = Instant::now();
                log::info!(
                    "status:{status}  ·  ble {} {}",
                    if ble::ready() { "ready" } else { "not ready" },
                    if ble::connected() { "connected" } else { "" }
                );
            }
            if status != term.status {
                term.status = status;
                term.touch();
            }
        }
        // With the keyboard on, it is at the bottom: the screen faces it.
        // Without, the accelerometer decides (as the kitchen screen).
        // `flip` turns it the other way round in either case.
        let keyboard_on = keyboard.as_ref().is_some_and(|k| k.present());
        let base = if keyboard_on {
            KEYBOARD_SIDE
        } else {
            orientation.flipped()
        };
        screen.set_flipped(base ^ shell.turned(), &mut term);
        if term.is_dirty() && last_frame.elapsed() >= FRAME {
            last_frame = Instant::now();
            screen.show(&mut term);
        }
        std::thread::sleep(TICK);
    }
}

fn status_line(
    name: &str,
    lan: &Option<lan::Lan>,
    rak: &Option<rak::Rak>,
    battery: &Option<battery::Battery>,
    keyboard: bool,
    phone: bool,
    up: Duration,
) -> String {
    let mut parts = vec![format!(" {name}")];
    parts.push(
        lan.as_ref()
            .map(|l| l.state().short())
            .unwrap_or_else(|| "no LAN".into()),
    );
    parts.push(
        rak.as_ref()
            .map(|r| r.state().describe())
            .unwrap_or_else(|| "no LoRa".into()),
    );
    if phone {
        parts.push("phone".into());
    } else if !keyboard {
        parts.push("no keyboard".into());
    }
    if let Some(b) = battery.as_ref().and_then(|b| b.describe()) {
        parts.push(b);
    }
    let s = up.as_secs();
    parts.push(format!("up {}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60));
    parts.join("  ·  ")
}
