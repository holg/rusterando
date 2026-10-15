//! Bridge between the kitchen-channel client and an optional display UI
//! (the `kds` feature). Plain data + a channel, no UI dependencies, so the
//! client compiles identically for the headless Pi build.

use kitchen_protocol::{BoardSnapshot, ClientMessage, ShopPanel};
use tokio::sync::mpsc;

/// Client → UI notifications. Sent from the network thread; the UI side
/// marshals them onto its own event loop.
#[derive(Debug, Clone)]
// Only the display build reads the payloads.
#[cfg_attr(not(feature = "kds"), allow(dead_code))]
pub enum UiEvent {
    /// Kitchen-channel session up (`true`) or down.
    Connected(bool),
    /// Printer state, sent on change: `Ok(device)` or `Err(reason)`.
    Printer(Result<String, String>),
    /// Receipts received but not printed yet (printer failing, retrying).
    PendingPrints(usize),
    /// Fresh board from the server.
    Board(BoardSnapshot),
    /// Shop open/closed state + today's hours ("Öffnungszeiten" page).
    Shop(ShopPanel),
    /// Admin alert: flash + gong now, with this title and text.
    Alert { title: String, text: String },
}

pub struct UiLink {
    pub events: Box<dyn Fn(UiEvent) + Send + Sync>,
    /// Messages from the touch UI (`BoardAdvance`, `Shop`), forwarded to the server
    /// while a session is up. Anything queued while offline is dropped at
    /// the next connect — the UI greys its buttons out then anyway.
    pub commands: mpsc::UnboundedReceiver<ClientMessage>,
}

impl UiLink {
    pub fn emit(&self, ev: UiEvent) {
        (self.events)(ev)
    }
}

/// `emit` on an optional link.
pub fn emit(ui: &Option<UiLink>, ev: UiEvent) {
    if let Some(ui) = ui {
        ui.emit(ev);
    }
}
