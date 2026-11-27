//! LAN client on Module13.2 LAN (W5500, SPI on the M5-Bus): ESP-IDF's
//! W5500 driver in polling mode (the INT jumper doesn't matter), DHCP, and
//! the client tools the shell offers:
//!
//! - `status`: link, address, gateway, DNS, MAC
//! - `ping <host>`: ICMP echo (4×)
//! - `get <url>`: HTTP(S) GET, status, headers and the start of the body
//! - `tcp <host>:<port>`: a line terminal over TCP (like `nc`): typed lines
//!   go out with CRLF, what comes back is shown
//!
//! This firmware has no Wi-Fi, so every connection goes over the cable.

use std::collections::VecDeque;
use std::io::{ErrorKind, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use embedded_svc::http::client::Client;
use esp_idf_svc::eth::{EspEth, EthDriver, SpiEthChipset, SpiEventSource};
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::hal::gpio::AnyOutputPin;
use esp_idf_svc::hal::units::FromValueType;
use esp_idf_svc::http::client::{Configuration as HttpConfig, EspHttpConnection};
use esp_idf_svc::netif::{EspNetif, NetifConfiguration};
use esp_idf_svc::ping::{self, EspPing, Reply};
use esp_idf_svc::sys::{self, esp};

use crate::bus::{Chip, SharedSpi};
use crate::terminal::Tone;
use crate::{say, Out};

/// W5500 SPI clock. At 16 MHz (ESP-IDF's example) a frame header came back
/// garbled now and then through the stacked modules, which throws the driver
/// out of step for good (see [`guard`]); the LAN needs far less. 4 MHz
/// didn't help against the wedge [`STUCK`] describes (2026-11-14).
const SPI_MHZ: u32 = 8;
/// Bad reads within one watch tick (500 ms) that mean the driver lost step.
/// A healthy driver has none.
const BAD_BURST: u32 = 20;
/// More incidents than this within [`RESTART_WINDOW`]: restart the Tab5
/// instead (the W5500 gets its hardware reset at boot).
const MAX_RESTARTS: usize = 3;
const RESTART_WINDOW: Duration = Duration::from_secs(10 * 60);
/// One incident: a burst of traffic (a big download) can throw the driver
/// out of step several times in a row — driver restarts within this time of
/// the first count as one. Counted singly, one download restarted the Tab5.
const INCIDENT: Duration = Duration::from_secs(60);
/// A driver restart this soon after the last one didn't help: the W5500
/// itself is stuck. Seen 2026-11-14 (idle, 4 MHz): every restart was met by
/// ~140 bad frames at once, for minutes, until the Tab5's restart reset the
/// chip — then the LAN was back at once. So: [`reset_chip`] instead.
const STUCK: Duration = Duration::from_secs(10);
/// How often the driver polls the W5500 (no interrupt line).
const POLL: Duration = Duration::from_millis(10);
/// Lines of an HTTP body shown.
const BODY_LINES: usize = 30;

#[derive(Clone, Default)]
pub struct State {
    pub link: bool,
    pub ip: Option<Ipv4Addr>,
    pub gateway: Option<Ipv4Addr>,
    pub mask: u8,
    pub dns: Option<Ipv4Addr>,
    pub mac: [u8; 6],
}

impl State {
    pub fn short(&self) -> String {
        match (self.link, self.ip) {
            (false, _) => "LAN no cable".into(),
            (true, None) => "LAN DHCP …".into(),
            (true, Some(ip)) => format!("LAN {ip}"),
        }
    }
}

#[derive(Clone)]
pub struct Lan {
    state: Arc<Mutex<State>>,
    /// `lan restart`: the watcher restarts the driver.
    restart: Arc<AtomicBool>,
    /// `lan reset`: the watcher resets the W5500 itself ([`reset_chip`]).
    reset: Arc<AtomicBool>,
}

impl Lan {
    pub fn state(&self) -> State {
        self.state.lock().map(|s| s.clone()).unwrap_or_default()
    }

    /// Restart the Ethernet driver (as after a lost step, see [`guard`]).
    pub fn restart(&self) {
        self.restart.store(true, Ordering::Relaxed);
    }

    /// Reset the W5500 itself, as at boot (as when a driver restart didn't
    /// help, see [`watch`]).
    pub fn reset(&self) {
        self.reset.store(true, Ordering::Relaxed);
    }

    pub fn print_status(&self, out: &Out) {
        let s = self.state();
        let mac = s
            .mac
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<Vec<_>>()
            .join(":");
        say(
            out,
            Tone::Normal,
            format!(
                "LAN W5500, MAC {mac}, link {}",
                if s.link { "up" } else { "down" }
            ),
        );
        match s.ip {
            Some(ip) => say(
                out,
                Tone::Normal,
                format!(
                    "  IP {ip}/{}  gateway {}  DNS {}",
                    s.mask,
                    s.gateway.map(|g| g.to_string()).unwrap_or("-".into()),
                    s.dns.map(|d| d.to_string()).unwrap_or("-".into())
                ),
            ),
            None => say(out, Tone::Dim, "  no address yet (DHCP)".into()),
        }
    }
}

/// Start the Ethernet driver in its own thread (it lives there for good).
/// Returns once the driver runs: its start resets the W5500 (and whatever
/// else hangs on that reset line).
/// Above Wi-Fi's 100 (ESP-IDF's default): the LAN is preferred when up.
const LAN_ROUTE_PRIORITY: u32 = 150;

pub fn start(spi: SharedSpi, chip: Chip, sysloop: EspSystemEventLoop, out: Out) -> Result<Lan> {
    guard::install();
    let state = Arc::new(Mutex::new(State::default()));
    let restart = Arc::new(AtomicBool::new(false));
    let reset = Arc::new(AtomicBool::new(false));
    let lan = Lan {
        state: state.clone(),
        restart: restart.clone(),
        reset: reset.clone(),
    };
    let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel::<Result<()>>(1);
    std::thread::Builder::new()
        .name("lan".into())
        .stack_size(8 * 1024)
        .spawn(move || match open(spi, chip, sysloop) {
            Ok((eth, mac)) => {
                let _ = ready_tx.send(Ok(()));
                watch(eth, mac, chip.rst, &state, &restart, &reset, &out);
            }
            Err(e) => {
                let _ = ready_tx.send(Err(e));
            }
        })?;
    ready_rx
        .recv_timeout(Duration::from_secs(5))
        .map_err(|_| anyhow!("W5500 driver didn't start"))??;
    Ok(lan)
}

type Eth = EspEth<'static, esp_idf_svc::eth::SpiEth<SharedSpi>>;

fn open(spi: SharedSpi, chip: Chip, sysloop: EspSystemEventLoop) -> Result<(Eth, [u8; 6])> {
    // The W5500 has no MAC address of its own: the chip's Ethernet MAC.
    let mut mac = [0u8; 6];
    // SAFETY: fills the 6-byte buffer.
    esp!(unsafe { sys::esp_read_mac(mac.as_mut_ptr(), sys::esp_mac_type_t_ESP_MAC_ETH) })?;
    // SAFETY: the pins found by bus::probe; nothing else uses them.
    let cs = unsafe { AnyOutputPin::steal(chip.cs as u8) };
    let rst = chip.rst.map(|r| unsafe { AnyOutputPin::steal(r as u8) });
    let driver = EthDriver::new_spi_with_event_source(
        spi,
        SpiEventSource::polling(POLL).map_err(|e| anyhow!("{e}"))?,
        Some(cs),
        rst,
        SpiEthChipset::W5500,
        SPI_MHZ.MHz().into(),
        Some(&mac),
        None,
        sysloop,
    )
    .context("W5500 driver")?;
    // The cable first: lwIP sends over the up interface with the highest
    // route priority, and ESP-IDF gives Wi-Fi 100, Ethernet only 60. With
    // 150 the LAN carries everything while it has an address; without it
    // (unplugged, no DHCP) Wi-Fi takes over by itself.
    let netif = EspNetif::new_with_conf(&NetifConfiguration {
        route_priority: LAN_ROUTE_PRIORITY,
        ..NetifConfiguration::eth_default_client()
    })?;
    let mut eth = EspEth::wrap_all(driver, netif)?;
    eth.start()?;
    Ok((eth, mac))
}

/// Keep the driver alive and the state current; report changes. Restarts
/// the driver when it lost step with the W5500 ([`guard`]) or on request —
/// and resets the W5500 itself when a restart didn't help.
fn watch(
    mut eth: Eth,
    mac: [u8; 6],
    rst: Option<i32>,
    state: &Mutex<State>,
    restart: &AtomicBool,
    reset: &AtomicBool,
    out: &Out,
) {
    say(
        out,
        Tone::Dim,
        "LAN started, waiting for cable and DHCP".into(),
    );
    let mut shown = State {
        mac,
        ..State::default()
    };
    let mut bad_seen = guard::bad_reads();
    let mut restarts: VecDeque<Instant> = VecDeque::new();
    // The guard's last restart or reset.
    let mut last_action: Option<Instant> = None;
    loop {
        let bad = guard::bad_reads().wrapping_sub(bad_seen);
        let reset_asked = reset.swap(false, Ordering::Relaxed);
        let asked = restart.swap(false, Ordering::Relaxed) || reset_asked;
        if bad >= BAD_BURST || asked {
            let stuck = !asked && last_action.is_some_and(|t| t.elapsed() < STUCK);
            if !asked {
                last_action = Some(Instant::now());
                restarts.retain(|t| t.elapsed() < RESTART_WINDOW);
                if restarts.back().is_none_or(|t| t.elapsed() >= INCIDENT) {
                    restarts.push_back(Instant::now());
                }
                if restarts.len() > MAX_RESTARTS {
                    say(
                        out,
                        Tone::Err,
                        "LAN: the W5500 keeps losing step, restarting the Tab5".into(),
                    );
                    log::error!("lan: W5500 keeps losing step, restarting");
                    std::thread::sleep(Duration::from_secs(1));
                    // SAFETY: plain restart.
                    unsafe { sys::esp_restart() };
                }
                let what = if stuck {
                    "the restart didn't help, resetting the chip"
                } else {
                    "restarting its driver"
                };
                say(out, Tone::Warn, format!("LAN: {bad} bad frames from the W5500, {what}"));
                log::warn!("lan: {bad} bad frames from the W5500, {what}");
            } else if reset_asked {
                say(out, Tone::Dim, "LAN: resetting the W5500".into());
            } else {
                say(out, Tone::Dim, "LAN: restarting the driver".into());
            }
            // Stop closes the W5500's socket, start opens it again: that
            // resets its buffer pointers. The DHCP that follows brings
            // frames, which end the driver's stuck receive loop.
            if let Err(e) = eth.stop() {
                say(out, Tone::Err, format!("LAN stop: {e}"));
            }
            if stuck || reset_asked {
                match reset_chip(&eth, &mac, rst) {
                    Ok(()) => log::info!("lan: W5500 reset, set up again"),
                    Err(e) => say(out, Tone::Err, format!("LAN reset: {e:#}")),
                }
            }
            std::thread::sleep(Duration::from_millis(200));
            if let Err(e) = eth.start() {
                say(out, Tone::Err, format!("LAN start: {e}"));
            }
            bad_seen = guard::bad_reads();
        } else {
            bad_seen = bad_seen.wrapping_add(bad);
        }

        let link = eth.is_connected().unwrap_or(false);
        let info = eth
            .netif()
            .get_ip_info()
            .ok()
            .filter(|i| !i.ip.is_unspecified());
        let now = State {
            link,
            ip: info.as_ref().map(|i| i.ip),
            gateway: info
                .as_ref()
                .map(|i| i.subnet.gateway)
                .filter(|g| !g.is_unspecified()),
            mask: info.as_ref().map(|i| i.subnet.mask.0).unwrap_or(0),
            dns: info
                .as_ref()
                .and_then(|i| i.dns)
                .filter(|d| !d.is_unspecified()),
            mac,
        };
        if now.link != shown.link {
            say(
                out,
                if now.link { Tone::Ok } else { Tone::Warn },
                format!("LAN link {}", if now.link { "up" } else { "down" }),
            );
        }
        if now.ip != shown.ip {
            if let Some(ip) = now.ip {
                say(
                    out,
                    Tone::Ok,
                    format!(
                        "LAN IP {ip}/{} via DHCP, gateway {}",
                        now.mask,
                        now.gateway.map(|g| g.to_string()).unwrap_or("-".into())
                    ),
                );
            }
        }
        if let Ok(mut s) = state.lock() {
            *s = now.clone();
        }
        shown = now;
        std::thread::sleep(Duration::from_millis(500));
    }
}

/// What the driver's install does to the chip (esp_eth_driver_install):
/// its reset line, then the MAC's and the PHY's setup — here in place, with
/// the driver stopped, so nothing else is torn down. Clears a W5500 that a
/// driver restart (socket closed and opened) doesn't.
fn reset_chip(eth: &Eth, mac: &[u8; 6], rst: Option<i32>) -> Result<()> {
    use esp_idf_svc::handle::RawHandle;

    let handle = eth.driver().handle();
    let mut m: *mut sys::esp_eth_mac_t = core::ptr::null_mut();
    let mut p: *mut sys::esp_eth_phy_t = core::ptr::null_mut();
    // SAFETY: the driver's own MAC and PHY, called while it is stopped
    // (nothing else calls them then), in the order of its install and
    // uninstall. The reset pin is the one the driver was given.
    unsafe {
        esp!(sys::esp_eth_get_mac_instance(handle, &mut m))?;
        esp!(sys::esp_eth_get_phy_instance(handle, &mut p))?;
        let (Some(mac_deinit), Some(mac_init), Some(phy_deinit), Some(phy_init)) =
            ((*m).deinit, (*m).init, (*p).deinit, (*p).init)
        else {
            bail!("driver without init/deinit");
        };
        // Errors don't matter here: the chip is reset next anyway.
        let _ = phy_deinit(p);
        let _ = mac_deinit(m);
        match rst {
            // Held low 2 ms: the W5500 wants at least 500 µs (ESP-IDF's own
            // reset gives it 100 µs).
            Some(pin) => {
                sys::gpio_set_direction(pin, sys::gpio_mode_t_GPIO_MODE_OUTPUT);
                sys::gpio_set_level(pin, 0);
                std::thread::sleep(Duration::from_millis(2));
                sys::gpio_set_level(pin, 1);
            }
            None => {
                if let Some(reset_hw) = (*p).reset_hw {
                    esp!(reset_hw(p))?;
                }
            }
        }
        // Its clock settles within 1 ms; the MAC's setup waits for the
        // version register as well.
        std::thread::sleep(Duration::from_millis(10));
        esp!(mac_init(m)).context("W5500 setup")?;
        // The reset cleared its MAC address (the frame filter needs it).
        esp!(sys::esp_eth_ioctl(
            handle,
            sys::esp_eth_io_cmd_t_ETH_CMD_S_MAC_ADDR,
            mac.as_ptr().cast_mut().cast()
        ))?;
        esp!(phy_init(p)).context("W5500 PHY setup")?;
    }
    Ok(())
}

/// ESP-IDF 5.4's W5500 driver has no way back after one garbled frame
/// header (a single bad SPI read is enough): it moves its read pointer by
/// the bogus length, so every later "header" is frame data; it logs
/// "received frame was truncated" hundreds of times a second, and its
/// receive loop never waits again — its task (priority 15) holds a CPU and
/// whatever else runs there starves (the kitchen screen stopped reacting).
/// Nothing reports this but the driver's log, so a log hook counts those
/// lines for [`watch`], and lets only a few of them through to the console.
mod guard {
    use core::ffi::{c_char, c_int, CStr};
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::OnceLock;

    use esp_idf_svc::sys;

    /// The driver's messages about what it read from the W5500
    /// (esp_eth_mac_w5500.c).
    const SIGNS: [&[u8]; 4] = [
        b"received frame was truncated",
        // A header with a too-short length: ESP-IDF 5.4 doesn't move its
        // read pointer then and reads the same header again every poll,
        // for good ("invalid frame length" itself is compiled out).
        b"unexpected error 0x%x",
        b"invalid frame length",
        // Logged after every failed SPI read of a frame.
        b"frame read from module failed",
    ];

    static BAD: AtomicU32 = AtomicU32::new(0);
    /// The log output before ours.
    static PREV: OnceLock<sys::vprintf_like_t> = OnceLock::new();

    /// Bad reads reported so far.
    pub fn bad_reads() -> u32 {
        BAD.load(Ordering::Relaxed)
    }

    pub fn install() {
        // SAFETY: swaps the log output function; `hook` passes everything on.
        PREV.get_or_init(|| unsafe { sys::esp_log_set_vprintf(Some(hook)) });
    }

    /// Every log line: the format string carries the message text.
    unsafe extern "C" fn hook(fmt: *const c_char, args: sys::va_list) -> c_int {
        if !fmt.is_null() {
            // SAFETY: a log format string, NUL-terminated.
            let text = unsafe { CStr::from_ptr(fmt) }.to_bytes();
            if SIGNS
                .iter()
                .any(|s| text.windows(s.len()).any(|w| w == *s))
            {
                let n = BAD.fetch_add(1, Ordering::Relaxed);
                // The first few, then one in 500: hundreds a second would
                // only slow everything down further.
                if n >= 5 && n % 500 != 0 {
                    return 0;
                }
            }
        }
        match PREV.get().copied().flatten() {
            // SAFETY: the previous output, with what we were given.
            Some(prev) => unsafe { prev(fmt, args) },
            // Only while being installed: that line is lost.
            None => 0,
        }
    }
}

fn resolve(host: &str) -> Result<Ipv4Addr> {
    if let Ok(ip) = host.parse() {
        return Ok(ip);
    }
    (host, 0)
        .to_socket_addrs()
        .with_context(|| format!("DNS {host}"))?
        .find_map(|a| match a {
            SocketAddr::V4(v4) => Some(*v4.ip()),
            SocketAddr::V6(_) => None,
        })
        .ok_or_else(|| anyhow!("{host}: no IPv4 address"))
}

/// `ping <host>` in the background.
pub fn ping(host: String, out: Out) {
    background("ping", out.clone(), move || {
        let ip = resolve(&host)?;
        say(&out, Tone::Dim, format!("ping {host} ({ip}) …"));
        let conf = ping::Configuration {
            count: 4,
            interval: Duration::from_secs(1),
            ..Default::default()
        };
        let reply_out = out.clone();
        let summary = EspPing::default().ping_details(ip, &conf, move |_, reply| match reply {
            Reply::Success(i) => say(
                &reply_out,
                Tone::Ok,
                format!(
                    "  {} bytes from {}: seq {} ttl {} time {} ms",
                    i.recv_len,
                    i.addr,
                    i.seqno,
                    i.ttl,
                    i.elapsed_time.as_millis()
                ),
            ),
            Reply::Timeout => say(&reply_out, Tone::Warn, "  timeout".into()),
        })?;
        say(
            &out,
            Tone::Normal,
            format!(
                "ping {host}: {} sent, {} received",
                summary.transmitted, summary.received
            ),
        );
        Ok(())
    });
}

/// `get <url>` in the background: HTTP or HTTPS (Mozilla's CA bundle).
pub fn get(url: String, out: Out) {
    background("get", out.clone(), move || {
        let url = if url.contains("://") {
            url
        } else {
            format!("http://{url}")
        };
        say(&out, Tone::Dim, format!("GET {url} …"));
        let started = Instant::now();
        let conn = EspHttpConnection::new(&HttpConfig {
            timeout: Some(Duration::from_secs(10)),
            crt_bundle_attach: Some(sys::esp_crt_bundle_attach),
            ..Default::default()
        })?;
        let mut client = Client::wrap(conn);
        let request = client.request(
            embedded_svc::http::Method::Get,
            &url,
            &[("User-Agent", "tab5-lab")],
        )?;
        let mut response = request.submit()?;
        let status = response.status();
        let tone = if (200..300).contains(&status) {
            Tone::Ok
        } else {
            Tone::Warn
        };
        say(
            &out,
            tone,
            format!("HTTP {status} in {} ms", started.elapsed().as_millis()),
        );
        for name in ["Content-Type", "Content-Length", "Server", "Location"] {
            if let Some(v) = response.header(name) {
                say(&out, Tone::Dim, format!("  {name}: {v}"));
            }
        }
        let mut body = Vec::new();
        let mut buf = [0u8; 1024];
        let mut total = 0usize;
        loop {
            let n = response
                .read(&mut buf)
                .map_err(|e| anyhow!("read: {e:?}"))?;
            if n == 0 {
                break;
            }
            total += n;
            if body.len() < 16 * 1024 {
                body.extend_from_slice(&buf[..n]);
            }
        }
        let text = String::from_utf8_lossy(&body);
        let lines: Vec<&str> = text.lines().collect();
        for line in lines.iter().take(BODY_LINES) {
            say(&out, Tone::Normal, (*line).to_string());
        }
        if lines.len() > BODY_LINES {
            say(
                &out,
                Tone::Dim,
                format!("… {} more lines", lines.len() - BODY_LINES),
            );
        }
        say(
            &out,
            Tone::Dim,
            format!("{total} bytes in {} ms", started.elapsed().as_millis()),
        );
        Ok(())
    });
}

/// A TCP line session: lines sent through [`Tcp::send`] go out with CRLF,
/// received data is shown line by line.
pub struct Tcp {
    lines: Sender<String>,
    open: Arc<AtomicBool>,
    pub peer: String,
}

impl Tcp {
    pub fn connect(target: String, out: Out) -> Tcp {
        let (tx, rx) = std::sync::mpsc::channel();
        let open = Arc::new(AtomicBool::new(true));
        let flag = open.clone();
        let peer = target.clone();
        background("tcp", out.clone(), move || {
            let r = session(&target, rx, &out);
            flag.store(false, Ordering::Relaxed);
            r
        });
        Tcp {
            lines: tx,
            open,
            peer,
        }
    }

    pub fn is_open(&self) -> bool {
        self.open.load(Ordering::Relaxed)
    }

    pub fn send(&self, line: String) {
        let _ = self.lines.send(line);
    }
}

fn session(target: &str, lines: Receiver<String>, out: &Out) -> Result<()> {
    let (host, port) = target
        .rsplit_once(':')
        .ok_or_else(|| anyhow!("use host:port"))?;
    let port: u16 = port.parse().map_err(|_| anyhow!("bad port {port:?}"))?;
    let ip = resolve(host)?;
    say(out, Tone::Dim, format!("connecting to {ip}:{port} …"));
    let addr = SocketAddr::from((ip, port));
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(5))
        .with_context(|| format!("connect {addr}"))?;
    stream.set_read_timeout(Some(Duration::from_millis(50)))?;
    stream.set_nodelay(true)?;
    say(
        out,
        Tone::Ok,
        format!("connected to {addr} — lines go out with CRLF, Esc closes"),
    );
    let mut pending = Vec::new();
    let mut buf = [0u8; 1024];
    loop {
        match lines.try_recv() {
            Ok(line) => {
                stream.write_all(line.as_bytes())?;
                stream.write_all(b"\r\n")?;
            }
            // The shell dropped the session (Esc): close.
            Err(TryRecvError::Disconnected) => {
                let _ = stream.shutdown(std::net::Shutdown::Both);
                say(out, Tone::Dim, format!("closed {addr}"));
                return Ok(());
            }
            Err(TryRecvError::Empty) => {}
        }
        match stream.read(&mut buf) {
            Ok(0) => {
                if !pending.is_empty() {
                    say(
                        out,
                        Tone::Rx,
                        String::from_utf8_lossy(&pending).into_owned(),
                    );
                }
                say(out, Tone::Warn, format!("{addr} closed the connection"));
                return Ok(());
            }
            Ok(n) => {
                pending.extend_from_slice(&buf[..n]);
                while let Some(pos) = pending.iter().position(|b| *b == b'\n') {
                    let line: Vec<u8> = pending.drain(..=pos).collect();
                    say(
                        out,
                        Tone::Rx,
                        String::from_utf8_lossy(&line)
                            .trim_end_matches(['\r', '\n'])
                            .to_string(),
                    );
                }
                // A prompt without newline (e.g. "login: ").
                if pending.len() > 200 {
                    say(
                        out,
                        Tone::Rx,
                        String::from_utf8_lossy(&pending).into_owned(),
                    );
                    pending.clear();
                }
            }
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                if !pending.is_empty() {
                    say(
                        out,
                        Tone::Rx,
                        String::from_utf8_lossy(&pending).into_owned(),
                    );
                    pending.clear();
                }
            }
            Err(e) => bail!("{addr}: {e}"),
        }
    }
}

/// Run a client tool in its own thread; errors go to the screen.
fn background(name: &str, out: Out, f: impl FnOnce() -> Result<()> + Send + 'static) {
    let err_out = out.clone();
    let tool = name.to_string();
    let r = std::thread::Builder::new()
        .name(name.into())
        .stack_size(12 * 1024)
        .spawn(move || {
            if let Err(e) = f() {
                say(&err_out, Tone::Err, format!("{tool}: {e:#}"));
            }
        });
    if let Err(e) = r {
        say(&out, Tone::Err, format!("{name}: no thread: {e}"));
    }
}
