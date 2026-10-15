//! Sys-admin area — platform operator UI on the APEX host (`rusterando.de`).
//!
//! What it does: onboard a shop from its Lieferando page without a shell.
//! The operator pastes the shop URL, the SERVER runs the same two tools the
//! SSH workflow uses — `rusterando-scrape/scrape_lieferando.py` (Playwright
//! Chromium, in its venv) and the `menu-import` binary — then hot-loads the
//! new tenant into the running process. The browser only polls the job.
//!
//! Layering: this module owns the shared DTOs, the `SysAdminOps` trait and the
//! server fns + components. The trait is IMPLEMENTED in the server crate
//! (`rusterando-server::sysadmin`), which owns the tenant registry, the child
//! processes and the filesystem; it's provided into request context as
//! `SysAdminHandle`. In single-tenant mode the handle is `None` and every
//! server fn here answers "not available".
//!
//! Auth: `Role::SysAdmin` (cookie `dp_session=sysadmin`, host-only, so it
//! never reaches a tenant subdomain) checked against the PROCESS env
//! `SYS_ADMIN_PASSWORD`. Every server fn additionally requires the request to
//! have resolved to the apex tenant — a tenant host gets "nicht verfügbar".

use leptos::prelude::*;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Shared DTOs (server ⇄ browser)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum JobState {
    /// Chromium is fetching the page / the scraper is running.
    #[default]
    Scraping,
    /// JSON on disk, preview ready — waiting for the operator to confirm.
    Scraped,
    /// `menu-import` + hot-load running.
    Provisioning,
    Done,
    Failed,
}

impl JobState {
    pub fn is_active(self) -> bool {
        matches!(self, JobState::Scraping | JobState::Provisioning)
    }
    pub fn label_de(self) -> &'static str {
        match self {
            JobState::Scraping => "Scrapen…",
            JobState::Scraped => "Vorschau bereit",
            JobState::Provisioning => "Shop wird angelegt…",
            JobState::Done => "Fertig",
            JobState::Failed => "Fehlgeschlagen",
        }
    }
}

/// What the scraped JSON says about the shop — shown before provisioning so
/// the operator can sanity-check name, address and menu size.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ImportPreview {
    pub name: String,
    /// Proposed tenant slug (override or slugified Lieferando slug).
    pub slug: String,
    pub source_slug: String,
    pub address_street: String,
    pub postcode: String,
    pub city: String,
    pub country: String,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    pub phone: String,
    pub owner: String,
    /// Weekdays with at least one delivery window.
    pub open_days: usize,
    /// (category name, item count) in menu order.
    pub categories: Vec<(String, usize)>,
    pub n_items: usize,
    pub n_sizes: usize,
    /// Distinct size labels seen ("22cm", "30cm", "klein", …), capped.
    pub size_labels: Vec<String>,
    /// A tenant with this slug is already loaded → provisioning REPLACES
    /// its menu (menu-import upsert) instead of creating a shop.
    pub existing_tenant: bool,
    /// The NRW cadastre lookup will be pre-configured for this shop.
    pub nrw_geo: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ProvisionResult {
    pub slug: String,
    pub public_url: String,
    pub admin_url: String,
    /// Fresh passwords when the .env was created by this run; the existing
    /// tenant's admin password (kitchen/driver empty) otherwise.
    pub admin_password: String,
    pub kitchen_password: String,
    pub driver_password: String,
    pub env_created: bool,
    pub env_file: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ImportJob {
    pub id: String,
    pub url: String,
    pub state: JobState,
    /// Child-process output lines (scraper + importer), newest last, capped.
    pub log: Vec<String>,
    pub preview: Option<ImportPreview>,
    pub result: Option<ProvisionResult>,
    pub error: Option<String>,
    pub started_at: String,
    /// Where the scraped menu JSON lives once the scrape finished:
    /// `rusterando-scrape/menu-json/menu-<slug>.json`, the same place the
    /// CLI workflow keeps them. Provisioning imports THIS file, so it can be
    /// hand-adjusted between "Scrapen" and "Shop anlegen".
    #[serde(default)]
    pub json_path: String,
}

/// One delivery zone of a tenant, as configured in its `delivery_zones`
/// table, plus what the local address index knows about it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ZoneInfo {
    pub id: String,
    /// Zone label; falls back to the postcode when unnamed.
    pub name: String,
    pub postcode: String,
    pub fee_cents: i64,
    pub min_order_cents: i64,
    pub eta_minutes: i64,
    pub is_active: bool,
    /// `none` | `polygon` | `radius` | `default` — how membership is decided.
    pub geometry_kind: String,
    /// Street-level overrides pointing at this zone (`geo_zone_streets`).
    pub street_overrides: i64,
    /// Delivery orders whose fee matches this zone's fee. Orders don't store
    /// a zone id, so this is an attribution by fee, not an exact count — it
    /// is blank when several zones share one fee.
    pub orders: i64,
    pub revenue_cents: i64,
}

/// Delivery configuration + local statistics for one tenant.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ZoneStats {
    pub zones: Vec<ZoneInfo>,
    /// Free-delivery threshold (`free_delivery_threshold_cents`), 0 = none.
    pub free_delivery_cents: i64,
    /// Delivery vs pickup split over all non-cancelled orders.
    pub delivery_orders: i64,
    pub pickup_orders: i64,
    /// Sum of collected delivery fees, and their average per delivery order.
    pub fees_collected_cents: i64,
    pub avg_fee_cents: i64,
    /// Local address index: streets + addresses imported for this tenant.
    pub geo_streets: i64,
    pub geo_addresses: i64,
    /// `local` | `nominatim` | … — the shop's address-check mode.
    pub lookup_mode: String,
}

/// What comparable shops charge in one category — the evidence an admin
/// needs when staging a new price list. Aggregated only; never exposes
/// another shop's individual item rows to a tenant admin.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct CategoryBenchmark {
    /// The OTHER shop's display name (not its slug — an admin reads names).
    pub shop: String,
    /// That shop's category whose name matched ours.
    pub category: String,
    pub item_count: i64,
    pub min_cents: i64,
    pub max_cents: i64,
    pub median_cents: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct TenantInfo {
    pub slug: String,
    pub name: String,
    pub public_url: String,
    pub env_file: String,
    pub database_url: String,
    pub items: i64,
    pub orders: i64,
    /// `menu_import_source` / `menu_import_at` from the tenant's app_settings
    /// (written by menu-import), empty when the menu was never imported.
    pub import_source: String,
    pub import_at: String,
    pub is_apex: bool,
    /// Staff logins straight out of the tenant's `.env.<parent>.<slug>`.
    /// Shown in the sys-admin table so the operator can hand a shop its
    /// credentials without an SSH session. Only ever sent to an
    /// authenticated sysadmin on the apex host.
    pub admin_password: String,
    pub kitchen_password: String,
    pub driver_password: String,
    pub zones: ZoneStats,
}

// ---------------------------------------------------------------------------
// Server-side contract, implemented by rusterando-server::sysadmin
// ---------------------------------------------------------------------------

#[cfg(feature = "ssr")]
#[async_trait::async_trait]
pub trait SysAdminOps: Send + Sync {
    /// The apex tenant's slug (`rusterando`); empty when no parent loaded.
    fn apex_slug(&self) -> String;
    async fn list_tenants(&self) -> Vec<TenantInfo>;
    /// Start a scrape job; returns the job id. Errors when a job is running.
    async fn start_scrape(&self, url: String, slug: Option<String>) -> Result<String, String>;
    async fn job(&self, id: &str) -> Option<ImportJob>;
    /// Run menu-import for a SCRAPED job and hot-load the tenant. The
    /// outcome lands in the job (`Done` + `result`, or `Failed` + `error`).
    async fn provision(&self, job_id: &str, slug: String) -> Result<(), String>;
    /// Re-read a tenant's .env + DB into the running process.
    async fn reload_tenant(&self, slug: &str) -> Result<(), String>;
    /// Price benchmarks from the OTHER loaded tenants, for the given category
    /// names. Aggregates only (count / min / max / median), so one shop's
    /// admin can price against the market without reading another shop's
    /// menu. `exclude` is the calling tenant, which is never its own
    /// benchmark.
    async fn price_benchmarks(
        &self,
        exclude_slug: &str,
        categories: &[String],
    ) -> Vec<CategoryBenchmark>;
    /// Unload a tenant and delete its `.env` + sqlite so it can be imported
    /// from scratch. Irreversible — the caller confirms first.
    async fn delete_tenant(&self, slug: &str) -> Result<String, String>;
}

#[cfg(feature = "ssr")]
pub type SysAdminHandle = Option<std::sync::Arc<dyn SysAdminOps>>;

#[cfg(feature = "ssr")]
mod ssr {
    use super::*;
    use crate::pages::session::{ssr as session, Role};
    use std::sync::Arc;

    /// The handle, only when this request hit the APEX host. Tenant hosts
    /// and single-tenant deployments get a clear "not available".
    pub fn apex_ops() -> Result<Arc<dyn SysAdminOps>, ServerFnError> {
        let ops = use_context::<SysAdminHandle>()
            .flatten()
            .ok_or_else(|| ServerFnError::new("Systemverwaltung ist hier nicht verfügbar"))?;
        let resolved = use_context::<crate::pages::settings::ResolvedTenant>().unwrap_or_default();
        if resolved.slug != ops.apex_slug() {
            return Err(ServerFnError::new(
                "Systemverwaltung nur über die Hauptdomain erreichbar",
            ));
        }
        Ok(ops)
    }

    /// Apex + signed in as SysAdmin.
    pub async fn require() -> Result<Arc<dyn SysAdminOps>, ServerFnError> {
        let ops = apex_ops()?;
        match session::current_role().await {
            Some(Role::SysAdmin) => Ok(ops),
            _ => Err(ServerFnError::new("nicht angemeldet")),
        }
    }
}

// ---------------------------------------------------------------------------
// Server fns
// ---------------------------------------------------------------------------

#[server(name = SysAdminLogin, prefix = "/api", endpoint = "sysadmin_login")]
pub async fn sysadmin_login(password: String) -> Result<bool, ServerFnError> {
    use crate::pages::session::{ssr as session, Role};
    ssr::apex_ops()?;
    match session::try_login(Role::SysAdmin, &password).await? {
        true => {
            leptos_axum::redirect("/sysadmin");
            Ok(true)
        }
        false => Ok(false),
    }
}

#[server(name = SysAdminLogout, prefix = "/api", endpoint = "sysadmin_logout")]
pub async fn sysadmin_logout() -> Result<(), ServerFnError> {
    crate::pages::session::ssr::logout().await?;
    leptos_axum::redirect("/sysadmin/login");
    Ok(())
}

/// Page guard. Unlike `session::require_role_or_redirect`, a shop `Admin`
/// session does NOT pass: sysadmin is a separate authority, not a superset.
/// Redirects to the login page and returns `false` so the page renders
/// nothing but the redirect (no chrome, no forms, no tenant data).
#[server(name = SysAdminGuard, prefix = "/api", endpoint = "sysadmin_guard")]
pub async fn sysadmin_guard() -> Result<bool, ServerFnError> {
    use crate::pages::session::{ssr as session, Role};
    // Apex-only; a tenant host never even reaches here (axum gate 404s first).
    if ssr::apex_ops().is_err() {
        leptos_axum::redirect("/");
        return Ok(false);
    }
    match session::current_role().await {
        Some(Role::SysAdmin) => Ok(true),
        _ => {
            leptos_axum::redirect("/sysadmin/login");
            Ok(false)
        }
    }
}

#[server(name = SysAdminTenants, prefix = "/api", endpoint = "sysadmin_tenants")]
pub async fn sysadmin_tenants() -> Result<Vec<TenantInfo>, ServerFnError> {
    let ops = ssr::require().await?;
    Ok(ops.list_tenants().await)
}

/// Form-post friendly: on success redirects to `/sysadmin?job=<id>`, on
/// error to `/sysadmin?error=…`. The page is server-rendered and refreshes
/// itself while a job runs, so it works with or without hydration.
#[server(name = SysAdminStartScrape, prefix = "/api", endpoint = "sysadmin_start_scrape")]
pub async fn sysadmin_start_scrape(url: String, slug: String) -> Result<String, ServerFnError> {
    let ops = ssr::require().await?;
    let slug = slug.trim().to_ascii_lowercase();
    let slug = if slug.is_empty() { None } else { Some(slug) };
    match ops.start_scrape(url.trim().to_string(), slug).await {
        Ok(id) => {
            leptos_axum::redirect(&format!("/sysadmin?job={id}"));
            Ok(id)
        }
        Err(e) => {
            leptos_axum::redirect(&format!("/sysadmin?error={}", urlencoding::encode(&e)));
            Ok(String::new())
        }
    }
}

#[server(name = SysAdminJob, prefix = "/api", endpoint = "sysadmin_job")]
pub async fn sysadmin_job(id: String) -> Result<Option<ImportJob>, ServerFnError> {
    let ops = ssr::require().await?;
    Ok(ops.job(&id).await)
}

#[server(name = SysAdminProvision, prefix = "/api", endpoint = "sysadmin_provision")]
pub async fn sysadmin_provision(job_id: String, slug: String) -> Result<(), ServerFnError> {
    let ops = ssr::require().await?;
    let target = match ops.provision(&job_id, slug.trim().to_ascii_lowercase()).await {
        Ok(()) => format!("/sysadmin?job={job_id}"),
        Err(e) => format!("/sysadmin?job={job_id}&error={}", urlencoding::encode(&e)),
    };
    leptos_axum::redirect(&target);
    Ok(())
}

/// Delete a tenant: unload it, then remove its `.env` and sqlite (with its
/// -wal/-shm siblings). `confirm` must repeat the slug — a mis-click can't
/// destroy a shop. The apex tenant can never be deleted.
#[server(name = SysAdminDelete, prefix = "/api", endpoint = "sysadmin_delete")]
pub async fn sysadmin_delete(slug: String, confirm: String) -> Result<(), ServerFnError> {
    let ops = ssr::require().await?;
    let slug = slug.trim().to_string();
    if confirm.trim() != slug {
        leptos_axum::redirect(&format!(
            "/sysadmin?error={}",
            urlencoding::encode(&format!(
                "Löschen abgebrochen — zur Bestätigung „{slug}“ eintippen"
            ))
        ));
        return Ok(());
    }
    let target = match ops.delete_tenant(&slug).await {
        Ok(what) => format!("/sysadmin?msg={}", urlencoding::encode(&what)),
        Err(e) => format!("/sysadmin?error={}", urlencoding::encode(&e)),
    };
    leptos_axum::redirect(&target);
    Ok(())
}

#[server(name = SysAdminReload, prefix = "/api", endpoint = "sysadmin_reload")]
pub async fn sysadmin_reload(slug: String) -> Result<(), ServerFnError> {
    let ops = ssr::require().await?;
    let target = match ops.reload_tenant(slug.trim()).await {
        Ok(()) => format!("/sysadmin?msg={}", urlencoding::encode(&format!("Mandant '{}' neu geladen", slug.trim()))),
        Err(e) => format!("/sysadmin?error={}", urlencoding::encode(&e)),
    };
    leptos_axum::redirect(&target);
    Ok(())
}

// ---------------------------------------------------------------------------
// Pages
// ---------------------------------------------------------------------------

#[component]
pub fn SysAdminLoginPage() -> impl IntoView {
    let action = ServerAction::<SysAdminLogin>::new();
    let pending = action.pending();
    let value = action.value();

    view! {
        <section class="admin-login sysadmin-login">
            <h1>"Systemverwaltung"</h1>
            <p class="muted">"Anmeldung für den Plattform-Betreiber (SYS_ADMIN_PASSWORD)."</p>
            <ActionForm action=action>
                <label>
                    <span>"Passwort"</span>
                    <input type="password" name="password" required autofocus/>
                </label>
                <button type="submit" disabled=move || pending.get()>
                    {move || if pending.get() { "Bitte warten…" } else { "Anmelden" }}
                </button>
            </ActionForm>
            {move || match value.get() {
                Some(Ok(true))  => Some(view! { <p class="ok">"Erfolg, leite weiter…"</p> }.into_any()),
                Some(Ok(false)) => Some(view! { <p class="error">"Falsches Passwort."</p> }.into_any()),
                Some(Err(e))    => Some(view! { <p class="error">{format!("Fehler: {e}")}</p> }.into_any()),
                None            => None,
            }}
        </section>
    }
}

/// Tenant table + Lieferando import wizard. Deliberately SERVER-RENDERED:
/// every action is a real form post (ActionForm → server fn → redirect back
/// here), the job is read from `?job=<id>` on each render, and a
/// `<meta http-equiv="refresh">` re-renders the page every 3 s while a job
/// is active. No client-side state → works with or without WASM hydration
/// (the shell currently has hydration debt on this branch) and still
/// enhances progressively when hydration is healthy.
///
/// Split into `job_panel` / `tenants_table`: one monolithic `view!` was deep
/// enough to overflow a tokio worker's stack in the debug build.
#[component]
pub fn SysAdminPage() -> impl IntoView {
    use leptos_router::hooks::use_query_map;

    let q = use_query_map();
    let job_id = q.with_untracked(|m| m.get("job").filter(|s| !s.is_empty()));
    let error_q = q.with_untracked(|m| m.get("error").filter(|s| !s.is_empty()));
    let msg_q = q.with_untracked(|m| m.get("msg").filter(|s| !s.is_empty()));
    let url_q = q.with_untracked(|m| m.get("url")).unwrap_or_default();
    let slug_q = q.with_untracked(|m| m.get("slug")).unwrap_or_default();

    let data = OnceResource::new({
        let job_id = job_id.clone();
        async move {
            // Hard gate FIRST: only a sysadmin session gets any data — and
            // because the whole page lives inside this resource, an
            // unauthenticated (or merely shop-admin) request renders no
            // wizard chrome at all, just the redirect.
            if !sysadmin_guard().await.unwrap_or(false) {
                return None;
            }
            let job = match &job_id {
                Some(id) => sysadmin_job(id.clone()).await.ok().flatten(),
                None => None,
            };
            let tenants = sysadmin_tenants().await;
            Some((job, tenants))
        }
    });

    let start = ServerAction::<SysAdminStartScrape>::new();
    let logout = ServerAction::<SysAdminLogout>::new();

    view! {
        <Suspense fallback=|| view! { <p class="muted">"Lade…"</p> }>
            {move || data.get().map(|d| match d {
                None => view! { <p class="muted">"Weiterleitung…"</p> }.into_any(),
                Some((job, tenants)) => {
                    let active = job.as_ref().map(|j| j.state.is_active()).unwrap_or(false);
                    // Slug prefill: explicit ?slug=, else the preview's proposal.
                    let slug_prefill = if !slug_q.is_empty() {
                        slug_q.clone()
                    } else {
                        job.as_ref().and_then(|j| j.preview.as_ref()).map(|p| p.slug.clone()).unwrap_or_default()
                    };
                    let job_view = job.clone().map(|j| job_panel(j, slug_prefill, active));
                    let tenants_view = match tenants {
                        Err(e) => view! { <p class="error">{format!("Fehler: {e}")}</p> }.into_any(),
                        Ok(list) => tenants_table(list, active),
                    };
                    let (error_q, msg_q, url_q, slug_q) = (error_q.clone(), msg_q.clone(), url_q.clone(), slug_q.clone());
                    view! {
                        // Browsers honour http-equiv=refresh in <body> as well;
                        // in streaming SSR the <head> is long gone by the time
                        // this Suspense resolves.
                        {active.then(|| view! { <meta http-equiv="refresh" content="3"/> })}
                        <section class="sysadmin">
                            <header class="sysadmin-bar">
                                <h1>"Rusterando — Systemverwaltung"</h1>
                                <ActionForm action=logout>
                                    <button type="submit" class="btn small">"Abmelden"</button>
                                </ActionForm>
                            </header>

                            <div class="sysadmin-card">
                                <h2>"Shop von Lieferando importieren"</h2>
                                <p class="muted">
                                    "Der Server öffnet die Lieferando-Seite in Chromium, liest Speisekarte + Adresse aus "
                                    "und legt den Mandanten an (.env, Datenbank, Menü, Öffnungszeiten). "
                                    "Ein vorhandener Slug ersetzt nur das Menü."
                                </p>
                                <ActionForm action=start attr:class="sysadmin-form">
                                    <label>
                                        <span>"Lieferando-URL"</span>
                                        <input type="url" name="url" required placeholder="https://www.lieferando.de/speisekarte/…" value=url_q/>
                                    </label>
                                    <label>
                                        <span>"Slug (optional)"</span>
                                        <input type="text" name="slug" placeholder="z. B. rimini" pattern="[a-z0-9\\-]*" value=slug_q/>
                                    </label>
                                    <button type="submit" class="btn primary" disabled=active>"Scrapen"</button>
                                </ActionForm>
                                {error_q.map(|e| view! { <p class="error">{e}</p> })}
                                {msg_q.map(|m| view! { <p class="ok">{m}</p> })}
                                {job_view}
                            </div>

                            <div class="sysadmin-card">
                                <h2>"Mandanten"</h2>
                                {tenants_view}
                            </div>
                        </section>
                    }.into_any()
                }
            })}
        </Suspense>
    }
}

/// Status chip + log + (when scraped) the preview with the provision form +
/// (when done) the result card.
fn job_panel(j: ImportJob, slug_prefill: String, active: bool) -> AnyView {
    let state = j.state;
    let log = j.log.join("\n");
    let jid = j.id.clone();
    let refresh_href = format!("/sysadmin?job={}", j.id);
    let chip_cls = format!("job-state {}", match state {
        JobState::Done => "done",
        JobState::Failed => "failed",
        _ => "active",
    });
    let json_path = j.json_path.clone();
    let preview = j.preview.clone().map(|p| preview_card(p, state, jid, slug_prefill, json_path));
    let result = j.result.clone().map(result_card);
    view! {
        <div class="sysadmin-job">
            <div class="job-head">
                <span class=chip_cls>{state.label_de()}</span>
                <span class="muted">{j.url.clone()}</span>
                {active.then(|| view! { <span class="muted">" · Seite aktualisiert sich alle 3 s"</span> })}
                <a class="btn small" href=refresh_href>"Aktualisieren"</a>
            </div>
            {j.error.clone().map(|e| view! { <p class="error">{e}</p> })}
            {preview}
            {result}
            <pre class="sysadmin-log">{log}</pre>
        </div>
    }
    .into_any()
}

fn preview_card(p: ImportPreview, state: JobState, jid: String, slug_prefill: String, json_path: String) -> AnyView {
    let provision = ServerAction::<SysAdminProvision>::new();
    let addr = format!("{}, {} {} {}", p.address_street, p.postcode, p.city, p.country);
    let coords = match (p.lat, p.lon) {
        (Some(a), Some(b)) => format!("{a:.5}, {b:.5}"),
        _ => "—".to_string(),
    };
    let dash = |s: &str| if s.is_empty() { "—".to_string() } else { s.to_string() };
    let menu_line = format!(
        "{} Kategorien · {} Artikel · {} Größen ({})",
        p.categories.len(),
        p.n_items,
        p.n_sizes,
        if p.size_labels.is_empty() { "ohne Label".to_string() } else { p.size_labels.join(", ") }
    );
    let geo_line = if p.nrw_geo {
        "NRW-Kataster wird vorkonfiguriert (GEO_BACKEND=cadastre_nrw)"
    } else {
        "nicht vorkonfiguriert (kein NRW-Ort erkannt)"
    };
    let action_label = if p.existing_tenant { "Menü dieses Mandanten ersetzen" } else { "Shop anlegen" };
    let can_provision = state == JobState::Scraped || state == JobState::Failed;
    let cats = p.categories.clone();
    let existing = p.existing_tenant;
    view! {
        <div class="sysadmin-preview">
            <h3>{p.name.clone()}</h3>
            <dl>
                <dt>"Adresse"</dt><dd>{addr}</dd>
                <dt>"Koordinaten"</dt><dd>{coords}</dd>
                <dt>"Telefon"</dt><dd>{dash(&p.phone)}</dd>
                <dt>"Inhaber"</dt><dd>{dash(&p.owner)}</dd>
                <dt>"Lieferzeiten"</dt><dd>{format!("{} Tage/Woche", p.open_days)}</dd>
                <dt>"Menü"</dt><dd>{menu_line}</dd>
                <dt>"Adressprüfung"</dt><dd>{geo_line}</dd>
                <dt>"Lieferando-Slug"</dt><dd>{p.source_slug.clone()}</dd>
                <dt>"JSON"</dt><dd><code>{json_path}</code><span class="muted">" — kann vor dem Anlegen von Hand angepasst werden; importiert wird diese Datei."</span></dd>
            </dl>
            <table class="sysadmin-cats"><tbody>
                {cats.into_iter().map(|(n, c)| view! { <tr><td>{n}</td><td class="num">{c}</td></tr> }).collect_view()}
            </tbody></table>
            {can_provision.then(|| view! {
                <ActionForm action=provision attr:class="sysadmin-form">
                    <input type="hidden" name="job_id" value=jid.clone()/>
                    <label>
                        <span>"Mandanten-Slug"</span>
                        <input type="text" name="slug" value=slug_prefill.clone() required pattern="[a-z0-9\\-]+"/>
                    </label>
                    {existing.then(|| view! {
                        <p class="warn">"Dieser Slug existiert bereits — .env und Passwörter bleiben, das Menü wird ersetzt (manuelle Menü-Änderungen gehen verloren)."</p>
                    })}
                    <button type="submit" class="btn primary">{action_label}</button>
                </ActionForm>
            })}
        </div>
    }
    .into_any()
}

fn result_card(r: ProvisionResult) -> AnyView {
    let staff = (!r.kitchen_password.is_empty()).then(|| view! {
        <dt>"Küche"</dt><dd><code>{r.kitchen_password.clone()}</code></dd>
        <dt>"Fahrer"</dt><dd><code>{r.driver_password.clone()}</code></dd>
    });
    let note = r.env_created.then(|| view! {
        <p class="muted">
            "Passwörter werden nur hier angezeigt (bis zum nächsten Import). Stripe, SMTP und APNs in der .env sind noch Platzhalter. "
            "Adressdaten: im Admin des Shops unter Liefergebiete → Adressprüfung → „Alles aus Adresse einrichten“."
        </p>
    });
    view! {
        <div class="sysadmin-result">
            <h3>{if r.env_created { "Shop angelegt" } else { "Menü ersetzt" }}</h3>
            <p><a href=r.public_url.clone() target="_blank">{r.public_url.clone()}</a></p>
            <p><a href=r.admin_url.clone() target="_blank">{r.admin_url.clone()}</a></p>
            <dl>
                <dt>"Admin-Passwort"</dt><dd><code>{r.admin_password.clone()}</code></dd>
                {staff}
                <dt>".env"</dt><dd><code>{r.env_file.clone()}</code></dd>
            </dl>
            {note}
        </div>
    }
    .into_any()
}

/// Cents → "12,50 €" (German formatting, same as the shop pages).
fn eur(cents: i64) -> String {
    format!("{},{:02}\u{00A0}€", cents / 100, (cents % 100).abs())
}

/// Delivery zones + local statistics for one tenant, folded away by default
/// so the tenant table stays scannable.
fn zones_panel(slug: &str, z: &ZoneStats) -> AnyView {
    if z.zones.is_empty() && z.delivery_orders == 0 {
        return view! { <span class="muted">"keine Zonen"</span> }.into_any();
    }
    let n_active = z.zones.iter().filter(|x| x.is_active).count();
    let summary = format!(
        "{} Zonen ({} aktiv) · {} Lieferungen / {} Abholungen",
        z.zones.len(),
        n_active,
        z.delivery_orders,
        z.pickup_orders
    );
    let geo_line = if z.geo_streets > 0 {
        format!(
            "Adressindex: {} Straßen · {} Adressen · Modus {}",
            z.geo_streets, z.geo_addresses, z.lookup_mode
        )
    } else {
        format!("Adressindex: nicht importiert · Modus {}", z.lookup_mode)
    };
    let fee_line = if z.delivery_orders > 0 {
        format!(
            "Liefergebühren: {} gesamt · ⌀ {}",
            eur(z.fees_collected_cents),
            eur(z.avg_fee_cents)
        )
    } else {
        "Liefergebühren: noch keine Lieferungen".to_string()
    };
    let free_line = if z.free_delivery_cents > 0 {
        format!("Gratis-Lieferung ab {}", eur(z.free_delivery_cents))
    } else {
        "Keine Gratis-Lieferschwelle".to_string()
    };
    let rows = z.zones.clone();
    let id = format!("zones-{slug}");
    view! {
        <details class="zones" id=id>
            <summary>{summary}</summary>
            <div class="zone-meta">
                <span>{free_line}</span>
                <span>{fee_line}</span>
                <span>{geo_line}</span>
            </div>
            <table class="zone-table">
                <thead><tr>
                    <th>"Zone"</th><th>"PLZ"</th><th class="num">"Gebühr"</th><th class="num">"MBW"</th>
                    <th class="num">"ETA"</th><th>"Geometrie"</th><th class="num">"Straßen"</th>
                    <th class="num">"Bestellungen"</th><th class="num">"Umsatz"</th>
                </tr></thead>
                <tbody>
                    {rows.into_iter().map(|r| {
                        let kind = match r.geometry_kind.as_str() {
                            "polygon" => "Polygon",
                            "radius" => "Umkreis",
                            "default" => "Standard",
                            _ => "—",
                        };
                        // Orders are attributed by fee; a shared fee can't be
                        // split, so show a dash rather than a wrong number.
                        let (orders, revenue) = if r.orders > 0 {
                            (r.orders.to_string(), eur(r.revenue_cents))
                        } else {
                            ("—".to_string(), "—".to_string())
                        };
                        view! {
                            <tr class:inactive=!r.is_active>
                                <td>{r.name}{(!r.is_active).then(|| view! { <span class="muted">" (inaktiv)"</span> })}</td>
                                <td>{r.postcode}</td>
                                <td class="num">{eur(r.fee_cents)}</td>
                                <td class="num">{eur(r.min_order_cents)}</td>
                                <td class="num">{format!("{} min", r.eta_minutes)}</td>
                                <td>{kind}</td>
                                <td class="num">{if r.street_overrides > 0 { r.street_overrides.to_string() } else { "—".to_string() }}</td>
                                <td class="num">{orders}</td>
                                <td class="num">{revenue}</td>
                            </tr>
                        }
                    }).collect_view()}
                </tbody>
            </table>
        </details>
    }
    .into_any()
}

fn tenants_table(list: Vec<TenantInfo>, active: bool) -> AnyView {
    view! {
        <table class="sysadmin-tenants">
            <thead><tr>
                <th>"Slug"</th><th>"Name"</th><th class="num">"Artikel"</th><th class="num">"Bestellungen"</th>
                <th>"Passwörter (Admin / Küche / Fahrer)"</th>
                <th>"Liefergebiete"</th>
                <th>"Letzter Import"</th><th></th>
            </tr></thead>
            <tbody>
                {list.into_iter().map(|t| tenant_row(t, active)).collect_view()}
            </tbody>
        </table>
    }
    .into_any()
}

fn tenant_row(t: TenantInfo, active: bool) -> AnyView {
    let admin_url = format!("{}/admin", t.public_url.trim_end_matches('/'));
    let src = t.import_source.clone();
    let has_src = !src.is_empty();
    let reimport = ServerAction::<SysAdminStartScrape>::new();
    let reload = ServerAction::<SysAdminReload>::new();
    let delete = ServerAction::<SysAdminDelete>::new();
    let (s_code, s_re, s_reload) = (t.slug.clone(), t.slug.clone(), t.slug.clone());
    let s_del = t.slug.clone();
    let import_at = if t.import_at.is_empty() { "—".to_string() } else { t.import_at.clone() };
    let src_link = has_src.then(|| {
        let (href, text) = (src.clone(), src.clone());
        view! { <br/><a class="muted" href=href target="_blank">{text}</a> }
    });
    let pw = |label: &'static str, value: String| {
        (!value.is_empty()).then(|| view! {
            <div class="pw"><span class="muted">{label}</span><code>{value}</code></div>
        })
    };
    let passwords = view! {
        {pw("A", t.admin_password.clone())}
        {pw("K", t.kitchen_password.clone())}
        {pw("F", t.driver_password.clone())}
    };
    let reimport_form = has_src.then(|| view! {
        <ActionForm action=reimport>
            <input type="hidden" name="url" value=src.clone()/>
            <input type="hidden" name="slug" value=s_re.clone()/>
            <button type="submit" class="btn small" disabled=active>"Neu importieren"</button>
        </ActionForm>
    });
    view! {
        <tr class:apex=t.is_apex>
            <td><code>{s_code}</code>{t.is_apex.then(|| view! { <span class="muted">" (Apex)"</span> })}</td>
            <td>{t.name.clone()}</td>
            <td class="num">{t.items}</td>
            <td class="num">{t.orders}</td>
            <td class="passwords">{passwords}</td>
            <td class="zones-cell">{zones_panel(&t.slug, &t.zones)}</td>
            <td>{import_at}{src_link}</td>
            <td class="actions">
                <a class="btn small" href=t.public_url.clone() target="_blank">"Shop"</a>
                <a class="btn small" href=admin_url target="_blank">"Admin"</a>
                {reimport_form}
                <ActionForm action=reload>
                    <input type="hidden" name="slug" value=s_reload/>
                    <button type="submit" class="btn small">"Neu laden"</button>
                </ActionForm>
                // Destructive: .env + sqlite are removed, so the shop can be
                // imported from scratch. The slug must be retyped — the
                // server refuses a mismatch. The apex tenant is never
                // deletable (guarded server-side too).
                {(!t.is_apex).then(|| view! {
                    <ActionForm action=delete attr:class="delete-form">
                        <input type="hidden" name="slug" value=s_del.clone()/>
                        <input type="text" name="confirm" placeholder="Slug zum Löschen"
                            size="14" pattern="[a-z0-9\\-]+"
                            title="Zum Bestätigen den Slug eintippen"/>
                        <button type="submit" class="btn small danger"
                            title="Mandant entladen und .env + Datenbank löschen">"Löschen"</button>
                    </ActionForm>
                })}
            </td>
        </tr>
    }
    .into_any()
}
