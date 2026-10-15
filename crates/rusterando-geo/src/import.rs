//! Address backends. [`AddressSource`] is the seam: everything above the
//! index (store, server functions, checkout) only sees [`ImportResult`], so a
//! second backend (an OSM extract for tenants outside NRW) plugs in here.
//!
//! [`CadastreNrw`] reads the Gebäudereferenzen NRW export
//! (`gebref_EPSG25832_ASCII.zip`, Geobasis NRW, Datenlizenz Deutschland –
//! Zero 2.0). Verified layout of the July 2026 export — one `gebref.txt`
//! inside the zip, no separate key file, street names inline, 21
//! semicolon-separated fields, CRLF, UTF-8:
//!
//! ```text
//!  0 N                    record marker
//!  1 DENW01HK10000a1o     object id
//!  2 A | B                quality (A = surveyed, B = derived)
//!  3 05                   Land
//!  4 Nordrhein-Westfalen
//!  5 9                    Regierungsbezirk
//!  6 Arnsberg
//!  7 11                   Kreis
//!  8 Bochum
//!  9 000                  Gemeinde
//! 10 Bochum
//! 11 0000                 Ortsteil key   (unused in the current export)
//! 12                      Ortsteil name  (empty in the current export)
//! 13 04086                Straßenschlüssel
//! 14 Obertor              street name
//! 15 50                   house number
//! 16                      suffix (a, b, …)
//! 17 32                   UTM zone
//! 18 370279.838           east  (EPSG:25832, without zone prefix)
//! 19 5705335.366          north
//! 20 2026-07-01           date
//! ```
//!
//! AGS = fields 3+5+7+9 (`05` `9` `11` `000` → `05911000`).
//!
//! Encoding is detected per line: valid UTF-8 is taken as is, anything else
//! is decoded as Windows-1252 (older exports were Latin-1). Street names
//! containing a replacement character after decoding are rejected, so a
//! wrong guess can never leak `�` into the index.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead, BufReader, Read};
use std::path::Path;

use crate::config::GeoConfig;
use crate::index::{GeoIndex, HouseNo, Street};
use crate::normalize::{normalize_street, parse_house_number};

/// One street with its house numbers, as produced by a backend.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportedStreet {
    pub ags: String,
    pub street_key: String,
    pub name: String,
    pub name_norm: String,
    pub postcode: String,
    pub city: String,
    pub numbers: Vec<HouseNo>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ImportSummary {
    pub rows_scanned: u64,
    /// Rows in a configured AGS that produced an address.
    pub rows_kept: u64,
    pub streets: u64,
    pub addresses: u64,
    /// Rows in a configured AGS that could not be used (bad house number,
    /// bad coordinates, wrong zone, malformed line), with reasons.
    pub skipped: u64,
    /// Addresses dropped by [`ImportResult::retain_in_zones`] because no
    /// delivery zone covers them (only when the tenant has geometry zones).
    pub dropped_outside_zones: u64,
    pub skip_reasons: BTreeMap<String, u64>,
    /// `(lat_min, lat_max, lon_min, lon_max)` of the kept rows.
    pub bbox: Option<(f64, f64, f64, f64)>,
    pub centroid: Option<(f64, f64)>,
    /// `utf-8` or `windows-1252`.
    pub encoding: String,
    /// Configured AGS with no postcode mapping (city falls back to the
    /// cadastre's municipality name, postcode stays empty).
    pub unmapped_ags: BTreeSet<String>,
}

impl ImportSummary {
    /// The hard sanity checks: at least one address, and the centroid inside
    /// the expected box (catches a wrong AGS or a broken coordinate parse).
    pub fn check(&self, expect_bbox: (f64, f64, f64, f64)) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.rows_kept > 0,
            "no rows kept — check GEO_AGS against the export"
        );
        let (lat, lon) = self
            .centroid
            .ok_or_else(|| anyhow::anyhow!("no centroid"))?;
        let (lat_min, lat_max, lon_min, lon_max) = expect_bbox;
        anyhow::ensure!(
            lat >= lat_min && lat <= lat_max && lon >= lon_min && lon <= lon_max,
            "centroid {lat:.5},{lon:.5} outside the expected box {expect_bbox:?}"
        );
        Ok(())
    }

    pub fn report(&self) -> String {
        let mut s = format!(
            "rows scanned: {}\nrows kept:    {}\nstreets:      {}\naddresses:    {}\nskipped:      {}\nencoding:     {}\n",
            self.rows_scanned, self.rows_kept, self.streets, self.addresses, self.skipped, self.encoding
        );
        for (why, n) in &self.skip_reasons {
            s.push_str(&format!("  skipped[{why}] = {n}\n"));
        }
        if self.dropped_outside_zones > 0 {
            s.push_str(&format!(
                "outside delivery zones (not stored): {}\n",
                self.dropped_outside_zones
            ));
        }
        if let Some((a, b, c, d)) = self.bbox {
            s.push_str(&format!(
                "bbox:         lat {a:.5}..{b:.5}  lon {c:.5}..{d:.5}\n"
            ));
        }
        if let Some((lat, lon)) = self.centroid {
            s.push_str(&format!("centroid:     {lat:.5} N, {lon:.5} E\n"));
        }
        if !self.unmapped_ags.is_empty() {
            s.push_str(&format!(
                "WARNING: no GEO_POSTCODE_MAP entry for AGS {:?} — postcode left empty\n",
                self.unmapped_ags
            ));
        }
        s
    }
}

#[derive(Debug)]
pub struct ImportResult {
    pub streets: Vec<ImportedStreet>,
    pub summary: ImportSummary,
}

impl ImportResult {
    /// Fold another import (e.g. a second municipality file) into this one.
    pub fn merge(&mut self, other: ImportResult) {
        let a = &mut self.summary;
        let b = other.summary;
        a.rows_scanned += b.rows_scanned;
        a.rows_kept += b.rows_kept;
        a.skipped += b.skipped;
        for (k, v) in b.skip_reasons {
            *a.skip_reasons.entry(k).or_default() += v;
        }
        a.unmapped_ags.extend(b.unmapped_ags);
        if b.encoding == "windows-1252" {
            a.encoding = b.encoding;
        }
        let (na, nb) = (a.rows_kept - b.rows_kept, b.rows_kept);
        a.centroid = match (a.centroid, b.centroid) {
            (Some(x), Some(y)) if na + nb > 0 => Some((
                (x.0 * na as f64 + y.0 * nb as f64) / (na + nb) as f64,
                (x.1 * na as f64 + y.1 * nb as f64) / (na + nb) as f64,
            )),
            (x, None) => x,
            (None, y) => y,
            (x, _) => x,
        };
        a.bbox = match (a.bbox, b.bbox) {
            (Some(x), Some(y)) => Some((x.0.min(y.0), x.1.max(y.1), x.2.min(y.2), x.3.max(y.3))),
            (x, None) => x,
            (None, y) => y,
        };
        self.streets.extend(other.streets);
        a.streets = self.streets.len() as u64;
        a.addresses = self.streets.iter().map(|s| s.numbers.len() as u64).sum();
    }

    /// Keep only streets that some delivery zone covers, so streets the shop
    /// never delivers to are absent from the DB and the suggestions.
    ///
    /// A street is kept when at least one of its addresses lies in a zone —
    /// and then with ALL its numbers: a number outside the zones must stay
    /// resolvable so the checkout can refuse it exactly ("außerhalb") instead
    /// of guessing from the nearest known number. Only fully-outside streets
    /// vanish. No-op when the tenant has no geometry / default zone yet.
    /// Re-run the import after widening the delivery geometry.
    pub fn retain_in_zones(
        &mut self,
        zones: &[crate::zones::ZoneDef],
        overrides: &[crate::zones::StreetOverride],
    ) {
        let has_geometry = zones.iter().any(|z| {
            z.active
                && matches!(
                    z.kind,
                    crate::zones::GeometryKind::Polygon
                        | crate::zones::GeometryKind::Radius
                        | crate::zones::GeometryKind::Default
                )
        });
        if !has_geometry {
            return;
        }
        let mut dropped = 0u64;
        self.streets.retain(|st| {
            let covered = st.numbers.iter().any(|h| {
                crate::zones::zone_for(Some((h.lat, h.lon)), None, true, zones, overrides).is_some()
            });
            if !covered {
                dropped += st.numbers.len() as u64;
            }
            covered
        });
        self.summary.dropped_outside_zones = dropped;
        self.summary.rows_kept = self.summary.rows_kept.saturating_sub(dropped);
        self.summary.streets = self.streets.len() as u64;
        self.summary.addresses = self.streets.iter().map(|s| s.numbers.len() as u64).sum();
        // bbox / centroid over what is actually stored.
        let mut bbox: Option<(f64, f64, f64, f64)> = None;
        let (mut lat_sum, mut lon_sum, mut n) = (0.0, 0.0, 0u64);
        for h in self.streets.iter().flat_map(|s| s.numbers.iter()) {
            bbox = Some(match bbox {
                None => (h.lat, h.lat, h.lon, h.lon),
                Some(b) => (
                    b.0.min(h.lat),
                    b.1.max(h.lat),
                    b.2.min(h.lon),
                    b.3.max(h.lon),
                ),
            });
            lat_sum += h.lat;
            lon_sum += h.lon;
            n += 1;
        }
        self.summary.bbox = bbox;
        self.summary.centroid = (n > 0).then(|| (lat_sum / n as f64, lon_sum / n as f64));
    }

    /// Build the runtime index directly (no DB round-trip) — used by
    /// `--sample` in tests and by the import's own sanity checks.
    pub fn into_index(self) -> GeoIndex {
        let mut streets = Vec::with_capacity(self.streets.len());
        let mut numbers = std::collections::HashMap::new();
        for (i, s) in self.streets.into_iter().enumerate() {
            let id = i as u32 + 1;
            streets.push(Street {
                id,
                name: s.name,
                name_norm: s.name_norm,
                postcode: s.postcode,
                city: s.city,
            });
            numbers.insert(id, s.numbers);
        }
        GeoIndex::new(streets, numbers)
    }
}

/// A backend that can produce the tenant's street/address list.
pub trait AddressSource {
    /// `source` is backend-specific: for [`CadastreNrw`] the downloaded zip.
    fn build_index(&self, cfg: &GeoConfig, source: &Path) -> anyhow::Result<ImportResult>;
}

/// Gebäudereferenzen NRW.
pub struct CadastreNrw;

/// Download URL of the whole-NRW export (~100 MB zip, ~4.5 M rows).
pub const NRW_DOWNLOAD_URL: &str =
    "https://www.opengeodata.nrw.de/produkte/geobasis/lk/akt/gebref_txt/gebref_EPSG25832_ASCII.zip";

impl AddressSource for CadastreNrw {
    fn build_index(&self, cfg: &GeoConfig, source: &Path) -> anyhow::Result<ImportResult> {
        with_source_reader(source, |rd| CadastreNrw::import_reader(cfg, rd))
    }
}

/// Run `f` over the data stream of `source`, which is either the official
/// download (a zip holding `gebref.txt`) or a plain text file in the same
/// layout — the extracted `gebref.txt`, or one municipality file produced by
/// [`split_by_ags`]. Detected by the zip magic, not the extension.
pub fn with_source_reader<T>(
    source: &Path,
    f: impl FnOnce(&mut dyn Read) -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    use std::io::{Seek, SeekFrom};
    let mut file = std::fs::File::open(source)
        .map_err(|e| anyhow::anyhow!("open {}: {e}", source.display()))?;
    let mut magic = [0u8; 2];
    let n = file.read(&mut magic)?;
    file.seek(SeekFrom::Start(0))?;
    if n == 2 && &magic == b"PK" {
        let mut archive = zip::ZipArchive::new(file)
            .map_err(|e| anyhow::anyhow!("read zip {}: {e}", source.display()))?;
        // The data file: `gebref.txt`, or failing that the largest .txt.
        let name = {
            let mut best: Option<(String, u64)> = None;
            for i in 0..archive.len() {
                let f = archive.by_index(i)?;
                let n = f.name().to_string();
                let lower = n.to_ascii_lowercase();
                if lower.ends_with("gebref.txt") {
                    best = Some((n, u64::MAX));
                    break;
                }
                if lower.ends_with(".txt") && best.as_ref().is_none_or(|(_, sz)| f.size() > *sz) {
                    best = Some((n, f.size()));
                }
            }
            best.map(|(n, _)| n)
                .ok_or_else(|| anyhow::anyhow!("no .txt data file inside {}", source.display()))?
        };
        let mut entry = archive.by_name(&name)?;
        tracing::info!("reading {} from {}", name, source.display());
        f(&mut entry)
    } else {
        tracing::info!("reading plain text {}", source.display());
        f(&mut file)
    }
}

/// One municipality of the split index.
#[derive(Debug, Clone, PartialEq)]
pub struct MunicipalityInfo {
    pub ags: String,
    pub name: String,
    pub rows: u64,
    /// `(lat_min, lat_max, lon_min, lon_max)` of its addresses — `None` for an
    /// index written by an older `split` without coordinates.
    pub bbox: Option<(f64, f64, f64, f64)>,
    pub centroid: Option<(f64, f64)>,
}

impl MunicipalityInfo {
    /// Could any address of this municipality lie within `metres` of the
    /// point? Cheap bbox test (over-selects; scan the file to confirm).
    pub fn may_reach(&self, lat: f64, lon: f64, metres: f64) -> bool {
        let Some((lat_min, lat_max, lon_min, lon_max)) = self.bbox else {
            return true;
        };
        let clat = lat.clamp(lat_min, lat_max);
        let clon = lon.clamp(lon_min, lon_max);
        crate::zones::haversine_m((lat, lon), (clat, clon)) <= metres
    }
}

/// Result of [`split_by_ags`].
#[derive(Debug, Default, Clone, PartialEq)]
pub struct SplitSummary {
    pub rows: u64,
    /// AGS → municipality (name, rows, bbox, centroid).
    pub municipalities: BTreeMap<String, MunicipalityInfo>,
}

/// Read `<out_dir>/index.csv` written by [`split_by_ags`]. Tolerates the
/// older 3-column layout (no coordinates).
pub fn read_split_index(out_dir: &Path) -> anyhow::Result<Vec<MunicipalityInfo>> {
    let path = out_dir.join("index.csv");
    let text = std::fs::read_to_string(&path)
        .map_err(|e| anyhow::anyhow!("read {}: {e}", path.display()))?;
    let mut out = Vec::new();
    for line in text.lines().skip(1) {
        let f: Vec<&str> = line.split(';').collect();
        if f.len() < 3 {
            continue;
        }
        let num = |i: usize| f.get(i).and_then(|v| v.trim().parse::<f64>().ok());
        let bbox = match (num(3), num(4), num(5), num(6)) {
            (Some(a), Some(b), Some(c), Some(d)) => Some((a, b, c, d)),
            _ => None,
        };
        let centroid = match (num(7), num(8)) {
            (Some(a), Some(b)) => Some((a, b)),
            _ => None,
        };
        out.push(MunicipalityInfo {
            ags: f[0].trim().to_string(),
            name: f[1].trim().to_string(),
            rows: f[2].trim().parse().unwrap_or(0),
            bbox,
            centroid,
        });
    }
    Ok(out)
}

/// Cut the whole-NRW export into one file per municipality
/// (`<out_dir>/<AGS>.txt`, same line layout, plus `<out_dir>/index.csv` with
/// `ags;name;rows;lat_min;lat_max;lon_min;lon_max;lat_c;lon_c` — the bounding
/// box and centroid let a shop find the municipalities within its delivery
/// radius without opening every file). One pass over the ~4.5 M rows (≈15 s); afterwards a
/// tenant import reads its handful of small files in milliseconds, and a new
/// shop anywhere in NRW can be set up without the 100 MB download.
pub fn split_by_ags(source: &Path, out_dir: &Path) -> anyhow::Result<SplitSummary> {
    use std::collections::HashMap;
    use std::fs::{File, OpenOptions};
    use std::io::{BufWriter, Write};

    std::fs::create_dir_all(out_dir)?;
    with_source_reader(source, |rd| {
        let mut summary = SplitSummary::default();
        // Keep at most this many files open (macOS defaults to 256 fds);
        // beyond it everything is flushed + closed and reopened in append
        // mode on demand.
        const MAX_OPEN: usize = 64;
        let mut open: HashMap<String, BufWriter<File>> = HashMap::new();
        let mut started: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        let mut buf = BufReader::with_capacity(1 << 16, rd);
        let mut raw = Vec::with_capacity(256);
        loop {
            raw.clear();
            let n = buf.read_until(b'\n', &mut raw)?;
            if n == 0 {
                break;
            }
            let line_end = raw.len()
                - raw
                    .iter()
                    .rev()
                    .take_while(|b| **b == b'\n' || **b == b'\r')
                    .count();
            if line_end == 0 {
                continue;
            }
            // AGS + name from the raw bytes — the fields are ASCII digits
            // except the name, which we copy verbatim.
            let f: Vec<&[u8]> = raw[..line_end].split(|b| *b == b';').take(20).collect();
            if f.len() < 20 {
                continue;
            }
            let ags = format!(
                "{}{}{}{}",
                String::from_utf8_lossy(f[3]),
                String::from_utf8_lossy(f[5]),
                String::from_utf8_lossy(f[7]),
                String::from_utf8_lossy(f[9])
            );
            if ags.len() != 8 || !ags.bytes().all(|b| b.is_ascii_digit()) {
                continue;
            }
            let name = String::from_utf8_lossy(f[10]).trim().to_string();
            if !open.contains_key(&ags) {
                if open.len() >= MAX_OPEN {
                    for (_, mut w) in open.drain() {
                        w.flush()?;
                    }
                }
                let path = out_dir.join(format!("{ags}.txt"));
                let file = if started.insert(ags.clone()) {
                    File::create(&path)?
                } else {
                    OpenOptions::new().append(true).open(&path)?
                };
                open.insert(ags.clone(), BufWriter::with_capacity(1 << 15, file));
            }
            let w = open.get_mut(&ags).expect("just opened");
            w.write_all(&raw[..line_end])?;
            w.write_all(b"\r\n")?;
            summary.rows += 1;
            let e = summary
                .municipalities
                .entry(ags)
                .or_insert_with(|| MunicipalityInfo {
                    ags: String::new(),
                    name,
                    rows: 0,
                    bbox: None,
                    centroid: None,
                });
            e.rows += 1;
            // Running bbox + centroid from the UTM coordinates (zone 32 only).
            let east = std::str::from_utf8(f[18])
                .ok()
                .and_then(|v| v.trim().parse::<f64>().ok());
            let north = std::str::from_utf8(f[19])
                .ok()
                .and_then(|v| v.trim().parse::<f64>().ok());
            if let (Some(east), Some(north)) = (east, north) {
                if f[17].trim_ascii() == b"32" {
                    let (lat, lon) = crate::utm::utm32n_to_wgs84(east, north);
                    e.bbox = Some(match e.bbox {
                        None => (lat, lat, lon, lon),
                        Some(b) => (b.0.min(lat), b.1.max(lat), b.2.min(lon), b.3.max(lon)),
                    });
                    let n = e.rows as f64;
                    e.centroid = Some(match e.centroid {
                        None => (lat, lon),
                        Some((a, b)) => (a + (lat - a) / n, b + (lon - b) / n),
                    });
                }
            }
        }
        for (_, mut w) in open.drain() {
            w.flush()?;
        }
        for (ags, m) in summary.municipalities.iter_mut() {
            m.ags = ags.clone();
        }
        let mut index = BufWriter::new(File::create(out_dir.join("index.csv"))?);
        writeln!(
            index,
            "ags;name;rows;lat_min;lat_max;lon_min;lon_max;lat_c;lon_c"
        )?;
        for (ags, m) in &summary.municipalities {
            let (a, b, c, d) = m.bbox.unwrap_or((0.0, 0.0, 0.0, 0.0));
            let (lc, nc) = m.centroid.unwrap_or((0.0, 0.0));
            writeln!(
                index,
                "{ags};{};{};{a:.5};{b:.5};{c:.5};{d:.5};{lc:.5};{nc:.5}",
                m.name, m.rows
            )?;
        }
        index.flush()?;
        Ok(summary)
    })
}

/// File name of the cached whole-NRW download inside the data dir.
pub const NRW_ZIP_NAME: &str = "gebref_EPSG25832_ASCII.zip";

/// `<data_dir>/gebref_EPSG25832_ASCII.zip`.
pub fn nrw_zip_path(data_dir: &Path) -> std::path::PathBuf {
    data_dir.join(NRW_ZIP_NAME)
}

/// `<split_dir>/index.csv` exists and every row carries a bounding box —
/// i.e. it was written by a `split_by_ags` that knows about coordinates.
/// An older 3-column index still works for the import but not for the
/// "municipalities within a radius" search.
pub fn split_index_current(out_dir: &Path) -> bool {
    match read_split_index(out_dir) {
        Ok(v) => !v.is_empty() && v.iter().all(|m| m.bbox.is_some()),
        Err(_) => false,
    }
}

/// Fetch the official export into `dest` unless a plausible file (> 1 MB)
/// is already there (`force` refetches). `progress` receives a short
/// status line now and then. Returns the size on disk and whether the file
/// was actually fetched.
pub fn download_nrw(
    dest: &Path,
    force: bool,
    progress: &mut dyn FnMut(String),
) -> anyhow::Result<(u64, bool)> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if !force && dest.is_file() {
        let len = std::fs::metadata(dest)?.len();
        if len > 1_000_000 {
            progress(format!(
                "using cached download {} ({} MB)",
                dest.display(),
                len / 1_000_000
            ));
            return Ok((len, false));
        }
    }
    progress(format!(
        "downloading {NRW_DOWNLOAD_URL} → {}",
        dest.display()
    ));
    let resp = ureq::get(NRW_DOWNLOAD_URL)
        .timeout(std::time::Duration::from_secs(1800))
        .call()
        .map_err(|e| anyhow::anyhow!("download: {e}"))?;
    let total: Option<u64> = resp
        .header("Content-Length")
        .and_then(|v| v.trim().parse().ok());
    let tmp = dest.with_extension("zip.part");
    let mut out = std::io::BufWriter::new(std::fs::File::create(&tmp)?);
    let mut rd = resp.into_reader();
    let mut buf = vec![0u8; 1 << 16];
    let mut n: u64 = 0;
    let mut next_report: u64 = 5_000_000;
    loop {
        let k = rd.read(&mut buf)?;
        if k == 0 {
            break;
        }
        std::io::Write::write_all(&mut out, &buf[..k])?;
        n += k as u64;
        if n >= next_report {
            next_report += 5_000_000;
            progress(match total {
                Some(t) if t > 0 => format!(
                    "\rdownloaded {} / {} MB ({}%)",
                    n / 1_000_000,
                    t / 1_000_000,
                    n * 100 / t
                ),
                _ => format!("\rdownloaded {} MB", n / 1_000_000),
            });
        }
    }
    std::io::Write::flush(&mut out)?;
    drop(out);
    anyhow::ensure!(
        n > 1_000_000,
        "download too small ({n} bytes) — not the cadastre export"
    );
    std::fs::rename(&tmp, dest)?;
    progress(format!("downloaded {} MB", n / 1_000_000));
    Ok((n, true))
}

/// What the dashboard button "NRW-Daten herunterladen & aufbereiten" runs:
/// the (cached) download into `data_dir`, then [`split_by_ags`] into
/// `<data_dir>/nrw`. Re-running is safe: the split rewrites every
/// municipality file and the index.
pub fn prepare_nrw(
    data_dir: &Path,
    force_download: bool,
    progress: &mut dyn FnMut(String),
) -> anyhow::Result<SplitSummary> {
    let zip = nrw_zip_path(data_dir);
    download_nrw(&zip, force_download, progress)?;
    let out = data_dir.join("nrw");
    progress(format!(
        "splitting into one file per municipality → {} (a minute or so)",
        out.display()
    ));
    let t0 = std::time::Instant::now();
    let s = split_by_ags(&zip, &out)?;
    progress(format!(
        "split {} rows into {} municipality files in {:.0} s",
        s.rows,
        s.municipalities.len(),
        t0.elapsed().as_secs_f64()
    ));
    Ok(s)
}

impl CadastreNrw {
    /// Parse the data file from any reader (the zip entry, a fixture, a
    /// test buffer). Streams line by line; never holds the file in memory.
    pub fn import_reader<R: Read>(cfg: &GeoConfig, reader: R) -> anyhow::Result<ImportResult> {
        let wanted: BTreeSet<&str> = cfg.ags.iter().map(String::as_str).collect();
        let mut summary = ImportSummary {
            encoding: "utf-8".into(),
            ..Default::default()
        };
        // (ags, street_key) → street; BTreeMap for a deterministic order.
        let mut streets: BTreeMap<(String, String), ImportedStreet> = BTreeMap::new();
        let mut seen: BTreeSet<(String, String, u32, String)> = BTreeSet::new();
        let (mut lat_sum, mut lon_sum) = (0.0_f64, 0.0_f64);
        let mut bbox: Option<(f64, f64, f64, f64)> = None;

        let mut rd = BufReader::with_capacity(1 << 16, reader);
        let mut raw = Vec::with_capacity(256);
        loop {
            raw.clear();
            let n = rd.read_until(b'\n', &mut raw)?;
            if n == 0 {
                break;
            }
            while raw.last().is_some_and(|b| *b == b'\n' || *b == b'\r') {
                raw.pop();
            }
            if raw.is_empty() {
                continue;
            }
            summary.rows_scanned += 1;
            let line: std::borrow::Cow<str> = match std::str::from_utf8(&raw) {
                Ok(s) => std::borrow::Cow::Borrowed(s),
                Err(_) => {
                    summary.encoding = "windows-1252".into();
                    let (decoded, _, _) = encoding_rs::WINDOWS_1252.decode(&raw);
                    std::borrow::Cow::Owned(decoded.into_owned())
                }
            };
            let f: Vec<&str> = line.split(';').collect();
            if f.len() < 20 {
                // Not a data row (header, footer, garbage). Only counts as a
                // skip if it looks like it was meant to be one.
                if f.len() > 5 {
                    skip(&mut summary, "malformed line");
                }
                continue;
            }
            let ags = format!("{}{}{}{}", f[3], f[5], f[7], f[9]);
            if !wanted.contains(ags.as_str()) {
                continue;
            }
            if f[17].trim() != "32" {
                skip(&mut summary, "utm zone != 32");
                continue;
            }
            let street_name = f[14].trim();
            if street_name.is_empty() || street_name.contains('\u{FFFD}') {
                skip(&mut summary, "bad street name");
                continue;
            }
            let Some((number, suffix)) =
                parse_house_number(&format!("{}{}", f[15].trim(), f[16].trim()))
            else {
                skip(&mut summary, "unparseable house number");
                continue;
            };
            let (Ok(east), Ok(north)) = (f[18].trim().parse::<f64>(), f[19].trim().parse::<f64>())
            else {
                skip(&mut summary, "bad coordinates");
                continue;
            };
            let (lat, lon) = crate::utm::utm32n_to_wgs84(east, north);
            if !lat.is_finite() || !lon.is_finite() {
                skip(&mut summary, "bad coordinates");
                continue;
            }
            let street_key = f[13].trim().to_string();
            let key = (ags.clone(), street_key.clone(), number, suffix.clone());
            if !seen.insert(key) {
                // Duplicate building record for one address: keep the first.
                continue;
            }
            let (postcode, city) = match cfg.postcode_city(&ags) {
                Some((p, c)) => (p.clone(), c.clone()),
                None => {
                    summary.unmapped_ags.insert(ags.clone());
                    (String::new(), f[10].trim().to_string())
                }
            };
            let entry = streets
                .entry((ags.clone(), street_key.clone()))
                .or_insert_with(|| ImportedStreet {
                    ags: ags.clone(),
                    street_key,
                    name: street_name.to_string(),
                    name_norm: normalize_street(street_name),
                    postcode,
                    city,
                    numbers: Vec::new(),
                });
            entry.numbers.push(HouseNo {
                number,
                suffix,
                lat,
                lon,
            });
            summary.rows_kept += 1;
            lat_sum += lat;
            lon_sum += lon;
            bbox = Some(match bbox {
                None => (lat, lat, lon, lon),
                Some(b) => (b.0.min(lat), b.1.max(lat), b.2.min(lon), b.3.max(lon)),
            });
        }

        let mut streets: Vec<ImportedStreet> = streets.into_values().collect();
        for s in &mut streets {
            s.numbers
                .sort_by(|a, b| a.number.cmp(&b.number).then(a.suffix.cmp(&b.suffix)));
        }
        summary.streets = streets.len() as u64;
        summary.addresses = streets.iter().map(|s| s.numbers.len() as u64).sum();
        summary.bbox = bbox;
        if summary.rows_kept > 0 {
            let n = summary.rows_kept as f64;
            summary.centroid = Some((lat_sum / n, lon_sum / n));
        }
        Ok(ImportResult { streets, summary })
    }
}

fn skip(summary: &mut ImportSummary, why: &str) {
    summary.skipped += 1;
    *summary.skip_reasons.entry(why.to_string()).or_default() += 1;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sample;

    #[test]
    fn fixture_utf8_two_ags_only_configured_kept() {
        let cfg = sample::config();
        let r = CadastreNrw::import_reader(&cfg, sample::SAMPLE_UTF8.as_bytes()).unwrap();
        let s = &r.summary;
        assert_eq!(s.encoding, "utf-8");
        assert_eq!(s.rows_scanned, sample::ROWS_TOTAL);
        assert_eq!(s.rows_kept, sample::ROWS_KEPT_LH);
        assert_eq!(s.skipped, 1, "{:?}", s.skip_reasons);
        assert_eq!(s.skip_reasons.get("unparseable house number"), Some(&1));
        assert!(r.streets.iter().all(|st| st.ags == "05558024"));
        assert_eq!(s.streets, sample::STREETS_LH);
        // Umlauts survived and are normalised.
        let hl = r
            .streets
            .iter()
            .find(|st| st.name == "Hermann-Löns-Straße")
            .unwrap();
        assert_eq!(hl.name_norm, "hermann loens str");
        assert_eq!(hl.postcode, "59348");
        assert_eq!(hl.city, "Lüdinghausen");
        // A suffix row is kept as (number, suffix).
        let bh = r
            .streets
            .iter()
            .find(|st| st.name == "Bahnhofstraße")
            .unwrap();
        assert!(bh.numbers.iter().any(|h| h.number == 12 && h.suffix == "a"));
        s.check(cfg.expect_bbox).unwrap();
    }

    #[test]
    fn fixture_windows1252_decodes_umlauts() {
        let cfg = sample::config();
        let bytes = sample::sample_windows1252();
        let r = CadastreNrw::import_reader(&cfg, bytes.as_slice()).unwrap();
        assert_eq!(r.summary.encoding, "windows-1252");
        assert!(r.streets.iter().any(|st| st.name == "Hermann-Löns-Straße"));
        assert!(r.streets.iter().any(|st| st.name == "Mühlenstraße"));
        assert!(r.streets.iter().all(|st| !st.name.contains('\u{FFFD}')));
        assert_eq!(r.summary.rows_kept, sample::ROWS_KEPT_LH);
    }

    #[test]
    fn zip_path_streams_the_data_file() {
        let cfg = sample::config();
        let dir = std::env::temp_dir().join(format!("rusterando-geo-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("gebref_fixture.zip");
        std::fs::write(&path, sample::sample_zip()).unwrap();
        let r = CadastreNrw.build_index(&cfg, &path).unwrap();
        assert_eq!(r.summary.rows_kept, sample::ROWS_KEPT_LH);
        let idx = r.into_index();
        assert_eq!(idx.suggest_streets("bahnhof")[0].name, "Bahnhofstraße");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn plain_text_source_and_split_round_trip() {
        let cfg = sample::config();
        let dir = std::env::temp_dir().join(format!("rusterando-geo-split-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // Plain text source (the extracted gebref.txt shape).
        let txt = dir.join("gebref.txt");
        std::fs::write(&txt, sample::SAMPLE_UTF8).unwrap();
        let r = CadastreNrw.build_index(&cfg, &txt).unwrap();
        assert_eq!(r.summary.rows_kept, sample::ROWS_KEPT_LH);
        // Split into per-municipality files.
        let out = dir.join("ags");
        let s = split_by_ags(&txt, &out).unwrap();
        assert_eq!(s.rows, sample::ROWS_TOTAL);
        assert_eq!(s.municipalities.len(), 2);
        assert_eq!(s.municipalities["05558024"].name, "Lüdinghausen");
        assert_eq!(
            s.municipalities["05558024"].rows,
            sample::ROWS_KEPT_LH + 1,
            "incl. the malformed row"
        );
        assert_eq!(s.municipalities["05558028"].rows, sample::ROWS_KEPT_NK);
        let idx = read_split_index(&out).unwrap();
        assert_eq!(idx.len(), 2);
        assert!(split_index_current(&out));
        assert!(!split_index_current(&dir.join("nowhere")));
        let lh = idx.iter().find(|m| m.ags == "05558024").unwrap();
        let (lat_min, lat_max, lon_min, lon_max) = lh.bbox.unwrap();
        assert!(
            lat_min > 51.7 && lat_max < 51.8 && lon_min > 7.3 && lon_max < 7.5,
            "{:?}",
            lh.bbox
        );
        assert!(lh.may_reach(51.768, 7.434, 500.0));
        assert!(!lh.may_reach(50.9, 6.9, 5_000.0), "Köln is not within 5 km");
        assert!(idx
            .iter()
            .find(|m| m.ags == "05558028")
            .unwrap()
            .centroid
            .is_some());
        // A split file imports on its own and yields the same result.
        let r2 = CadastreNrw
            .build_index(&cfg, &out.join("05558024.txt"))
            .unwrap();
        assert_eq!(r2.summary.rows_kept, sample::ROWS_KEPT_LH);
        assert_eq!(r2.summary.streets, sample::STREETS_LH);
        // The other municipality's file contains none of ours.
        let r3 = CadastreNrw
            .build_index(&cfg, &out.join("05558028.txt"))
            .unwrap();
        assert_eq!(r3.summary.rows_kept, 0);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn both_ags_when_configured() {
        let mut cfg = sample::config();
        cfg.ags.push("05558028".into());
        let r = CadastreNrw::import_reader(&cfg, sample::SAMPLE_UTF8.as_bytes()).unwrap();
        assert_eq!(
            r.summary.rows_kept,
            sample::ROWS_KEPT_LH + sample::ROWS_KEPT_NK
        );
        assert!(
            r.summary.unmapped_ags.contains("05558028"),
            "second AGS has no postcode map"
        );
    }

    #[test]
    fn retain_in_zones_drops_uncovered_addresses() {
        use crate::zones::{GeometryKind, ZoneDef};
        let cfg = sample::config();
        let mut r = CadastreNrw::import_reader(&cfg, sample::SAMPLE_UTF8.as_bytes()).unwrap();
        let before = r.summary.addresses;
        // Nothing to filter by → untouched.
        r.retain_in_zones(&[], &[]);
        assert_eq!(r.summary.addresses, before);
        // A 1.5 km circle around the town centre: Berenbrock (far out) and
        // Olfener Straße (south-west) fall outside.
        let centre = ZoneDef {
            id: "c".into(),
            name: "Centre".into(),
            priority: 10,
            fee_cents: 150,
            min_order_cents: None,
            eta_minutes: 30,
            active: true,
            kind: GeometryKind::Radius,
            geometry: crate::zones::parse_geometry(
                GeometryKind::Radius,
                Some("51.7680,7.4336,1500"),
                None,
            )
            .unwrap(),
        };
        r.retain_in_zones(&[centre], &[]);
        assert!(r.summary.dropped_outside_zones > 0);
        assert_eq!(
            r.summary.addresses + r.summary.dropped_outside_zones,
            before
        );
        assert!(
            !r.streets.iter().any(|s| s.name == "Berenbrock"),
            "far street removed entirely"
        );
        assert!(
            r.streets.iter().any(|s| s.name == "Am Markt"),
            "centre street kept"
        );
        assert_eq!(r.summary.streets, r.streets.len() as u64);
        // A street that is only partly inside keeps every number (3 of
        // Bahnhofstraße's 13 lie beyond 1.5 km), so an outside number is
        // still resolvable and gets refused exactly at checkout.
        let bh = r
            .streets
            .iter()
            .find(|s| s.name == "Bahnhofstraße")
            .unwrap();
        assert_eq!(bh.numbers.len(), 13);
    }

    #[test]
    fn empty_or_wrong_ags_fails_check() {
        let mut cfg = sample::config();
        cfg.ags = vec!["05111000".into()];
        let r = CadastreNrw::import_reader(&cfg, sample::SAMPLE_UTF8.as_bytes()).unwrap();
        assert!(r.summary.check(cfg.expect_bbox).is_err());
    }
}
