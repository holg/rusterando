//! The kitchen display screen: `ui/kitchen.slint` plus everything that fills
//! it from the server's board and shop state. Shared by the STM32 board
//! (`rusterando-printer --features kds-*`) and the browser mirror
//! (`kds-web`, WASM), so both always show the same thing.
//!
//! All screen state lives in [`Core`], a thread-local on the UI thread.
//! The host — board or browser — feeds it server events ([`apply`]), sends
//! the taps it gets back ([`wire`]'s `send`) to the server, and adds what
//! only it knows to each frame: the printer chip and the Start page's
//! utility buttons ([`Host::decorate`]).

pub mod demo;

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;
use std::time::Duration;

use chrono::{Local, NaiveTime, Timelike};
use kitchen_protocol::{
    board_transition_allowed, BoardOrder, BoardSnapshot, BoardStatus, ClientMessage,
    OrderChannel, OrderForKitchen, PaymentStatus, ShopAction, ShopLight, ShopPanel,
};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use tracing::info;
pub use web_time::Instant;

slint::include_modules!();

/// Re-render this often so waiting times and the clock move on their own.
const TICK: Duration = Duration::from_secs(10);
/// A not-yet-started order this young gets the highlighted card.
const FRESH_SECS: i64 = 3 * 60;
/// New-order alert: this many white flashes, each step (on or off) this long.
const FLASHES: u32 = 3;
const FLASH_STEP: Duration = Duration::from_millis(250);
/// A Storno tap must be confirmed within this long.
const CANCEL_CONFIRM: Duration = Duration::from_secs(5);
/// A new order switches to the board unless someone touched the screen
/// this recently (don't pull a button away from a finger).
const BUSY_GRACE: Duration = Duration::from_secs(15);
/// Online this long without a board → the server is too old (pre-2.1).
const NO_BOARD_AFTER: Duration = Duration::from_secs(5);
/// How long a message (refused shop action, Testdruck result) stays up.
pub const NOTICE_FOR: Duration = Duration::from_secs(8);

/// Server → screen events (the host's transport delivers them).
#[derive(Debug, Clone)]
pub enum Event {
    /// Session to the server up (`true`) or down.
    Connected(bool),
    /// Fresh board.
    Board(BoardSnapshot),
    /// Shop open/closed state + today's hours ("Öffnungszeiten" page).
    Shop(ShopPanel),
    /// Admin alert: flash now, with this title and text.
    Alert { title: String, text: String },
}

/// What [`apply`] asks the host to do beyond re-rendering.
#[derive(Debug, Default)]
pub struct Outcome {
    /// Bring the screen back from the screensaver (board only), and why.
    pub wake: Option<&'static str>,
    /// New order / admin alert: [`flash`] it, and ring the gong.
    pub flash: Option<(String, String)>,
}

impl Core {
    /// Unix seconds now, by the server's clock (ours until the first
    /// snapshot).
    pub fn now(&self) -> i64 {
        match self.server_clock {
            Some((at_server, at)) => at_server + at.elapsed().as_secs() as i64,
            None => local_now(),
        }
    }
}

#[derive(Default)]
pub struct Core {
    pub board: Option<BoardSnapshot>,
    /// The server's clock at the last snapshot, and when (monotonic) it
    /// arrived. "Now" counts on from there, so a jump of our own clock —
    /// the Tab5 sets its clock by SNTP only after the first snapshot — can't
    /// skew waiting times (that showed "seit 29880964 min" = since 1970).
    pub server_clock: Option<(i64, Instant)>,
    pub online: bool,
    /// Order id of the open detail view.
    pub detail: Option<String>,
    /// Sample data, moves applied locally (no server).
    pub demo: bool,
    /// Last touch anywhere; `None` until the UI is up.
    pub last_activity: Option<Instant>,
    /// Ids on the previous board, to spot orders that just arrived.
    seen: HashSet<String>,
    /// (id, status) of the last board seen and the last shop state (without
    /// its notice) — kept across reconnects, so only a real change (not a
    /// re-sent snapshot after a network blip) counts as news.
    board_sig: Vec<(String, BoardStatus)>,
    shop_seen: Option<ShopPanel>,
    /// A board has been seen in this process (the first one is no news).
    board_seen: bool,
    /// Shop state for the "Öffnungszeiten" page and the header chip.
    pub shop: Option<ShopPanel>,
    /// 0 Start · 1 Küche (default) · 2 Öffnungszeiten.
    pub page: i32,
    /// Stepper values; `None` = follow today's hours.
    pick_open: Option<String>,
    pick_close: Option<String>,
    /// A message for the user (`true` = good news), until it times out.
    pub notice: Option<(String, bool, Instant)>,
    /// When the current session came up (for the "server sends no board" hint).
    connected_at: Option<Instant>,
    /// Storno armed for this order (first tap), until CANCEL_CONFIRM passes.
    cancel_armed: Option<(String, Instant)>,
}

impl Core {
    fn touched(&mut self) {
        self.last_activity = Some(Instant::now());
    }

    /// Show a message for [`NOTICE_FOR`].
    pub fn set_notice(&mut self, text: String, ok: bool) {
        self.notice = Some((text, ok, Instant::now()));
    }
}

/// What only the host knows, added to every frame after the shared part.
pub trait Host {
    /// Printer chip, Start page utility buttons and info line.
    fn decorate(&self, w: &KitchenWindow, core: &Core);
    /// `false` while the screen must not be drawn (board: lent to the
    /// screensaver). State is kept; the next render after that shows it.
    fn may_render(&self) -> bool {
        true
    }
}

thread_local! {
    static CORE: RefCell<Core> = RefCell::new(Core { page: 1, ..Core::default() });
    static HOST: RefCell<Option<Rc<dyn Host>>> = const { RefCell::new(None) };
    static TICKER: slint::Timer = slint::Timer::default();
    static FLASH_TIMER: slint::Timer = slint::Timer::default();
}

/// Read or change the screen state.
pub fn with<R>(f: impl FnOnce(&mut Core) -> R) -> R {
    CORE.with(|c| f(&mut c.borrow_mut()))
}

/// Hook the window's callbacks up. `send` carries a tap to the server
/// (`BoardAdvance`, `Shop`); in demo mode taps are applied locally instead.
/// Also starts the clock tick.
pub fn wire(w: &KitchenWindow, host: Rc<dyn Host>, send: impl Fn(ClientMessage) + 'static) {
    HOST.with(|h| *h.borrow_mut() = Some(host));
    with(|c| c.touched());
    let send = Rc::new(send);

    let weak = w.as_weak();
    w.on_open(move |id| {
        let Some(w) = weak.upgrade() else { return };
        with(|c| {
            c.detail = Some(id.to_string());
            c.touched();
        });
        render(&w);
    });
    let weak = w.as_weak();
    w.on_close_detail(move || {
        let Some(w) = weak.upgrade() else { return };
        with(|c| {
            c.detail = None;
            c.touched();
        });
        render(&w);
    });
    w.on_activity(|| with(|c| c.touched()));
    let weak = w.as_weak();
    w.on_cancel_tap(move |id| {
        let Some(w) = weak.upgrade() else { return };
        let confirmed = with(|c| {
            c.touched();
            let armed = c
                .cancel_armed
                .take()
                .is_some_and(|(armed_id, at)| armed_id == id.as_str() && at.elapsed() < CANCEL_CONFIRM);
            if !armed {
                c.cancel_armed = Some((id.to_string(), Instant::now()));
            }
            armed
        });
        if confirmed {
            w.invoke_advance(id, status_index(BoardStatus::Cancelled));
            return;
        }
        render(&w);
        // Disarm visibly once the confirmation window has passed.
        let weak = w.as_weak();
        slint::Timer::single_shot(CANCEL_CONFIRM, move || {
            if let Some(w) = weak.upgrade() {
                render(&w);
            }
        });
    });
    let weak = w.as_weak();
    w.on_go_page(move |page| {
        let Some(w) = weak.upgrade() else { return };
        with(|c| {
            c.page = page.clamp(0, 2);
            c.detail = None;
            c.touched();
        });
        render(&w);
    });
    let weak = w.as_weak();
    let shop_send = send.clone();
    w.on_shop(move |code, arg| {
        let Some(w) = weak.upgrade() else { return };
        let action = with(|c| {
            c.touched();
            let action = shop_action(c, code, arg)?;
            if c.demo {
                demo::apply_shop(&mut c.shop, &action);
                return None;
            }
            Some(action)
        });
        // The server answers with a fresh panel.
        if let Some(action) = action {
            shop_send(ClientMessage::Shop(action));
        }
        render(&w);
    });
    let weak = w.as_weak();
    w.on_pick_step(move |which, delta| {
        let Some(w) = weak.upgrade() else { return };
        with(|c| {
            c.touched();
            let (open_now, close_now) = picks(c);
            if which == 0 {
                c.pick_open = Some(min_to_hhmm(hhmm_to_min(&open_now, 16) + delta as i64));
            } else {
                c.pick_close = Some(min_to_hhmm(hhmm_to_min(&close_now, 22) + delta as i64));
            }
        });
        render(&w);
    });
    let weak = w.as_weak();
    w.on_advance(move |id, to| {
        let Some(w) = weak.upgrade() else { return };
        let Some(to) = status_from_index(to) else { return };
        let msg = with(|c| {
            c.detail = None;
            c.touched();
            let from = c
                .board
                .as_ref()
                .and_then(|b| b.orders.iter().find(|o| o.id == id.as_str()))
                .map(|o| o.status)?;
            if c.demo {
                demo::apply_move(&mut c.board, &id, to);
                return None;
            }
            // The server answers with a fresh snapshot; until then the
            // card stays where it was.
            Some(ClientMessage::BoardAdvance {
                order_id: id.to_string(),
                from,
                to,
            })
        });
        if let Some(msg) = msg {
            send(msg);
        }
        render(&w);
    });

    let weak = w.as_weak();
    TICKER.with(|t| {
        t.start(slint::TimerMode::Repeated, TICK, move || {
            if let Some(w) = weak.upgrade() {
                render(&w);
            }
        })
    });
}

/// Fill the demo board (no server): sample orders, moves applied locally.
pub fn start_demo() {
    with(|c| {
        c.demo = true;
        c.online = true;
        c.board = Some(demo::snapshot());
        c.shop = Some(demo::shop());
    });
}

/// Apply a server event. `saver_shown`: the board's screensaver is up (any
/// change then wakes it). Renders unless the outcome asks for a wake — the
/// host's wake renders.
pub fn apply(w: &KitchenWindow, ev: Event, saver_shown: bool) -> Outcome {
    let mut out = Outcome::default();
    with(|c| match ev {
        Event::Connected(up) => {
            c.online = up;
            if up {
                // A new session: what the last one showed may come from
                // another server (or a test). Only this server's board
                // and shop state count from now on.
                c.board = None;
                c.shop = None;
                c.detail = None;
                c.connected_at = Some(Instant::now());
                let weak = w.as_weak();
                slint::Timer::single_shot(NO_BOARD_AFTER + Duration::from_secs(1), move || {
                    if let Some(w) = weak.upgrade() {
                        render(&w);
                    }
                });
            } else {
                c.connected_at = None;
            }
        }
        Event::Alert { title, text } => {
            // Admin "Alarm auslösen": out of the screensaver, onto the
            // board, flash + gong.
            if saver_shown {
                out.wake = Some("admin alert");
            }
            c.page = 1;
            c.detail = None;
            out.flash = Some((title, text));
        }
        Event::Shop(mut panel) => {
            let notice = std::mem::take(&mut panel.notice);
            if !notice.is_empty() {
                c.set_notice(notice, false);
            }
            // Opened, closed, paused, delivery off, today's hours —
            // from the web, a terminal or the schedule: show it.
            let changed = c.shop_seen.as_ref().is_some_and(|prev| prev != &panel);
            c.shop_seen = Some(panel.clone());
            if changed && saver_shown {
                out.wake = Some("shop status changed");
            }
            c.shop = Some(panel);
        }
        Event::Board(board) => {
            c.server_clock = Some((board.server_now_unix, Instant::now()));
            let arrived = board
                .orders
                .iter()
                .any(|o| o.status == BoardStatus::Received && !c.seen.contains(&o.id));
            let prev_seen = std::mem::take(&mut c.seen);
            c.seen = board.orders.iter().map(|o| o.id.clone()).collect();
            // Any order change (status, Storno, handed over, new) wakes the
            // screensaver to the board — e.g. an admin action on the web.
            let sig: Vec<(String, BoardStatus)> =
                board.orders.iter().map(|o| (o.id.clone(), o.status)).collect();
            let changed = !c.board_sig.is_empty() && sig != c.board_sig;
            c.board_sig = sig;
            if changed && saver_shown {
                out.wake = Some("orders changed");
                c.page = 1;
                c.detail = None;
            }
            if arrived && c.board_seen {
                // Flash the screen: which order(s) just came in.
                let new: Vec<&BoardOrder> = board
                    .orders
                    .iter()
                    .filter(|o| o.status == BoardStatus::Received && !prev_seen.contains(&o.id))
                    .collect();
                out.flash = Some(match new.as_slice() {
                    [o] => (
                        "NEUE BESTELLUNG".to_string(),
                        format!(
                            "#{} · {} · {}",
                            short_number(&o.order),
                            if is_delivery(&o.order) { "LIEF" } else { "ABH" },
                            if matches!(o.order.payment, PaymentStatus::Prepaid) {
                                "BEZAHLT"
                            } else {
                                "BAR"
                            }
                        ),
                    ),
                    many => (format!("{} NEUE BESTELLUNGEN", many.len()), String::new()),
                });
            }
            c.board_seen = true;
            if arrived {
                // A new order is shown on the board: from the screensaver
                // always, from the hours page or a detail unless someone is
                // using the screen right now.
                let busy = c.last_activity.is_some_and(|t| t.elapsed() < BUSY_GRACE);
                if saver_shown || !busy {
                    if c.page != 1 || c.detail.is_some() {
                        info!("new order: back to the board");
                    }
                    c.page = 1;
                    c.detail = None;
                }
                if saver_shown {
                    out.wake = Some("new order");
                }
            }
            // Close a detail whose order left the board elsewhere.
            if let Some(id) = &c.detail {
                if !board.orders.iter().any(|o| &o.id == id) {
                    c.detail = None;
                }
            }
            c.board = Some(board);
        }
    });
    if out.wake.is_none() {
        render(w);
    }
    out
}

/// Flash the screen white FLASHES times (on/off every FLASH_STEP) with the
/// new order on it. A new alert restarts the sequence. The gong is the
/// host's.
pub fn flash(w: &KitchenWindow, title: String, text: String) {
    info!(%title, %text, "new order alert");
    w.set_flash_title(title.into());
    w.set_flash_text(text.into());
    w.set_flash(true);
    let weak = w.as_weak();
    let mut steps = 1u32;
    FLASH_TIMER.with(|t| {
        t.start(slint::TimerMode::Repeated, FLASH_STEP, move || {
            let Some(w) = weak.upgrade() else { return };
            steps += 1;
            if steps >= FLASHES * 2 {
                w.set_flash(false);
                FLASH_TIMER.with(|t| t.stop());
            } else {
                w.set_flash(steps % 2 == 1);
            }
        })
    });
}

/// Redraw everything from [`Core`], then let the host add its part.
pub fn render(w: &KitchenWindow) {
    let host = HOST.with(|h| h.borrow().clone());
    if host.as_ref().is_some_and(|h| !h.may_render()) {
        return;
    }
    CORE.with(|c| {
        let c = c.borrow();
        render_core(w, &c);
        if let Some(h) = &host {
            h.decorate(w, &c);
        }
    });
}

fn render_core(w: &KitchenWindow, c: &Core) {
    let now = c.now();

    w.set_clock(Local::now().format("%H:%M").to_string().into());
    w.set_online(c.online);
    w.set_have_board(c.board.is_some());
    let (banner, warn) = if !c.online {
        if c.board.is_some() {
            ("Keine Verbindung zum Server, letzter Stand", false)
        } else {
            ("Verbinde mit dem Server …", false)
        }
    } else if c.board.is_none() && c.connected_at.is_some_and(|t| t.elapsed() >= NO_BOARD_AFTER) {
        ("Verbunden – Server sendet keine Bestellungen (Server-Update nötig)", true)
    } else {
        ("", false)
    };
    w.set_banner(banner.into());
    w.set_banner_warn(warn);
    fill_shop(w, c);

    let (mut new, mut work, mut ready) = (Vec::new(), Vec::new(), Vec::new());
    let orders = c.board.as_ref().map(|b| b.orders.as_slice()).unwrap_or(&[]);
    for o in orders {
        let card = card(o, now);
        match o.status {
            BoardStatus::Received => new.push(card),
            BoardStatus::Preparing => work.push(card),
            BoardStatus::Ready => ready.push(card),
            // Exits never appear in a snapshot.
            BoardStatus::PickedUp | BoardStatus::OutForDelivery | BoardStatus::Cancelled => {}
        }
    }
    w.set_col_new(model(new));
    w.set_col_work(model(work));
    w.set_col_ready(model(ready));

    let open = c.detail.as_ref().and_then(|id| orders.iter().find(|o| &o.id == id));
    match open {
        Some(o) => {
            let armed = c
                .cancel_armed
                .as_ref()
                .is_some_and(|(id, at)| id == &o.id && at.elapsed() < CANCEL_CONFIRM);
            fill_detail(w, o, now, armed);
            w.set_detail_open(true);
        }
        None => w.set_detail_open(false),
    }
}

fn model<T: Clone + 'static>(v: Vec<T>) -> ModelRc<T> {
    ModelRc::new(VecModel::from(v))
}

fn card(o: &BoardOrder, now: i64) -> Card {
    let t = timing(&o.order, now);
    Card {
        id: o.id.as_str().into(),
        number: short_number(&o.order).into(),
        delivery: is_delivery(&o.order),
        when: t.when.into(),
        age: t.age.into(),
        urgency: t.urgency,
        summary: o
            .order
            .items
            .iter()
            .map(|i| format!("{}× {}", i.qty, i.name))
            .collect::<Vec<_>>()
            .join(" · ")
            .into(),
        fresh: o.status == BoardStatus::Received && now - o.order.created_at_unix < FRESH_SECS,
        test: o.order.is_test_mode,
        paid: matches!(o.order.payment, PaymentStatus::Prepaid),
    }
}

fn fill_detail(w: &KitchenWindow, o: &BoardOrder, now: i64, cancel_armed: bool) {
    let order = &o.order;
    let delivery = is_delivery(order);
    let t = timing(order, now);

    w.set_d_id(o.id.as_str().into());
    w.set_d_number(
        if order.display_label.is_empty() {
            order.display_number.to_string()
        } else {
            order.display_label.clone()
        }
        .into(),
    );
    w.set_d_delivery(delivery);
    w.set_d_test(order.is_test_mode);
    let paid = matches!(order.payment, PaymentStatus::Prepaid);
    w.set_d_paid(paid);
    w.set_d_pay_short(
        match order.payment {
            PaymentStatus::CollectOnDelivery { amount_cents } => format!("BAR {}", eur(amount_cents)),
            PaymentStatus::Prepaid => "BEZAHLT".to_string(),
        }
        .into(),
    );

    let mut head = vec![if t.scheduled {
        format!("für {}", t.when)
    } else {
        "sofort".to_string()
    }];
    if !order.created_at_label.is_empty() {
        head.push(format!("Eingang {}", order.created_at_label));
    }
    head.push(format!("seit {} min", t.waited_min));
    w.set_d_head(head.join(" · ").into());

    let mut who = order.customer.name.clone();
    if let Some(phone) = order.customer.phone.as_deref().filter(|p| !p.is_empty()) {
        who.push_str(" · ");
        who.push_str(phone);
    }
    w.set_d_who(who.into());
    let address = match &order.channel {
        OrderChannel::Delivery { address } => {
            let mut a = format!("{}, {} {}", address.street, address.postal, address.city);
            if let Some(bell) = address.bell.as_deref().filter(|b| !b.is_empty()) {
                a.push_str(" · Klingel: ");
                a.push_str(bell);
            }
            a
        }
        OrderChannel::DineIn { table } => format!("Tisch {table}"),
        OrderChannel::Pickup => String::new(),
    };
    w.set_d_address(address.into());

    let lines: Vec<Line> = order
        .items
        .iter()
        .map(|i| Line {
            text: format!("{}× {}", i.qty, i.name).into(),
            sub: i.modifications.join(" · ").into(),
        })
        .collect();
    w.set_d_lines(model(lines));
    w.set_d_note(order.note.clone().unwrap_or_default().into());
    w.set_d_pay(
        match order.payment {
            PaymentStatus::CollectOnDelivery { amount_cents } => {
                format!("Bar kassieren: {} · Summe {}", eur(amount_cents), eur(order.total_cents))
            }
            PaymentStatus::Prepaid => format!("Bezahlt · Summe {}", eur(order.total_cents)),
        }
        .into(),
    );

    let (primary, back) = actions(o.status, delivery);
    let label = |a: Action| -> (SharedString, i32) {
        match a {
            Some((text, to)) if board_transition_allowed(o.status, to, delivery) => {
                (text.into(), status_index(to))
            }
            _ => (SharedString::new(), 0),
        }
    };
    w.set_d_cancel_armed(cancel_armed);
    w.set_d_cancel_text(
        if !cancel_armed {
            "Bestellung stornieren".to_string()
        } else if matches!(order.payment, PaymentStatus::Prepaid) {
            "Wirklich stornieren? Nochmal tippen (online bezahlt: Erstattung im Admin)".to_string()
        } else {
            "Wirklich stornieren? Nochmal tippen".to_string()
        }
        .into(),
    );
    let (p_text, p_to) = label(primary);
    let (b_text, b_to) = label(back);
    w.set_d_primary(p_text);
    w.set_d_primary_to(p_to);
    w.set_d_back(b_text);
    w.set_d_back_to(b_to);
}

/// A detail-view button: label + target status.
type Action = Option<(&'static str, BoardStatus)>;

/// Start page date line ("Dienstag, 6. Oktober").
fn date_text() -> String {
    const DAYS: [&str; 7] = [
        "Montag", "Dienstag", "Mittwoch", "Donnerstag", "Freitag", "Samstag", "Sonntag",
    ];
    const MONTHS: [&str; 12] = [
        "Januar", "Februar", "März", "April", "Mai", "Juni", "Juli", "August", "September",
        "Oktober", "November", "Dezember",
    ];
    use chrono::Datelike;
    let today = Local::now();
    format!(
        "{}, {}. {}",
        DAYS[today.weekday().num_days_from_monday() as usize],
        today.day(),
        MONTHS[today.month0() as usize]
    )
}

/// Header chip + "Öffnungszeiten" page from the last [`ShopPanel`].
fn fill_shop(w: &KitchenWindow, c: &Core) {
    w.set_page(c.page);
    let (notice, ok) = c
        .notice
        .as_ref()
        .filter(|(_, _, at)| at.elapsed() < NOTICE_FOR)
        .map(|(n, ok, _)| (n.clone(), *ok))
        .unwrap_or_default();
    w.set_shop_notice(notice.into());
    w.set_notice_ok(ok);
    w.set_date_text(date_text().into());
    let (pick_open, pick_close) = picks(c);
    w.set_pick_open(pick_open.into());
    w.set_pick_close(pick_close.into());
    let Some(p) = &c.shop else {
        w.set_shop_light(-1);
        w.set_shop_short("Shop …".into());
        w.set_shop_status("Noch kein Stand vom Server".into());
        w.set_today_text(SharedString::new());
        return;
    };
    let snoozing = !p.snooze_until.is_empty();
    let forced = !p.force_open_until.is_empty();
    w.set_shop_light(if p.orders_paused || snoozing {
        1
    } else {
        match p.light {
            ShopLight::Open => 0,
            ShopLight::OpensLater => 1,
            ShopLight::Closed => 2,
        }
    });
    let mut short = if p.orders_paused {
        "Pausiert".to_string()
    } else if snoozing {
        format!("Pause bis {}", p.snooze_until)
    } else {
        match p.light {
            ShopLight::Open if forced => "Offen (manuell)".to_string(),
            ShopLight::Open => "Offen".to_string(),
            ShopLight::OpensLater => "Noch zu".to_string(),
            ShopLight::Closed => "Geschlossen".to_string(),
        }
    };
    if p.delivery_paused {
        short.push_str(" · nur Abholung");
    }
    w.set_shop_short(short.into());
    // Same wording as the /admin/hours status line.
    w.set_shop_status(
        if p.orders_paused {
            "Online-Bestellungen sind PAUSIERT".to_string()
        } else if snoozing {
            format!("Pausiert — automatisch wieder offen ab {} Uhr", p.snooze_until)
        } else if forced {
            format!("Manuell GEÖFFNET (Sonderöffnung bis {} Uhr)", p.force_open_until)
        } else if p.light == ShopLight::Open {
            "Online-Bestellungen laufen (geöffnet)".to_string()
        } else {
            p.reason.clone()
        }
        .into(),
    );
    w.set_shop_is_open(p.light == ShopLight::Open);
    w.set_delivery_paused(p.delivery_paused);
    w.set_overridden(p.orders_paused || snoozing || forced);
    w.set_gate_open(p.gate_open);
    w.set_today_text(
        if p.today_close.is_empty() {
            "Heute kein regulärer Betrieb".to_string()
        } else {
            format!(
                "Heute {}–{} Uhr{}",
                p.today_open,
                p.today_close,
                if p.today_extended { " (angepasst)" } else { "" }
            )
        }
        .into(),
    );
}

/// Current stepper values: what the user dialled in, else today's hours.
fn picks(c: &Core) -> (String, String) {
    let today = |f: fn(&ShopPanel) -> &String, dflt: &str| {
        c.shop
            .as_ref()
            .map(f)
            .filter(|t| !t.is_empty())
            .cloned()
            .unwrap_or_else(|| dflt.to_string())
    };
    (
        c.pick_open.clone().unwrap_or_else(|| today(|p| &p.today_open, "16:00")),
        c.pick_close.clone().unwrap_or_else(|| today(|p| &p.today_close, "22:00")),
    )
}

/// The page's buttons as [`ShopAction`]s. Codes (from kitchen.slint):
/// 0 open/close (arg 1/0) · 1 back to plan · 2 pause `arg` min ·
/// 3 delivery off/on (arg 1/0) · 4 open `arg` min earlier · 5 close `arg`
/// min later · 6 open from the "ab" stepper · 7 close at the "bis" stepper.
/// Relative bumps mirror /admin/hours (base 16:00 / 22:00 on a day
/// without hours); the server validates everything.
fn shop_action(c: &mut Core, code: i32, arg: i32) -> Option<ShopAction> {
    let today_open = c.shop.as_ref().map(|p| p.today_open.clone()).unwrap_or_default();
    let today_close = c.shop.as_ref().map(|p| p.today_close.clone()).unwrap_or_default();
    Some(match code {
        0 => ShopAction::SetOpen(arg == 1),
        1 => ShopAction::ResetToSchedule,
        2 => ShopAction::Snooze {
            minutes: arg.clamp(0, 24 * 60) as u16,
        },
        3 => ShopAction::SetDeliveryPaused(arg == 1),
        4 => ShopAction::SetTodayHours {
            open: Some(min_to_hhmm(hhmm_to_min(&today_open, 16) - arg as i64)),
            close: None,
        },
        5 => ShopAction::SetTodayHours {
            open: None,
            close: Some(min_to_hhmm(hhmm_to_min(&today_close, 22) + arg as i64)),
        },
        6 => {
            let (open, _) = picks(c);
            c.pick_open = None;
            ShopAction::SetTodayHours {
                open: Some(open),
                close: None,
            }
        }
        7 => {
            let (_, close) = picks(c);
            c.pick_close = None;
            ShopAction::SetTodayHours {
                open: None,
                close: Some(close),
            }
        }
        _ => return None,
    })
}

/// "HH:MM" → minutes since midnight (`default_h`:00 when unparseable).
fn hhmm_to_min(s: &str, default_h: i64) -> i64 {
    match s.split_once(':') {
        Some((h, m)) => {
            h.trim().parse::<i64>().unwrap_or(default_h) * 60 + m.trim().parse::<i64>().unwrap_or(0)
        }
        None => default_h * 60,
    }
}

fn min_to_hhmm(total: i64) -> String {
    let t = total.clamp(0, 23 * 60 + 59);
    format!("{:02}:{:02}", t / 60, t % 60)
}

/// (primary, back) button for an order in `status`.
fn actions(status: BoardStatus, delivery: bool) -> (Action, Action) {
    use BoardStatus::*;
    match status {
        Received => (Some(("Starten", Preparing)), None),
        Preparing => (Some(("Fertig", Ready)), Some(("Zurück zu Neu", Received))),
        Ready if delivery => (
            Some(("Fahrer unterwegs", OutForDelivery)),
            Some(("Zurück in Arbeit", Preparing)),
        ),
        Ready => (
            Some(("Abgeholt", PickedUp)),
            Some(("Zurück in Arbeit", Preparing)),
        ),
        PickedUp | OutForDelivery | Cancelled => (None, None),
    }
}

struct Timing {
    /// "19:30" (pre-order) or "sofort".
    when: String,
    /// Card's right-hand figure: "12 min", "in 25 min", "fällig".
    age: String,
    urgency: i32,
    scheduled: bool,
    waited_min: i64,
}

/// Waiting time for ASAP orders, time-to-due for pre-orders. The due label
/// is shop-local "HH:MM" (or "DD.MM. HH:MM" for another day, shown as-is);
/// compared against the local clock, so the screen runs in the shop's TZ.
fn timing(o: &OrderForKitchen, now: i64) -> Timing {
    let waited_min = (now - o.created_at_unix).max(0) / 60;
    match o.pickup_time_label.as_deref().filter(|l| !l.is_empty()) {
        Some(label) => {
            let (age, urgency) = match NaiveTime::parse_from_str(label, "%H:%M") {
                Ok(due) => {
                    let mins = (due.num_seconds_from_midnight() as i64
                        - Local::now().time().num_seconds_from_midnight() as i64)
                        / 60;
                    if mins <= 0 {
                        ("fällig".to_string(), 2)
                    } else {
                        (format!("in {mins} min"), if mins <= 15 { 1 } else { 0 })
                    }
                }
                Err(_) => (String::new(), 0),
            };
            Timing {
                when: label.to_string(),
                age,
                urgency,
                scheduled: true,
                waited_min,
            }
        }
        None => Timing {
            when: "sofort".to_string(),
            age: format!("{waited_min} min"),
            urgency: if waited_min >= 35 {
                2
            } else if waited_min >= 20 {
                1
            } else {
                0
            },
            scheduled: false,
            waited_min,
        },
    }
}

/// The big number on the Bon: the last segment of "DP-1105-0012".
fn short_number(o: &OrderForKitchen) -> String {
    o.display_label
        .rsplit('-')
        .next()
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| o.display_number.to_string())
}

fn is_delivery(o: &OrderForKitchen) -> bool {
    matches!(o.channel, OrderChannel::Delivery { .. })
}

fn eur(cents: u32) -> String {
    format!("{},{:02} €", cents / 100, cents % 100)
}

/// Unix seconds now (browser-safe).
pub fn local_now() -> i64 {
    web_time::SystemTime::now()
        .duration_since(web_time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Index ↔ status for the Slint side (it has no enum of ours).
fn status_index(s: BoardStatus) -> i32 {
    match s {
        BoardStatus::Received => 0,
        BoardStatus::Preparing => 1,
        BoardStatus::Ready => 2,
        BoardStatus::PickedUp => 3,
        BoardStatus::OutForDelivery => 4,
        BoardStatus::Cancelled => 5,
    }
}

fn status_from_index(i: i32) -> Option<BoardStatus> {
    Some(match i {
        0 => BoardStatus::Received,
        1 => BoardStatus::Preparing,
        2 => BoardStatus::Ready,
        3 => BoardStatus::PickedUp,
        4 => BoardStatus::OutForDelivery,
        5 => BoardStatus::Cancelled,
        _ => return None,
    })
}
