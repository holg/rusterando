//! The Cardputer's kitchen screen: the board as one list (Neu, In Arbeit,
//! Fertig), keys for the moves, a detail page per order, an info page.
//!
//! Keys (Küche):
//!   ↑ ↓ (Fn + ; .  or just ; .)  choose an order · 1–9 the n-th
//!   Enter   next step (Starten → Fertig → Abgeholt / Fahrer unterwegs)
//!   z       one step back           Del (Fn + Bksp) twice: Storno
//!   Space   details (Esc, Space or Bksp: back)
//!   Tab     Küche → Terminal → Info → Küche
//!
//! Terminal: the Tab5's shell (tab5-lab) — `help`, `rak …`, `wifi …`,
//! `relay …`; Esc leaves the LoRa chat, Fn + ↑ ↓ the history, Ctrl+L clears.
//!
//! The same events as the Tab5 come in ([`UiMsg`], from feed.rs), the same
//! moves go out (`ClientMessage::BoardAdvance`, to the feed's action loop).

use std::collections::HashSet;
use std::rc::Rc;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::Result;
use kitchen_protocol::{BoardOrder, BoardSnapshot, BoardStatus, ClientMessage, OrderChannel};
use slint::platform::software_renderer::MinimalSoftwareWindow;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use crate::display::{self, Display};
use crate::keyboard::{Key, Keyboard, Press};
use crate::terminal::{Terminal, Tone};
use crate::{CardWindow, Line, Row};

/// The Terminal page's parts.
pub struct Lab {
    pub shell: crate::shell::Shell,
    pub term: Terminal,
    /// Lines from every thread (`say`, the log tee).
    pub lines: Receiver<(Tone, String)>,
    pub rak: Option<crate::rak::Rak>,
}

/// Server → screen (feed.rs, relay.rs): the Tab5's messages.
pub enum UiMsg {
    /// Feed up/down, board, shop panel, admin alert, LoRa relay.
    Event(kds_ui::Event),
    /// A refused move or a login problem.
    Notice(String),
    /// A success note.
    Info(String),
}

/// Rows the list shows at once.
const ROWS: usize = 4;
/// Lines a detail or info page shows at once.
const LINES: usize = 8;
/// Terminal: output lines above the input line, characters per line.
const TERM_LINES: usize = 8;
const TERM_COLS: usize = 41;
const TICK: Duration = Duration::from_millis(20);
/// A message stays in the footer this long.
const NOTICE_FOR: Duration = Duration::from_secs(6);
/// Storno must be confirmed (second Del) within this long.
const CANCEL_CONFIRM: Duration = Duration::from_secs(5);
/// New order: white flashes, each step this long.
const FLASH_STEP: Duration = Duration::from_millis(250);
const FLASHES: u32 = 3;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    List,
    Detail,
    Terminal,
    Info,
}

struct State {
    board: Option<BoardSnapshot>,
    /// The server's clock at the last board, and when it arrived.
    server_clock: Option<(i64, Instant)>,
    online: bool,
    relay_from: Option<String>,
    page: Page,
    /// Order id under the cursor.
    selected: Option<String>,
    /// First visible line of a detail/info page.
    scroll: usize,
    notice: Option<(String, bool, Instant)>,
    cancel_armed: Option<(String, Instant)>,
    /// Orders already seen (a new id rings).
    seen: HashSet<String>,
    board_seen: bool,
    flash_until: Option<Instant>,
    demo: bool,
}

impl State {
    fn now(&self) -> i64 {
        match self.server_clock {
            Some((at_server, at)) => at_server + at.elapsed().as_secs() as i64,
            None => kds_ui::local_now(),
        }
    }

    fn set_notice(&mut self, text: impl Into<String>, ok: bool) {
        self.notice = Some((text.into(), ok, Instant::now()));
    }

    /// The board's orders still in the kitchen: Neu, In Arbeit, Fertig,
    /// oldest first in each.
    fn orders(&self) -> Vec<&BoardOrder> {
        let mut v: Vec<&BoardOrder> = self
            .board
            .iter()
            .flat_map(|b| b.orders.iter())
            .filter(|o| rank(o.status).is_some())
            .collect();
        v.sort_by_key(|o| (rank(o.status), o.order.created_at_unix));
        v
    }

    fn selected_index(&self, orders: &[&BoardOrder]) -> Option<usize> {
        let id = self.selected.as_deref()?;
        orders.iter().position(|o| o.id == id)
    }

    fn selected_order(&self) -> Option<&BoardOrder> {
        let id = self.selected.as_deref()?;
        self.board.as_ref()?.orders.iter().find(|o| o.id == id)
    }
}

fn rank(s: BoardStatus) -> Option<i32> {
    match s {
        BoardStatus::Received => Some(0),
        BoardStatus::Preparing => Some(1),
        BoardStatus::Ready => Some(2),
        _ => None,
    }
}

#[allow(clippy::too_many_arguments)]
pub fn run(
    window: Rc<MinimalSoftwareWindow>,
    mut display: Display,
    mut keyboard: Option<Keyboard>,
    status: Arc<Mutex<String>>,
    rx: Receiver<UiMsg>,
    taps: Sender<ClientMessage>,
    demo: bool,
    mut lab: Lab,
) -> Result<()> {
    let screen = CardWindow::new().map_err(|e| anyhow::anyhow!("screen: {e:?}"))?;
    screen.show().map_err(|e| anyhow::anyhow!("show: {e:?}"))?;
    let mut st = State {
        board: None,
        server_clock: None,
        online: false,
        relay_from: None,
        page: Page::List,
        selected: None,
        scroll: 0,
        notice: None,
        cancel_armed: None,
        seen: HashSet::new(),
        board_seen: false,
        flash_until: None,
        demo,
    };
    if demo {
        apply(&mut st, kds_ui::Event::Board(kds_ui::demo::snapshot()));
        st.set_notice("Demo: keine Verbindung zum Shop", false);
    }
    let mut last_render = Instant::now() - Duration::from_secs(60);
    let mut dirty = true;
    loop {
        slint::platform::update_timers_and_animations();
        while let Ok(msg) = rx.try_recv() {
            match msg {
                UiMsg::Event(ev) => apply(&mut st, ev),
                UiMsg::Notice(text) => st.set_notice(text, false),
                UiMsg::Info(text) => st.set_notice(text, true),
            }
            dirty = true;
        }
        if let Some(kb) = keyboard.as_mut() {
            match kb.keys() {
                Ok(keys) => {
                    for k in keys {
                        if st.page == Page::Terminal && k.key != Key::Tab {
                            lab.shell.key(k.key, &mut lab.term);
                        } else {
                            on_key(&mut st, k, &taps);
                        }
                        dirty = true;
                    }
                }
                Err(e) => log::warn!("keyboard: {e:#}"),
            }
        }
        while let Ok((tone, text)) = lab.lines.try_recv() {
            lab.term.push(tone, &text);
        }
        lab.shell.tick(&mut lab.term);
        if st.page == Page::Terminal && lab.term.is_dirty() {
            dirty = true;
        }
        if st.notice.as_ref().is_some_and(|(_, _, at)| at.elapsed() > NOTICE_FOR) {
            st.notice = None;
            dirty = true;
        }
        if st.flash_until.is_some_and(|t| Instant::now() > t) {
            st.flash_until = None;
            dirty = true;
        }
        // Waiting times and the clock move on their own.
        if dirty || st.flash_until.is_some() || last_render.elapsed() > Duration::from_secs(15) {
            render(&screen, &st, &status, &mut lab);
            last_render = Instant::now();
            dirty = false;
        }
        window.draw_if_needed(|renderer| {
            let mut lines = display::Lines::new(&mut display);
            renderer.render_by_line(&mut lines);
        });
        std::thread::sleep(TICK);
    }
}

/// An event from the server (or the demo).
fn apply(st: &mut State, ev: kds_ui::Event) {
    match ev {
        kds_ui::Event::Connected(up) => {
            st.online = up;
            if up {
                st.relay_from = None;
            }
        }
        kds_ui::Event::Relay(from) => st.relay_from = from,
        kds_ui::Event::Board(board) => {
            st.server_clock = Some((board.server_now_unix, Instant::now()));
            let ids: HashSet<String> = board.orders.iter().map(|o| o.id.clone()).collect();
            let fresh: Vec<&BoardOrder> = board
                .orders
                .iter()
                .filter(|o| o.status == BoardStatus::Received && !st.seen.contains(&o.id))
                .collect();
            if st.board_seen && !fresh.is_empty() {
                // A new order: flash, and put the cursor on it.
                st.selected = Some(fresh[0].id.clone());
                st.page = Page::List;
                st.flash_until = Some(Instant::now() + FLASH_STEP * (2 * FLASHES - 1));
                log::info!("ui: new order #{}", kds_ui::short_number(&fresh[0].order));
            }
            st.seen = ids;
            st.board_seen = true;
            st.board = Some(board);
            // The cursor stays on its order; gone → the first one.
            let orders = st.orders();
            if st.selected_index(&orders).is_none() {
                st.selected = orders.first().map(|o| o.id.clone());
                if st.page == Page::Detail {
                    st.page = Page::List;
                }
            }
        }
        kds_ui::Event::Shop(_) => {}
        kds_ui::Event::Alert { title, text } => {
            st.set_notice(format!("{title}: {text}"), false);
            st.flash_until = Some(Instant::now() + FLASH_STEP * (2 * FLASHES - 1));
        }
    }
}

fn on_key(st: &mut State, p: Press, taps: &Sender<ClientMessage>) {
    let key = p.key;
    match st.page {
        Page::List => match key {
            Key::Up | Key::Char(';') => step(st, -1),
            Key::Down | Key::Char('.') => step(st, 1),
            Key::Char(c @ '1'..='9') => {
                let orders = st.orders();
                if let Some(o) = orders.get(c as usize - '1' as usize) {
                    st.selected = Some(o.id.clone());
                }
            }
            Key::Enter => advance(st, taps, true),
            Key::Char('z') => advance(st, taps, false),
            Key::Delete | Key::Char('x') => cancel(st, taps),
            Key::Char(' ') | Key::Right if st.selected_order().is_some() => {
                st.page = Page::Detail;
                st.scroll = 0;
            }
            Key::Tab => st.page = Page::Terminal,
            _ => {}
        },
        Page::Terminal => {
            if key == Key::Tab {
                st.page = Page::Info;
                st.scroll = 0;
            }
        }
        Page::Detail => match key {
            Key::Up | Key::Char(';') => st.scroll = st.scroll.saturating_sub(1),
            Key::Down | Key::Char('.') => st.scroll += 1,
            Key::Enter => advance(st, taps, true),
            Key::Char('z') => advance(st, taps, false),
            Key::Delete | Key::Char('x') => cancel(st, taps),
            Key::Esc | Key::Backspace | Key::Char(' ') | Key::Left | Key::Tab => st.page = Page::List,
            _ => {}
        },
        Page::Info => match key {
            Key::Up | Key::Char(';') => st.scroll = st.scroll.saturating_sub(1),
            Key::Down | Key::Char('.') => st.scroll += 1,
            Key::Tab | Key::Esc | Key::Backspace => st.page = Page::List,
            _ => {}
        },
    }
}

/// Move the cursor by `by` orders.
fn step(st: &mut State, by: i32) {
    let orders = st.orders();
    if orders.is_empty() {
        return;
    }
    let at = st.selected_index(&orders).unwrap_or(0) as i32;
    let to = (at + by).clamp(0, orders.len() as i32 - 1) as usize;
    st.selected = Some(orders[to].id.clone());
}

/// Next step (`forward`) or one back, for the order under the cursor.
fn advance(st: &mut State, taps: &Sender<ClientMessage>, forward: bool) {
    let Some(o) = st.selected_order() else { return };
    let (next, back) = kds_ui::actions(o.status, kds_ui::is_delivery(&o.order));
    let Some((label, to)) = (if forward { next } else { back }) else {
        return;
    };
    let (id, from) = (o.id.clone(), o.status);
    let number = kds_ui::short_number(&o.order);
    send(st, taps, id, from, to);
    st.set_notice(format!("#{number}: {label}"), true);
}

/// Storno: the first Del arms it, a second within CANCEL_CONFIRM sends it.
fn cancel(st: &mut State, taps: &Sender<ClientMessage>) {
    let Some(o) = st.selected_order() else { return };
    let (id, from, number) = (o.id.clone(), o.status, kds_ui::short_number(&o.order));
    match &st.cancel_armed {
        Some((armed, at)) if *armed == id && at.elapsed() < CANCEL_CONFIRM => {
            st.cancel_armed = None;
            send(st, taps, id, from, BoardStatus::Cancelled);
            st.set_notice(format!("#{number}: storniert"), true);
        }
        _ => {
            st.cancel_armed = Some((id, Instant::now()));
            st.set_notice(format!("#{number} stornieren? Del noch einmal"), false);
        }
    }
}

fn send(st: &mut State, taps: &Sender<ClientMessage>, id: String, from: BoardStatus, to: BoardStatus) {
    if st.demo {
        kds_ui::demo::apply_move(&mut st.board, &id, to);
        return;
    }
    let msg = ClientMessage::BoardAdvance { order_id: id, from, to };
    if taps.send(msg).is_err() {
        st.set_notice("Keine Verbindung zum Shop", false);
    }
}

fn render(screen: &CardWindow, st: &State, status: &Arc<Mutex<String>>, lab: &mut Lab) {
    let now = st.now();
    let orders = st.orders();
    let count = |s: BoardStatus| orders.iter().filter(|o| o.status == s).count() as i32;
    screen.set_n_new(count(BoardStatus::Received));
    screen.set_n_work(count(BoardStatus::Preparing));
    screen.set_n_ready(count(BoardStatus::Ready));
    screen.set_clock(clock(now).into());
    screen.set_battery(crate::battery::percent().map(|p| format!("{p}%")).unwrap_or_default().into());
    screen.set_battery_low(crate::battery::percent().is_some_and(|p| p < 20));
    screen.set_link(if st.online {
        1
    } else if st.relay_from.is_some() {
        2
    } else {
        0
    });
    screen.set_flash(st.flash_until.is_some_and(|t| {
        // On, off, on …: the phase from the end backwards.
        let left = t.saturating_duration_since(Instant::now()).as_millis() / FLASH_STEP.as_millis();
        left % 2 == 0
    }));
    let (notice, ok) = match &st.notice {
        Some((text, ok, _)) => (text.clone(), *ok),
        None => (String::new(), false),
    };
    screen.set_notice(notice.into());
    screen.set_notice_ok(ok);
    if st.page != Page::Terminal {
        screen.set_term_input(SharedString::new());
    }

    match st.page {
        Page::List => {
            screen.set_page(0);
            screen.set_heading("KÜCHE".into());
            let sel = st.selected_index(&orders).unwrap_or(0);
            // Keep the cursor's row in view.
            let first = sel.saturating_sub(ROWS - 1).min(orders.len().saturating_sub(ROWS));
            let rows: Vec<Row> = orders
                .iter()
                .enumerate()
                .skip(first)
                .take(ROWS)
                .map(|(i, o)| row(o, now, i == sel))
                .collect();
            screen.set_above(if first > 0 { format!("↑{first}") } else { String::new() }.into());
            let rest = orders.len().saturating_sub(first + ROWS);
            screen.set_below(if rest > 0 { format!("↓{rest}") } else { String::new() }.into());
            screen.set_rows(ModelRc::new(VecModel::from(rows)));
            screen.set_empty_text(
                if st.board.is_none() {
                    status.lock().map(|s| s.clone()).unwrap_or_default()
                } else {
                    "Keine offenen Bestellungen".to_string()
                }
                .into(),
            );
            screen.set_hint("⏎ weiter · z zurück · ␣ Details · Tab Terminal".into());
        }
        Page::Detail => {
            screen.set_page(1);
            let lines = st.selected_order().map(|o| detail(o, now)).unwrap_or_default();
            let (title, hint) = match st.selected_order() {
                Some(o) => {
                    let (next, _) = kds_ui::actions(o.status, kds_ui::is_delivery(&o.order));
                    (
                        format!("#{}", kds_ui::short_number(&o.order)),
                        match next {
                            Some((label, _)) => format!("⏎ {label} · z zurück · Del Storno · Esc"),
                            None => "Esc zurück".to_string(),
                        },
                    )
                }
                None => ("—".to_string(), "Esc zurück".to_string()),
            };
            screen.set_heading(title.into());
            screen.set_hint(hint.into());
            set_lines(screen, &lines, st.scroll);
        }
        Page::Terminal => {
            screen.set_page(3);
            screen.set_heading("TERMINAL".into());
            screen.set_hint("help · Esc Chat aus · Fn ↑↓ Verlauf · Tab Info".into());
            // The newest lines, wrapped to the screen's width.
            let mut wrapped: Vec<Line> = Vec::new();
            for (tone, text) in lab.term.lines().iter().rev() {
                let chars: Vec<char> = text.chars().collect();
                let mut parts: Vec<String> =
                    chars.chunks(TERM_COLS).map(|c| c.iter().collect()).collect();
                if parts.is_empty() {
                    parts.push(String::new());
                }
                for part in parts.into_iter().rev() {
                    wrapped.push(line(part, tone_kind(*tone)));
                }
                if wrapped.len() >= TERM_LINES {
                    break;
                }
            }
            wrapped.truncate(TERM_LINES);
            wrapped.reverse();
            screen.set_lines(ModelRc::new(VecModel::from(wrapped)));
            let (before, after) = lab.term.input_split();
            let prompt = lab.term.prompt.replace("tab5", "card");
            let input = format!("{prompt}{before}_{after}");
            // The end of a long input stays in view.
            let n = input.chars().count();
            let shown: String = input.chars().skip(n.saturating_sub(TERM_COLS)).collect();
            screen.set_term_input(shown.into());
            screen.set_above(SharedString::new());
            screen.set_below(SharedString::new());
            lab.term.drawn();
        }
        Page::Info => {
            screen.set_page(2);
            screen.set_heading("INFO".into());
            screen.set_hint("Tab zurück zur Küche".into());
            let link = status.lock().map(|s| s.clone()).unwrap_or_default();
            let mut lines = vec![
                line(crate::board::device_name(), 2),
                line(format!("Version {}", crate::ota::VERSION), 1),
                line(link, 0),
                line(
                    if st.online {
                        "Shop: verbunden".to_string()
                    } else if let Some(from) = &st.relay_from {
                        format!("Shop: über LoRa von {from}")
                    } else if st.demo {
                        "Shop: Demo (kein Passwort)".to_string()
                    } else {
                        "Shop: nicht verbunden".to_string()
                    },
                    // Green online, amber over LoRa, red without either.
                    if st.online {
                        4
                    } else if st.relay_from.is_some() {
                        3
                    } else {
                        2
                    },
                ),
            ];
            lines.push(line(
                match &lab.rak {
                    Some(r) => r.state().describe(),
                    None => "LoRa: keine Cap".to_string(),
                },
                1,
            ));
            lines.push(line(
                crate::battery::describe(),
                if crate::battery::percent().is_some_and(|p| p < 20) { 3 } else { 1 },
            ));
            lines.push(line(format!("RAM frei {} KB", crate::free_kb()), 1));
            set_lines(screen, &lines, st.scroll);
        }
    }
}

fn set_lines(screen: &CardWindow, lines: &[Line], scroll: usize) {
    let first = scroll.min(lines.len().saturating_sub(LINES));
    let shown: Vec<Line> = lines.iter().skip(first).take(LINES).cloned().collect();
    screen.set_lines(ModelRc::new(VecModel::from(shown)));
    screen.set_above(if first > 0 { format!("↑{first}") } else { String::new() }.into());
    let rest = lines.len().saturating_sub(first + LINES);
    screen.set_below(if rest > 0 { format!("↓{rest}") } else { String::new() }.into());
}

/// Terminal tones onto the line kinds of the screen.
fn tone_kind(t: Tone) -> i32 {
    match t {
        Tone::Normal | Tone::Rx => 0,
        Tone::Dim | Tone::Tx => 1,
        Tone::Err => 2,
        Tone::Warn => 3,
        Tone::Ok | Tone::Echo => 4,
    }
}

fn line(text: impl Into<SharedString>, kind: i32) -> Line {
    Line { text: text.into(), kind }
}

fn row(o: &BoardOrder, now: i64, selected: bool) -> Row {
    let t = kds_ui::timing(&o.order, now);
    let tag = match &o.order.channel {
        OrderChannel::Delivery { .. } => "Lieferung".to_string(),
        OrderChannel::Pickup => "Abholung".to_string(),
        OrderChannel::DineIn { table } => format!("Tisch {table}"),
    };
    let when = if t.scheduled { format!("{} · {}", t.when, t.age) } else { t.age.clone() };
    let items = o
        .order
        .items
        .iter()
        .map(|it| format!("{}× {}", it.qty, it.name))
        .collect::<Vec<_>>()
        .join(" · ");
    Row {
        number: kds_ui::short_number(&o.order).into(),
        tag: tag.into(),
        when: when.into(),
        items: items.into(),
        status: rank(o.status).unwrap_or(0),
        urgency: t.urgency,
        selected,
    }
}

/// The order's lines for the detail page.
fn detail(o: &BoardOrder, now: i64) -> Vec<Line> {
    let ord = &o.order;
    let t = kds_ui::timing(ord, now);
    let mut v = Vec::new();
    let what = match &ord.channel {
        OrderChannel::Delivery { address } => format!("Lieferung · {} {}", address.street, address.city),
        OrderChannel::Pickup => "Abholung".to_string(),
        OrderChannel::DineIn { table } => format!("Tisch {table}"),
    };
    v.push(line(format!("{what}"), 2));
    v.push(line(
        if t.scheduled { format!("fällig {} ({})", t.when, t.age) } else { format!("sofort · seit {}", t.age) },
        if t.urgency == 2 { 3 } else { 1 },
    ));
    for it in &ord.items {
        v.push(line(format!("{}× {}", it.qty, it.name), 0));
        for m in &it.modifications {
            v.push(line(format!("    {m}"), 1));
        }
    }
    if let Some(note) = ord.note.as_deref().filter(|n| !n.trim().is_empty()) {
        v.push(line(format!("Notiz: {note}"), 3));
    }
    let name = ord.customer.name.trim();
    if !name.is_empty() {
        v.push(line(name.to_string(), 1));
    }
    v
}

/// "HH:MM" in the shop's time (TZ is set at start).
fn clock(now: i64) -> String {
    // SAFETY: localtime_r writes into the struct we pass.
    unsafe {
        let t: esp_idf_svc::sys::time_t = now as _;
        let mut tm: esp_idf_svc::sys::tm = core::mem::zeroed();
        esp_idf_svc::sys::localtime_r(&t, &mut tm);
        format!("{:02}:{:02}", tm.tm_hour, tm.tm_min)
    }
}
