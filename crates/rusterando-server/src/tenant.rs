//! Tenancy mode detection — single-tenant (Model A) vs in-process
//! multi-tenant (Model B). See `docs/multi_tenant.md`.
//!
//! Model A is the **fail-safe default**: a real shop (davidspizzeria.de) must
//! never be dragged into multi-tenant mode by accident. Model B requires an
//! unambiguous positive signal. This module owns the gate logic and produces
//! the [`DeploymentInfo`] snapshot the admin diagnostics panel reads.
//!
//! `detect()` powers the loud boot log + admin panel. [`Tenant`] / [`Tenants`]
//! own the in-process registry: in Model A a single passthrough tenant wraps
//! the global pool (byte-for-byte today's behaviour); in Model B one tenant
//! per `.env.<slug>`, resolved per request from the `X-Tenant` header. This
//! pass routes the PER-REQUEST POOL (data isolation); per-tenant cached
//! handles (branding/theme caches) are a follow-up.

use axum::response::IntoResponse;
use rusterando_frontend::pages::settings::DeploymentInfo;
use sqlx::SqlitePool;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Resolve the runtime deployment snapshot from env + the working directory.
/// Pure except for the `.env*` glob + env reads — call once at boot.
pub fn detect() -> DeploymentInfo {
    let db_url =
        std::env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite:./data/rusterando.db".into());
    let tenant_slug = db_stem(&db_url);
    let service_name = std::env::var("LEPTOS_OUTPUT_NAME").unwrap_or_else(|_| tenant_slug.clone());
    let site_addr = std::env::var("LEPTOS_SITE_ADDR").unwrap_or_default();

    let env_file = std::env::var("ENV_FILE").ok().filter(|s| !s.is_empty());
    let kill_switch = std::env::var("MULTI_TENANT")
        .map(|v| v == "0")
        .unwrap_or(false);

    // The env files visible in the working directory. bare `.env` counts.
    let env_files = glob_env_files();

    // --- The 4-gate rule (first match wins; Model A is the default) ---------
    // 1. MULTI_TENANT=0 kill-switch  -> single-tenant, forced.
    // 2. ENV_FILE pinned             -> single-tenant, forced.
    // 3. count `.env*` <= 1          -> single-tenant.
    // 4. >=2 env files               -> multi-tenant candidate; the registry
    //    (follow-up) enforces distinct DBs and may fall back to single. For
    //    DETECTION we report multi-tenant here.
    let multi_tenant = !kill_switch && env_file.is_none() && env_files.len() >= 2;

    // What we report as "loaded": the pinned ENV_FILE, else the discovered set
    // (single-tenant collapses to the one file actually in effect).
    let reported_env_files = match (&env_file, multi_tenant) {
        (Some(f), _) => vec![f.clone()],
        (None, true) => env_files,
        (None, false) => {
            // Single-tenant: the bare `.env` if present, else whatever the one
            // discovered file was, else "(env injected)" for prod systemd
            // which sets the environment directly without a file on disk.
            if env_files.is_empty() {
                vec!["(env injected by systemd)".to_string()]
            } else {
                env_files
            }
        }
    };

    DeploymentInfo {
        multi_tenant,
        env_files: reported_env_files,
        tenant_slug,
        service_name,
        database_url: db_url,
        site_addr,
        // host + subdomain are per-request — filled in by the diagnostics
        // server fn from the live headers, not known at boot.
        host: String::new(),
        subdomain: String::new(),
    }
}

/// One-line summary for the boot log. Single-tenant is info-level; multi-tenant
/// is warn-level so a real shop that unexpectedly trips it is obvious in the
/// journal.
pub fn log_mode(info: &DeploymentInfo) {
    if info.multi_tenant {
        tracing::warn!(
            "MODE: multi-tenant — {} env files: {}",
            info.env_files.len(),
            info.env_files.join(", ")
        );
    } else {
        tracing::info!(
            "MODE: single-tenant ({}) — db {} — env {}",
            info.tenant_slug,
            info.database_url,
            info.env_files.join(", ")
        );
    }
}

/// `sqlite:./data/davidspizzeria.db?foo=1` -> `davidspizzeria`.
fn db_stem(db_url: &str) -> String {
    let path = db_url
        .trim_start_matches("sqlite:")
        .trim_start_matches("//")
        .split('?')
        .next()
        .unwrap_or("");
    std::path::Path::new(path)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "rusterando".to_string())
}

/// All `.env*` files in the current working directory (sorted, deduped).
/// Bare `.env` counts toward the multi-tenant trigger. Backup-ish siblings
/// (`.env.bak`, `.env.example`, `.env.*.example`) are excluded so a stray
/// template can't flip a real shop — only real profile files count.
fn glob_env_files() -> Vec<String> {
    let mut out: Vec<String> = glob::glob(".env*")
        .map(|paths| {
            paths
                .filter_map(Result::ok)
                .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
                .filter(|name| !is_non_profile_env(name))
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out.dedup();
    out
}

/// Exclude obvious non-profile files so they don't count toward the trigger:
/// `.env.example`, `.env.demo.example`, `.env.bak`, editor swap files.
fn is_non_profile_env(name: &str) -> bool {
    name.ends_with(".example")
        || name.ends_with(".bak")
        || name.ends_with('~')
        || name.ends_with(".swp")
        || name.contains(".example.")
}

// ---------------------------------------------------------------------------
// Tenant registry (in-process). MUST be `Clone` + `Send + Sync` — a clone is
// stashed in the request extensions by `resolve_tenant`, and the `Parts` clone
// leptos_axum makes only carries `Clone` extension entries (a non-Clone Tenant
// would silently vanish from the per-request context). Pool routing only this
// pass; per-tenant cached handles are a follow-up.
// ---------------------------------------------------------------------------

/// Per-tenant cached config handles — the same shallow-`Arc` write-through
/// handles AppState holds, but one set per tenant so each shop serves its own
/// branding/theme/i18n/etc. from its own DB. Built in `build_tenant`; provided
/// into the request context so every `use_context::<…Handle>()` (read AND the
/// `update_setting` write-through) targets the current tenant.
#[derive(Clone)]
pub struct TenantHandles {
    pub theme: rusterando_frontend::pages::settings::ThemeHandle,
    pub branding: rusterando_frontend::branding::BrandingHandle,
    pub i18n: rusterando_frontend::pages::settings::I18nHandle,
    pub orders_paused: rusterando_frontend::pages::settings::OrdersPausedHandle,
    pub stripe_mode: rusterando_frontend::stripe::StripeModeHandle,
    pub jsonld: rusterando_frontend::pages::seo::JsonLdHandle,
}

/// One tenant's resolved runtime: its slug + SQLite pool + cached handles. In
/// Model A this wraps the process's single global pool + global handles
/// (passthrough — zero behaviour change); in Model B one set per `.env.<slug>`.
#[derive(Clone)]
pub struct Tenant {
    pub slug: String,
    pub pool: SqlitePool,
    /// Resolved `DATABASE_URL` — the isolation key (distinct per tenant).
    pub database_url: String,
    /// Per-tenant cached config handles.
    pub handles: TenantHandles,
}

impl Tenant {
    /// Model-A passthrough: wrap the process's already-built global pool +
    /// handles, so server fns see exactly today's state (zero rebuild, zero
    /// behaviour change). Used when the binary is single-tenant.
    pub fn passthrough(
        slug: String,
        pool: SqlitePool,
        database_url: String,
        handles: TenantHandles,
    ) -> Self {
        Self {
            slug,
            pool,
            database_url,
            handles,
        }
    }
}

/// Slug → Tenant. `Clone` is cheap (Arc). Empty + unused in Model A (the
/// single tenant is provided directly); the registry exists for Model B
/// lookups + hot reload.
#[derive(Clone, Default)]
pub struct Tenants(pub Arc<RwLock<HashMap<String, Tenant>>>);

impl Tenants {
    pub fn new() -> Self {
        Self(Arc::new(RwLock::new(HashMap::new())))
    }

    pub async fn get(&self, slug: &str) -> Option<Tenant> {
        self.0.read().await.get(slug).cloned()
    }

    pub async fn slugs(&self) -> Vec<String> {
        let mut v: Vec<String> = self.0.read().await.keys().cloned().collect();
        v.sort();
        v
    }

    /// Build `.env.<slug>` into a tenant and insert/replace it. Rejects a NEW
    /// slug whose resolved DB collides with an already-loaded tenant's
    /// (distinct DBs are mandatory). Re-loading the SAME slug onto its own DB
    /// is fine (in-place replace).
    pub async fn load(&self, slug: &str) -> anyhow::Result<()> {
        let tenant = build_tenant(slug).await?;
        let mut map = self.0.write().await;
        if let Some((existing_slug, _)) = map
            .iter()
            .find(|(s, t)| s.as_str() != slug && t.database_url == tenant.database_url)
        {
            anyhow::bail!(
                "tenant '{slug}' DATABASE_URL collides with already-loaded '{existing_slug}' \
                 ({}) — distinct DBs are required",
                tenant.database_url
            );
        }
        map.insert(slug.to_owned(), tenant);
        Ok(())
    }

    /// Directly insert a pre-built tenant (used for the Model-A passthrough,
    /// which wraps the global pool rather than re-opening from `.env`).
    pub async fn insert(&self, tenant: Tenant) {
        self.0.write().await.insert(tenant.slug.clone(), tenant);
    }
}

/// `[a-z0-9-]+` — it becomes a filename, so guard against path traversal.
pub fn valid_slug(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// Build a tenant from `.env.<slug>`. Parses the env file into a LOCAL map
/// (never mutates the process environment — that would clobber other tenants),
/// opens `data/<slug>.sqlite` (WAL + 5s busy_timeout, matching the Model-A
/// pool), and runs migrations. Per-tenant handles are a follow-up.
pub async fn build_tenant(slug: &str) -> anyhow::Result<Tenant> {
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

    anyhow::ensure!(valid_slug(slug), "invalid tenant slug: {slug:?}");

    let env_path = format!(".env.{slug}");
    let vars: HashMap<String, String> = dotenvy::from_path_iter(&env_path)
        .map_err(|e| anyhow::anyhow!("read {env_path}: {e}"))?
        .filter_map(Result::ok)
        .collect();

    let database_url = vars
        .get("DATABASE_URL")
        .cloned()
        .unwrap_or_else(|| format!("sqlite:./data/{slug}.sqlite"));

    // Ensure the parent dir exists for a default file URL.
    if let Some(stripped) = database_url.strip_prefix("sqlite:") {
        let path = stripped
            .trim_start_matches("//")
            .split('?')
            .next()
            .unwrap_or("");
        if let Some(parent) = std::path::Path::new(path).parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).ok();
            }
        }
    }

    let connect_opts: SqliteConnectOptions = database_url
        .parse::<SqliteConnectOptions>()
        .map_err(|e| anyhow::anyhow!("parse DATABASE_URL for {slug}: {e}"))?
        .create_if_missing(true)
        .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
        .busy_timeout(std::time::Duration::from_secs(5));

    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(connect_opts)
        .await
        .map_err(|e| anyhow::anyhow!("open pool for {slug}: {e}"))?;

    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .map_err(|e| anyhow::anyhow!("migrate {slug}: {e}"))?;

    // TODO(model-b follow-up): seed_demo_if_empty(&pool, slug) — idempotent
    // demo menu for an empty showroom tenant. Left out this pass.

    let handles = build_handles(&pool).await;

    Ok(Tenant {
        slug: slug.to_owned(),
        pool,
        database_url,
        handles,
    })
}

/// Build the 6 cached config handles from a pool — the same boot sequence
/// main.rs runs for the global handles, in the same order (branding BEFORE
/// jsonld, which consumes the branding snapshot). Used per-tenant in Model B;
/// Model A reuses the globals it already built (see `Tenant::passthrough`).
pub async fn build_handles(pool: &SqlitePool) -> TenantHandles {
    use rusterando_frontend::branding::{ssr::load_branding, BrandingHandle};
    use rusterando_frontend::pages::seo::{build_restaurant_jsonld, JsonLdHandle};
    use rusterando_frontend::pages::settings::{
        ssr::{i18n_enabled, orders_paused, theme as theme_ssr},
        I18nHandle, OrdersPausedHandle, ThemeHandle,
    };
    use rusterando_frontend::stripe::{ssr::load_mode, StripeModeHandle};

    let theme = ThemeHandle::new(theme_ssr(pool).await);
    let branding = BrandingHandle::new(load_branding(pool).await);
    let jsonld = JsonLdHandle::new(build_restaurant_jsonld(pool, &branding.get()).await);
    let stripe_mode = StripeModeHandle::new(load_mode(pool).await);
    let i18n = I18nHandle::new(i18n_enabled(pool).await);
    let orders_paused = OrdersPausedHandle::new(orders_paused(pool).await);

    TenantHandles {
        theme,
        branding,
        i18n,
        orders_paused,
        stripe_mode,
        jsonld,
    }
}

// ---------------------------------------------------------------------------
// Request routing. `resolve_tenant` runs as an axum middleware BEFORE the
// Leptos handlers; it inserts the resolved `Tenant` into request extensions.
// leptos_axum auto-provides the request `Parts` (incl. extensions) into the
// Leptos context, so the context closures read the tenant back and
// `provide_context(tenant.pool)` — the per-request pool swap. No server-fn
// site changes.
// ---------------------------------------------------------------------------

/// Shared routing state behind the middleware. Holds the registry plus the
/// Model-A passthrough tenant (the single global-pool tenant), so one
/// middleware serves both modes.
#[derive(Clone)]
pub struct TenantRouter {
    pub tenants: Tenants,
    pub multi_tenant: bool,
    /// Model A: the single tenant wrapping the global pool. Always inserted
    /// regardless of headers. `None` would be a bug (we always build it).
    pub single: Tenant,
}

/// Subdomain labels that mean "no specific tenant" → landing page passthrough.
fn is_apex_label(label: &str) -> bool {
    label.is_empty() || label == "rusterando" || label == "www"
}

/// Resolve the tenant for this request and stash it in extensions.
///
/// - **Model A**: always insert the single passthrough tenant (the header is
///   irrelevant — there's exactly one shop). Behaviour identical to today.
/// - **Model B**: read `X-Tenant` (nginx), fall back to the first `Host` label
///   for header-less local testing. Apex/`www`/empty → pass through with NO
///   tenant (landing page). Unknown slug → 404. Known → insert.
pub async fn resolve_tenant(
    axum::extract::State(router): axum::extract::State<TenantRouter>,
    mut req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use axum::http::{header, StatusCode};

    if !router.multi_tenant {
        req.extensions_mut().insert(router.single.clone());
        return next.run(req).await;
    }

    // Model B: derive the slug from X-Tenant, else the first Host label.
    let slug = req
        .headers()
        .get("x-tenant")
        .and_then(|v| v.to_str().ok())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_ascii_lowercase())
        .or_else(|| {
            req.headers()
                .get(header::HOST)
                .and_then(|v| v.to_str().ok())
                .and_then(|h| h.split(':').next())
                .and_then(|h| h.split('.').next())
                .map(|s| s.to_ascii_lowercase())
        })
        .unwrap_or_default();

    if is_apex_label(&slug) {
        // Landing page — no tenant context. (Server fns that need a tenant
        // will error cleanly; the apex page itself is static.)
        return next.run(req).await;
    }
    if !valid_slug(&slug) {
        return StatusCode::BAD_REQUEST.into_response();
    }
    match router.tenants.get(&slug).await {
        Some(t) => {
            req.extensions_mut().insert(t);
            next.run(req).await
        }
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

/// `GET /__whoami` — transport probe. Reports the mode + the tenant this
/// request resolved to (or "(none)" on the apex / single-tenant default).
pub async fn whoami(
    axum::extract::State(router): axum::extract::State<TenantRouter>,
    req: axum::extract::Request,
) -> String {
    let mode = if router.multi_tenant {
        "multi-tenant"
    } else {
        "single-tenant"
    };
    let xtenant = req
        .headers()
        .get("x-tenant")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    // /__whoami is on a sub-router WITHOUT the resolve_tenant layer, so resolve
    // the slug here the same way the middleware would: single-tenant → the one
    // passthrough tenant; multi-tenant → the X-Tenant header (apex → none).
    let resolved = if !router.multi_tenant {
        router.single.slug.clone()
    } else if is_apex_label(&xtenant.to_ascii_lowercase()) {
        "(none)".to_string()
    } else {
        let slug = xtenant.to_ascii_lowercase();
        match router.tenants.get(&slug).await {
            Some(_) => slug,
            None => format!("(unknown: {slug})"),
        }
    };
    format!(
        "mode     = {mode}\ntenant   = {resolved}\nx-tenant = {xtenant}\nenv      = .env.{resolved}\ndb       = data/{resolved}.sqlite\n"
    )
}

/// `POST /admin/tenant/{slug}/reload` — hot-add/replace a tenant without a
/// restart. Token-gated by `ADMIN_TOKEN` (unset/empty ⇒ endpoint closed).
/// Multi-tenant only; a 404 in single-tenant mode (there's nothing to reload).
pub async fn reload_tenant(
    axum::extract::State(router): axum::extract::State<TenantRouter>,
    axum::extract::Path(slug): axum::extract::Path<String>,
    headers: axum::http::HeaderMap,
) -> axum::response::Response {
    use axum::http::StatusCode;

    if !router.multi_tenant {
        return StatusCode::NOT_FOUND.into_response();
    }
    let want = std::env::var("ADMIN_TOKEN").unwrap_or_default();
    let got = headers
        .get("x-admin-token")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if want.is_empty() || got != want {
        return StatusCode::FORBIDDEN.into_response();
    }
    if !valid_slug(&slug) {
        return StatusCode::BAD_REQUEST.into_response();
    }
    match router.tenants.load(&slug).await {
        Ok(()) => (StatusCode::OK, format!("loaded {slug}\n")).into_response(),
        Err(e) => {
            tracing::warn!("reload tenant {slug} failed: {e}");
            (StatusCode::BAD_REQUEST, format!("reload failed: {e}\n")).into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn db_stem_strips_prefix_and_query() {
        assert_eq!(db_stem("sqlite:./data/davidspizzeria.db"), "davidspizzeria");
        assert_eq!(db_stem("sqlite://data/flizza.sqlite?x=1"), "flizza");
        assert_eq!(db_stem("sqlite:"), "rusterando");
    }

    #[test]
    fn non_profile_env_files_excluded() {
        assert!(is_non_profile_env(".env.example"));
        assert!(is_non_profile_env(".env.demo.example"));
        assert!(is_non_profile_env(".env.bak"));
        assert!(!is_non_profile_env(".env"));
        assert!(!is_non_profile_env(".env.davids"));
        assert!(!is_non_profile_env(".env.rusterando.flizza"));
    }
}
