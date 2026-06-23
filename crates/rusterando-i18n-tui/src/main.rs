//! i18n-tui — manage translations over SSH, iPhone-optimised.
//!
//! Run on the VPS (or anywhere with the tenant DBs + the built pack wasm):
//!   i18n-tui [--data <dir>] [--strings <dir>] [--pack <wasm>] [--compare <sqlite>]
//! Defaults: --data ./data  --strings ./rusterando-scrape
//!           --pack <newest target/site/pkg/i18n/*_bg.wasm>
//!
//! Per (phrase, locale) it shows THREE sources with provenance, colour-coded:
//!   #1 DB     — the tenant's `_<lang>` override cell        (green)
//!   #2 wasm   — the built translation pack, executed live   (cyan)
//!   #3 cmp    — an optional compare translations SQLite      (yellow)
//! The "preferred" value is DB-first (DB → wasm → compare). Filter by source
//! state, edit (writes the DB override), and export the filtered set to a
//! translations SQLite for merging into another build.
//!
//! iPhone/SSH-optimised: single-column narrow layout, single-key actions (no
//! modifier chords the soft keyboard hides), resilient to small terminals, clean
//! terminal restore on exit OR panic.

mod compare;
mod data;
mod packwasm;
mod wasmsrc;

use anyhow::Result;
use compare::CompareDb;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use data::{Phrase, LOCALES};
use packwasm::PackWasm;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};
use rusqlite::Connection;
use std::path::PathBuf;

fn main() -> Result<()> {
    let mut data_dir = PathBuf::from("./data");
    let mut strings_dir = PathBuf::from("./rusterando-scrape");
    let mut pack_path: Option<PathBuf> = None;
    let mut compare_path: Option<PathBuf> = None;
    let mut merge_path: Option<PathBuf> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--data" => data_dir = PathBuf::from(args.next().unwrap_or_default()),
            "--strings" => strings_dir = PathBuf::from(args.next().unwrap_or_default()),
            "--pack" => pack_path = args.next().map(PathBuf::from),
            "--compare" => compare_path = args.next().map(PathBuf::from),
            "--merge" => merge_path = args.next().map(PathBuf::from),
            "-h" | "--help" => {
                println!(
                    "i18n-tui [--data <dir>] [--pack <wasm>] [--compare <sqlite>] [--merge <sqlite>]\n\
                     \n\
                     Tenant flow: per (phrase,locale) shows DB / executed-wasm / compare; edit DB override; export.\n\
                     Wasm-source: from the tenant picker, 'Wasm-Quelle bearbeiten' → edit menu-translations.sqlite\n\
                     (the canonical wasm source), filter by profile/locale, edit, rebuild (build-i18n-pack.sh).\n\
                     --merge <sqlite> merges another translations sqlite into the wasm source at startup."
                );
                return Ok(());
            }
            _ => {}
        }
    }

    // The wasm (source #2) supplies BOTH the gap membership and the live value,
    // so the menu_strings JSON coverage is no longer needed here (it was only a
    // membership set). `--strings` is accepted for compatibility but unused.
    let _ = &strings_dir;
    let dbs = data::list_tenant_dbs(&data_dir)?;
    if dbs.is_empty() {
        eprintln!(
            "no tenant DBs (*.sqlite / *.db) under {}",
            data_dir.display()
        );
        return Ok(());
    }

    // Source #2: the built pack wasm. Default to the newest in target/site/pkg.
    let pack_path = pack_path.or_else(|| newest_pack_wasm());
    let pack = match &pack_path {
        Some(p) => match PackWasm::load(p) {
            Ok(w) => Some(w),
            Err(e) => {
                eprintln!("pack wasm load failed ({e}) — #2 column will be blank");
                None
            }
        },
        None => None,
    };

    // Source #3: optional compare SQLite.
    let cmp = match &compare_path {
        Some(p) => CompareDb::load(p)?,
        None => CompareDb::default(),
    };

    // Custom enter/leave so we can ALSO capture mouse (taps) — ratatui::init()
    // doesn't. A panic hook restores the terminal first (critical over SSH: you
    // can't easily un-wedge a phone terminal).
    let mut app = App::new(dbs, pack, cmp);
    // Optional one-shot merge of another translations sqlite into the wasm
    // source (the "merge wasms" path) — applied before the UI starts.
    if let (Some(mp), Some(w)) = (&merge_path, app.wsrc.as_mut()) {
        match w.merge_from(mp) {
            Ok(n) => app.status = format!("merged {n} Einträge aus {}", mp.display()),
            Err(e) => app.status = format!("merge fehlgeschlagen: {e}"),
        }
    }

    let mut terminal = tui_enter()?;
    let res = app.run(&mut terminal);
    tui_leave();
    res
}

/// Enter the alternate screen with raw mode + mouse capture, and install a
/// panic hook that restores the terminal before printing the panic.
fn tui_enter() -> Result<ratatui::DefaultTerminal> {
    use crossterm::{event::EnableMouseCapture, terminal::EnterAlternateScreen};
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        tui_leave();
        default_hook(info);
    }));
    crossterm::terminal::enable_raw_mode()?;
    crossterm::execute!(std::io::stdout(), EnterAlternateScreen, EnableMouseCapture)?;
    let backend = ratatui::backend::CrosstermBackend::new(std::io::stdout());
    Ok(ratatui::Terminal::new(backend)?)
}

/// Best-effort restore (also called from the panic hook). Idempotent.
fn tui_leave() {
    use crossterm::{event::DisableMouseCapture, terminal::LeaveAlternateScreen};
    let _ = crossterm::execute!(std::io::stdout(), LeaveAlternateScreen, DisableMouseCapture);
    let _ = crossterm::terminal::disable_raw_mode();
}

/// Newest `*_bg.wasm` under target/site/pkg/i18n (the default pack source).
fn newest_pack_wasm() -> Option<PathBuf> {
    let dir = std::path::Path::new("target/site/pkg/i18n");
    let mut best: Option<(std::time::SystemTime, PathBuf)> = None;
    for e in std::fs::read_dir(dir).ok()?.flatten() {
        let p = e.path();
        if p.file_name()
            .and_then(|n| n.to_str())
            .map(|n| n.ends_with("_bg.wasm"))
            .unwrap_or(false)
        {
            if let Ok(m) = e.metadata().and_then(|m| m.modified()) {
                if best.as_ref().map(|(t, _)| m > *t).unwrap_or(true) {
                    best = Some((m, p));
                }
            }
        }
    }
    best.map(|(_, p)| p)
}

enum Screen {
    Tenants,
    Rows,
    Edit,
    /// Wasm-SOURCE browser: the translations.sqlite the pack is built from,
    /// grouped/filtered by build profile.
    WasmSrc,
    /// Edit one wasm-source translation (writes the source sqlite).
    WasmEdit,
}

/// Which source a value comes from — drives the colour + provenance label.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Origin {
    Db,
    Wasm,
    Compare,
    None,
}

/// Filter over the row set, by source state.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Filter {
    All,
    InDb,       // has a DB override
    OnlyWasm,   // DB empty, wasm has it
    MissingAll, // no DB, no wasm, no compare
    Differs,    // sources disagree
}

impl Filter {
    fn label(self) -> &'static str {
        match self {
            Filter::All => "alle",
            Filter::InDb => "in DB",
            Filter::OnlyWasm => "nur wasm",
            Filter::MissingAll => "fehlt überall",
            Filter::Differs => "weicht ab",
        }
    }
    fn next(self) -> Self {
        match self {
            Filter::All => Filter::InDb,
            Filter::InDb => Filter::OnlyWasm,
            Filter::OnlyWasm => Filter::MissingAll,
            Filter::MissingAll => Filter::Differs,
            Filter::Differs => Filter::All,
        }
    }
}

/// One locale's resolved sources for a phrase (a table cell).
#[derive(Clone, Default)]
struct Cell {
    db: Option<String>,
    wasm: Option<String>,
    cmp: Option<String>,
}

impl Cell {
    /// DB-first preferred value + where it's from.
    fn preferred(&self) -> (Origin, &str) {
        if let Some(v) = &self.db {
            (Origin::Db, v)
        } else if let Some(v) = &self.wasm {
            (Origin::Wasm, v)
        } else if let Some(v) = &self.cmp {
            (Origin::Compare, v)
        } else {
            (Origin::None, "")
        }
    }
}

/// ONE row per German phrase; one [`Cell`] per locale (the multilingual table).
#[derive(Clone)]
struct Row {
    phrase: Phrase,
    cells: Vec<Cell>, // len == LOCALES.len(), index-aligned with LOCALES
}

impl Row {
    /// Does ANY locale cell match the filter? (a phrase row is shown if at least
    /// one of its locale cells is in the filtered state).
    fn matches(&self, f: Filter) -> bool {
        if matches!(f, Filter::All) {
            return true;
        }
        self.cells.iter().any(|c| match f {
            Filter::All => true,
            Filter::InDb => c.db.is_some(),
            Filter::OnlyWasm => c.db.is_none() && c.wasm.is_some(),
            Filter::MissingAll => c.db.is_none() && c.wasm.is_none() && c.cmp.is_none(),
            Filter::Differs => {
                let vals: Vec<&str> = [c.db.as_deref(), c.wasm.as_deref(), c.cmp.as_deref()]
                    .into_iter()
                    .flatten()
                    .collect();
                vals.windows(2).any(|w| w[0] != w[1])
            }
        })
    }
}

struct App {
    dbs: Vec<PathBuf>,
    pack: Option<PackWasm>,
    cmp: CompareDb,
    screen: Screen,
    tenant_sel: ListState,
    conn: Option<Connection>,
    tenant_name: String,
    all_rows: Vec<Row>, // unfiltered
    view: Vec<usize>,   // indices into all_rows after filter
    sel: ListState,
    col: usize,  // selected locale COLUMN (index into LOCALES) — the edit target
    hcol: usize, // leftmost visible locale column (horizontal scroll)
    filter: Filter,
    input: String,
    status: String,
    quit: bool,
    // Big action-menu overlay (popup). When open, it captures input; actions
    // are picked by number key or tap. `menu_hits` maps screen rects → action
    // for tap hit-testing (rebuilt each draw).
    overlay: bool,
    menu_hits: Vec<(Rect, Action)>,
    // Tap hit-testing for the main list rows (rebuilt each draw): (rect, view_i).
    row_hits: Vec<(Rect, usize)>,
    // The bottom menu-bar rect (tapping it opens the overlay).
    menu_bar: Rect,
    // ---- wasm-source mode ----
    wsrc: Option<wasmsrc::WasmSrc>,
    wsrc_profile: usize, // index into wasmsrc::PROFILES
    wsrc_locale: usize,  // index into the current profile's locales (filter)
    wview: Vec<usize>,   // indices into wsrc.rows after profile+locale filter
    wsel: ListState,
    wsrc_hits: Vec<(Rect, usize)>, // tap rects → wview index
}

/// A menu action. The overlay lists the ones valid for the current screen.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Action {
    Open,   // tenants: open selected
    Edit,   // rows: edit selected
    Delete, // rows/wasmsrc: delete the selected cell's value
    Filter, // rows: cycle filter
    Export, // rows: export filtered → sqlite
    Back,   // leave current screen
    Save,   // edit: save
    Cancel, // edit: cancel
    Quit,
    // wasm-source mode
    WasmSrc,     // enter the wasm-source browser
    NextProfile, // cycle build profile
    NextLocale,  // cycle locale within profile
    Rebuild,     // rebuild the wasm from source
}

impl Action {
    fn label(self) -> &'static str {
        match self {
            Action::Open => "Öffnen",
            Action::Edit => "Bearbeiten",
            Action::Delete => "Eintrag löschen",
            Action::Filter => "Filter wechseln",
            Action::Export => "Export → SQLite",
            Action::Back => "Zurück",
            Action::Save => "Speichern",
            Action::Cancel => "Abbrechen",
            Action::Quit => "Beenden",
            Action::WasmSrc => "Wasm-Quelle bearbeiten",
            Action::NextProfile => "Profil wechseln",
            Action::NextLocale => "Sprache wechseln",
            Action::Rebuild => "Wasm neu bauen",
        }
    }
}

impl App {
    fn new(dbs: Vec<PathBuf>, pack: Option<PackWasm>, cmp: CompareDb) -> Self {
        let mut tenant_sel = ListState::default();
        tenant_sel.select(Some(0));
        Self {
            dbs,
            pack,
            cmp,
            screen: Screen::Tenants,
            tenant_sel,
            conn: None,
            tenant_name: String::new(),
            all_rows: Vec::new(),
            view: Vec::new(),
            sel: ListState::default(),
            col: 0,
            hcol: 0,
            filter: Filter::All,
            input: String::new(),
            status: String::new(),
            quit: false,
            overlay: false,
            menu_hits: Vec::new(),
            row_hits: Vec::new(),
            menu_bar: Rect::default(),
            wsrc: wasmsrc::WasmSrc::open(std::path::Path::new(
                "rusterando-scrape/menu-translations.sqlite",
            ))
            .ok(),
            wsrc_profile: 0,
            wsrc_locale: 0,
            wview: Vec::new(),
            wsel: ListState::default(),
            wsrc_hits: Vec::new(),
        }
    }

    // ---- wasm-source mode --------------------------------------------------

    fn cur_profile(&self) -> &'static wasmsrc::Profile {
        &wasmsrc::PROFILES[self.wsrc_profile % wasmsrc::PROFILES.len()]
    }

    /// The locales in the active profile (the table's columns).
    fn wprofile_locales(&self) -> Vec<&'static str> {
        self.cur_profile().locales.to_vec()
    }

    /// The currently selected locale COLUMN within the active profile.
    fn cur_wlocale(&self) -> &'static str {
        let locs = self.wprofile_locales();
        locs[self.wsrc_locale % locs.len()]
    }

    fn enter_wasmsrc(&mut self) {
        if self.wsrc.is_some() {
            self.wsrc_profile = 0;
            self.wsrc_locale = 0;
            self.rebuild_wview();
            self.screen = Screen::WasmSrc;
        } else {
            self.status = "keine menu-translations.sqlite gefunden".into();
        }
    }

    /// The wasm-source value for (german, locale, topic=menu), if any.
    fn wsrc_value(&self, german: &str, locale: &str) -> Option<String> {
        let w = self.wsrc.as_ref()?;
        w.rows
            .iter()
            .find(|r| r.german == german && r.locale == locale && r.topic == "menu")
            .map(|r| r.value.clone())
    }

    /// Distinct German phrases in the active profile's `menu` topic (the table's
    /// rows). `wview` holds these german strings' indices into `wgermans`.
    fn wgermans(&self) -> Vec<String> {
        let Some(w) = &self.wsrc else {
            return Vec::new();
        };
        let mut seen = std::collections::BTreeSet::new();
        let mut out = Vec::new();
        for r in &w.rows {
            if r.topic == "menu" && seen.insert(r.german.clone()) {
                out.push(r.german.clone());
            }
        }
        out
    }

    /// Rebuild the distinct-german row index for the wasm-source table.
    fn rebuild_wview(&mut self) {
        let germans = self.wgermans();
        self.wview = (0..germans.len()).collect();
        self.wsel
            .select(if self.wview.is_empty() { None } else { Some(0) });
        // keep selected locale column in range for the active profile
        let nloc = self.wprofile_locales().len();
        if self.wsrc_locale >= nloc {
            self.wsrc_locale = 0;
        }
    }

    fn wsrc_move(&mut self, delta: i32) {
        let n = self.wview.len() as i32;
        if n == 0 {
            return;
        }
        let cur = self.wsel.selected().unwrap_or(0) as i32;
        self.wsel.select(Some((cur + delta).rem_euclid(n) as usize));
    }

    /// The German phrase of the selected wasm-source row.
    fn cur_wgerman(&self) -> Option<String> {
        let vi = self.wsel.selected()?;
        let i = *self.wview.get(vi)?;
        self.wgermans().into_iter().nth(i)
    }

    fn begin_wedit(&mut self) {
        let loc = self.cur_wlocale();
        if let Some(g) = self.cur_wgerman() {
            self.input = self.wsrc_value(&g, loc).unwrap_or_default();
            self.status.clear();
            self.screen = Screen::WasmEdit;
        }
    }

    fn save_wedit(&mut self) -> Result<()> {
        let Some(g) = self.cur_wgerman() else {
            return Ok(());
        };
        let loc = self.cur_wlocale();
        if let Some(w) = self.wsrc.as_mut() {
            w.set(loc, &g, "menu", &self.input)?;
        }
        self.status = format!("Quelle gespeichert: {loc} menu");
        self.rebuild_wview();
        self.screen = Screen::WasmSrc;
        Ok(())
    }

    fn wsrc_rebuild(&mut self) -> Result<()> {
        let prof = self.cur_profile().name;
        match wasmsrc::rebuild(prof) {
            Ok(msg) => self.status = format!("rebuilt [{prof}]: {msg}"),
            Err(e) => self.status = format!("rebuild fehlgeschlagen: {e}"),
        }
        Ok(())
    }

    /// The actions the overlay offers on the current screen (numbered 1..).
    fn actions(&self) -> Vec<Action> {
        match self.screen {
            Screen::Tenants => vec![Action::Open, Action::WasmSrc, Action::Quit],
            Screen::Rows => vec![
                Action::Edit,
                Action::Delete,
                Action::Filter,
                Action::Export,
                Action::Back,
                Action::Quit,
            ],
            Screen::Edit => vec![Action::Save, Action::Cancel],
            Screen::WasmSrc => vec![
                Action::Edit,
                Action::Delete,
                Action::NextProfile,
                Action::NextLocale,
                Action::Rebuild,
                Action::Back,
                Action::Quit,
            ],
            Screen::WasmEdit => vec![Action::Save, Action::Cancel],
        }
    }

    /// Run a menu action (from a number key or a tap).
    fn run_action(&mut self, a: Action) -> Result<()> {
        self.overlay = false;
        match a {
            Action::Open => self.open_selected_tenant()?,
            Action::Edit => match self.screen {
                Screen::WasmSrc => self.begin_wedit(),
                _ => self.begin_edit(),
            },
            Action::Delete => match self.screen {
                Screen::WasmSrc => self.delete_wcell()?,
                _ => self.delete_cell()?,
            },
            Action::Filter => {
                self.filter = self.filter.next();
                self.apply_filter();
            }
            Action::Export => self.export()?,
            Action::Back => self.go_back(),
            Action::Save => match self.screen {
                Screen::WasmEdit => self.save_wedit()?,
                _ => self.save_edit()?,
            },
            Action::Cancel => {
                self.screen = match self.screen {
                    Screen::WasmEdit => Screen::WasmSrc,
                    _ => Screen::Rows,
                }
            }
            Action::Quit => self.quit = true,
            Action::WasmSrc => self.enter_wasmsrc(),
            Action::NextProfile => {
                self.wsrc_profile = (self.wsrc_profile + 1) % wasmsrc::PROFILES.len();
                self.wsrc_locale = 0;
                self.rebuild_wview();
            }
            Action::NextLocale => {
                let n = self.cur_profile().locales.len();
                self.wsrc_locale = (self.wsrc_locale + 1) % n;
                self.rebuild_wview();
            }
            Action::Rebuild => self.wsrc_rebuild()?,
        }
        Ok(())
    }

    /// Delete the selected tenant cell's DB OVERRIDE (clears `<base>_<locale>`).
    /// The pack/German fallback then applies — this removes only the per-shop
    /// override, never the origin German.
    fn delete_cell(&mut self) -> Result<()> {
        let Some(&i) = self.sel.selected().and_then(|vi| self.view.get(vi)) else {
            return Ok(());
        };
        let col = self.col;
        let locale = LOCALES[col];
        let phrase = self.all_rows[i].phrase.clone();
        let conn = self.conn.as_ref().expect("tenant open");
        let n = data::set_override(conn, &phrase, locale, "")?; // "" deletes
        if let Some(c) = self.all_rows[i].cells.get_mut(col) {
            c.db = None;
        }
        self.status = format!(
            "DB-Override gelöscht: {}_{} ({n} Zeilen)",
            phrase.field.base(),
            locale
        );
        self.apply_filter();
        Ok(())
    }

    /// Delete the selected wasm-source entry (the row for german × selected
    /// locale, topic=menu). The wasm then won't carry it → German shows.
    fn delete_wcell(&mut self) -> Result<()> {
        let Some(g) = self.cur_wgerman() else {
            return Ok(());
        };
        let loc = self.cur_wlocale();
        if let Some(w) = self.wsrc.as_mut() {
            w.set(loc, &g, "menu", "")?; // "" deletes the source row
        }
        self.status = format!("Quelle gelöscht: {loc} · {}", truncate(&g, 24));
        self.rebuild_wview();
        Ok(())
    }

    fn begin_edit(&mut self) {
        let col = self.col;
        if let Some(r) = self.current_row() {
            if let Some(c) = r.cells.get(col) {
                // Pre-fill with the DB override else the wasm value for the
                // SELECTED locale column.
                self.input = c.db.clone().or_else(|| c.wasm.clone()).unwrap_or_default();
                self.status.clear();
                self.screen = Screen::Edit;
            }
        }
    }

    fn go_back(&mut self) {
        match self.screen {
            Screen::Rows => {
                self.conn = None;
                self.all_rows.clear();
                self.view.clear();
                self.screen = Screen::Tenants;
            }
            Screen::Edit => self.screen = Screen::Rows,
            Screen::WasmSrc => self.screen = Screen::Tenants,
            Screen::WasmEdit => self.screen = Screen::WasmSrc,
            Screen::Tenants => self.quit = true,
        }
    }

    fn run(&mut self, terminal: &mut ratatui::DefaultTerminal) -> Result<()> {
        while !self.quit {
            terminal.draw(|f| self.draw(f))?;
            match event::read()? {
                Event::Key(k) if k.kind == KeyEventKind::Press => self.on_key(k.code)?,
                Event::Mouse(m) => self.on_mouse(m)?,
                _ => {}
            }
        }
        Ok(())
    }

    /// Unified key handling: the overlay (when open) intercepts; otherwise the
    /// per-screen keys. `m`/space toggles the overlay on the list screens.
    fn on_key(&mut self, code: KeyCode) -> Result<()> {
        if self.overlay {
            let acts = self.actions();
            match code {
                KeyCode::Esc | KeyCode::Char('m') | KeyCode::Char(' ') => self.overlay = false,
                KeyCode::Char(c @ '1'..='9') => {
                    let idx = (c as u8 - b'1') as usize;
                    if let Some(&a) = acts.get(idx) {
                        self.run_action(a)?;
                    }
                }
                _ => {}
            }
            return Ok(());
        }
        // Edit screens never show the overlay-toggle (typing needs the keys).
        let editing = matches!(self.screen, Screen::Edit | Screen::WasmEdit);
        if !editing && matches!(code, KeyCode::Char('m') | KeyCode::Char(' ')) {
            self.overlay = true;
            return Ok(());
        }
        match self.screen {
            Screen::Tenants => self.on_tenants_key(code)?,
            Screen::Rows => self.on_rows_key(code)?,
            Screen::Edit => self.on_edit_key(code)?,
            Screen::WasmSrc => self.on_wasmsrc_key(code)?,
            Screen::WasmEdit => self.on_wedit_key(code)?,
        }
        Ok(())
    }

    fn on_wasmsrc_key(&mut self, code: KeyCode) -> Result<()> {
        match code {
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Char('b') | KeyCode::Esc => self.go_back(),
            KeyCode::Char('j') | KeyCode::Down => self.wsrc_move(1),
            KeyCode::Char('k') | KeyCode::Up => self.wsrc_move(-1),
            KeyCode::Char('p') => {
                self.wsrc_profile = (self.wsrc_profile + 1) % wasmsrc::PROFILES.len();
                self.wsrc_locale = 0;
                self.rebuild_wview();
            }
            KeyCode::Char('l') | KeyCode::Right => {
                let n = self.cur_profile().locales.len();
                self.wsrc_locale = (self.wsrc_locale + 1) % n;
            }
            KeyCode::Char('h') | KeyCode::Left => {
                let n = self.cur_profile().locales.len();
                self.wsrc_locale = (self.wsrc_locale + n - 1) % n;
            }
            KeyCode::Char('r') => self.wsrc_rebuild()?,
            KeyCode::Char('e') | KeyCode::Enter => self.begin_wedit(),
            KeyCode::Char('d') | KeyCode::Delete => self.delete_wcell()?,
            _ => {}
        }
        Ok(())
    }

    fn on_wedit_key(&mut self, code: KeyCode) -> Result<()> {
        match code {
            KeyCode::Esc => self.screen = Screen::WasmSrc,
            KeyCode::Enter => self.save_wedit()?,
            KeyCode::Backspace => {
                self.input.pop();
            }
            KeyCode::Char(c) => self.input.push(c),
            _ => {}
        }
        Ok(())
    }

    /// Tap handling. Overlay open → tap a button. Else: tap a list row to select
    /// (second tap on the same row = activate), tap the FAB to open the overlay.
    fn on_mouse(&mut self, m: crossterm::event::MouseEvent) -> Result<()> {
        use crossterm::event::{MouseButton, MouseEventKind};
        // Scroll wheel moves the selection on list screens.
        match m.kind {
            MouseEventKind::ScrollDown => {
                match self.screen {
                    Screen::Rows => self.row_move(1),
                    Screen::Tenants => self.tenant_move(1),
                    Screen::WasmSrc => self.wsrc_move(1),
                    _ => {}
                }
                return Ok(());
            }
            MouseEventKind::ScrollUp => {
                match self.screen {
                    Screen::Rows => self.row_move(-1),
                    Screen::Tenants => self.tenant_move(-1),
                    Screen::WasmSrc => self.wsrc_move(-1),
                    _ => {}
                }
                return Ok(());
            }
            MouseEventKind::Down(MouseButton::Left) => {}
            _ => return Ok(()),
        }
        let (x, y) = (m.column, m.row);
        if self.overlay {
            // Hit-test the overlay buttons.
            if let Some(&(_, a)) = self.menu_hits.iter().find(|(r, _)| hit(*r, x, y)) {
                self.run_action(a)?;
            } else {
                self.overlay = false; // tap outside closes
            }
            return Ok(());
        }
        // Tap the bottom menu bar → open the overlay.
        if hit(self.menu_bar, x, y) {
            self.overlay = true;
            return Ok(());
        }
        // Tap a list row → select it; a second tap on the already-selected row
        // activates (open tenant / edit row).
        if let Some(&(_, vi)) = self.row_hits.iter().find(|(r, _)| hit(*r, x, y)) {
            match self.screen {
                Screen::Tenants => {
                    let was = self.tenant_sel.selected() == Some(vi);
                    self.tenant_sel.select(Some(vi));
                    if was {
                        self.open_selected_tenant()?;
                    }
                }
                Screen::Rows => {
                    let was = self.sel.selected() == Some(vi);
                    self.sel.select(Some(vi));
                    if was {
                        self.begin_edit();
                    }
                }
                _ => {}
            }
        }
        // Wasm-source list taps.
        if matches!(self.screen, Screen::WasmSrc) {
            if let Some(&(_, vi)) = self.wsrc_hits.iter().find(|(r, _)| hit(*r, x, y)) {
                let was = self.wsel.selected() == Some(vi);
                self.wsel.select(Some(vi));
                if was {
                    self.begin_wedit();
                }
            }
        }
        Ok(())
    }

    // ---- tenant picker -----------------------------------------------------

    fn on_tenants_key(&mut self, code: KeyCode) -> Result<()> {
        match code {
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Char('j') | KeyCode::Down => self.tenant_move(1),
            KeyCode::Char('k') | KeyCode::Up => self.tenant_move(-1),
            KeyCode::Char('w') | KeyCode::Char('W') => self.enter_wasmsrc(),
            KeyCode::Enter => self.open_selected_tenant()?,
            _ => {}
        }
        Ok(())
    }

    fn tenant_move(&mut self, delta: i32) {
        let n = self.dbs.len() as i32;
        if n == 0 {
            return;
        }
        let cur = self.tenant_sel.selected().unwrap_or(0) as i32;
        self.tenant_sel
            .select(Some((cur + delta).rem_euclid(n) as usize));
    }

    fn open_selected_tenant(&mut self) -> Result<()> {
        let i = self.tenant_sel.selected().unwrap_or(0);
        let path = self.dbs[i].clone();
        self.tenant_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("?")
            .to_string();
        self.conn = Some(data::open(&path)?);
        self.build_all_rows()?;
        self.apply_filter();
        self.screen = Screen::Rows;
        Ok(())
    }

    // ---- rows --------------------------------------------------------------

    /// Build ONE row per German phrase, each carrying a [`Cell`] per locale
    /// (origin + translations across all 7 languages in a single row).
    fn build_all_rows(&mut self) -> Result<()> {
        let conn = self.conn.as_ref().expect("tenant open");
        let phrases = data::shop_phrases(conn)?;
        let mut rows = Vec::with_capacity(phrases.len());
        for p in &phrases {
            let mut cells = Vec::with_capacity(LOCALES.len());
            for &locale in LOCALES {
                let db = data::current_override(conn, p, locale)?;
                let wasm = match &mut self.pack {
                    Some(w) => w.menu_lookup(locale, &p.german).unwrap_or(None),
                    None => None,
                };
                let cmp = self.cmp.get(locale, &p.german).map(|s| s.to_string());
                cells.push(Cell { db, wasm, cmp });
            }
            rows.push(Row {
                phrase: p.clone(),
                cells,
            });
        }
        self.all_rows = rows;
        Ok(())
    }

    fn apply_filter(&mut self) {
        let f = self.filter;
        self.view = self
            .all_rows
            .iter()
            .enumerate()
            .filter(|(_, r)| r.matches(f))
            .map(|(i, _)| i)
            .collect();
        self.sel
            .select(if self.view.is_empty() { None } else { Some(0) });
    }

    fn on_rows_key(&mut self, code: KeyCode) -> Result<()> {
        match code {
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Char('b') | KeyCode::Esc => self.go_back(),
            KeyCode::Char('j') | KeyCode::Down => self.row_move(1),
            KeyCode::Char('k') | KeyCode::Up => self.row_move(-1),
            // Horizontal: move the selected LOCALE column (h/l or ←/→).
            KeyCode::Char('l') | KeyCode::Right => {
                self.col = (self.col + 1) % LOCALES.len();
            }
            KeyCode::Char('h') | KeyCode::Left => {
                self.col = (self.col + LOCALES.len() - 1) % LOCALES.len();
            }
            KeyCode::Char('f') => {
                self.filter = self.filter.next();
                self.apply_filter();
            }
            KeyCode::Char('x') => self.export()?,
            KeyCode::Char('e') | KeyCode::Enter => self.begin_edit(),
            KeyCode::Char('d') | KeyCode::Delete => self.delete_cell()?,
            _ => {}
        }
        Ok(())
    }

    fn current_row(&self) -> Option<&Row> {
        let vi = self.sel.selected()?;
        self.view.get(vi).and_then(|&i| self.all_rows.get(i))
    }

    fn row_move(&mut self, delta: i32) {
        let n = self.view.len() as i32;
        if n == 0 {
            return;
        }
        let cur = self.sel.selected().unwrap_or(0) as i32;
        self.sel.select(Some((cur + delta).rem_euclid(n) as usize));
    }

    fn export(&mut self) -> Result<()> {
        // Export the FILTERED set's preferred values (every locale cell) to a
        // translations SQLite.
        let out = PathBuf::from(format!("./{}-translations.sqlite", self.tenant_slug()));
        let mut rows: Vec<(String, String, String, String)> = Vec::new();
        for &i in &self.view {
            let Some(r) = self.all_rows.get(i) else {
                continue;
            };
            for (li, c) in r.cells.iter().enumerate() {
                let (origin, val) = c.preferred();
                if val.is_empty() {
                    continue;
                }
                rows.push((
                    LOCALES[li].to_string(),
                    r.phrase.german.clone(),
                    val.to_string(),
                    origin_tag(origin).to_string(),
                ));
            }
        }
        let n = compare::export(&out, &rows)?;
        self.status = format!("exportiert: {n} Einträge → {}", out.display());
        Ok(())
    }

    fn tenant_slug(&self) -> String {
        self.tenant_name
            .rsplit_once('.')
            .map(|(s, _)| s.to_string())
            .unwrap_or_else(|| self.tenant_name.clone())
    }

    // ---- edit --------------------------------------------------------------

    fn on_edit_key(&mut self, code: KeyCode) -> Result<()> {
        match code {
            KeyCode::Esc => self.screen = Screen::Rows,
            KeyCode::Enter => self.save_edit()?,
            KeyCode::Backspace => {
                self.input.pop();
            }
            KeyCode::Char(c) => self.input.push(c),
            _ => {}
        }
        Ok(())
    }

    fn save_edit(&mut self) -> Result<()> {
        let Some(&i) = self.sel.selected().and_then(|vi| self.view.get(vi)) else {
            return Ok(());
        };
        let col = self.col;
        let locale = LOCALES[col];
        let phrase = self.all_rows[i].phrase.clone();
        let conn = self.conn.as_ref().expect("tenant open");
        let n = data::set_override(conn, &phrase, locale, &self.input)?;
        // Reflect the write in the selected cell's DB value.
        if let Some(c) = self.all_rows[i].cells.get_mut(col) {
            c.db = if self.input.trim().is_empty() {
                None
            } else {
                Some(self.input.trim().to_string())
            };
        }
        self.status = format!(
            "gespeichert: {}_{} → {} Zeile(n)",
            phrase.field.base(),
            locale,
            n
        );
        self.apply_filter();
        self.screen = Screen::Rows;
        Ok(())
    }

    // ---- render ------------------------------------------------------------

    fn draw(&mut self, f: &mut Frame) {
        self.row_hits.clear();
        self.menu_hits.clear();
        self.wsrc_hits.clear();
        match self.screen {
            Screen::Tenants => self.draw_tenants(f),
            Screen::Rows => self.draw_rows(f),
            Screen::Edit => self.draw_edit(f),
            Screen::WasmSrc => self.draw_wasmsrc(f),
            Screen::WasmEdit => self.draw_wedit(f),
        }
        if self.overlay {
            self.draw_overlay(f);
        }
    }

    fn draw_wasmsrc(&mut self, f: &mut Frame) {
        let chunks = Layout::vertical([
            Constraint::Length(1), // header
            Constraint::Length(1), // status
            Constraint::Min(1),    // list
            Constraint::Length(1), // menu bar
        ])
        .split(f.area());

        let prof = self.cur_profile();
        let locs = self.wprofile_locales();
        f.render_widget(
            Paragraph::new(
                Line::from(format!(
                    " WASM-QUELLE  Profil:{}  {} Zeilen × {} Sprachen ",
                    prof.name,
                    self.wview.len(),
                    locs.len(),
                ))
                .bold(),
            )
            .style(Style::new().fg(Color::White).bg(Color::Magenta)),
            chunks[0],
        );
        if !self.status.is_empty() {
            f.render_widget(
                Paragraph::new(Line::from(self.status.clone()).bold())
                    .style(Style::new().fg(Color::Green)),
                chunks[1],
            );
        }

        // Multilingual table: German origin + one column per profile locale.
        let list_area = chunks[2];
        let germans = self.wgermans();
        let de_w = (list_area.width as usize * 38 / 100).clamp(10, 26);
        let col_w = 12usize;
        let vis = ((list_area.width as usize).saturating_sub(de_w + 2) / col_w).max(1);
        // horizontal scroll on wsrc_locale
        let mut hcol = 0usize;
        if self.wsrc_locale >= vis {
            hcol = self.wsrc_locale + 1 - vis;
        }
        let last = (hcol + vis).min(locs.len());

        // Header
        let mut hdr: Vec<Span> = vec![Span::styled(
            format!("{:<de_w$}", "DEUTSCH", de_w = de_w),
            Style::new().fg(Color::Gray).bold(),
        )];
        for li in hcol..last {
            let st = if li == self.wsrc_locale {
                Style::new().fg(Color::Black).bg(Color::White).bold()
            } else {
                Style::new().fg(Color::Gray).bold()
            };
            hdr.push(Span::raw(" "));
            hdr.push(Span::styled(
                format!("{:<w$}", locs[li].to_uppercase(), w = col_w - 1),
                st,
            ));
        }
        f.render_widget(
            Paragraph::new(Line::from(hdr)),
            Rect {
                x: list_area.x,
                y: list_area.y,
                width: list_area.width,
                height: 1,
            },
        );

        let body = Rect {
            x: list_area.x,
            y: list_area.y + 1,
            width: list_area.width,
            height: list_area.height.saturating_sub(1),
        };
        let top = self.wsel.offset();
        let sel_vi = self.wsel.selected();
        let mut items: Vec<ListItem> = Vec::new();
        for (vi, &i) in self.wview.iter().enumerate() {
            let Some(g) = germans.get(i) else { continue };
            let screen_i = vi.saturating_sub(top);
            if vi >= top && screen_i < body.height as usize {
                self.wsrc_hits.push((
                    Rect {
                        x: body.x,
                        y: body.y + screen_i as u16,
                        width: body.width,
                        height: 1,
                    },
                    vi,
                ));
            }
            let is_sel = sel_vi == Some(vi);
            let de = truncate(g, de_w);
            let de_len = de.chars().count();
            let mut spans: Vec<Span> = vec![Span::styled(de, Style::new().fg(Color::White).bold())];
            if de_len < de_w {
                spans.push(Span::raw(" ".repeat(de_w - de_len)));
            }
            for li in hcol..last {
                let v = self.wsrc_value(g, locs[li]).unwrap_or_default();
                let txt = truncate(&v, col_w - 1);
                let mut st = Style::new().fg(if v.is_empty() {
                    Color::DarkGray
                } else {
                    Color::Cyan
                });
                if is_sel && li == self.wsrc_locale {
                    st = st.bg(Color::Indexed(238)).bold();
                }
                spans.push(Span::raw(" "));
                spans.push(Span::styled(format!("{:<w$}", txt, w = col_w - 1), st));
            }
            let line = Line::from(spans);
            items.push(ListItem::new(if is_sel {
                line.style(Style::new().bg(Color::Indexed(236)))
            } else {
                line
            }));
        }
        let list = List::new(items).highlight_symbol("");
        f.render_stateful_widget(list, body, &mut self.wsel);

        self.draw_menu_bar(f, chunks[3]);
    }

    fn draw_wedit(&mut self, f: &mut Frame) {
        let chunks = Layout::vertical([
            Constraint::Length(6),
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(f.area());

        let german = self.cur_wgerman();
        let loc = self.cur_wlocale();
        let mut lines: Vec<Line> = Vec::new();
        if let Some(g) = &german {
            let cur = self.wsrc_value(g, loc).unwrap_or_default();
            lines.push(
                Line::from(format!(
                    " {} · menu · {} ",
                    loc.to_uppercase(),
                    self.cur_profile().name
                ))
                .style(Style::new().fg(Color::White).bg(Color::Magenta))
                .bold(),
            );
            lines.push(Line::from(""));
            lines.push(Line::from(format!("DE: {g}")).bold());
            lines.push(Line::from(vec![
                Span::raw("wasm-Wert: "),
                Span::styled(cur, Style::new().fg(Color::Cyan)),
            ]));
        }
        f.render_widget(
            Paragraph::new(lines).wrap(Wrap { trim: true }).block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Wasm-Quelle "),
            ),
            chunks[0],
        );
        f.render_widget(
            Paragraph::new(Line::from(format!("{}█", self.input)).bold())
                .wrap(Wrap { trim: false })
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(" Übersetzung (tippen) "),
                ),
            chunks[1],
        );
        f.render_widget(
            Paragraph::new(
                Line::from(" Enter speichern · Esc abbrechen · leer = entfernen ").bold(),
            )
            .style(Style::new().fg(Color::Black).bg(Color::Cyan)),
            chunks[3],
        );
    }

    /// The exact key hints the current screen responds to — shown verbatim in
    /// the bottom bar so the keys are always visible (not hidden behind the
    /// overlay). These ARE the keys the per-screen handlers use.
    fn screen_keys(&self) -> &'static str {
        match self.screen {
            Screen::Tenants => "j/k ↕ · ⏎ öffnen · W Wasm · q Ende",
            Screen::Rows => {
                "j/k ↕ · h/l Sprache ↔ · e Bearb · d Lösch · f Filter · x Export · b · q"
            }
            Screen::Edit => "⏎ Speichern · Esc Abbruch",
            Screen::WasmSrc => {
                "j/k ↕ · h/l Sprache ↔ · e Bearb · d Lösch · p Profil · r Bauen · b · q"
            }
            Screen::WasmEdit => "⏎ Speichern · Esc Abbruch",
        }
    }

    /// Bottom bar: the per-screen KEY HINTS (always visible) + a "☰ MENÜ" tag
    /// signalling the big action overlay (Leertaste / tap). Tapping the bar
    /// opens the overlay; the keys remain usable directly.
    fn draw_menu_bar(&mut self, f: &mut Frame, area: Rect) {
        if self.overlay {
            let bar =
                Paragraph::new(Line::from(" ✕  Menü schließen (Esc / Leertaste) ").centered())
                    .style(Style::new().fg(Color::Black).bg(Color::Cyan).bold());
            f.render_widget(bar, area);
            self.menu_bar = area;
            return;
        }
        // "☰" tag (left) + the live key hints (rest), one high-contrast line.
        let line = Line::from(vec![
            Span::styled(" ☰ ", Style::new().fg(Color::Black).bg(Color::Cyan).bold()),
            Span::raw(" "),
            Span::styled(self.screen_keys(), Style::new().fg(Color::White).bold()),
        ]);
        f.render_widget(Paragraph::new(line).bg(Color::Indexed(236)), area);
        self.menu_bar = area;
    }

    fn draw_tenants(&mut self, f: &mut Frame) {
        let chunks = Layout::vertical([
            Constraint::Length(2), // title
            Constraint::Min(1),    // list
            Constraint::Length(1), // menu bar
        ])
        .split(f.area());

        let pack = if self.pack.is_some() {
            "● wasm"
        } else {
            "○ wasm"
        };
        let cmp = if self.cmp.is_empty() { "" } else { "  ● cmp" };
        f.render_widget(
            Paragraph::new(Line::from(format!(" MANDANT WÄHLEN     {pack}{cmp}")).bold())
                .style(Style::new().fg(Color::White).bg(Color::Blue)),
            chunks[0],
        );

        // Big, spaced rows; record hit rects for taps.
        let list_area = chunks[1];
        let items: Vec<ListItem> = self
            .dbs
            .iter()
            .enumerate()
            .map(|(i, p)| {
                if i < list_area.height as usize {
                    self.row_hits.push((
                        Rect {
                            x: list_area.x,
                            y: list_area.y + i as u16,
                            width: list_area.width,
                            height: 1,
                        },
                        i,
                    ));
                }
                ListItem::new(Line::from(format!(
                    "  {}",
                    p.file_name().and_then(|n| n.to_str()).unwrap_or("?")
                )))
            })
            .collect();
        let list = List::new(items)
            .highlight_style(Style::new().fg(Color::Black).bg(Color::Cyan).bold())
            .highlight_symbol("▶ ");
        f.render_stateful_widget(list, list_area, &mut self.tenant_sel);

        self.draw_menu_bar(f, chunks[2]);
    }

    fn draw_rows(&mut self, f: &mut Frame) {
        let chunks = Layout::vertical([
            Constraint::Length(1), // header
            Constraint::Length(1), // status
            Constraint::Min(1),    // list
            Constraint::Length(1), // menu bar
        ])
        .split(f.area());

        f.render_widget(
            Paragraph::new(
                Line::from(format!(
                    " {}  [{}]  {}/{} ",
                    self.tenant_name,
                    self.filter.label(),
                    self.view.len(),
                    self.all_rows.len()
                ))
                .bold(),
            )
            .style(Style::new().fg(Color::White).bg(Color::Blue)),
            chunks[0],
        );
        // Status line, else a colour LEGEND so you can SEE which colour is
        // which source (DB / wasm / compare) — the wasm IS the cyan column.
        if !self.status.is_empty() {
            f.render_widget(
                Paragraph::new(Line::from(self.status.clone()).bold())
                    .style(Style::new().fg(Color::Green)),
                chunks[1],
            );
        } else {
            f.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::raw(" Quelle: "),
                    Span::styled("DB", Style::new().fg(Color::Green).bold()),
                    Span::raw(" · "),
                    Span::styled("wasm", Style::new().fg(Color::Cyan).bold()),
                    Span::raw(" · "),
                    Span::styled("cmp", Style::new().fg(Color::Yellow).bold()),
                ])),
                chunks[1],
            );
        }

        let list_area = chunks[2];
        // Multilingual table: column 0 = German origin, then the visible locale
        // columns from `hcol`. The selected locale `col` is highlighted in the
        // column header so you know which cell `e` edits.
        let cells: Vec<(usize, Cell)> = self
            .current_row()
            .map(|r| r.cells.clone())
            .unwrap_or_default()
            .into_iter()
            .enumerate()
            .collect();
        let _ = cells; // (header below derives columns from LOCALES)
        let de_w = (list_area.width as usize * 38 / 100).clamp(10, 26);
        let col_w = 12usize; // each locale column width
        let vis_cols = ((list_area.width as usize).saturating_sub(de_w + 2) / col_w).max(1);
        self.hcol = self.hcol.min(self.col); // keep selected col in/left of view
        if self.col >= self.hcol + vis_cols {
            self.hcol = self.col + 1 - vis_cols;
        }
        let last = (self.hcol + vis_cols).min(LOCALES.len());

        // Column header row (German + locale codes, selected one inverse).
        let mut hdr: Vec<Span> = vec![Span::styled(
            format!("{:<de_w$}", "DEUTSCH", de_w = de_w),
            Style::new().fg(Color::Gray).bold(),
        )];
        for li in self.hcol..last {
            let code = LOCALES[li].to_uppercase();
            let st = if li == self.col {
                Style::new().fg(Color::Black).bg(Color::White).bold()
            } else {
                Style::new().fg(Color::Gray).bold()
            };
            hdr.push(Span::raw(" "));
            hdr.push(Span::styled(format!("{:<w$}", code, w = col_w - 1), st));
        }
        let header_area = Rect {
            x: list_area.x,
            y: list_area.y,
            width: list_area.width,
            height: 1,
        };
        f.render_widget(Paragraph::new(Line::from(hdr)), header_area);

        // Body rows.
        let body = Rect {
            x: list_area.x,
            y: list_area.y + 1,
            width: list_area.width,
            height: list_area.height.saturating_sub(1),
        };
        let top = self.sel.offset();
        let sel_vi = self.sel.selected();
        let mut items: Vec<ListItem> = Vec::new();
        for (vi, &i) in self.view.iter().enumerate() {
            let Some(r) = self.all_rows.get(i) else {
                continue;
            };
            let screen_i = vi.saturating_sub(top);
            if vi >= top && screen_i < body.height as usize {
                self.row_hits.push((
                    Rect {
                        x: body.x,
                        y: body.y + screen_i as u16,
                        width: body.width,
                        height: 1,
                    },
                    vi,
                ));
            }
            let is_sel = sel_vi == Some(vi);
            let mut spans: Vec<Span> = vec![Span::styled(
                truncate(&r.phrase.german, de_w),
                Style::new().fg(Color::White).bold(),
            )];
            // pad german col
            let de_len = truncate(&r.phrase.german, de_w).chars().count();
            if de_len < de_w {
                spans.push(Span::raw(" ".repeat(de_w - de_len)));
            }
            for li in self.hcol..last {
                let cell = &r.cells[li];
                let (origin, val) = cell.preferred();
                let txt = truncate(val, col_w - 1);
                let mut st = Style::new().fg(origin_color(origin));
                if is_sel && li == self.col {
                    st = st.bg(Color::Indexed(238)).bold();
                }
                spans.push(Span::raw(" "));
                spans.push(Span::styled(format!("{:<w$}", txt, w = col_w - 1), st));
            }
            let line = Line::from(spans);
            items.push(ListItem::new(if is_sel {
                line.style(Style::new().bg(Color::Indexed(236)))
            } else {
                line
            }));
        }
        let list = List::new(items).highlight_symbol("");
        f.render_stateful_widget(list, body, &mut self.sel);

        self.draw_menu_bar(f, chunks[3]);
    }

    fn draw_edit(&mut self, f: &mut Frame) {
        let chunks = Layout::vertical([
            Constraint::Length(9), // sources
            Constraint::Length(3), // input
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(f.area());

        let col = self.col;
        let locale = LOCALES[col];
        let row = self.current_row().cloned();
        let mut lines: Vec<Line> = Vec::new();
        if let Some(r) = &row {
            let cell = r.cells.get(col).cloned().unwrap_or_default();
            // Big locale header (bold, inverse) so it's legible without glasses.
            lines.push(
                Line::from(format!(
                    " {} · {} ",
                    locale.to_uppercase(),
                    r.phrase.field.base()
                ))
                .style(Style::new().fg(Color::White).bg(Color::Blue))
                .bold(),
            );
            lines.push(Line::from(""));
            lines.push(Line::from(format!("DE: {}", r.phrase.german)).bold());
            lines.push(Line::from(""));
            lines.push(source_line("#1 DB ", cell.db.as_deref(), Origin::Db));
            lines.push(source_line("#2 wasm", cell.wasm.as_deref(), Origin::Wasm));
            lines.push(source_line("#3 cmp ", cell.cmp.as_deref(), Origin::Compare));
            let (origin, _) = cell.preferred();
            lines.push(Line::from(vec![
                Span::raw("→ bevorzugt: "),
                Span::styled(
                    origin_tag(origin),
                    Style::new().fg(origin_color(origin)).bold(),
                ),
            ]));
        }
        f.render_widget(
            Paragraph::new(lines)
                .wrap(Wrap { trim: true })
                .block(Block::default().borders(Borders::ALL).title(" Quellen ")),
            chunks[0],
        );

        f.render_widget(
            Paragraph::new(Line::from(format!("{}█", self.input)).bold())
                .wrap(Wrap { trim: false })
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(" DB-Override (tippen) "),
                ),
            chunks[1],
        );
        f.render_widget(
            Paragraph::new(Line::from(" Enter speichern · Esc abbrechen · leer = löschen ").bold())
                .style(Style::new().fg(Color::Black).bg(Color::Cyan)),
            chunks[3],
        );
    }

    /// The big action-menu overlay popup. Large numbered, high-contrast buttons;
    /// pick by number key or tap. Records each button's rect into `menu_hits`.
    fn draw_overlay(&mut self, f: &mut Frame) {
        let acts = self.actions();
        // Each action is a 3-row button → height = title(1)+gap + 3*n + border.
        let btn_h: u16 = 3;
        let inner_h = acts.len() as u16 * btn_h + 1; // +1 title
        let w = 34u16;
        let area = centered(f.area(), w, inner_h + 2);

        // Dim backdrop + bordered panel.
        f.render_widget(ratatui::widgets::Clear, area);
        let panel = Block::default()
            .borders(Borders::ALL)
            .title(" AKTION WÄHLEN ")
            .border_style(Style::new().fg(Color::Cyan).bold());
        let inner = panel.inner(area);
        f.render_widget(panel, area);

        let mut y = inner.y;
        for (i, &a) in acts.iter().enumerate() {
            let r = Rect {
                x: inner.x,
                y,
                width: inner.width,
                height: btn_h,
            };
            // Big button: number + label, centered, inverse cyan.
            let label = format!("  {}   {}  ", i + 1, a.label());
            f.render_widget(
                Paragraph::new(vec![Line::from(""), Line::from(label).centered()])
                    .style(Style::new().fg(Color::Black).bg(Color::Cyan).bold()),
                r,
            );
            self.menu_hits.push((r, a));
            y += btn_h;
        }
    }
}

/// Colour for a source origin (the provenance colour code).
fn origin_color(o: Origin) -> Color {
    match o {
        Origin::Db => Color::Green,
        Origin::Wasm => Color::Cyan,
        Origin::Compare => Color::Yellow,
        Origin::None => Color::DarkGray,
    }
}

fn origin_tag(o: Origin) -> &'static str {
    match o {
        Origin::Db => "DB",
        Origin::Wasm => "wasm",
        Origin::Compare => "cmp",
        Origin::None => "—",
    }
}

/// A source line in the edit screen: "#1 DB : value" coloured, or dimmed "—".
fn source_line(label: &str, value: Option<&str>, origin: Origin) -> Line<'static> {
    match value {
        Some(v) => Line::from(vec![
            Span::styled(format!("{label}: "), Style::new().fg(origin_color(origin))),
            Span::raw(v.to_string()),
        ]),
        None => Line::from(vec![
            Span::styled(format!("{label}: "), Style::new().dim()),
            Span::styled("—", Style::new().dim()),
        ]),
    }
}

/// Is point (x,y) inside rect r?
fn hit(r: Rect, x: u16, y: u16) -> bool {
    x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height
}

/// Centre a `w`×`h` rect inside `area` (clamped).
fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width);
    let h = h.min(area.height);
    Rect {
        x: area.x + (area.width.saturating_sub(w)) / 2,
        y: area.y + (area.height.saturating_sub(h)) / 2,
        width: w,
        height: h,
    }
}

/// Truncate to `max` chars with an ellipsis.
fn truncate(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max {
        s.to_string()
    } else {
        let cut = max.saturating_sub(1);
        let mut out: String = chars[..cut].iter().collect();
        out.push('…');
        out
    }
}
