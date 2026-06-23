//! Wasm-SOURCE layer: the canonical `menu-translations.sqlite` the pack wasm is
//! built from (see rusterando-scrape/migrate_to_source_sqlite.py +
//! gen_i18n_pack.py --source-sqlite). A `.wasm` is compiled and can't be edited
//! in place, so "edit the wasm" means edit THIS sqlite then rebuild.
//!
//! Schema (translations): (locale, german, value, topic, source), PK
//! (locale, german, topic). Topic groups the rows the way build profiles slice
//! them (menu / shop / kitchen / shared).
//!
//! This module: read all rows, filter by profile (which locales+topics a build
//! profile includes), upsert an edited/merged value, and trigger a rebuild by
//! shelling out to scripts/build-i18n-pack.sh.

use anyhow::{Context, Result};
use rusqlite::Connection;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// A build profile's slice: which locales + topics it includes. Mirrors
/// i18n-pack.toml's `[profiles.*]` (kept here to avoid a TOML dep; update if the
/// manifest's profiles change — the builder remains the source of truth for the
/// actual build).
pub struct Profile {
    pub name: &'static str,
    pub locales: &'static [&'static str],
    /// Topics this profile includes (mirrors i18n-pack.toml). The TUI table
    /// currently shows only the `menu` topic; kept for completeness + future
    /// topic columns. The builder is the source of truth for the actual slice.
    #[allow(dead_code)]
    pub topics: &'static [&'static str],
}

pub const PROFILES: &[Profile] = &[
    Profile {
        name: "fat",
        locales: &["en", "fr", "it", "es", "pt", "ru", "cn"],
        topics: &["shared", "shop", "kitchen", "menu"],
    },
    Profile {
        name: "small",
        // 'shared' is auto-included by the builder.
        locales: &["en"],
        topics: &["shared", "shop", "menu"],
    },
];

/// One source row.
#[derive(Clone)]
pub struct SrcRow {
    pub locale: String,
    pub german: String,
    pub value: String,
    pub topic: String,
    pub source: String,
}

impl SrcRow {
    /// Is this row included in `profile`'s slice (locale ∈ locales, topic ∈ topics)?
    #[allow(dead_code)]
    pub fn in_profile(&self, p: &Profile) -> bool {
        p.locales.contains(&self.locale.as_str()) && p.topics.contains(&self.topic.as_str())
    }
}

/// The wasm-source sqlite, loaded into memory for the TUI.
pub struct WasmSrc {
    pub path: PathBuf,
    pub rows: Vec<SrcRow>,
}

impl WasmSrc {
    /// Ensure the schema exists + load all rows.
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)
            .with_context(|| format!("open wasm-source {}", path.display()))?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS translations (
                locale TEXT NOT NULL,
                german TEXT NOT NULL,
                value  TEXT NOT NULL,
                topic  TEXT NOT NULL,
                source TEXT NOT NULL DEFAULT '',
                PRIMARY KEY (locale, german, topic)
            );",
        )?;
        let mut rows = Vec::new();
        {
            let mut stmt = conn.prepare(
                "SELECT locale, german, value, topic, source FROM translations ORDER BY topic, german, locale",
            )?;
            let it = stmt.query_map([], |r| {
                Ok(SrcRow {
                    locale: r.get(0)?,
                    german: r.get(1)?,
                    value: r.get(2)?,
                    topic: r.get(3)?,
                    source: r.get::<_, String>(4).unwrap_or_default(),
                })
            })?;
            for row in it.flatten() {
                rows.push(row);
            }
        }
        Ok(Self {
            path: path.to_path_buf(),
            rows,
        })
    }

    /// Upsert a translation (edit/add). `topic` defaults to "menu". Empty value
    /// DELETES the row (the wasm then won't carry it). Updates both the DB and
    /// the in-memory `rows`. Returns true if a row now exists.
    pub fn set(&mut self, locale: &str, german: &str, topic: &str, value: &str) -> Result<bool> {
        let conn = Connection::open(&self.path)?;
        let v = value.trim();
        if v.is_empty() {
            conn.execute(
                "DELETE FROM translations WHERE locale=?1 AND german=?2 AND topic=?3",
                rusqlite::params![locale, german, topic],
            )?;
            self.rows
                .retain(|r| !(r.locale == locale && r.german == german && r.topic == topic));
            return Ok(false);
        }
        conn.execute(
            "INSERT INTO translations (locale, german, value, topic, source) VALUES (?1,?2,?3,?4,'tui')
             ON CONFLICT(locale, german, topic) DO UPDATE SET value=excluded.value, source='tui'",
            rusqlite::params![locale, german, v, topic],
        )?;
        // reflect in memory
        if let Some(r) = self
            .rows
            .iter_mut()
            .find(|r| r.locale == locale && r.german == german && r.topic == topic)
        {
            r.value = v.to_string();
            r.source = "tui".into();
        } else {
            self.rows.push(SrcRow {
                locale: locale.into(),
                german: german.into(),
                value: v.into(),
                topic: topic.into(),
                source: "tui".into(),
            });
        }
        Ok(true)
    }

    /// Merge another source sqlite into this one: for each (locale, german,
    /// topic) present in `other`, upsert its value here (other wins on conflict).
    /// Returns how many rows were written. Used for "merge wasms".
    pub fn merge_from(&mut self, other: &Path) -> Result<usize> {
        let src = WasmSrc::open(other)?;
        let mut n = 0;
        for r in &src.rows {
            if self.set(&r.locale, &r.german, &r.topic, &r.value)? {
                n += 1;
            }
        }
        Ok(n)
    }

    /// The locales present in the source (sorted).
    #[allow(dead_code)]
    pub fn locales(&self) -> Vec<String> {
        let mut s: BTreeMap<String, ()> = BTreeMap::new();
        for r in &self.rows {
            s.insert(r.locale.clone(), ());
        }
        s.into_keys().collect()
    }
}

/// Trigger a pack rebuild for `profile` by shelling out to the build script.
/// Returns the script's combined output (for the TUI status). Runs in the repo
/// root (cwd), where the script + sqlite live.
pub fn rebuild(profile: &str) -> Result<String> {
    let out = std::process::Command::new("bash")
        .arg("scripts/build-i18n-pack.sh")
        .arg("--profile")
        .arg(profile)
        .output()
        .context("run build-i18n-pack.sh")?;
    let mut s = String::from_utf8_lossy(&out.stdout).into_owned();
    s.push_str(&String::from_utf8_lossy(&out.stderr));
    if !out.status.success() {
        anyhow::bail!("rebuild failed: {}", s.lines().last().unwrap_or(""));
    }
    // Return the last meaningful line (the "done … data_hash=" summary).
    Ok(s.lines()
        .rev()
        .find(|l| l.contains("done") || l.contains("data_hash"))
        .unwrap_or("rebuilt")
        .trim()
        .to_string())
}
