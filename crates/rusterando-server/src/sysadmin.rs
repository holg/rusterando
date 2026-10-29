//! Sys-admin operations: the server half of `rusterando_frontend::pages::sysadmin`.
//!
//! Runs the SAME two tools the SSH onboarding workflow uses, as child
//! processes from the service's working directory, and streams their output
//! into an in-memory job the browser polls:
//!
//!   1. `SCRAPER_PYTHON SCRAPER_SCRIPT <url> -o tmp/imports/<job>.json`
//!      (Playwright Chromium inside the scraper's venv → menu + address JSON)
//!   2. `MENU_IMPORT_BIN --parent <apex> --shop <slug> --menu <json> --set …`
//!      (.env + sqlite + menu + meta; fresh staff passwords via `--set`)
//!   3. `Tenants::load_by_slug(slug)` — hot-load, no restart, no ADMIN_TOKEN.
//!
//! Env (all optional, defaults match the production layout):
//!   SCRAPER_PYTHON   rusterando-scrape/env_play_pdf_py12/bin/python (else python3)
//!   SCRAPER_SCRIPT   rusterando-scrape/scrape_lieferando.py
//!   MENU_IMPORT_BIN  ./menu-import-linux-x64, else target/{release,debug}/menu-import
//!   IMPORT_TMP_DIR   tmp/imports (scratch while Chromium runs)
//!   MENU_JSON_DIR    rusterando-scrape/menu-json — the finished scrape is moved
//!                    here as menu-<slug>.json (same as the CLI) and imported
//!                    from there, so it can be hand-edited before provisioning
//!
//! One job at a time (a scrape holds Chromium + a few hundred MB); jobs are
//! kept in memory (last 20) — the durable record is the import stamp
//! menu-import writes into the tenant's app_settings.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use rusterando_frontend::pages::sysadmin::{
    CategoryBenchmark, ImportJob, ImportPreview, JobState, ProvisionResult, SysAdminOps,
    TenantInfo, ZoneInfo, ZoneStats,
};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::RwLock;

use crate::tenant::{slug_from_filename, valid_slug, Tenants};

const SCRAPE_TIMEOUT: Duration = Duration::from_secs(240);
const IMPORT_TIMEOUT: Duration = Duration::from_secs(180);
const LOG_CAP: usize = 500;
const JOBS_KEPT: usize = 20;

#[derive(Debug, Clone)]
pub struct SysAdminConfig {
    pub python: PathBuf,
    pub script: PathBuf,
    pub menu_import: PathBuf,
    pub tmp_dir: PathBuf,
    pub menu_json_dir: PathBuf,
    /// Parent/apex slug (`rusterando`) — `.env.<parent>.<slug>` naming.
    pub parent: String,
    /// Parent domain (`rusterando.de`) — tenants become `<slug>.<domain>`.
    pub domain: String,
}

impl SysAdminConfig {
    pub fn from_env() -> Self {
        fn first_existing(cands: &[&str], fallback: &str) -> PathBuf {
            cands
                .iter()
                .map(PathBuf::from)
                .find(|p| p.exists())
                .unwrap_or_else(|| PathBuf::from(fallback))
        }
        let python = std::env::var("SCRAPER_PYTHON")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                first_existing(
                    &["rusterando-scrape/env_play_pdf_py12/bin/python"],
                    "python3",
                )
            });
        let script = std::env::var("SCRAPER_SCRIPT")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("rusterando-scrape/scrape_lieferando.py"));
        let menu_import = std::env::var("MENU_IMPORT_BIN")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                first_existing(
                    &[
                        "./menu-import-linux-x64",
                        "./target/release/menu-import",
                        "./target/debug/menu-import",
                    ],
                    "menu-import",
                )
            });
        let tmp_dir = std::env::var("IMPORT_TMP_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("tmp/imports"));
        let menu_json_dir = std::env::var("MENU_JSON_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("rusterando-scrape/menu-json"));
        let parent = {
            let s = slug_from_filename(&std::env::var("ENV_FILE").unwrap_or_default());
            if s.is_empty() || s == ".env" {
                "rusterando".to_string()
            } else {
                s
            }
        };
        let domain = std::env::var("PUBLIC_URL")
            .ok()
            .and_then(|u| {
                let no_scheme = u.split("://").nth(1).unwrap_or(&u).to_string();
                let host = no_scheme
                    .split('/')
                    .next()
                    .unwrap_or("")
                    .trim_start_matches("www.");
                (!host.is_empty()).then(|| host.to_string())
            })
            .unwrap_or_else(|| format!("{parent}.de"));
        Self {
            python,
            script,
            menu_import,
            tmp_dir,
            menu_json_dir,
            parent,
            domain,
        }
    }
}

pub struct SysAdmin {
    tenants: Tenants,
    apex_slug: String,
    cfg: SysAdminConfig,
    jobs: RwLock<HashMap<String, ImportJob>>,
    /// Insertion order for pruning.
    order: RwLock<Vec<String>>,
    busy: AtomicBool,
}

impl SysAdmin {
    pub fn new(tenants: Tenants, apex_slug: String) -> Arc<Self> {
        let cfg = SysAdminConfig::from_env();
        tracing::info!(
            python = %cfg.python.display(),
            script = %cfg.script.display(),
            menu_import = %cfg.menu_import.display(),
            parent = %cfg.parent,
            domain = %cfg.domain,
            "sysadmin: import tooling resolved"
        );
        Arc::new(Self {
            tenants,
            apex_slug,
            cfg,
            jobs: RwLock::new(HashMap::new()),
            order: RwLock::new(Vec::new()),
            busy: AtomicBool::new(false),
        })
    }

    fn json_path(&self, id: &str) -> PathBuf {
        self.cfg.tmp_dir.join(format!("{id}.json"))
    }

    async fn push_log(jobs: &RwLock<HashMap<String, ImportJob>>, id: &str, line: String) {
        // Child tools may emit ANSI colour codes (tracing's fmt layer defaults
        // to ANSI even on a pipe); strip them + any other control chars so the
        // job JSON stays clean and the <pre> readable.
        let line = strip_ansi(&line);
        let mut map = jobs.write().await;
        if let Some(j) = map.get_mut(id) {
            if j.log.len() >= LOG_CAP {
                j.log.remove(0);
            }
            j.log.push(line);
        }
    }

    /// Mark the job failed. The child's LAST OUTPUT LINES are appended to the
    /// error and written to the journal: a bare "Prozess endete mit exit
    /// status: 1" told neither the operator nor the log reader anything, and
    /// the real cause ("0 items scraped") was only visible in the in-memory
    /// job log.
    async fn fail(&self, id: &str, msg: String) {
        let tail: Vec<String> = {
            let jobs = self.jobs.read().await;
            jobs.get(id)
                .map(|j| j.log.iter().rev().take(4).rev().cloned().collect())
                .unwrap_or_default()
        };
        if tail.is_empty() {
            tracing::warn!("sysadmin job {id}: {msg}");
        } else {
            tracing::warn!(
                "sysadmin job {id}: {msg} — letzte Ausgabe: {}",
                tail.join(" | ")
            );
        }
        let full = if tail.is_empty() {
            msg
        } else {
            format!("{msg}\n{}", tail.join("\n"))
        };
        if let Some(j) = self.jobs.write().await.get_mut(id) {
            j.state = JobState::Failed;
            j.error = Some(full);
        }
        self.busy.store(false, Ordering::SeqCst);
    }

    /// Spawn `cmd`, stream stdout+stderr lines into the job log, enforce a
    /// timeout (kill on expiry) and require exit 0.
    async fn run_logged(
        self: &Arc<Self>,
        id: &str,
        mut cmd: tokio::process::Command,
        timeout: Duration,
    ) -> Result<(), String> {
        let shown = format!("$ {:?}", cmd.as_std());
        Self::push_log(&self.jobs, id, shown).await;
        cmd.stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = cmd
            .spawn()
            .map_err(|e| format!("Start fehlgeschlagen: {e}"))?;
        let mut pumps = Vec::new();
        if let Some(out) = child.stdout.take() {
            let me = Arc::clone(self);
            let id = id.to_string();
            pumps.push(tokio::spawn(async move {
                let mut lines = BufReader::new(out).lines();
                while let Ok(Some(l)) = lines.next_line().await {
                    Self::push_log(&me.jobs, &id, l).await;
                }
            }));
        }
        if let Some(err) = child.stderr.take() {
            let me = Arc::clone(self);
            let id = id.to_string();
            pumps.push(tokio::spawn(async move {
                let mut lines = BufReader::new(err).lines();
                while let Ok(Some(l)) = lines.next_line().await {
                    Self::push_log(&me.jobs, &id, l).await;
                }
            }));
        }
        let status = match tokio::time::timeout(timeout, child.wait()).await {
            Ok(Ok(st)) => st,
            Ok(Err(e)) => return Err(format!("Prozessfehler: {e}")),
            Err(_) => {
                let _ = child.kill().await;
                return Err(format!("Timeout nach {} s", timeout.as_secs()));
            }
        };
        for p in pumps {
            let _ = p.await;
        }
        if status.success() {
            Ok(())
        } else {
            Err(format!("Prozess endete mit {status}"))
        }
    }

    /// Build the preview out of the scraper's JSON (serde_json::Value — the
    /// importer owns the strict schema; here we only read what's shown).
    async fn preview_from_json(
        &self,
        path: &Path,
        slug_override: Option<&str>,
    ) -> Result<ImportPreview, String> {
        let raw = tokio::fs::read_to_string(path)
            .await
            .map_err(|e| format!("JSON nicht lesbar ({}): {e}", path.display()))?;
        let v: serde_json::Value =
            serde_json::from_str(&raw).map_err(|e| format!("JSON ungültig: {e}"))?;
        let s = |p: &[&str]| -> String {
            let mut cur = &v;
            for k in p {
                cur = cur.get(*k).unwrap_or(&serde_json::Value::Null);
            }
            cur.as_str().unwrap_or("").trim().to_string()
        };
        let f = |p: &[&str]| -> Option<f64> {
            let mut cur = &v;
            for k in p {
                cur = cur.get(*k).unwrap_or(&serde_json::Value::Null);
            }
            cur.as_f64().filter(|x| *x != 0.0)
        };
        let source_slug = s(&["restaurant", "slug"]);
        let name = s(&["restaurant", "name"]);
        let slug = slug_override
            .map(|x| x.to_string())
            .filter(|x| !x.is_empty())
            .unwrap_or_else(|| {
                slugify(if source_slug.is_empty() {
                    &name
                } else {
                    &source_slug
                })
            });
        let mut categories = Vec::new();
        let mut n_items = 0usize;
        let mut n_sizes = 0usize;
        let mut labels: Vec<String> = Vec::new();
        for cat in v
            .get("categories")
            .and_then(|c| c.as_array())
            .into_iter()
            .flatten()
        {
            let items = cat
                .get("items")
                .and_then(|i| i.as_array())
                .cloned()
                .unwrap_or_default();
            let name = cat
                .get("name")
                .and_then(|n| n.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            if items.is_empty() {
                continue;
            }
            categories.push((name, items.len()));
            n_items += items.len();
            for it in &items {
                for sz in it
                    .get("sizes")
                    .and_then(|x| x.as_array())
                    .into_iter()
                    .flatten()
                {
                    n_sizes += 1;
                    let l = sz
                        .get("label")
                        .and_then(|x| x.as_str())
                        .unwrap_or("")
                        .trim()
                        .to_string();
                    if !l.is_empty() && !labels.contains(&l) && labels.len() < 8 {
                        labels.push(l);
                    }
                }
            }
        }
        if n_items == 0 {
            return Err("Scrape enthält keine Artikel".to_string());
        }
        let open_days = v
            .get("meta")
            .and_then(|m| m.get("hours"))
            .and_then(|h| h.as_object())
            .map(|h| {
                h.values()
                    .filter(|d| d.as_array().map(|a| !a.is_empty()).unwrap_or(false))
                    .count()
            })
            .unwrap_or(0);
        let postcode = s(&["meta", "postcode"]);
        let country = s(&["meta", "country"]);
        let nrw_geo = {
            let c = country.to_ascii_lowercase();
            (c.is_empty() || c == "de" || c == "deutschland" || c == "germany")
                && postcode
                    .parse::<u32>()
                    .map(|p| matches!(p, 32000..=33999 | 40000..=53999 | 57000..=59999))
                    .unwrap_or(false)
        };
        let existing_tenant = self.tenants.get(&slug).await.is_some();
        Ok(ImportPreview {
            name,
            slug,
            source_slug,
            address_street: s(&["meta", "address_street"]),
            postcode,
            city: s(&["meta", "city"]),
            country,
            lat: f(&["meta", "lat"]),
            lon: f(&["meta", "lon"]),
            phone: s(&["meta", "phone"]),
            owner: s(&["meta", "owner"]),
            open_days,
            categories,
            n_items,
            n_sizes,
            size_labels: labels,
            existing_tenant,
            nrw_geo,
        })
    }

    fn env_file_for(&self, slug: &str) -> String {
        format!(".env.{}.{slug}", self.cfg.parent)
    }

    async fn run_scrape(self: Arc<Self>, id: String, url: String, slug_override: Option<String>) {
        if let Err(e) = tokio::fs::create_dir_all(&self.cfg.tmp_dir).await {
            self.fail(&id, format!("tmp-Verzeichnis: {e}")).await;
            return;
        }
        let out = self.json_path(&id);
        let mut cmd = tokio::process::Command::new(&self.cfg.python);
        cmd.arg(&self.cfg.script).arg(&url).arg("-o").arg(&out);
        // Unbuffered Python so progress lines arrive while Chromium works.
        cmd.env("PYTHONUNBUFFERED", "1");
        // Do NOT touch HOME: Playwright derives its default browser cache
        // from it (~/Library/Caches/ms-playwright on macOS, ~/.cache on
        // Linux). Where the service user can't read that cache (www-data in
        // prod vs. root's install) set PLAYWRIGHT_BROWSERS_PATH in the service
        // env — it's inherited by this child. See .env.example.
        if let Err(e) = self.run_logged(&id, cmd, SCRAPE_TIMEOUT).await {
            // The scraper exits 1 when it found no items at all — the usual
            // cause is a shop whose catalog the page didn't ship.
            let hint = if e.contains("exit status: 1") {
                " (keine Artikel gefunden — Shop-Seite prüfen)"
            } else {
                ""
            };
            self.fail(&id, format!("Scrape fehlgeschlagen{hint}: {e}"))
                .await;
            return;
        }
        match self.preview_from_json(&out, slug_override.as_deref()).await {
            Ok(p) => {
                // Keep the scrape where the CLI workflow keeps it, named by the
                // Lieferando slug (re-scraping the same shop overwrites in
                // place, like the CLI). Provisioning imports from THERE.
                let name = if p.source_slug.is_empty() {
                    p.slug.clone()
                } else {
                    p.source_slug.clone()
                };
                let dest = self.cfg.menu_json_dir.join(format!("menu-{name}.json"));
                let json_path = match tokio::fs::create_dir_all(&self.cfg.menu_json_dir).await {
                    Ok(()) => match tokio::fs::copy(&out, &dest).await {
                        Ok(_) => {
                            let _ = tokio::fs::remove_file(&out).await;
                            Self::push_log(
                                &self.jobs,
                                &id,
                                format!("[sysadmin] JSON gespeichert: {}", dest.display()),
                            )
                            .await;
                            dest
                        }
                        Err(e) => {
                            Self::push_log(&self.jobs, &id, format!("[sysadmin] Kopie nach {} fehlgeschlagen ({e}) — importiere aus tmp", dest.display())).await;
                            out.clone()
                        }
                    },
                    Err(e) => {
                        Self::push_log(&self.jobs, &id, format!("[sysadmin] menu-json Verzeichnis nicht anlegbar ({e}) — importiere aus tmp")).await;
                        out.clone()
                    }
                };
                Self::push_log(
                    &self.jobs,
                    &id,
                    format!(
                        "[sysadmin] Vorschau: {} — {} Kategorien, {} Artikel, Slug '{}'",
                        p.name,
                        p.categories.len(),
                        p.n_items,
                        p.slug
                    ),
                )
                .await;
                if let Some(j) = self.jobs.write().await.get_mut(&id) {
                    j.preview = Some(p);
                    j.json_path = json_path.display().to_string();
                    j.state = JobState::Scraped;
                }
                self.busy.store(false, Ordering::SeqCst);
            }
            Err(e) => self.fail(&id, e).await,
        }
    }

    async fn run_provision(self: Arc<Self>, id: String, slug: String) {
        let json = {
            let stored = self
                .jobs
                .read()
                .await
                .get(&id)
                .map(|j| j.json_path.clone())
                .unwrap_or_default();
            if stored.is_empty() {
                self.json_path(&id)
            } else {
                PathBuf::from(stored)
            }
        };
        Self::push_log(
            &self.jobs,
            &id,
            format!("[sysadmin] importiere {}", json.display()),
        )
        .await;
        let env_file = self.env_file_for(&slug);
        let env_created = !Path::new(&env_file).exists();
        let (admin_pw, kitchen_pw, driver_pw) = (gen_password(), gen_password(), gen_password());

        let mut cmd = tokio::process::Command::new(&self.cfg.menu_import);
        cmd.arg("--parent")
            .arg(&self.cfg.parent)
            .arg("--shop")
            .arg(&slug)
            .arg("--menu")
            .arg(&json)
            .arg("--domain")
            .arg(&self.cfg.domain)
            .arg("--root")
            .arg(".");
        if env_created {
            for (k, v) in [
                ("ADMIN_PASSWORD", &admin_pw),
                ("KITCHEN_PASSWORD", &kitchen_pw),
                ("DRIVER_PASSWORD", &driver_pw),
            ] {
                cmd.arg("--set").arg(format!("{k}={v}"));
            }
        }
        cmd.env("RUST_LOG", "info,sqlx=warn").env("NO_COLOR", "1");
        if let Err(e) = self.run_logged(&id, cmd, IMPORT_TIMEOUT).await {
            self.fail(&id, format!("menu-import fehlgeschlagen: {e}"))
                .await;
            return;
        }

        Self::push_log(
            &self.jobs,
            &id,
            format!("[sysadmin] lade Mandant '{slug}'…"),
        )
        .await;
        if let Err(e) = self.tenants.load_by_slug(&slug).await {
            self.fail(
                &id,
                format!("Mandant importiert, aber Laden fehlgeschlagen: {e:#}"),
            )
            .await;
            return;
        }
        let tenant = self.tenants.get(&slug).await;
        let public_url = read_env_var(&env_file, "PUBLIC_URL")
            .unwrap_or_else(|| format!("https://{slug}.{}", self.cfg.domain));
        let result = ProvisionResult {
            slug: slug.clone(),
            admin_url: format!("{}/admin", public_url.trim_end_matches('/')),
            public_url,
            admin_password: if env_created {
                admin_pw
            } else {
                tenant
                    .as_ref()
                    .and_then(|t| t.auth.admin_password.clone())
                    .unwrap_or_default()
            },
            kitchen_password: if env_created {
                kitchen_pw
            } else {
                String::new()
            },
            driver_password: if env_created {
                driver_pw
            } else {
                String::new()
            },
            env_created,
            env_file,
        };
        Self::push_log(
            &self.jobs,
            &id,
            format!("[sysadmin] fertig: {}", result.public_url),
        )
        .await;
        if let Some(j) = self.jobs.write().await.get_mut(&id) {
            j.result = Some(result);
            j.state = JobState::Done;
        }
        self.busy.store(false, Ordering::SeqCst);
    }
}

#[async_trait::async_trait]
impl SysAdminOps for SysAdmin {
    fn apex_slug(&self) -> String {
        self.apex_slug.clone()
    }

    async fn list_tenants(&self) -> Vec<TenantInfo> {
        let mut slugs = self.tenants.slugs().await;
        slugs.sort();
        let mut out = Vec::with_capacity(slugs.len());
        for slug in slugs {
            let Some(t) = self.tenants.get(&slug).await else {
                continue;
            };
            let setting = |key: &'static str| {
                let pool = t.pool.clone();
                async move {
                    sqlx::query_scalar::<_, String>("SELECT value FROM app_settings WHERE key = ?1")
                        .bind(key)
                        .fetch_optional(&pool)
                        .await
                        .ok()
                        .flatten()
                        .unwrap_or_default()
                }
            };
            let count = |sql: &'static str| {
                let pool = t.pool.clone();
                async move {
                    sqlx::query_scalar::<_, i64>(sql)
                        .fetch_one(&pool)
                        .await
                        .unwrap_or(0)
                }
            };
            let zones = zone_stats(&t.pool).await;
            out.push(TenantInfo {
                name: setting("shop_name").await,
                import_source: setting("menu_import_source").await,
                import_at: setting("menu_import_at").await,
                items: count("SELECT COUNT(*) FROM menu_items").await,
                orders: count("SELECT COUNT(*) FROM orders").await,
                public_url: read_env_var(&t.env_file, "PUBLIC_URL").unwrap_or_default(),
                // From the tenant's own `.env` (already parsed into its
                // TenantAuth at load time) so the operator can read a shop's
                // logins here instead of over SSH.
                admin_password: t.auth.admin_password.clone().unwrap_or_default(),
                kitchen_password: t.auth.kitchen_password.clone().unwrap_or_default(),
                driver_password: t.auth.driver_password.clone().unwrap_or_default(),
                env_file: t.env_file.clone(),
                database_url: t.database_url.clone(),
                is_apex: slug == self.apex_slug,
                zones,
                slug,
            });
        }
        out
    }

    async fn start_scrape(&self, url: String, slug: Option<String>) -> Result<String, String> {
        validate_source_url(&url)?;
        if let Some(s) = &slug {
            if !valid_slug(s) {
                return Err(format!("Ungültiger Slug '{s}' — nur a-z, 0-9 und '-'"));
            }
        }
        if self.busy.swap(true, Ordering::SeqCst) {
            return Err("Es läuft bereits ein Import — bitte warten".to_string());
        }
        let id = uuid::Uuid::new_v4().simple().to_string();
        let job = ImportJob {
            id: id.clone(),
            url: url.clone(),
            state: JobState::Scraping,
            log: Vec::new(),
            preview: None,
            result: None,
            error: None,
            started_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            json_path: String::new(),
        };
        {
            let mut map = self.jobs.write().await;
            let mut order = self.order.write().await;
            map.insert(id.clone(), job);
            order.push(id.clone());
            while order.len() > JOBS_KEPT {
                let old = order.remove(0);
                map.remove(&old);
                let _ = std::fs::remove_file(self.json_path(&old));
            }
        }
        // `self` is behind the trait's Arc — re-wrap for the task. The handle
        // in context is `Arc<dyn SysAdminOps>`, so we hold our own Arc via
        // the registry pattern below.
        let me = SELF_REGISTRY
            .get()
            .and_then(|w| w.upgrade())
            .ok_or_else(|| "sysadmin: Instanz nicht registriert".to_string())?;
        tokio::spawn(me.run_scrape(id.clone(), url, slug));
        Ok(id)
    }

    async fn job(&self, id: &str) -> Option<ImportJob> {
        self.jobs.read().await.get(id).cloned()
    }

    async fn provision(&self, job_id: &str, slug: String) -> Result<(), String> {
        if !valid_slug(&slug) {
            return Err(format!("Ungültiger Slug '{slug}' — nur a-z, 0-9 und '-'"));
        }
        if matches!(slug.as_str(), "www" | "localhost" | "127") || slug == self.cfg.parent {
            return Err(format!("'{slug}' ist reserviert"));
        }
        {
            let map = self.jobs.read().await;
            let j = map
                .get(job_id)
                .ok_or_else(|| "Job nicht gefunden".to_string())?;
            // Scraped, or a failed provisioning whose scrape JSON is still on
            // disk (e.g. importer error fixed / slug corrected) → retry.
            let json_ok = if j.json_path.is_empty() {
                self.json_path(job_id).exists()
            } else {
                Path::new(&j.json_path).exists()
            };
            let retryable = j.state == JobState::Failed && j.preview.is_some() && json_ok;
            if j.state != JobState::Scraped && !retryable {
                return Err(format!("Job ist im Zustand {:?}, nicht bereit", j.state));
            }
        }
        if self.busy.swap(true, Ordering::SeqCst) {
            return Err("Es läuft bereits ein Import — bitte warten".to_string());
        }
        if let Some(j) = self.jobs.write().await.get_mut(job_id) {
            j.state = JobState::Provisioning;
            j.error = None;
            if let Some(p) = &mut j.preview {
                p.slug = slug.clone();
            }
        }
        let me = SELF_REGISTRY
            .get()
            .and_then(|w| w.upgrade())
            .ok_or_else(|| "sysadmin: Instanz nicht registriert".to_string())?;
        tokio::spawn(me.run_provision(job_id.to_string(), slug));
        Ok(())
    }

    async fn price_benchmarks(
        &self,
        exclude_slug: &str,
        categories: &[String],
    ) -> Vec<CategoryBenchmark> {
        if categories.is_empty() {
            return Vec::new();
        }
        let mut out = Vec::new();
        for slug in self.tenants.slugs().await {
            // Never benchmark a shop against itself, and skip the apex
            // showroom (its menu is demo data, not a real price point).
            if slug == exclude_slug || slug == self.apex_slug {
                continue;
            }
            let Some(t) = self.tenants.get(&slug).await else {
                continue;
            };
            let shop = sqlx::query_scalar::<_, String>(
                "SELECT value FROM app_settings WHERE key = 'shop_name'",
            )
            .fetch_optional(&t.pool)
            .await
            .ok()
            .flatten()
            .unwrap_or_else(|| slug.clone());

            for want in categories {
                // Match on the category NAME: item names never line up across
                // shops (a kebab shop and an Italian restaurant share no
                // dishes), but "Pizza" means the same thing everywhere.
                // LIKE with wildcards catches "Pizza" vs "Familien-Pizza".
                let pattern = format!("%{}%", want.trim());
                let prices = sqlx::query_scalar::<_, i64>(
                    "SELECT i.price_small_cents
                       FROM menu_items i
                       JOIN menu_categories c ON c.id = i.category_id
                      WHERE i.is_listed = 1
                        AND i.price_small_cents > 0
                        AND c.name LIKE ?1
                      ORDER BY i.price_small_cents",
                )
                .bind(&pattern)
                .fetch_all(&t.pool)
                .await
                .unwrap_or_default();

                // Fewer than 3 items is not a market signal, it's noise.
                if prices.len() < 3 {
                    continue;
                }
                let median = prices[prices.len() / 2];
                out.push(CategoryBenchmark {
                    shop: shop.clone(),
                    category: want.clone(),
                    item_count: prices.len() as i64,
                    min_cents: *prices.first().unwrap_or(&0),
                    max_cents: *prices.last().unwrap_or(&0),
                    median_cents: median,
                });
            }
        }
        out
    }

    async fn reload_tenant(&self, slug: &str) -> Result<(), String> {
        if !valid_slug(slug) {
            return Err(format!("Ungültiger Slug '{slug}'"));
        }
        self.tenants
            .load_by_slug(slug)
            .await
            .map_err(|e| format!("{e:#}"))
    }

    async fn delete_tenant(&self, slug: &str) -> Result<String, String> {
        if !valid_slug(slug) {
            return Err(format!("Ungültiger Slug '{slug}'"));
        }
        if slug == self.apex_slug || slug == self.cfg.parent {
            return Err(format!(
                "'{slug}' ist der Apex-Mandant und kann nicht gelöscht werden"
            ));
        }
        // Resolve the paths from the LIVE tenant (its real env_file +
        // DATABASE_URL) before unloading — never from a reconstructed name,
        // so we can't delete a file that belongs to something else.
        let tenant = self
            .tenants
            .get(slug)
            .await
            .ok_or_else(|| format!("Mandant '{slug}' ist nicht geladen"))?;
        let env_file = tenant.env_file.clone();
        let db_path = tenant
            .database_url
            .strip_prefix("sqlite:")
            .map(|p| {
                p.trim_start_matches("//")
                    .split('?')
                    .next()
                    .unwrap_or("")
                    .to_string()
            })
            .filter(|p| !p.is_empty());
        drop(tenant);

        self.tenants.remove(slug).await;

        let mut removed: Vec<String> = Vec::new();
        let mut failed: Vec<String> = Vec::new();
        let mut rm = |path: &str| match std::fs::remove_file(path) {
            Ok(()) => removed.push(path.to_string()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => failed.push(format!("{path} ({e})")),
        };
        if let Some(db) = &db_path {
            // WAL + shared-memory siblings, or a re-import inherits old pages.
            rm(db);
            rm(&format!("{db}-wal"));
            rm(&format!("{db}-shm"));
        }
        rm(&env_file);

        tracing::warn!(slug, removed = removed.len(), "sysadmin: tenant deleted");
        if failed.is_empty() {
            Ok(format!(
                "Mandant '{slug}' gelöscht ({} Datei(en): {})",
                removed.len(),
                removed.join(", ")
            ))
        } else {
            Err(format!(
                "Mandant '{slug}' entladen, aber nicht alle Dateien gelöscht: {}",
                failed.join(", ")
            ))
        }
    }
}

/// The one live instance, so trait methods (which only see `&self`) can hand
/// an owned `Arc<SysAdmin>` to the spawned job tasks.
static SELF_REGISTRY: std::sync::OnceLock<std::sync::Weak<SysAdmin>> = std::sync::OnceLock::new();

impl SysAdmin {
    /// Register the instance for the background tasks. Call once after `new`.
    pub fn register(this: &Arc<Self>) {
        let _ = SELF_REGISTRY.set(Arc::downgrade(this));
    }
}

/// Remove `ESC[…m`-style sequences and stray control characters.
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for d in chars.by_ref() {
                    if d.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        if c.is_control() && c != '\t' {
            continue;
        }
        out.push(c);
    }
    out
}

/// Delivery zones + local delivery statistics for one tenant. Every query is
/// best-effort: a tenant DB that predates a table/column simply yields zeros
/// rather than failing the whole tenant list.
async fn zone_stats(pool: &sqlx::SqlitePool) -> ZoneStats {
    let scalar = |sql: &'static str| async move {
        sqlx::query_scalar::<_, i64>(sql)
            .fetch_one(pool)
            .await
            .unwrap_or(0)
    };

    let rows = sqlx::query_as::<_, (String, Option<String>, String, i64, i64, i64, i64, String)>(
        "SELECT id, name, postcode, fee_cents, min_order_cents, eta_minutes, is_active,
                COALESCE(geometry_kind, 'none')
         FROM delivery_zones
         ORDER BY is_active DESC, priority DESC, fee_cents, postcode",
    )
    .fetch_all(pool)
    .await
    .unwrap_or_default();

    // Orders carry a fee, not a zone id. Attribute by fee, and only when the
    // fee identifies exactly one zone — otherwise leave the count blank.
    let mut fee_counts: HashMap<i64, usize> = HashMap::new();
    for r in &rows {
        *fee_counts.entry(r.3).or_default() += 1;
    }

    let mut zones = Vec::with_capacity(rows.len());
    for (id, name, postcode, fee_cents, min_order_cents, eta_minutes, is_active, geometry_kind) in
        rows
    {
        let street_overrides = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM geo_zone_streets WHERE zone_id = ?1",
        )
        .bind(&id)
        .fetch_one(pool)
        .await
        .unwrap_or(0);
        // Unambiguous fee → count its delivery orders; otherwise report none.
        let (orders, revenue_cents) = if fee_counts.get(&fee_cents).copied().unwrap_or(0) == 1 {
            sqlx::query_as::<_, (i64, Option<i64>)>(
                "SELECT COUNT(*), SUM(total_cents) FROM orders
                 WHERE order_type = 'delivery' AND status != 'cancelled'
                   AND delivery_fee_cents = ?1",
            )
            .bind(fee_cents)
            .fetch_one(pool)
            .await
            .map(|(c, sum)| (c, sum.unwrap_or(0)))
            .unwrap_or((0, 0))
        } else {
            (0, 0)
        };
        zones.push(ZoneInfo {
            name: name
                .filter(|n| !n.trim().is_empty())
                .unwrap_or_else(|| postcode.clone()),
            id,
            postcode,
            fee_cents,
            min_order_cents,
            eta_minutes,
            is_active: is_active != 0,
            geometry_kind,
            street_overrides,
            orders,
            revenue_cents,
        });
    }

    let delivery_orders = scalar(
        "SELECT COUNT(*) FROM orders WHERE order_type = 'delivery' AND status != 'cancelled'",
    )
    .await;
    let fees_collected_cents = scalar(
        "SELECT COALESCE(SUM(delivery_fee_cents), 0) FROM orders
         WHERE order_type = 'delivery' AND status != 'cancelled'",
    )
    .await;

    ZoneStats {
        zones,
        free_delivery_cents: sqlx::query_scalar::<_, String>(
            "SELECT value FROM app_settings WHERE key = 'free_delivery_threshold_cents'",
        )
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(0),
        pickup_orders: scalar(
            "SELECT COUNT(*) FROM orders WHERE order_type != 'delivery' AND status != 'cancelled'",
        )
        .await,
        avg_fee_cents: if delivery_orders > 0 {
            fees_collected_cents / delivery_orders
        } else {
            0
        },
        delivery_orders,
        fees_collected_cents,
        geo_streets: scalar("SELECT COUNT(*) FROM geo_streets").await,
        geo_addresses: scalar("SELECT COUNT(*) FROM geo_addresses").await,
        lookup_mode: sqlx::query_scalar::<_, String>(
            "SELECT value FROM app_settings WHERE key = 'address_lookup_mode'",
        )
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| "nominatim".to_string()),
    }
}

/// Only Lieferando / Takeaway storefronts — the scraper understands nothing
/// else, and the operator shouldn't be able to point a headless browser at
/// arbitrary hosts from the server.
fn validate_source_url(url: &str) -> Result<(), String> {
    let Some(rest) = url.strip_prefix("https://") else {
        return Err("URL muss mit https:// beginnen".to_string());
    };
    let host = rest.split('/').next().unwrap_or("").to_ascii_lowercase();
    let ok = host == "lieferando.de"
        || host.ends_with(".lieferando.de")
        || host.ends_with(".lieferando.at")
        || host.ends_with(".takeaway.com")
        || host.ends_with(".thuisbezorgd.nl");
    if ok {
        Ok(())
    } else {
        Err(format!("Nicht unterstützte Quelle: {host}"))
    }
}

/// Same rule as menu-import's `slugify` so the preview shows the slug the
/// importer will actually use.
fn slugify(s: &str) -> String {
    s.trim()
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

/// 16 chars from a v4 UUID's hex — plenty for a staff password that the
/// operator hands over once and the shop can change in its own .env.
fn gen_password() -> String {
    uuid::Uuid::new_v4().simple().to_string()[..16].to_string()
}

fn read_env_var(env_file: &str, key: &str) -> Option<String> {
    dotenvy::from_path_iter(env_file)
        .ok()?
        .filter_map(Result::ok)
        .find(|(k, _)| k == key)
        .map(|(_, v)| v)
        .filter(|v| !v.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_url_whitelist() {
        assert!(validate_source_url("https://www.lieferando.de/speisekarte/x").is_ok());
        assert!(validate_source_url("https://lieferando.de/x").is_ok());
        assert!(validate_source_url("http://www.lieferando.de/x").is_err());
        assert!(validate_source_url("https://evil.example/x").is_err());
        assert!(validate_source_url("https://www.lieferando.de.evil.example/x").is_err());
    }

    #[test]
    fn slugify_matches_importer() {
        assert_eq!(
            slugify("Pizzeria Rimini Lüdinghausen"),
            "pizzeria-rimini-l-dinghausen"
        );
        assert_eq!(slugify("--a--b--"), "a-b");
    }

    #[test]
    fn strips_ansi_and_control_chars() {
        assert_eq!(
            strip_ansi("\u{1b}[2m2026\u{1b}[0m INFO\u{1b}[32m ok\u{1b}[0m\r"),
            "2026 INFO ok"
        );
        assert_eq!(strip_ansi("plain\ttab"), "plain\ttab");
    }

    #[test]
    fn passwords_are_16_chars() {
        let p = gen_password();
        assert_eq!(p.len(), 16);
        assert!(p.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
