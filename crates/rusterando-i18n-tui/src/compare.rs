//! Source #3: an optional **compare/export translations SQLite**.
//!
//! Schema (created on export, read on load):
//!   CREATE TABLE translations (
//!     locale TEXT, german TEXT, value TEXT, source TEXT,
//!     PRIMARY KEY (locale, german)
//!   );
//!
//! `source` records where the value came from (db / wasm / compare / manual) so
//! a merge keeps provenance. Loaded read-only as source #3 for the 3-source
//! view; written by the export.

use anyhow::{Context, Result};
use rusqlite::Connection;
use std::collections::HashMap;
use std::path::Path;

/// In-memory compare set: `(locale, german) -> (value, source)`.
#[derive(Default)]
pub struct CompareDb {
    map: HashMap<(String, String), (String, String)>,
}

impl CompareDb {
    /// Load a compare SQLite (read-only). Missing file / missing table → empty
    /// (the compare column just shows blank), never an error — it's optional.
    pub fn load(path: &Path) -> Result<Self> {
        let mut db = CompareDb::default();
        if !path.exists() {
            return Ok(db);
        }
        let conn = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .with_context(|| format!("open compare {}", path.display()))?;
        // Tolerate a missing table.
        let mut stmt = match conn.prepare("SELECT locale, german, value, source FROM translations")
        {
            Ok(s) => s,
            Err(_) => return Ok(db),
        };
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3).unwrap_or_default(),
            ))
        })?;
        for row in rows.flatten() {
            let (locale, german, value, source) = row;
            db.map.insert((locale, german), (value, source));
        }
        Ok(db)
    }

    /// The compare value for `(locale, german)`, if present + non-empty.
    pub fn get(&self, locale: &str, german: &str) -> Option<&str> {
        self.map
            .get(&(locale.to_string(), german.to_string()))
            .map(|(v, _)| v.as_str())
            .filter(|v| !v.trim().is_empty())
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

/// Export rows to a translations SQLite (create/replace the table). Each row is
/// `(locale, german, value, source)`. Overwrites an existing same-key row
/// (last write wins) so re-exporting is idempotent.
pub fn export(path: &Path, rows: &[(String, String, String, String)]) -> Result<usize> {
    let conn = Connection::open(path).with_context(|| format!("open export {}", path.display()))?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS translations (
            locale TEXT NOT NULL,
            german TEXT NOT NULL,
            value  TEXT NOT NULL,
            source TEXT NOT NULL DEFAULT '',
            PRIMARY KEY (locale, german)
        );",
    )?;
    let tx = conn.unchecked_transaction()?;
    let mut n = 0usize;
    {
        let mut stmt = tx.prepare(
            "INSERT INTO translations (locale, german, value, source) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(locale, german) DO UPDATE SET value = excluded.value, source = excluded.source",
        )?;
        for (locale, german, value, source) in rows {
            if value.trim().is_empty() {
                continue; // don't export empties
            }
            stmt.execute(rusqlite::params![locale, german, value, source])?;
            n += 1;
        }
    }
    tx.commit()?;
    Ok(n)
}
