//! Data layer for the i18n TUI: coverage (from the menu_strings JSON), the
//! shop's German phrases + edits (from the tenant SQLite DB).
//!
//! DB-first model: editing a translation writes the `_<lang>` OVERRIDE column on
//! `menu_items` (the deliberate per-shop override slot). The bulk translation
//! vocabulary lives in the pack (menu_strings JSON); this tool fills per-shop
//! gaps + overrides.

use anyhow::{Context, Result};
use rusqlite::Connection;
use rusterando_i18n_core::SetCoverage;
use std::path::Path;

/// The 7 non-German locales (German is the base, never a target).
pub const LOCALES: &[&str] = &["en", "fr", "it", "es", "pt", "ru", "cn"];

/// Which base column a phrase came from — decides the `_<lang>` column to write.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Name,
    Description,
}

impl Field {
    pub fn base(self) -> &'static str {
        match self {
            Field::Name => "name",
            Field::Description => "description",
        }
    }
}

/// A distinct German source phrase the shop uses, plus which column it's from.
#[derive(Clone)]
pub struct Phrase {
    pub german: String,
    pub field: Field,
}

/// Build coverage from the `menu_strings.<locale>.json` files in `dir`. A
/// non-empty value means "(locale, german) is translated in the pack" — exactly
/// the set the mgmt wasm bakes. Currently unused (the executed pack wasm is the
/// live source #2), kept as the no-wasm fallback path.
#[allow(dead_code)]
pub fn load_coverage(dir: &Path) -> Result<SetCoverage> {
    let mut pairs: Vec<(String, String)> = Vec::new();
    for &loc in LOCALES {
        let path = dir.join(format!("menu_strings.{loc}.json"));
        let Ok(raw) = std::fs::read_to_string(&path) else {
            continue; // a missing locale file just means no coverage for it
        };
        let map: std::collections::BTreeMap<String, String> =
            serde_json::from_str(&raw).with_context(|| format!("parse {}", path.display()))?;
        for (german, value) in map {
            if !value.trim().is_empty() {
                pairs.push((loc.to_string(), german));
            }
        }
    }
    Ok(SetCoverage::from_pairs(pairs))
}

/// List the tenant DBs found under `data/` (files ending `.sqlite` or `.db`,
/// excluding sidecar `-wal`/`-shm`). Returned sorted.
pub fn list_tenant_dbs(data_dir: &Path) -> Result<Vec<std::path::PathBuf>> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(data_dir) {
        for entry in rd.flatten() {
            let p = entry.path();
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if (name.ends_with(".sqlite") || name.ends_with(".db"))
                && !name.contains("-wal")
                && !name.contains("-shm")
            {
                out.push(p);
            }
        }
    }
    out.sort();
    Ok(out)
}

/// Open a tenant DB read/write (WAL, busy timeout — coexists with the running
/// server which holds the same DB).
pub fn open(db_path: &Path) -> Result<Connection> {
    let conn = Connection::open(db_path).with_context(|| format!("open {}", db_path.display()))?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    Ok(conn)
}

/// The shop's distinct German phrases from `menu_items` (name + description),
/// each tagged with its field. Empty/null values skipped.
pub fn shop_phrases(conn: &Connection) -> Result<Vec<Phrase>> {
    let mut out = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for (field, col) in [(Field::Name, "name"), (Field::Description, "description")] {
        let sql = format!(
            "SELECT DISTINCT TRIM({col}) FROM menu_items \
             WHERE {col} IS NOT NULL AND TRIM({col}) <> ''"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        for german in rows.flatten() {
            if seen.insert((field as u8, german.clone())) {
                out.push(Phrase { german, field });
            }
        }
    }
    Ok(out)
}

/// Read the current DB override for `(phrase, locale)` — the `_<lang>` cell.
/// `None` if empty (so the pack/German fallback applies). Matches on the German
/// base value (the same join key the importer uses).
pub fn current_override(conn: &Connection, p: &Phrase, locale: &str) -> Result<Option<String>> {
    let col = format!("{}_{}", p.field.base(), locale);
    let sql = format!(
        "SELECT {col} FROM menu_items WHERE TRIM({}) = ?1 \
         AND {col} IS NOT NULL AND TRIM({col}) <> '' LIMIT 1",
        p.field.base()
    );
    let v: Option<String> = conn.query_row(&sql, [&p.german], |r| r.get(0)).ok();
    Ok(v.filter(|s| !s.trim().is_empty()))
}

/// Write the DB override `<col>_<locale>` for every row whose German base equals
/// `p.german`. Returns rows updated. An empty `value` clears the override (back
/// to pack/German). NEVER touches the German base column.
pub fn set_override(conn: &Connection, p: &Phrase, locale: &str, value: &str) -> Result<usize> {
    let col = format!("{}_{}", p.field.base(), locale);
    let v = value.trim();
    let sql = format!(
        "UPDATE menu_items SET {col} = ?1 WHERE TRIM({}) = ?2",
        p.field.base()
    );
    let bind_val: Option<&str> = if v.is_empty() { None } else { Some(v) };
    let n = conn.execute(&sql, rusqlite::params![bind_val, &p.german])?;
    Ok(n)
}
