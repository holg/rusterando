use std::sync::Arc;

use axum::Router;
use rusterando_frontend::app::{shell, App};
use rusterando_server::notify::{EmailNotifier, LogNotifier, NotifierHandle};
// (`notify` is re-exported from the frontend crate via davidspizzeria-server::lib)
use leptos::prelude::*;
use leptos_axum::{generate_route_list, LeptosRoutes};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;
use tower_cookies::CookieManagerLayer;

#[derive(Clone)]
struct AppState {
    leptos_options: LeptosOptions,
    #[allow(dead_code)]
    db: SqlitePool,
    #[allow(dead_code)]
    admin_password: Arc<String>,
    #[allow(dead_code)]
    notifier: NotifierHandle,
    /// `None` when APNS_KEY_PATH isn't set (local dev, or before the
    /// admin uploads the auth key). Push-trigger code is no-op in that
    /// case rather than erroring.
    apns: Option<rusterando_server::apns::ApnsHandle>,
    /// Cached site theme ("warm" | "dark"), seeded from `app_settings`
    /// at boot, rewritten in-place by `update_setting`. Read in `shell()`
    /// to set `<html class="theme-…">` server-side (no FOUC).
    #[allow(dead_code)]
    theme: rusterando_frontend::pages::settings::ThemeHandle,
    /// Cached shop-contact data (name, address, phone, email, owner,
    /// tax + bank), seeded from `app_settings` at boot, rewritten by
    /// `update_setting`. Read by PDF generation, email rendering,
    /// Impressum, home meta, etc.
    #[allow(dead_code)]
    branding: rusterando_frontend::branding::BrandingHandle,
    /// Kitchen-printer outbox + broadcast channel. `None` when
    /// KITCHEN_LISTEN_ADDR isn't set (printerless deploys like the
    /// rusterando demo) — `place_order` and the Stripe webhook both
    /// guard with `if let Some(_)` so the outbox table stays empty.
    #[allow(dead_code)]
    kitchen: Option<rusterando_server::kitchen::KitchenChannel>,
    /// Active Stripe mode (sandbox vs live), seeded from
    /// `app_settings.stripe_mode` at boot and write-through-updated
    /// by `update_setting`. Read at the point of use by
    /// `place_order`, the two webhook handlers, and the
    /// payment-method detail lookup.
    #[allow(dead_code)]
    stripe_mode: rusterando_frontend::stripe::StripeModeHandle,
    /// Multilingual UI toggle. `false` = single-locale (German-only)
    /// shop, the locale switcher is hidden, /<lang>/* routes 404.
    /// `true` = the locale switcher appears in the header and
    /// prefixed routes serve translated content. Same boot-read +
    /// write-through pattern as the other handles.
    #[allow(dead_code)]
    i18n: rusterando_frontend::pages::settings::I18nHandle,
    /// Manual "pause online orders" switch. `true` = reject all online
    /// orders regardless of opening hours and show a closed banner.
    /// Same boot-read + write-through pattern as the other handles.
    #[allow(dead_code)]
    orders_paused: rusterando_frontend::pages::settings::OrdersPausedHandle,
    /// Live customer channel hub (SSE). Server fns publish order events
    /// (admin messages, status changes); the `/api/live/orders/{id}` SSE
    /// route subscribes and streams matching events to the customer.
    live: rusterando_frontend::live::LiveHub,
    /// Base directory for admin-uploaded images, `<site_root>/img/uploads`.
    /// Uploads live INSIDE the static site root so nginx serves them from
    /// `html/img/uploads/` directly (the `data/` dir — DB included — is
    /// never web-reachable). The deploy script excludes `img/uploads/`
    /// from its `rsync --delete` so uploads survive releases.
    uploads_dir: Arc<str>,
    /// Cached Restaurant JSON-LD string, built at boot + rebuilt at the admin
    /// edit paths that change its inputs (menu, branding, weekly hours). Read
    /// synchronously during SSR so `/` and `/menu` carry structured data in
    /// the first byte with no per-request query; never emitted on the client.
    jsonld: rusterando_frontend::pages::seo::JsonLdHandle,
    /// Shutdown broadcast. Flipped to `true` the moment SIGTERM/SIGINT lands,
    /// BEFORE axum stops accepting. The long-lived SSE handlers select on this
    /// and close their streams immediately so graceful shutdown never blocks
    /// on an open `/api/live/*` connection (the old cause of the >60 s 502
    /// restart window — see the shutdown block in `main`).
    shutdown: tokio::sync::watch::Receiver<bool>,
}

impl axum::extract::FromRef<AppState> for LeptosOptions {
    fn from_ref(state: &AppState) -> Self {
        state.leptos_options.clone()
    }
}

/// Load the dotenv file, honouring an `ENV_FILE` override so a single
/// checkout can run any tenant/profile without symlinking or sourcing:
///
///   ENV_FILE=.env.rusterando.flizza cargo leptos watch
///
/// Falls back to the default `.env` (search up from CWD) when `ENV_FILE`
/// is unset. Either way, dotenvy does NOT override vars already present in
/// the environment, so an explicit `KEY=… cargo …` on the command line
/// still wins.
fn load_env() {
    match std::env::var("ENV_FILE") {
        Ok(path) if !path.is_empty() => {
            // Explicit operator request — fail loud. dotenvy aborts the
            // file on the first malformed line, so a silent miss would
            // boot the server on half-loaded config (wrong DB/defaults).
            dotenvy::from_filename(&path).unwrap_or_else(|e| {
                panic!("ENV_FILE={path} could not be loaded: {e}");
            });
        }
        _ => {
            // Default: best-effort `.env` (absent is fine for prod where
            // the systemd unit injects the environment directly).
            let _ = dotenvy::dotenv();
        }
    }
}

#[tokio::main]
async fn main() {
    load_env();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,sqlx=warn".into()),
        )
        .init();

    rusterando_frontend::register_server_fns();

    let conf = get_configuration(None).expect("read Leptos config");
    let leptos_options = conf.leptos_options;
    let addr = leptos_options.site_addr;

    // Admin uploads land under the static site root (`target/site` in dev,
    // `html` on prod) so nginx serves them from `/img/uploads/` directly —
    // the `data/` dir stays off the web root. `site_root` is leptos's own
    // notion of that directory, so we always agree with whatever serves
    // `/img/`. The deploy script's `rsync --delete` excludes `img/uploads/`
    // so these runtime files aren't wiped on release.
    let uploads_dir: Arc<str> =
        Arc::from(format!("{}/img/uploads", leptos_options.site_root).as_str());

    let db_url =
        std::env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite:./data/rusterando.db".into());

    // Make sure ./data exists when using the default file URL.
    if let Some(stripped) = db_url.strip_prefix("sqlite:") {
        let path = stripped.trim_start_matches("//");
        if let Some(parent) = std::path::Path::new(path).parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).ok();
            }
        }
    }

    let connect_opts: SqliteConnectOptions = db_url
        .parse::<SqliteConnectOptions>()
        .expect("parse DATABASE_URL")
        .create_if_missing(true)
        // Wait up to 5 s on SQLITE_BUSY instead of erroring immediately.
        // sqlite serialises writes via WAL but a slow read on a checkpoint
        // can still hand a BUSY back to a concurrent writer; the default
        // behaviour is to return an error to sqlx, which then bubbles up
        // as a 500. With this set, transient lock contention smooths
        // itself out without ever surfacing to the customer. 5 s is well
        // above the longest transaction we run (admin bulk imports take
        // ~200 ms in the wild).
        .busy_timeout(std::time::Duration::from_secs(5));

    let db = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(connect_opts)
        .await
        .expect("open sqlite pool");

    // Two-layer migrations:
    //   * the embedded folder under repo `migrations/` is the
    //     brand-neutral baseline schema. Compile-time included via
    //     sqlx::migrate!() so every binary carries its own schema.
    //   * an optional runtime overlay folder (env: MIGRATIONS_OVERLAY)
    //     is applied AFTER the baseline. Used by per-deployment
    //     overlays (e.g. Davids' menu seed) that must not leak into
    //     the public repo. Each overlay folder is a normal sqlx
    //     migrations dir, with its own version timestamps.
    //
    // sqlx tracks both layers in the same _sqlx_migrations table; the
    // overlay's versions must NOT clash with baseline versions. Use a
    // distinct timestamp prefix (e.g. 99999999000001) in overlay files
    // so they sort after every baseline migration.
    sqlx::migrate!("../../migrations")
        .run(&db)
        .await
        .expect("run baseline migrations");
    tracing::info!("baseline migrations applied");

    if let Ok(overlay_dir) = std::env::var("MIGRATIONS_OVERLAY") {
        let overlay_path = std::path::PathBuf::from(&overlay_dir);
        match sqlx::migrate::Migrator::new(overlay_path.as_path()).await {
            Ok(migrator) => {
                migrator.run(&db).await.expect("run overlay migrations");
                tracing::info!("overlay migrations applied: {overlay_dir}");
            }
            Err(e) => {
                tracing::warn!(
                    "MIGRATIONS_OVERLAY={overlay_dir} could not be read: {e} — skipping overlay"
                );
            }
        }
    }

    // Seed the PDF cover + theme libraries if they're still empty
    // (fresh deploy, or first run after the 20260522 migration on a
    // shop that never set the legacy single-value overrides). This
    // makes sure the admin sees at least one theme + one cover in the
    // /admin/pdf switcher instead of an empty list. Idempotent.
    if let Err(e) = rusterando_server::pdf::seed_pdf_library_if_empty(&db, &uploads_dir).await {
        tracing::warn!("seed_pdf_library_if_empty failed (non-fatal): {e}");
    }

    // Backfill SEO slugs on menu_categories (added by the 20260528 migration).
    // Idempotent: only fills rows with a NULL/empty slug, derived from
    // seo_slug(name), with a -N suffix on collision so /menu/<slug> is unique.
    if let Err(e) = backfill_category_slugs(&db).await {
        tracing::warn!("backfill_category_slugs failed (non-fatal): {e}");
    }

    let admin_password = std::env::var("ADMIN_PASSWORD").unwrap_or_else(|_| {
        tracing::warn!("ADMIN_PASSWORD not set — defaulting to 'admin' for local dev");
        "admin".to_string()
    });

    let notifier: NotifierHandle = match EmailNotifier::from_env() {
        Some(e) => {
            tracing::info!(
                "notifier: SMTP via {}:{} ({})",
                std::env::var("SMTP_HOST").unwrap_or_default(),
                std::env::var("SMTP_PORT").unwrap_or_default(),
                std::env::var("EMAIL_FROM").unwrap_or_default(),
            );
            Arc::new(e)
        }
        None => {
            tracing::warn!(
                "notifier: SMTP env not fully set (need SMTP_HOST, SMTP_USER, SMTP_PASS) — \
                 falling back to LogNotifier"
            );
            Arc::new(LogNotifier)
        }
    };

    let apns = match rusterando_server::apns::ApnsHandle::from_env(db.clone()) {
        Ok(a) => a,
        Err(e) => {
            tracing::warn!(target: "apns", error = %e, "APNs disabled");
            None
        }
    };

    // One-time read of the site theme from app_settings. Subsequent
    // updates flow through update_setting -> ThemeHandle::set, which
    // rewrites this same Arc in place.
    let theme_initial = rusterando_frontend::pages::settings::ssr::theme(&db).await;
    let theme = rusterando_frontend::pages::settings::ThemeHandle::new(theme_initial);

    // Same pattern for shop-contact data — one boot read, write-through
    // on /admin/branding edits via update_setting.
    let branding_initial = rusterando_frontend::branding::ssr::load_branding(&db).await;
    let branding = rusterando_frontend::branding::BrandingHandle::new(branding_initial);

    // Build the cached Restaurant JSON-LD once at boot (after the category
    // slug backfill, so menu data is final). Rebuilt by the admin edit paths.
    let jsonld = rusterando_frontend::pages::seo::JsonLdHandle::new(
        rusterando_frontend::pages::seo::build_restaurant_jsonld(&db, &branding.get()).await,
    );

    // Stripe mode — sandbox vs live. Same handle pattern: one boot
    // read, write-through on `update_setting('stripe_mode', …)`. The
    // S_STRIPE_* / L_STRIPE_* env vars are loaded from .env above;
    // we don't validate them here so missing keys surface at the
    // first request that needs them.
    let stripe_mode_initial = rusterando_frontend::stripe::ssr::load_mode(&db).await;
    let stripe_mode = rusterando_frontend::stripe::StripeModeHandle::new(stripe_mode_initial);
    tracing::info!(mode = ?stripe_mode_initial, "Stripe mode at boot");

    // i18n toggle — same boot read + write-through pattern. Default
    // is false (single-locale) so a fresh deployment behaves like
    // Davids' until the admin flips the switch in /admin/settings.
    let i18n_initial = rusterando_frontend::pages::settings::ssr::i18n_enabled(&db).await;
    let i18n = rusterando_frontend::pages::settings::I18nHandle::new(i18n_initial);
    tracing::info!(enabled = i18n_initial, "i18n toggle at boot");

    // Online-order pause switch — same boot read + write-through pattern.
    // Default false (orders follow opening hours only).
    let orders_paused_initial = rusterando_frontend::pages::settings::ssr::orders_paused(&db).await;
    let orders_paused =
        rusterando_frontend::pages::settings::OrdersPausedHandle::new(orders_paused_initial);
    tracing::info!(
        paused = orders_paused_initial,
        "orders-paused toggle at boot"
    );

    // Live customer channel hub (SSE). In-memory broadcast; nothing to
    // seed from the DB.
    let live = rusterando_frontend::live::LiveHub::new();

    // Kitchen-printer subsystem (see src/kitchen.rs). Spun up only
    // when KITCHEN_LISTEN_ADDR is set in .env so printerless deploys
    // skip the outbox writes entirely. Always loopback in production
    // — the SSH reverse tunnel from each shop's Pi forwards into it.
    let kitchen = match std::env::var("KITCHEN_LISTEN_ADDR") {
        Ok(addr_str) => match addr_str.parse::<std::net::SocketAddr>() {
            Ok(addr) => {
                let chan = rusterando_server::kitchen::KitchenChannel::new(db.clone());
                let listener_chan = chan.clone();
                // Respawn loop: on 2026-05-30 the listener exited once
                // with "Too many open files" and stayed dead the rest of
                // the evening — the printer Pi reconnected to a closed
                // port for 5 hours. The listener is a long-lived bind+
                // accept; if it ever returns Err (transient I/O hiccup,
                // listener_chan.clone() poisoned, anything), we log it
                // and respawn after a short backoff so the kitchen
                // subsystem self-heals without needing the whole process
                // to restart. The bind() inside run_listener will fail
                // if another instance is still holding the port — that's
                // the desired guard against double-bind.
                tokio::spawn(async move {
                    let mut backoff_s: u64 = 1;
                    loop {
                        let started = std::time::Instant::now();
                        match rusterando_server::kitchen::run_listener(listener_chan.clone(), addr)
                            .await
                        {
                            Ok(()) => {
                                tracing::info!("kitchen listener ended cleanly — respawning");
                                backoff_s = 1;
                            }
                            Err(e) => {
                                tracing::error!(
                                    error = %e,
                                    uptime_s = started.elapsed().as_secs(),
                                    backoff_s,
                                    "kitchen listener exited with error — respawning",
                                );
                            }
                        }
                        // Cap the backoff at 30 s so a printer Pi
                        // reconnecting after a long outage isn't stuck
                        // for minutes. If we ran for >60 s before failing,
                        // reset the backoff: a long-running listener that
                        // crashes once is healthy enough to start fresh.
                        if started.elapsed().as_secs() > 60 {
                            backoff_s = 1;
                        }
                        tokio::time::sleep(std::time::Duration::from_secs(backoff_s)).await;
                        backoff_s = (backoff_s * 2).min(30);
                    }
                });
                tracing::info!(%addr, "kitchen listener enabled (with respawn)");
                Some(chan)
            }
            Err(e) => {
                tracing::warn!(
                    addr = %addr_str,
                    error = %e,
                    "KITCHEN_LISTEN_ADDR set but unparseable — kitchen disabled",
                );
                None
            }
        },
        Err(_) => {
            tracing::info!("KITCHEN_LISTEN_ADDR unset — kitchen subsystem disabled");
            None
        }
    };

    // Shutdown broadcast: `false` until a signal lands, then `true`. The
    // sender lives in the shutdown future below; the receiver is cloned into
    // AppState so SSE handlers can close their streams the instant we begin
    // draining (otherwise they pin the process open until SIGKILL).
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    // Extra receiver for the post-signal drain cap (the AppState one is moved
    // in below for the SSE handlers).
    let shutdown_rx_for_cap = shutdown_rx.clone();

    let state = AppState {
        leptos_options: leptos_options.clone(),
        db: db.clone(),
        admin_password: Arc::new(admin_password),
        notifier: notifier.clone(),
        apns,
        theme,
        branding,
        kitchen,
        stripe_mode,
        i18n,
        orders_paused,
        live,
        uploads_dir: uploads_dir.clone(),
        jsonld,
        shutdown: shutdown_rx,
    };

    let routes = generate_route_list(App);

    // When the frontend was built with `--features i18n`, also register
    // a /<lang>/* mirror of every canonical route so axum can dispatch
    // /en/menu, /it/menu, etc. to the same Leptos handler. Without
    // this, the auto-generated route table only contains the canonical
    // paths (built once at boot with an empty mock RequestUrl), so
    // /en/menu falls through to the 404 catch-all.
    //
    // The Router base inside App() then strips the prefix per-request
    // so the inner Routes tree matches the canonical version. Default
    // locale (de) is intentionally excluded — its prefix is canonicalised
    // away by the locale_canonicalize_middleware below.
    // Always expand: every binary registers /<lang>/* routes for the
    // 7 non-default locales. Whether they ACTUALLY serve customer
    // content is decided at request time by `i18n_guard_middleware`
    // below, which 404s any prefixed path when the admin toggle is
    // off (Davids' default), so the SEO leak is closed.
    let routes = expand_locale_routes(routes);

    // Trait-erased push handle so server fns in the frontend crate
    // can broadcast without depending on davidspizzeria-server's
    // concrete ApnsHandle type.
    let broadcast_sink: rusterando_frontend::pages::push::BroadcastSinkHandle =
        state.apns.as_ref().map(|h| {
            std::sync::Arc::new(h.clone())
                as std::sync::Arc<dyn rusterando_frontend::pages::push::BroadcastSink>
        });

    // Same trait-erasure trick for the kitchen-printer outbox. Frontend
    // server fns (place_order, admin reprint endpoint) call this without
    // importing kitchen-protocol or this crate's KitchenChannel.
    let kitchen_sink: rusterando_frontend::pages::push::KitchenSinkHandle =
        state.kitchen.as_ref().map(|k| {
            std::sync::Arc::new(k.clone())
                as std::sync::Arc<dyn rusterando_frontend::pages::push::KitchenSink>
        });

    // Static menu.pdf cache rebuilder. The frontend admin server fns call this
    // (via MenuPdfCacheHandle in context) after menu/extras/branding edits so
    // nginx's offline fallback PDF stays current.
    let menu_pdf_site_url =
        std::env::var("PUBLIC_URL").unwrap_or_else(|_| format!("http://{addr}"));
    let menu_pdf_cache: rusterando_frontend::pages::push::MenuPdfCacheHandle = Some(
        std::sync::Arc::new(rusterando_server::pdf::MenuPdfCacheImpl {
            site_root: leptos_options.site_root.to_string(),
            uploads_dir: state.uploads_dir.to_string(),
            site_url: menu_pdf_site_url,
            branding: state.branding.clone(),
        }) as std::sync::Arc<dyn rusterando_frontend::pages::push::MenuPdfCache>,
    );

    // Prime the cache at boot so the fallback exists from the first request,
    // even before any admin edit. Best-effort + non-blocking — a slow Typst
    // render must not delay the listener coming up.
    {
        let cache = menu_pdf_cache.clone();
        let db = db.clone();
        tokio::spawn(async move {
            if let Some(c) = cache {
                if let Err(e) = c.rebuild(&db).await {
                    tracing::warn!("initial menu.pdf cache build failed: {e}");
                }
            }
        });
    }

    let app = Router::new()
        .leptos_routes_with_context(
            &state,
            routes,
            {
                let db = db.clone();
                let pwd = state.admin_password.clone();
                let notifier = notifier.clone();
                let apns = state.apns.clone();
                let sink = broadcast_sink.clone();
                let theme = state.theme.clone();
                let branding = state.branding.clone();
                let kitchen_sink = kitchen_sink.clone();
                let menu_pdf_cache = menu_pdf_cache.clone();
                let stripe_mode = state.stripe_mode.clone();
                let i18n = state.i18n.clone();
                let orders_paused = state.orders_paused.clone();
                let live = state.live.clone();
                let uploads =
                    rusterando_frontend::pages::settings::UploadsDir(state.uploads_dir.clone());
                let jsonld = state.jsonld.clone();
                move || {
                    provide_context(db.clone());
                    provide_context(pwd.clone());
                    provide_context(notifier.clone());
                    provide_context(apns.clone());
                    provide_context(sink.clone());
                    provide_context(theme.clone());
                    provide_context(branding.clone());
                    // KitchenSinkHandle — None on printerless deploys.
                    provide_context(kitchen_sink.clone());
                    // MenuPdfCacheHandle — drives the nginx offline-fallback refresh.
                    provide_context(menu_pdf_cache.clone());
                    provide_context(stripe_mode.clone());
                    provide_context(i18n.clone());
                    provide_context(orders_paused.clone());
                    provide_context(live.clone());
                    provide_context(uploads.clone());
                    provide_context(jsonld.clone());
                }
            },
            {
                let opts = leptos_options.clone();
                move || shell(opts.clone())
            },
        )
        // Specific routes BEFORE the /api/{*fn_name} catch-all so the Leptos
        // server-fn dispatcher doesn't swallow them.
        //
        // Stripe webhooks: per-mode routes are authoritative — the
        // signing secret used to verify the payload is hardwired by
        // the URL, so Stripe's sandbox dashboard can never end up
        // mutating live orders or vice versa. The legacy single
        // route is kept as an alias that dispatches via the active
        // mode (for shops whose Stripe dashboard still points at it).
        .route(
            "/api/webhooks/sandbox/stripe",
            axum::routing::post(stripe_webhook_sandbox),
        )
        .route(
            "/api/webhooks/live/stripe",
            axum::routing::post(stripe_webhook_live),
        )
        .route(
            "/api/webhook/stripe",
            axum::routing::post(stripe_webhook_legacy),
        )
        .route(
            "/api/webhooks/stripe",
            axum::routing::post(stripe_webhook_legacy),
        )
        // Admin image upload — specific path, registered BEFORE the
        // server-fn catch-all so leptos_axum doesn't swallow it.
        // The handler enforces the admin cookie internally.
        .route(
            "/api/admin/upload_image",
            axum::routing::post(upload_image_handler),
        )
        // PDF-Editor test-render: takes a Typst-source body, runs the
        // exact same render path the real /menu.pdf uses, returns
        // 200 OK on success or 400 + error text on failure. Lets the
        // editor reject broken templates without poisoning the live
        // setting.
        .route(
            "/api/admin/pdf/test_render",
            axum::routing::post(pdf_test_render_handler),
        )
        // Cover library: multipart upload of a new entry into
        // pdf_cover_images. Returns JSON {id, filename}.
        .route(
            "/api/admin/pdf/cover/upload",
            axum::routing::post(pdf_cover_upload_handler),
        )
        // Serve uploaded photos. They live under the static site root
        // (`<site_root>/img/uploads`) so on prod nginx serves them
        // straight from `html/img/uploads/` — the app's ServeDir here is
        // the fallback (dev, or if a request slips past nginx). The
        // deploy script excludes `img/uploads/` from its `rsync --delete`
        // so cargo-leptos rebuilds / releases don't wipe runtime uploads.
        .nest_service(
            "/img/uploads",
            tower_http::services::ServeDir::new(uploads_dir.as_ref()),
        );

    // Smoke-test endpoint for the kitchen printer chain. Lets us POST
    // a hand-crafted KitchenEvent and watch the receipt come out the
    // Pi's printer without going through Stripe. Compiled in only
    // under `--features debug-inject`; production builds omit it.
    #[cfg(feature = "debug-inject")]
    let app = app.route(
        "/api/kitchen/debug/inject",
        axum::routing::post(kitchen_debug_inject_handler),
    );

    let app = app
        .route("/api/{*fn_name}", axum::routing::any(server_fn_handler))
        .route("/qr.svg", axum::routing::get(qr_svg_handler))
        .route("/menu.pdf", axum::routing::get(menu_pdf_handler))
        // SEO: let crawlers discover the public pages and steer clear of
        // the transactional/admin/kitchen/driver areas. Both read
        // PUBLIC_URL + the live i18n toggle at request time so no rebuild
        // is needed when either changes.
        .route("/sitemap.xml", axum::routing::get(sitemap_handler))
        .route("/robots.txt", axum::routing::get(robots_handler))
        // Live customer channel (SSE): streams order events (admin
        // messages, status changes) to the customer's open /orders/{id}
        // page. Registered before the /api/{*fn_name} catch-all so it
        // isn't swallowed by the server-fn router.
        .route(
            "/api/live/orders/{id}",
            axum::routing::get(live_order_sse_handler),
        )
        // Global shop open/closed channel — home + cart subscribe so the
        // pause/force-open state flips without a reload.
        .route("/api/live/shop", axum::routing::get(live_shop_sse_handler))
        // Process-health probe — fd usage + uptime. Designed for an
        // external uptime monitor to poll every minute and alert on
        // `status != "ok"`. Unauthenticated, counters only.
        .route("/api/healthz", axum::routing::get(healthz_handler))
        .route(
            "/admin/history.csv",
            axum::routing::get(history_csv_handler),
        )
        // Apple Universal Links / Keychain webcredentials manifest. iOS
        // fetches this once after install, caches it for ~24h. Apple is
        // strict: must return 200 with `Content-Type: application/json`,
        // no redirects, no compression header mismatch. We bake the bytes
        // into the binary so deploys can never accidentally serve a
        // redirected/empty response.
        .route(
            "/.well-known/apple-app-site-association",
            axum::routing::get(apple_app_site_association_handler),
        )
        .fallback(leptos_axum::file_and_error_handler::<LeptosOptions, _>(
            shell,
        ))
        // /de/* canonicalisation runs first so all downstream layers
        // (rate-limit, admin-auth, Leptos) see the canonical URL.
        .layer(axum::middleware::from_fn(locale_canonicalize_middleware))
        // Then the i18n-toggle guard: 404 /<lang>/* when the admin
        // setting is off. Needs state to read the I18nHandle, so it
        // uses from_fn_with_state.
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            i18n_guard_middleware,
        ))
        // Rate limit must wrap EVERYTHING so it sees server-fn paths that
        // leptos_axum auto-registers (those bypass our /api/{*fn_name} catch-all).
        .layer(axum::middleware::from_fn(rate_limit_middleware))
        // Reject unauthenticated SSR requests to /admin/* (other than
        // /admin/login). API server-fns enforce auth themselves.
        .layer(axum::middleware::from_fn(admin_auth_middleware))
        .layer(CookieManagerLayer::new())
        .with_state(state);

    // Self-monitor: sample fd usage every 30 s. Logs proof-of-life
    // periodically when healthy, escalates on a leak, PROACTIVELY
    // EXITS at 95% so systemd's `Restart=always` brings us back
    // before axum's accept() starts rejecting with EMFILE. See
    // crates/rusterando-server/src/health.rs and
    // [[feedback-in-memory-first]] in the memory store. The whole
    // thing is in-process (one readdir + one getrlimit per tick)
    // — no disk writes, no nginx round-trip, no external probe.
    rusterando_server::health::spawn_self_monitor(std::time::Duration::from_secs(30));

    // Bind with a short retry window. During `systemctl restart` the old
    // process drains for up to 5 s while still holding the port; rather than
    // panicking and bouncing through systemd's RestartSec (which adds visible
    // 502s), we wait here and slot in the instant the port frees. ~8 s of
    // retries covers the 5 s drain cap plus margin; beyond that something is
    // genuinely wrong and we exit so systemd surfaces it.
    let listener = {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(8);
        loop {
            match tokio::net::TcpListener::bind(&addr).await {
                Ok(l) => break l,
                Err(e) if std::time::Instant::now() < deadline => {
                    tracing::warn!(error = %e, "bind {addr} failed — old process still draining, retrying in 250ms");
                    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                }
                Err(e) => {
                    tracing::error!(error = %e, "bind {addr} failed after 8s — giving up");
                    std::process::exit(1);
                }
            }
        }
    };
    // Label the listen line with the active tenant (the DB filename stem,
    // e.g. "flizza" / "davidspizzeria") rather than a hardcoded name, so
    // multi-tenant runs are self-evident in the logs.
    let tenant = std::env::var("DATABASE_URL")
        .ok()
        .and_then(|u| {
            std::path::Path::new(
                u.trim_start_matches("sqlite:")
                    .split('?')
                    .next()
                    .unwrap_or(""),
            )
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| "rusterando".into());
    tracing::info!("{tenant} listening on http://{addr}");

    // Graceful shutdown on SIGTERM (sent by `systemctl restart` /
    // `systemctl stop`). The signal handler stops `accept()` on the
    // listener, lets in-flight requests run to completion for up to
    // 5 s (place_order is ~200-500 ms, menu.pdf render ~2 s — 5 s
    // comfortably covers everything), then drops the sqlite pool so
    // WAL is checkpointed cleanly before the process ends. Without
    // this, customers mid-checkout get RST'd connections and the
    // next startup has to roll back WAL rather than starting fresh.
    //
    // SIGINT (Ctrl-C in dev) takes the same path so a developer's
    // ^C doesn't differ from prod's restart-by-systemd.
    //
    // We deliberately do NOT drain longer than 5 s: anything still
    // running past that is wedged and SIGKILL (delivered by systemd
    // after TimeoutStopSec=90 s) is the right answer. Also, customer-
    // facing restart-window perception scales linearly with the
    // drain timeout.
    // The shutdown future resolves ONLY when a signal arrives. It flips the
    // SSE-close watch, then resolves — `with_graceful_shutdown` then stops
    // accept() and drains in-flight requests. CRITICAL: this future must NOT
    // resolve on its own; if it did (or if we wrapped the whole serve in a
    // timeout), the server would shut itself down mid-operation. The 5 s drain
    // cap below is armed only AFTER the signal, never during normal serving.
    let db_for_shutdown = db.clone();
    let shutdown_signal = async move {
        let ctrl_c = async {
            let _ = tokio::signal::ctrl_c().await;
        };
        #[cfg(unix)]
        let terminate = async {
            use tokio::signal::unix::{signal, SignalKind};
            if let Ok(mut sig) = signal(SignalKind::terminate()) {
                sig.recv().await;
            } else {
                // If we can't install the SIGTERM handler (rare —
                // misconfigured kernel?) fall back to a future that
                // never resolves so Ctrl-C still works.
                std::future::pending::<()>().await;
            }
        };
        #[cfg(not(unix))]
        let terminate = std::future::pending::<()>();

        tokio::select! {
            _ = ctrl_c => tracing::info!("SIGINT received — draining"),
            _ = terminate => tracing::info!("SIGTERM received — draining"),
        }

        // Tell the SSE handlers to close NOW (and the separate drain-cap
        // future, which watches the same channel). Without this, a single open
        // `/api/live/*` stream keeps the graceful drain blocked until SIGKILL
        // (~90 s), which held the listen port and forced the freshly-started
        // process into a bind-retry loop — the >60 s 502 window.
        let _ = shutdown_tx.send(true);
    };

    // Run serve and the post-signal 5 s cap concurrently. `serve` completes on
    // its own once the graceful drain finishes (the common case, in ms). The
    // cap future waits for the shutdown watch to flip, then sleeps 5 s and
    // wins the race only if the drain is wedged — at which point we log and
    // fall through to exit. Normal serving never trips it because the watch
    // stays `false` until a real signal.
    let serve = axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal);

    let drain_cap = {
        let mut rx = shutdown_rx_for_cap;
        async move {
            // Wait until shutdown is actually signalled.
            while !*rx.borrow() {
                if rx.changed().await.is_err() {
                    // Sender dropped — process is going away anyway.
                    return;
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        }
    };

    tokio::select! {
        res = serve => {
            if let Err(e) = res {
                tracing::error!(error = %e, "axum::serve exited with error");
            }
        }
        _ = drain_cap => {
            tracing::warn!("graceful drain hit 5 s cap — forcing shutdown");
        }
    }

    // Post-drain: explicit sqlite pool close so WAL is checkpointed
    // before exit. `pool.close()` waits for all live connections to
    // be returned to the pool then runs a final checkpoint; with the
    // 5-s drain above complete, this is effectively instant. Bounded
    // by a 2-s timeout so a wedged checkpoint doesn't hold the
    // shutdown forever.
    tracing::info!("shutting down sqlite pool");
    let _ = tokio::time::timeout(std::time::Duration::from_secs(2), db_for_shutdown.close()).await;
    tracing::info!("graceful shutdown complete");
}

#[derive(serde::Deserialize)]
struct QrParams {
    url: Option<String>,
    size: Option<u32>,
}

/// Bytes of the AASA manifest, baked into the binary at compile time so
/// deploys can never accidentally serve a redirected or empty response.
/// Edit `public/.well-known/apple-app-site-association` to change.
const APPLE_APP_SITE_ASSOCIATION: &[u8] =
    include_bytes!("../../../public/.well-known/apple-app-site-association");

/// Process-health check for external uptime monitors. Returns the
/// current file-descriptor count + soft limit + percentage so a
/// monitoring service (or `curl https://davidspizzeria.de/api/healthz |
/// jq .status`) can alert before the box runs out of fds again. The
/// `status` field is the same `ok`/`warn`/`crit` ladder the periodic
/// in-process logger uses — see `health::status_for_pct`. The endpoint
/// is intentionally unauthenticated: it returns counters only, no PII,
/// no cart/order data, and the rate limiter still applies. A failure
/// to read the fd table degrades to `{fds: null, status: "ok"}` rather
/// than 500 — we never want the healthcheck itself to flap.
async fn healthz_handler() -> axum::response::Response {
    use axum::http::{header, StatusCode};
    use axum::response::IntoResponse;
    let snap = rusterando_server::health::fd_snapshot();
    let body = serde_json::to_string(&snap).unwrap_or_else(|_| "{}".to_string());
    // 200 even on warn/crit — operators read the JSON to decide. A
    // non-200 here would cause uptime monitors to alert on EVERY
    // elevated-fd reading instead of letting them threshold themselves.
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        body,
    )
        .into_response()
}

/// Serves the Apple App Site Association manifest. Apple's link verifier
/// is strict: it requires `200 OK`, exact `Content-Type: application/json`,
/// no redirects, and no `Content-Encoding` mismatch. The cache lives on
/// the device for ~24h after install — bumping the file means waiting or
/// reinstalling.
async fn apple_app_site_association_handler() -> impl axum::response::IntoResponse {
    use axum::http::header;
    (
        [
            (header::CONTENT_TYPE, "application/json"),
            // Apple recommends short TTL; this lets us iterate during dev.
            (header::CACHE_CONTROL, "public, max-age=3600"),
        ],
        APPLE_APP_SITE_ASSOCIATION,
    )
}

/// The only crawlable, indexable pages. Everything else the app serves
/// is transactional (`/checkout`, `/orders/{id}`) or staff-only
/// (`/admin/*`, `/kitchen/*`, `/driver/*`) — those are excluded from the
/// sitemap and `Disallow`ed in robots.txt below. Paths are relative to
/// the site root, no trailing slash except the empty home path.
const PUBLIC_PAGES: &[&str] = &["", "menu", "datenschutz", "impressum"];

/// Non-default locale prefixes, mirrored 1:1 from `expand_locale_routes`.
/// The default locale (de) is served un-prefixed and acts as `x-default`.
const SITEMAP_LOCALES: &[&str] = &["en", "fr", "it", "es", "pt", "ru", "cn"];

/// Fill `menu_categories.slug` for any row missing one (NULL/empty), derived
/// from `seo_slug(name)`. Idempotent: untouched rows keep their slug. On a
/// slug collision (two categories slugging to the same string) it appends
/// `-2`, `-3`, … so each `/menu/<slug>` resolves to exactly one category.
async fn backfill_category_slugs(db: &SqlitePool) -> Result<(), sqlx::Error> {
    use rusterando_shared::models::seo_slug;

    // Existing non-empty slugs, so generated ones don't collide with them.
    let mut taken: std::collections::HashSet<String> = sqlx::query_scalar::<_, String>(
        "SELECT slug FROM menu_categories WHERE slug IS NOT NULL AND slug <> ''",
    )
    .fetch_all(db)
    .await?
    .into_iter()
    .collect();

    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT id, name FROM menu_categories WHERE slug IS NULL OR slug = '' ORDER BY sort_order, id",
    )
    .fetch_all(db)
    .await?;

    for (id, name) in rows {
        let base = seo_slug(&name);
        let base = if base.is_empty() {
            "kategorie".to_string()
        } else {
            base
        };
        let mut slug = base.clone();
        let mut n = 2;
        while taken.contains(&slug) {
            slug = format!("{base}-{n}");
            n += 1;
        }
        sqlx::query("UPDATE menu_categories SET slug = ?1 WHERE id = ?2")
            .bind(&slug)
            .bind(&id)
            .execute(db)
            .await?;
        taken.insert(slug);
    }
    Ok(())
}

/// `/sitemap.xml` — lists the public pages so Google et al. can find and
/// index them. The base URL comes from `PUBLIC_URL` (so dev/staging/prod
/// each advertise their own origin) and the per-page `<xhtml:link
/// hreflang>` alternates are emitted only when i18n is enabled, matching
/// exactly which `/<lang>/*` URLs the router will actually serve.
async fn sitemap_handler(
    axum::extract::State(state): axum::extract::State<AppState>,
) -> impl axum::response::IntoResponse {
    use axum::http::header;

    let base = rusterando_frontend::pages::home::site_url();
    let base = base.trim_end_matches('/');
    let i18n_on = state.i18n.get();

    // Helper: full absolute URL for a page in a given locale prefix.
    // `prefix == ""` is the default-locale (de), un-prefixed URL.
    let url_for = |prefix: &str, page: &str| match (prefix, page) {
        ("", "") => base.to_string(),
        ("", p) => format!("{base}/{p}"),
        (lang, "") => format!("{base}/{lang}"),
        (lang, p) => format!("{base}/{lang}/{p}"),
    };

    let mut xml = String::from(r#"<?xml version="1.0" encoding="UTF-8"?>"#);
    xml.push('\n');
    xml.push_str(
        r#"<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9" xmlns:xhtml="http://www.w3.org/1999/xhtml">"#,
    );
    xml.push('\n');

    // Emit one `<url>` per locale for a page path (no leading slash; "" =
    // home). Shared by the static pages and the DB-driven dynamic pages so
    // both get identical hreflang treatment.
    let emit_page = |xml: &mut String, page: &str| {
        let locales: Vec<&str> = if i18n_on {
            std::iter::once("")
                .chain(SITEMAP_LOCALES.iter().copied())
                .collect()
        } else {
            vec![""]
        };
        for &loc in &locales {
            xml.push_str("  <url>\n");
            xml.push_str(&format!("    <loc>{}</loc>\n", url_for(loc, page)));
            if i18n_on {
                xml.push_str(&format!(
                    r#"    <xhtml:link rel="alternate" hreflang="x-default" href="{}"/>"#,
                    url_for("", page)
                ));
                xml.push('\n');
                xml.push_str(&format!(
                    r#"    <xhtml:link rel="alternate" hreflang="de" href="{}"/>"#,
                    url_for("", page)
                ));
                xml.push('\n');
                for &lang in SITEMAP_LOCALES {
                    xml.push_str(&format!(
                        r#"    <xhtml:link rel="alternate" hreflang="{lang}" href="{}"/>"#,
                        url_for(lang, page)
                    ));
                    xml.push('\n');
                }
            }
            xml.push_str("  </url>\n");
        }
    };

    // 1) Static public pages.
    for &page in PUBLIC_PAGES {
        emit_page(&mut xml, page);
    }

    // 2) Dynamic category pages — /menu/<slug> per active category.
    let cat_slugs: Vec<String> = sqlx::query_scalar::<_, String>(
        "SELECT COALESCE(slug, '') FROM menu_categories WHERE is_active = 1 AND slug IS NOT NULL AND slug <> '' ORDER BY sort_order",
    )
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();
    for slug in cat_slugs {
        emit_page(&mut xml, &format!("menu/{slug}"));
    }

    // 3) Dynamic delivery landing pages — /lieferservice/<slug> per active
    //    zone (slug derived from the zone name, same fn the page resolves by).
    let zone_names: Vec<String> = sqlx::query_scalar::<_, String>(
        "SELECT COALESCE(name, '') FROM delivery_zones WHERE is_active = 1 AND name IS NOT NULL AND name <> '' ORDER BY name",
    )
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();
    let mut seen = std::collections::HashSet::new();
    for name in zone_names {
        let slug = rusterando_shared::models::seo_slug(&name);
        // Several postcodes can share a zone name → one landing page each.
        if !slug.is_empty() && seen.insert(slug.clone()) {
            emit_page(&mut xml, &format!("lieferservice/{slug}"));
        }
    }

    xml.push_str("</urlset>\n");

    (
        [
            (header::CONTENT_TYPE, "application/xml; charset=utf-8"),
            (header::CACHE_CONTROL, "public, max-age=3600"),
        ],
        xml,
    )
}

/// `/robots.txt` — allows the public site, blocks the staff and
/// transactional areas from being crawled or indexed, and points
/// crawlers at the sitemap (absolute URL, as the spec requires).
async fn robots_handler() -> impl axum::response::IntoResponse {
    use axum::http::header;

    let base = rusterando_frontend::pages::home::site_url();
    let base = base.trim_end_matches('/');

    let body = format!(
        "User-agent: *\n\
         Disallow: /admin\n\
         Disallow: /kitchen\n\
         Disallow: /driver\n\
         Disallow: /checkout\n\
         Disallow: /orders/\n\
         Allow: /\n\
         \n\
         Sitemap: {base}/sitemap.xml\n"
    );

    (
        [
            (header::CONTENT_TYPE, "text/plain; charset=utf-8"),
            (header::CACHE_CONTROL, "public, max-age=3600"),
        ],
        body,
    )
}

/// `/api/live/orders/{id}` — SSE stream of live events for one order.
/// The customer's `/orders/{id}` page opens an `EventSource` here; the
/// server fans out `LiveEvent`s (admin messages, status changes) that
/// match this `order_id`. Each event is sent as a JSON `data:` line.
///
/// A keep-alive comment is emitted periodically so reverse proxies don't
/// reap the idle connection. `EventSource` reconnects automatically if the
/// stream drops, and the page also has the persisted state, so a missed
/// event is not fatal.
async fn live_order_sse_handler(
    axum::extract::Path(order_id): axum::extract::Path<String>,
    axum::extract::State(state): axum::extract::State<AppState>,
) -> axum::response::Sse<
    impl futures::Stream<Item = Result<axum::response::sse::Event, std::convert::Infallible>>,
> {
    use axum::response::sse::{Event, KeepAlive, Sse};
    use futures::StreamExt;

    let rx = state.live.subscribe();
    // BroadcastStream yields Err on lag; we drop those and keep only
    // events for this order. recv resumes after a lag, so the stream lives on.
    let stream = tokio_stream::wrappers::BroadcastStream::new(rx).filter_map(move |res| {
        let want = order_id.clone();
        async move {
            let ev = res.ok()?;
            if ev.order_id != want {
                return None;
            }
            // JSON-encode the LiveEvent as the SSE `data` payload.
            let json = serde_json::to_string(&ev).ok()?;
            Some(Ok(Event::default().data(json)))
        }
    });
    // End the stream the moment shutdown is signalled, so it never pins the
    // process open during a graceful drain. The browser's EventSource just
    // reconnects to the new process. `take_until` completes the stream when
    // the shutdown future resolves.
    let stream = stream.take_until(wait_for_shutdown(state.shutdown.clone()));

    // Keep-alive every 5s instead of axum's 15s default. The keep-alive
    // write is what detects a TCP RST/FIN from the client and tears down
    // the stream — without it, a closed browser tab keeps the upstream
    // TCP socket in CLOSE_WAIT until the next event arrives, and rare
    // events (shop pause + order status) mean those sockets can sit
    // half-open for minutes/hours. Empirically (2026-05-30) this leaks
    // ~1 fd per 8 closed clients with the 15s default; 5s reduces the
    // window 3× and brings steady-state fds in line with active clients.
    Sse::new(stream).keep_alive(KeepAlive::new().interval(std::time::Duration::from_secs(5)))
}

/// Resolves when the shutdown watch flips to `true` (SIGTERM/SIGINT received).
/// Used by the SSE handlers via `take_until` so long-lived streams close at
/// the start of a graceful drain instead of blocking it. Returns immediately
/// if the channel is already set or the sender has dropped.
async fn wait_for_shutdown(mut rx: tokio::sync::watch::Receiver<bool>) {
    if *rx.borrow() {
        return;
    }
    // `changed()` errors only if the sender dropped — treat that as "shut
    // down" too (the process is going away).
    while rx.changed().await.is_ok() {
        if *rx.borrow() {
            return;
        }
    }
}

/// `/api/live/shop` — SSE stream of global shop open/closed changes. Home +
/// cart open an `EventSource` here so the pause / force-open state flips
/// live (no reload). Filters the shared hub to `ShopStatus` events.
async fn live_shop_sse_handler(
    axum::extract::State(state): axum::extract::State<AppState>,
) -> axum::response::Sse<
    impl futures::Stream<Item = Result<axum::response::sse::Event, std::convert::Infallible>>,
> {
    use axum::response::sse::{Event, KeepAlive, Sse};
    use futures::StreamExt;
    use rusterando_shared::models::LiveKind;

    let rx = state.live.subscribe();
    let stream = tokio_stream::wrappers::BroadcastStream::new(rx).filter_map(|res| async move {
        let ev = res.ok()?;
        // Only the global shop-status events go on this channel.
        if !matches!(ev.kind, LiveKind::ShopStatus { .. }) {
            return None;
        }
        let json = serde_json::to_string(&ev).ok()?;
        Some(Ok(Event::default().data(json)))
    });
    // Close on shutdown so this stream never blocks the graceful drain.
    let stream = stream.take_until(wait_for_shutdown(state.shutdown.clone()));

    // 5s keep-alive (see live_order_sse_handler for the why).
    Sse::new(stream).keep_alive(KeepAlive::new().interval(std::time::Duration::from_secs(5)))
}

/// Max upload size — keeps decompression bombs out and matches what a
/// pizzeria photo realistically needs (a 4K JPEG comfortably fits).
const MAX_UPLOAD_BYTES: usize = 5 * 1024 * 1024;

/// `/api/admin/upload_image` — multipart POST that admin uses to attach
/// a photo to the hero / an offer / a gallery item. Streams bytes,
/// validates magic bytes, content-hashes the result, writes to
/// Debug-only: hand-injects a KitchenEvent into the outbox + broadcast
/// channel. Use case is smoke-testing the printer chain end-to-end
/// without firing a real order through Stripe. Returns 200 with the
/// assigned seq id on success, 503 when kitchen is disabled in .env,
/// 500 on any other error. Only wired into the router when the
/// `debug-inject` cargo feature is active.
#[cfg(feature = "debug-inject")]
async fn kitchen_debug_inject_handler(
    axum::extract::State(state): axum::extract::State<AppState>,
    axum::Json(event): axum::Json<kitchen_protocol::KitchenEvent>,
) -> axum::response::Response {
    use axum::http::StatusCode;
    use axum::response::IntoResponse;

    let Some(chan) = state.kitchen.as_ref() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "kitchen disabled — KITCHEN_LISTEN_ADDR unset",
        )
            .into_response();
    };

    match chan.broadcast(event).await {
        Ok(seq) => {
            tracing::info!(%seq, "kitchen debug-injected");
            (StatusCode::OK, format!("queued seq={seq}\n")).into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "kitchen debug-inject failed");
            (StatusCode::INTERNAL_SERVER_ERROR, format!("{e}\n")).into_response()
        }
    }
}

/// `data/uploads/<hash>.<ext>`, returns `{ "path": "/img/uploads/<hash>.<ext>" }`.
///
/// Why content-hash names? Re-uploading the same photo dedupes; the URL
/// is cache-stable forever; we never overwrite a file someone is still
/// linking to.
async fn upload_image_handler(
    axum::extract::State(state): axum::extract::State<AppState>,
    headers: axum::http::HeaderMap,
    mut multipart: axum::extract::Multipart,
) -> axum::response::Response {
    use axum::http::StatusCode;
    use axum::response::IntoResponse;
    use sha2::{Digest, Sha256};

    // Cookie gate. The leptos `require_admin` helper isn't reachable from
    // a non-server-fn route, so we read the same cookie ourselves.
    let admin_ok = headers
        .get(axum::http::header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .map(|s| {
            s.split(';')
                .any(|c| c.trim().eq_ignore_ascii_case("admin_session=ok"))
        })
        .unwrap_or(false);
    if !admin_ok {
        return (StatusCode::UNAUTHORIZED, "nicht angemeldet").into_response();
    }

    // Walk fields. We only care about a single `file` field; ignore the rest.
    let mut buf: Option<Vec<u8>> = None;
    while let Ok(Some(field)) = multipart.next_field().await {
        if field.name() != Some("file") {
            continue;
        }
        match field.bytes().await {
            Ok(b) => {
                if b.len() > MAX_UPLOAD_BYTES {
                    return (StatusCode::PAYLOAD_TOO_LARGE, "Datei zu groß (max. 5 MB).")
                        .into_response();
                }
                buf = Some(b.to_vec());
                break;
            }
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    format!("Upload fehlgeschlagen: {e}"),
                )
                    .into_response();
            }
        }
    }
    let Some(bytes) = buf else {
        return (StatusCode::BAD_REQUEST, "Kein 'file' Feld.").into_response();
    };

    // Magic-byte sniff. PNG, JPEG, WebP only — no SVG (stored XSS), no
    // GIF / AVIF / HEIC for v1.
    let ext = match bytes.as_slice() {
        b if b.starts_with(b"\x89PNG\r\n\x1a\n") => "png",
        b if b.starts_with(&[0xFF, 0xD8, 0xFF]) => "jpg",
        b if b.len() >= 12 && &b[0..4] == b"RIFF" && &b[8..12] == b"WEBP" => "webp",
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                "Nur JPEG, PNG oder WebP unterstützt.",
            )
                .into_response();
        }
    };

    // Hash → filename.
    let mut h = Sha256::new();
    h.update(&bytes);
    let hex = h
        .finalize()
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect::<String>();
    let filename = format!("{hex}.{ext}");

    let dir = std::path::PathBuf::from(state.uploads_dir.as_ref());
    if let Err(e) = std::fs::create_dir_all(&dir) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("mkdir uploads: {e}"),
        )
            .into_response();
    }
    let path = dir.join(&filename);
    // Skip the write if the file already exists (content-hash → identical bytes).
    if !path.exists() {
        if let Err(e) = std::fs::write(&path, &bytes) {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("write upload: {e}"),
            )
                .into_response();
        }
    }

    let public_path = format!("/img/uploads/{filename}");
    let body = format!(r#"{{"path":"{public_path}"}}"#);
    (
        StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        body,
    )
        .into_response()
}

/// `/api/admin/pdf/cover/upload` — multipart POST that adds a new entry
/// to the `pdf_cover_images` library. Two fields:
///   `file`  — the image bytes (JPEG or PNG only; max 5 MB)
///   `label` — short human-readable label, e.g. "Sommer-Aktion"
///
/// Writes to `data/uploads/covers/<sha256>.<ext>` and inserts a row,
/// returning `{ "id": <new row id>, "filename": "<sha256>.<ext>" }`.
/// The new cover is NOT auto-activated — admin clicks the radio in
/// the UI to switch.
async fn pdf_cover_upload_handler(
    axum::extract::State(state): axum::extract::State<AppState>,
    headers: axum::http::HeaderMap,
    mut multipart: axum::extract::Multipart,
) -> axum::response::Response {
    use axum::http::StatusCode;
    use axum::response::IntoResponse;
    use sha2::{Digest, Sha256};

    let admin_ok = headers
        .get(axum::http::header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .map(|s| {
            s.split(';')
                .any(|c| c.trim().eq_ignore_ascii_case("admin_session=ok"))
        })
        .unwrap_or(false);
    if !admin_ok {
        return (StatusCode::UNAUTHORIZED, "nicht angemeldet").into_response();
    }

    let mut buf: Option<Vec<u8>> = None;
    let mut label: Option<String> = None;
    while let Ok(Some(field)) = multipart.next_field().await {
        match field.name() {
            Some("file") => match field.bytes().await {
                Ok(b) => {
                    if b.len() > MAX_UPLOAD_BYTES {
                        return (StatusCode::PAYLOAD_TOO_LARGE, "Datei zu groß (max. 5 MB).")
                            .into_response();
                    }
                    buf = Some(b.to_vec());
                }
                Err(e) => {
                    return (
                        StatusCode::BAD_REQUEST,
                        format!("Upload fehlgeschlagen: {e}"),
                    )
                        .into_response();
                }
            },
            Some("label") => {
                if let Ok(s) = field.text().await {
                    let trimmed = s.trim();
                    if !trimmed.is_empty() {
                        label = Some(trimmed.to_string());
                    }
                }
            }
            _ => {}
        }
    }
    let Some(bytes) = buf else {
        return (StatusCode::BAD_REQUEST, "Kein 'file' Feld.").into_response();
    };
    let label = label.unwrap_or_else(|| "Cover".to_string());

    // Cover-only image whitelist: JPEG and PNG. WebP is fine for
    // hero/gallery images (the /api/admin/upload_image route) but not
    // worth the print-pipeline risk for cover photos.
    let (ext, mime) = match bytes.as_slice() {
        b if b.starts_with(b"\x89PNG\r\n\x1a\n") => ("png", "image/png"),
        b if b.starts_with(&[0xFF, 0xD8, 0xFF]) => ("jpg", "image/jpeg"),
        _ => {
            return (StatusCode::BAD_REQUEST, "Nur JPEG oder PNG unterstützt.").into_response();
        }
    };

    let mut h = Sha256::new();
    h.update(&bytes);
    let hex = h
        .finalize()
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect::<String>();
    let filename = format!("{hex}.{ext}");

    let dir = std::path::PathBuf::from(state.uploads_dir.as_ref()).join("covers");
    if let Err(e) = std::fs::create_dir_all(&dir) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("mkdir covers: {e}"),
        )
            .into_response();
    }
    let path = dir.join(&filename);
    let size = bytes.len() as i64;
    if !path.exists() {
        if let Err(e) = std::fs::write(&path, &bytes) {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("write upload: {e}"),
            )
                .into_response();
        }
    }

    let row: Result<(i64,), _> = sqlx::query_as(
        "INSERT INTO pdf_cover_images (label, filename, mime_type, size_bytes)
         VALUES (?1, ?2, ?3, ?4) RETURNING id",
    )
    .bind(&label)
    .bind(&filename)
    .bind(mime)
    .bind(size)
    .fetch_one(&state.db)
    .await;

    match row {
        Ok((id,)) => {
            let body = format!(r#"{{"id":{id},"filename":"{filename}"}}"#);
            (
                StatusCode::OK,
                [(axum::http::header::CONTENT_TYPE, "application/json")],
                body,
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("db insert cover: {e}"),
        )
            .into_response(),
    }
}

/// `/api/admin/pdf/test_render` — test-compile a Typst source against
/// the live menu payload. Returns 200 + empty body on success, 400 +
/// error text on Typst compile failure. The body is the raw Typst
/// source as plain text (UTF-8). Used by the PDF editor to validate
/// a template change before persisting it.
async fn pdf_test_render_handler(
    axum::extract::State(state): axum::extract::State<AppState>,
    headers: axum::http::HeaderMap,
    body: String,
) -> axum::response::Response {
    use axum::http::StatusCode;
    use axum::response::IntoResponse;

    let admin_ok = headers
        .get(axum::http::header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .map(|s| {
            s.split(';')
                .any(|c| c.trim().eq_ignore_ascii_case("admin_session=ok"))
        })
        .unwrap_or(false);
    if !admin_ok {
        return (StatusCode::UNAUTHORIZED, "nicht angemeldet").into_response();
    }

    let site_url = std::env::var("PUBLIC_URL").unwrap_or_else(|_| "http://127.0.0.1:3001".into());
    let branding = state.branding.get();
    let payload = match rusterando_server::pdf::load_menu_payload(&state.db, site_url, &branding)
        .await
    {
        Ok(p) => p,
        Err(e) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, format!("Datenbank: {e}")).into_response();
        }
    };
    // Pull live override images so the test-render uses the same
    // assets the real PDF will. Substitute the supplied template
    // source instead of the persisted one.
    let mut overrides =
        rusterando_server::pdf::load_pdf_overrides(&state.db, &state.uploads_dir).await;
    let source = if body.trim().is_empty() {
        rusterando_server::pdf::TEMPLATE_SRC.to_string()
    } else {
        body
    };
    overrides.template_source = Some(source);

    let result = tokio::task::spawn_blocking(move || {
        // Editor preview renders the default tri-fold WITH ingredients for
        // now; a future editor control can pass the chosen format/condensed
        // flag here.
        rusterando_server::pdf::render_menu_pdf(&payload, &overrides, "trifold", true)
    })
    .await;
    match result {
        Ok(Ok(_bytes)) => (StatusCode::OK, "").into_response(),
        Ok(Err(e)) => (StatusCode::BAD_REQUEST, format!("{e}")).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("render panicked: {e}"),
        )
            .into_response(),
    }
}

async fn qr_svg_handler(
    axum::extract::Query(p): axum::extract::Query<QrParams>,
) -> impl axum::response::IntoResponse {
    use axum::http::header;
    use axum::response::IntoResponse;
    use qrcode::render::svg;
    use qrcode::QrCode;

    let url = p
        .url
        .unwrap_or_else(rusterando_frontend::pages::home::site_url);
    let size = p.size.unwrap_or(512).clamp(64, 4096);

    let body = match QrCode::new(url.as_bytes()) {
        Ok(code) => code
            .render::<svg::Color>()
            .min_dimensions(size, size)
            .quiet_zone(true)
            .dark_color(svg::Color("#2a1a0f"))
            .light_color(svg::Color("#ffffff"))
            .build(),
        Err(_) => {
            return (axum::http::StatusCode::BAD_REQUEST, "QR encoding failed").into_response()
        }
    };

    (
        [
            (header::CONTENT_TYPE, "image/svg+xml; charset=utf-8"),
            (header::CACHE_CONTROL, "public, max-age=3600"),
        ],
        body,
    )
        .into_response()
}

#[derive(serde::Deserialize)]
struct HistoryCsvParams {
    from: Option<String>,
    to: Option<String>,
    /// "true" / "1" / "yes" → einbeziehen. Default: nur Live, weil der
    /// CSV-Export typischerweise dem Steuerberater übergeben wird.
    #[serde(default)]
    include_test: Option<String>,
}

async fn history_csv_handler(
    axum::extract::State(state): axum::extract::State<AppState>,
    axum::extract::Query(p): axum::extract::Query<HistoryCsvParams>,
    cookies: tower_cookies::Cookies,
) -> impl axum::response::IntoResponse {
    use axum::http::header;
    use axum::http::StatusCode;
    use axum::response::IntoResponse;
    use std::fmt::Write;

    // Cookie auth gate (same as require_admin in server-fn world).
    let authed = cookies
        .get("admin_session")
        .map(|c| c.value() == "ok")
        .unwrap_or(false);
    if !authed {
        return (StatusCode::UNAUTHORIZED, "nicht angemeldet").into_response();
    }

    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let week_ago = (chrono::Local::now() - chrono::Duration::days(7))
        .format("%Y-%m-%d")
        .to_string();
    let from = p.from.unwrap_or(week_ago);
    let to = p.to.unwrap_or(today.clone());

    if from.len() != 10 || to.len() != 10 {
        return (StatusCode::BAD_REQUEST, "from/to must be YYYY-MM-DD").into_response();
    }

    let from_ts = format!("{from} 00:00:00");
    let to_ts = format!("{to} 23:59:59");

    // Default = nur Live. Steuerberater darf NIE Test-Umsatz sehen.
    let include_test = p
        .include_test
        .as_deref()
        .map(|s| matches!(s, "true" | "1" | "yes"))
        .unwrap_or(false);
    let mode_clause = if include_test {
        ""
    } else {
        " AND stripe_mode = 'live'"
    };

    let sql = format!(
        "SELECT order_number, created_at, status, contact_name, contact_phone,
                contact_email, scheduled_for, total_cents
         FROM orders
         WHERE created_at BETWEEN ?1 AND ?2{mode_clause}
         ORDER BY created_at ASC",
    );
    let rows = match sqlx::query_as::<
        _,
        (
            String,         // order_number
            String,         // created_at
            String,         // status
            String,         // contact_name
            String,         // contact_phone
            String,         // contact_email
            Option<String>, // scheduled_for
            i64,            // total_cents
        ),
    >(&sql)
    .bind(&from_ts)
    .bind(&to_ts)
    .fetch_all(&state.db)
    .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("history csv query: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "DB Fehler").into_response();
        }
    };

    let mut body = String::new();
    // UTF-8 BOM so Excel for Windows decodes umlauts correctly
    body.push('\u{FEFF}');
    body.push_str("Bestellnummer;Erstellt;Status;Name;Telefon;E-Mail;Abholung;Summe (€)\n");
    for r in rows {
        let pickup = match &r.6 {
            Some(s) => s
                .split_whitespace()
                .nth(1)
                .map(|t| format!("Heute {t}"))
                .unwrap_or_else(|| s.clone()),
            None => "ASAP".to_string(),
        };
        let euros = format!("{},{:02}", r.7 / 100, (r.7 % 100).abs());
        let _ = writeln!(
            body,
            "{};{};{};{};{};{};{};{}",
            csv_escape(&r.0),
            csv_escape(&r.1),
            csv_escape(&r.2),
            csv_escape(&r.3),
            csv_escape(&r.4),
            csv_escape(&r.5),
            csv_escape(&pickup),
            euros,
        );
    }

    (
        [
            (header::CONTENT_TYPE, "text/csv; charset=utf-8".to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"bestellungen_{from}_{to}.csv\""),
            ),
        ],
        body,
    )
        .into_response()
}

fn csv_escape(s: &str) -> String {
    if s.contains(';') || s.contains('"') || s.contains('\n') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// Selects which Stripe key set a webhook handler validates against.
/// The route URL is the authoritative source — `/api/webhooks/sandbox/stripe`
/// only ever accepts sandbox-signed payloads, never live. Stops
/// cross-mode signature confusion at the entry point.
#[derive(Clone, Copy, Debug)]
enum WebhookMode {
    Sandbox,
    Live,
    /// Legacy `/api/webhooks/stripe` — reads the active StripeMode
    /// from the app_settings handle and uses that mode's secret.
    /// Kept for backwards compatibility with shops whose Stripe
    /// dashboard still points at the old single endpoint.
    Active,
}

impl WebhookMode {
    fn resolve_keys(self, state: &AppState) -> rusterando_frontend::stripe::StripeKeys {
        use rusterando_frontend::stripe::{StripeKeys, StripeMode};
        match self {
            Self::Sandbox => StripeKeys::for_mode(StripeMode::Sandbox),
            Self::Live => StripeKeys::for_mode(StripeMode::Live),
            Self::Active => state.stripe_mode.active_keys(),
        }
    }
}

async fn stripe_webhook_sandbox(
    state: axum::extract::State<AppState>,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> impl axum::response::IntoResponse {
    stripe_webhook_handler(WebhookMode::Sandbox, state, headers, body).await
}

async fn stripe_webhook_live(
    state: axum::extract::State<AppState>,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> impl axum::response::IntoResponse {
    stripe_webhook_handler(WebhookMode::Live, state, headers, body).await
}

/// Legacy single-endpoint webhook. Dispatches to whichever mode is
/// currently active in app_settings. Kept so an existing Stripe
/// dashboard endpoint at `/api/webhooks/stripe` (or `/api/webhook/stripe`)
/// doesn't 404 after the migration; new deploys should use the
/// mode-specific routes.
async fn stripe_webhook_legacy(
    state: axum::extract::State<AppState>,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> impl axum::response::IntoResponse {
    stripe_webhook_handler(WebhookMode::Active, state, headers, body).await
}

async fn stripe_webhook_handler(
    mode: WebhookMode,
    axum::extract::State(state): axum::extract::State<AppState>,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> impl axum::response::IntoResponse {
    use axum::http::StatusCode;
    use stripe::{EventObject, EventType, Webhook};

    let keys = mode.resolve_keys(&state);
    let secret = if keys.webhook_secret.is_empty() {
        tracing::error!(
            ?mode,
            stripe_mode = ?keys.mode,
            "webhook secret missing in .env; rejecting webhook",
        );
        return StatusCode::INTERNAL_SERVER_ERROR;
    } else {
        keys.webhook_secret.clone()
    };

    let sig = match headers
        .get("stripe-signature")
        .and_then(|v| v.to_str().ok())
    {
        Some(s) => s,
        None => return StatusCode::BAD_REQUEST,
    };
    let payload = match std::str::from_utf8(&body) {
        Ok(p) => p,
        Err(_) => return StatusCode::BAD_REQUEST,
    };

    let event = match Webhook::construct_event(payload, sig, &secret) {
        Ok(e) => e,
        Err(err) => {
            tracing::warn!(?err, "stripe webhook signature invalid");
            return StatusCode::BAD_REQUEST;
        }
    };

    // Idempotency: short-circuit on duplicate delivery.
    let event_id = event.id.to_string();
    let already: Option<(String,)> =
        sqlx::query_as("SELECT id FROM processed_stripe_events WHERE id = ?1")
            .bind(&event_id)
            .fetch_optional(&state.db)
            .await
            .unwrap_or(None);
    if already.is_some() {
        tracing::info!(%event_id, "stripe webhook: duplicate delivery, ignoring");
        return StatusCode::OK;
    }

    match event.type_ {
        EventType::PaymentIntentSucceeded => {
            if let EventObject::PaymentIntent(intent) = event.data.object {
                let order_id = intent.metadata.get("order_id").cloned().unwrap_or_default();
                if order_id.is_empty() {
                    tracing::warn!(intent_id = %intent.id, "stripe webhook: missing metadata.order_id");
                    return StatusCode::OK; // ack, nothing we can do
                }
                let method_label =
                    lookup_payment_method_detail(&intent.id.to_string(), &keys.secret_key).await;
                if let Err(e) = mark_order_paid(
                    &state,
                    &order_id,
                    &intent.id.to_string(),
                    method_label.as_deref(),
                )
                .await
                {
                    tracing::error!(?e, %order_id, "mark_order_paid failed");
                    return StatusCode::INTERNAL_SERVER_ERROR; // Stripe will retry
                }
            }
        }
        EventType::PaymentIntentPaymentFailed => {
            if let EventObject::PaymentIntent(intent) = event.data.object {
                let order_id = intent.metadata.get("order_id").cloned().unwrap_or_default();
                if !order_id.is_empty() {
                    // Look up the order number for the push body before we
                    // flip status (the row exists either way).
                    let order_number: Option<String> =
                        sqlx::query_scalar("SELECT order_number FROM orders WHERE id = ?1")
                            .bind(&order_id)
                            .fetch_optional(&state.db)
                            .await
                            .ok()
                            .flatten();

                    let _ = sqlx::query(
                        "UPDATE orders
                         SET payment_status = 'failed',
                             status = 'cancelled',
                             updated_at = CURRENT_TIMESTAMP
                         WHERE id = ?1",
                    )
                    .bind(&order_id)
                    .execute(&state.db)
                    .await;

                    if let (Some(apns), Some(num)) = (&state.apns, order_number) {
                        let title = "Zahlung fehlgeschlagen".to_string();
                        let body = format!("{num} — Bestellung wurde storniert.");
                        apns.notify("admin", &title, &body, None);
                    }
                }
            }
        }
        _ => {} // ignore everything else
    }

    let _ =
        sqlx::query("INSERT INTO processed_stripe_events (id) VALUES (?1) ON CONFLICT DO NOTHING")
            .bind(&event_id)
            .execute(&state.db)
            .await;

    StatusCode::OK
}

/// Flip the order to paid + received, then fire the customer confirmation
/// email. Returns Err if any DB op fails so Stripe will retry the webhook.
async fn mark_order_paid(
    state: &AppState,
    order_id: &str,
    intent_id: &str,
    method_detail: Option<&str>,
) -> anyhow::Result<()> {
    use rusterando_frontend::pages::order::notify::{OrderItemSummary, OrderSummary};

    sqlx::query(
        "UPDATE orders
         SET payment_status = 'paid',
             status = CASE WHEN status = 'pending_payment' THEN 'received' ELSE status END,
             stripe_payment_intent_id = ?1,
             payment_method_detail = COALESCE(?2, payment_method_detail),
             updated_at = CURRENT_TIMESTAMP
         WHERE id = ?3",
    )
    .bind(intent_id)
    .bind(method_detail)
    .bind(order_id)
    .execute(&state.db)
    .await?;

    // Reload the order + items so we can fire the email confirmation now
    // that payment is durable.
    let row: (
        String,         // 0 order_number
        String,         // 1 contact_name
        String,         // 2 contact_phone
        String,         // 3 contact_email
        Option<String>, // 4 scheduled_for
        i64,            // 5 total_cents
        String,         // 6 order_type
        i64,            // 7 subtotal_cents
        i64,            // 8 delivery_fee_cents
        Option<String>, // 9 delivery_address_json
        Option<String>, // 10 voucher_code
        i64,            // 11 voucher_discount_cents
    ) = sqlx::query_as(
        "SELECT order_number, contact_name, contact_phone, contact_email,
                scheduled_for, total_cents, order_type, subtotal_cents,
                delivery_fee_cents, delivery_address_json,
                voucher_code, voucher_discount_cents
         FROM orders WHERE id = ?1",
    )
    .bind(order_id)
    .fetch_one(&state.db)
    .await?;

    let item_rows: Vec<(Option<String>, String, i64, String, i64, Option<String>)> =
        sqlx::query_as(
            "SELECT menu_number_snapshot, name_snapshot, quantity, options_json,
                line_total_cents, extras_json
         FROM order_items WHERE order_id = ?1 ORDER BY id",
        )
        .bind(order_id)
        .fetch_all(&state.db)
        .await?;

    let is_delivery = row.6 == "delivery";
    let pickup_label = match row.4.as_deref() {
        Some(s) => format!("Heute {}", s.split_whitespace().nth(1).unwrap_or(s)),
        None => {
            if is_delivery {
                "Schnellstmöglich".to_string()
            } else {
                "ASAP".to_string()
            }
        }
    };

    let delivery_address_text = if is_delivery {
        row.9
            .as_deref()
            .and_then(|j| serde_json::from_str::<serde_json::Value>(j).ok())
            .map(|v| {
                let street = v.get("street").and_then(|x| x.as_str()).unwrap_or("");
                let house = v.get("house_number").and_then(|x| x.as_str()).unwrap_or("");
                let plz = v.get("postcode").and_then(|x| x.as_str()).unwrap_or("");
                let city = v.get("city").and_then(|x| x.as_str()).unwrap_or("");
                let zone = v.get("zone_name").and_then(|x| x.as_str()).unwrap_or("");
                let notes = v
                    .get("notes")
                    .and_then(|x| x.as_str())
                    .filter(|s| !s.is_empty());
                let mut s = format!("  {street} {house}\n  {plz} {city}\n  Liefergebiet: {zone}");
                if let Some(n) = notes {
                    s.push_str(&format!("\n  Hinweis: {n}"));
                }
                s
            })
            .unwrap_or_default()
    } else {
        String::new()
    };

    let public_url = std::env::var("PUBLIC_URL").unwrap_or_else(|_| "http://127.0.0.1:3001".into());
    let summary = OrderSummary {
        order_number: row.0.clone(),
        contact_name: row.1,
        contact_phone: row.2,
        contact_email: row.3,
        pickup_time_label: pickup_label,
        total_cents: row.5,
        items: item_rows
            .into_iter()
            .map(|(menu_number, name, qty, opts, line_total, extras_json)| {
                let v: serde_json::Value = serde_json::from_str(&opts).unwrap_or_default();
                let variant = v
                    .get("size_label")
                    .and_then(|x| x.as_str())
                    .map(String::from);
                let extras = extras_json
                    .as_deref()
                    .and_then(|j| serde_json::from_str(j).ok())
                    .unwrap_or_default();
                OrderItemSummary {
                    quantity: qty,
                    menu_number: menu_number.filter(|s| !s.is_empty()),
                    name,
                    variant,
                    line_total_cents: line_total,
                    extras,
                }
            })
            .collect(),
        public_url: format!("{public_url}/orders/{order_id}"),
        payment_method: "card".into(),
        order_type: row.6,
        delivery_address_text,
        delivery_fee_cents: row.8,
        subtotal_cents: row.7,
        voucher_code: row.10.clone().unwrap_or_default(),
        voucher_discount_cents: row.11,
        branding: state.branding.get(),
    };

    // New-order push: kitchen + admin always, driver only on deliveries
    // so they can start planning the route while the kitchen prepares.
    if let Some(apns) = &state.apns {
        let body = format!(
            "{name} · {total} · {n} Pos.",
            name = summary.contact_name,
            total = rusterando_shared::models::format_eur(summary.total_cents),
            n = summary.items.len(),
        );
        let title = format!("Neue Bestellung {}", summary.order_number);
        let deep_link = summary.public_url.clone();
        let mut roles: Vec<&str> = vec!["kitchen", "admin"];
        if summary.order_type == "delivery" {
            roles.push("driver");
        }
        apns.notify_roles(&roles, &title, &body, Some(&deep_link));
    }

    // Kitchen printer: now that payment is durable, broadcast the
    // NewOrder event so the Pi prints the receipt. Best-effort —
    // failures are logged but don't fail the webhook (Stripe would
    // otherwise retry and we'd duplicate the print).
    if let Some(kitchen) = state.kitchen.as_ref() {
        use rusterando_frontend::pages::push::KitchenSink;
        if let Err(e) = kitchen.broadcast_new_order(&state.db, order_id).await {
            tracing::warn!(
                order_id,
                error = %e,
                "kitchen broadcast (card path) failed",
            );
        }
    }

    let notifier = state.notifier.clone();
    let order_number_log = summary.order_number.clone();
    tokio::spawn(async move {
        if let Err(e) = notifier.send_confirmation(&summary).await {
            tracing::warn!("[order {order_number_log}] notifier failed: {e}");
        } else {
            tracing::info!("[order {order_number_log}] paid + confirmation sent");
        }
    });

    Ok(())
}

/// Resolve a short label for the instrument that actually settled the
/// payment — used in the invoice ("Online bezahlt · Apple Pay") and for
/// fee reconciliation against Stripe's monthly statement. Returns None if
/// the API call fails or the shape is unexpected; the webhook still acks.
async fn lookup_payment_method_detail(intent_id: &str, stripe_secret: &str) -> Option<String> {
    use stripe::{Client, Expandable, PaymentIntent, PaymentIntentId};

    if stripe_secret.is_empty() {
        return None;
    }
    let client = Client::new(stripe_secret.to_string());
    let id: PaymentIntentId = intent_id.parse().ok()?;
    let intent = PaymentIntent::retrieve(&client, &id, &["latest_charge.payment_method_details"])
        .await
        .ok()?;

    let charge = match intent.latest_charge? {
        Expandable::Object(c) => c,
        Expandable::Id(_) => return None, // expansion didn't resolve
    };
    let pmd = charge.payment_method_details?;

    // Wallet wins over plain card brand: Apple Pay / Google Pay / Link are
    // what we want on the invoice, even though the underlying type is "card".
    if let Some(card) = pmd.card.as_ref() {
        if let Some(w) = card.wallet.as_ref() {
            let label = match w.type_.as_str() {
                "apple_pay" => "Apple Pay",
                "google_pay" => "Google Pay",
                "link" => "Link",
                "samsung_pay" => "Samsung Pay",
                "amex_express_checkout" => "Amex Express Checkout",
                "masterpass" => "Masterpass",
                "visa_checkout" => "Visa Checkout",
                other => return Some(other.to_string()),
            };
            return Some(label.to_string());
        }
        if let Some(brand) = card.brand.as_deref() {
            // "visa" -> "Karte (Visa)" reads cleaner on a German receipt.
            return Some(format!("Karte ({})", capitalize(brand)));
        }
    }

    // Non-card method (sepa_debit, klarna, …): pmd.type_ is the discriminator.
    let t = pmd.type_;
    if t.is_empty() {
        None
    } else {
        Some(capitalize(&t))
    }
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(ch) => ch.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

/// `?format=` on `/menu.pdf`. Absent / unknown → the default tri-fold.
#[derive(serde::Deserialize)]
struct MenuPdfParams {
    format: Option<String>,
    /// `?condensed=1` drops the per-item ingredients line for the airy,
    /// title+price-only in-house menu. Absent / 0 = the full hand-out menu
    /// with ingredients (the default).
    condensed: Option<String>,
}

async fn menu_pdf_handler(
    axum::extract::State(state): axum::extract::State<AppState>,
    axum::extract::Query(p): axum::extract::Query<MenuPdfParams>,
) -> impl axum::response::IntoResponse {
    use axum::http::header;
    use axum::http::StatusCode;
    use axum::response::IntoResponse;

    // Only known formats; anything else falls back to the tri-fold default so
    // a typo in the query never errors the endpoint.
    let format = match p.format.as_deref() {
        Some("a5-zickzack") => "a5-zickzack",
        _ => "trifold",
    };
    // Ingredients are ON by default (full hand-out menu). `?condensed=1`
    // (also accepts "true"/"yes") flips to the title+price-only in-house menu.
    let condensed = matches!(
        p.condensed.as_deref(),
        Some("1") | Some("true") | Some("yes")
    );
    let show_ingredients = !condensed;
    // Distinct download filename per format + variant.
    let variant = if condensed { "-kompakt" } else { "" };
    let filename = match format {
        "a5-zickzack" => format!("davids-pizzeria-speisekarte-a5-zickzack{variant}.pdf"),
        _ => format!("davids-pizzeria-speisekarte{variant}.pdf"),
    };

    let site_url = std::env::var("PUBLIC_URL").unwrap_or_else(|_| "http://127.0.0.1:3001".into());

    let branding = state.branding.get();
    match rusterando_server::pdf::build_menu_pdf(
        &state.db,
        site_url,
        &branding,
        &state.uploads_dir,
        format,
        show_ingredients,
    )
    .await
    {
        Ok(bytes) => (
            [
                (header::CONTENT_TYPE, "application/pdf".to_string()),
                (
                    header::CONTENT_DISPOSITION,
                    format!("inline; filename=\"{filename}\""),
                ),
                (header::CACHE_CONTROL, "public, max-age=300".to_string()),
            ],
            bytes,
        )
            .into_response(),
        Err(e) => {
            tracing::error!("menu pdf failed: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("PDF Fehler: {e}"),
            )
                .into_response()
        }
    }
}

async fn server_fn_handler(
    axum::extract::State(state): axum::extract::State<AppState>,
    req: axum::extract::Request,
) -> axum::response::Response {
    use axum::response::IntoResponse;

    // Rate limiting lives in the router-level middleware (`rate_limit_middleware`)
    // so it covers BOTH this catch-all AND the server-fn routes that leptos_axum
    // auto-registers under their explicit endpoints.
    let sink: rusterando_frontend::pages::push::BroadcastSinkHandle =
        state.apns.as_ref().map(|h| {
            std::sync::Arc::new(h.clone())
                as std::sync::Arc<dyn rusterando_frontend::pages::push::BroadcastSink>
        });

    let kitchen_sink: rusterando_frontend::pages::push::KitchenSinkHandle =
        state.kitchen.as_ref().map(|k| {
            std::sync::Arc::new(k.clone())
                as std::sync::Arc<dyn rusterando_frontend::pages::push::KitchenSink>
        });

    leptos_axum::handle_server_fns_with_context(
        {
            let db = state.db.clone();
            let pwd = state.admin_password.clone();
            let notifier = state.notifier.clone();
            let apns = state.apns.clone();
            let theme = state.theme.clone();
            let branding = state.branding.clone();
            let stripe_mode = state.stripe_mode.clone();
            let i18n = state.i18n.clone();
            let orders_paused = state.orders_paused.clone();
            let live = state.live.clone();
            let uploads =
                rusterando_frontend::pages::settings::UploadsDir(state.uploads_dir.clone());
            let jsonld = state.jsonld.clone();
            move || {
                provide_context(db.clone());
                provide_context(pwd.clone());
                provide_context(notifier.clone());
                provide_context(apns.clone());
                provide_context(sink.clone());
                provide_context(theme.clone());
                provide_context(branding.clone());
                provide_context(kitchen_sink.clone());
                provide_context(stripe_mode.clone());
                provide_context(i18n.clone());
                provide_context(orders_paused.clone());
                provide_context(live.clone());
                provide_context(uploads.clone());
                provide_context(jsonld.clone());
            }
        },
        req,
    )
    .await
    .into_response()
}

/// Rate-limit middleware. Lives at the router level so it sees BOTH our
/// hand-rolled routes and the server-fn paths that leptos_axum auto-registers
/// (those don't go through `server_fn_handler`).
/// For each canonical route the leptos route generator produced,
/// emit additional `/<lang><canonical>` copies for every non-default
/// locale. axum then dispatches `/en/menu` to the same Leptos handler
/// that serves `/menu`; the App's `Router base=...` strips the prefix
/// inside the handler so the inner Routes tree matches as canonical.
///
/// Default locale (de) is intentionally skipped — its prefix is
/// canonicalised away to `/` by the locale_canonicalize_middleware
/// so /de/foo is never actually served.
fn expand_locale_routes(
    routes: Vec<leptos_axum::AxumRouteListing>,
) -> Vec<leptos_axum::AxumRouteListing> {
    use leptos_axum::AxumRouteListing;
    // Same set as Locale::ALL minus the default. Hard-coded here
    // because pulling the enum from the frontend crate would require
    // the i18n feature to be on for the dep too, which is fine — the
    // whole function is already #[cfg(feature = "i18n")].
    let prefixes = ["en", "fr", "it", "es", "pt", "ru", "cn"];
    let mut out = Vec::with_capacity(routes.len() * (1 + prefixes.len()));
    for r in &routes {
        out.push(r.clone());
    }
    for prefix in prefixes {
        for r in &routes {
            let p = r.path();
            // `path` is something like "/", "/menu", "/orders/{id}", …
            //
            // Register both "/<lang>" and "/<lang>/" for the home
            // route — axum treats them as distinct paths and a stray
            // trailing slash would otherwise 404.
            let make = |path: String| {
                AxumRouteListing::new(
                    path,
                    Default::default(),
                    [leptos_router::Method::Get, leptos_router::Method::Post],
                    Vec::<leptos_router::static_routes::RegenerationFn>::new(),
                )
            };
            if p == "/" {
                out.push(make(format!("/{prefix}")));
                out.push(make(format!("/{prefix}/")));
            } else {
                out.push(make(format!("/{prefix}{p}")));
            }
        }
    }
    out
}

/// Closes the SEO leak for single-locale deployments: when the admin
/// hasn't enabled i18n, /<lang>/* paths (for the 7 non-default
/// locales) should 404 instead of secretly serving translated content.
/// Wired before the leptos route handler so prefixed URLs don't reach
/// the route table even though it carries them.
///
/// Implemented as middleware (not a route) so a single `if` decides
/// the fate of all 7 prefixes without enumerating them in axum's
/// router tree.
async fn i18n_guard_middleware(
    axum::extract::State(state): axum::extract::State<AppState>,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use axum::http::StatusCode;
    use axum::response::IntoResponse;

    if state.i18n.get() {
        return next.run(req).await;
    }
    // i18n disabled — block the 7 non-default locale prefixes.
    let p = req.uri().path();
    for prefix in ["/en", "/fr", "/it", "/es", "/pt", "/ru", "/cn"] {
        if p == prefix || p.starts_with(&format!("{prefix}/")) {
            return (StatusCode::NOT_FOUND, "").into_response();
        }
    }
    next.run(req).await
}

/// `/de/...` is the redundant prefix for the default locale and should
/// 302 to the canonical un-prefixed URL. Browsing /de/menu lands on
/// /menu. Runs regardless of the i18n toggle — even single-locale
/// shops benefit from canonicalising stray /de/ links nobody made
/// themselves but a bot might guess.
async fn locale_canonicalize_middleware(
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use axum::http::{header, StatusCode};
    use axum::response::IntoResponse;

    let path = req.uri().path();
    if let Some(rest) = path.strip_prefix("/de/") {
        let qs = req
            .uri()
            .query()
            .map(|q| format!("?{q}"))
            .unwrap_or_default();
        let target = format!("/{rest}{qs}");
        return (StatusCode::FOUND, [(header::LOCATION, target)]).into_response();
    }
    if path == "/de" {
        let qs = req
            .uri()
            .query()
            .map(|q| format!("?{q}"))
            .unwrap_or_default();
        return (StatusCode::FOUND, [(header::LOCATION, format!("/{qs}"))]).into_response();
    }
    next.run(req).await
}

async fn rate_limit_middleware(
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use axum::http::StatusCode;
    use axum::response::IntoResponse;

    let path = req.uri().path();
    let phone_lookup = path.ends_with("/lookup_customer_by_phone");
    let address_lookup = path.ends_with("/validate_address");
    if phone_lookup || address_lookup {
        let ip = client_ip(req.headers(), req.extensions());
        // Both endpoints share the same bucket — together they're "look-up
        // anything about a customer" — so a probe can't burn through twice
        // the budget by alternating endpoints.
        if !LOOKUP_LIMITER.allow(&ip) {
            tracing::warn!(target: "ratelimit", %ip, %path, "lookup throttled");
            return (StatusCode::TOO_MANY_REQUESTS, "rate limit").into_response();
        }
    }
    next.run(req).await
}

/// Block any SSR request to `/admin/*` for unauthenticated users.
/// Without this, a non-logged-in visitor sees the admin shell + tile
/// grid (just with empty / errored data inside the Suspense), which is
/// both an information leak (route names are visible) and confusing UX.
///
/// Allowed paths through unconditionally:
///   - /admin/login                 (the login form itself)
///   - /api/...                     (server-fns: they enforce auth themselves
///                                   and we don't want to bounce a JSON
///                                   POST to an HTML redirect)
///   - everything outside /admin    (the public site)
///
/// Auth check is the same `admin_session=ok` cookie that
/// `require_admin()` reads server-side. Unauthenticated requests get a
/// 303 redirect to `/admin/login` — 303 is the right status for "your
/// GET turned into a redirect because of policy", and works on every
/// browser + the iOS WebView shell.
async fn admin_auth_middleware(
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use axum::http::{header, StatusCode};
    use axum::response::IntoResponse;

    let path = req.uri().path();

    // Pass-through: anything that isn't a top-level /admin SSR route.
    let is_admin_route = path == "/admin" || path.starts_with("/admin/");
    if !is_admin_route {
        return next.run(req).await;
    }
    // The login page must remain reachable so admins can sign in.
    if path == "/admin/login" || path.starts_with("/admin/login/") {
        return next.run(req).await;
    }

    // Cookie sniff. Same predicate as require_admin() but we read the
    // raw header here so we don't need to plumb tower_cookies into the
    // middleware (which is layered above the cookie manager for the
    // SSR catch-all).
    let authed = req
        .headers()
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .map(|s| {
            s.split(';')
                .any(|c| c.trim().eq_ignore_ascii_case("admin_session=ok"))
        })
        .unwrap_or(false);

    if authed {
        return next.run(req).await;
    }

    tracing::info!(target: "admin_auth", %path, "unauthenticated /admin/* — redirecting to login");
    (StatusCode::SEE_OTHER, [(header::LOCATION, "/admin/login")]).into_response()
}

/// Best-effort caller IP. Behind nginx we trust the leftmost `X-Forwarded-For`
/// entry; locally we fall back to the connection's peer address. This is good
/// enough for soft rate-limiting (the worst case is a shared NAT being
/// throttled together).
fn client_ip(headers: &axum::http::HeaderMap, ext: &axum::http::Extensions) -> String {
    if let Some(h) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()) {
        if let Some(first) = h.split(',').next() {
            let s = first.trim();
            if !s.is_empty() {
                return s.to_string();
            }
        }
    }
    if let Some(info) = ext.get::<axum::extract::ConnectInfo<std::net::SocketAddr>>() {
        return info.0.ip().to_string();
    }
    "unknown".to_string()
}

/// Tiny in-memory token bucket: 5 hits per 60s per IP. No external dependency,
/// no persistent state — a process restart resets every counter, which is
/// fine for the threat model.
struct RateLimiter {
    inner: std::sync::Mutex<std::collections::HashMap<String, RateBucket>>,
    limit: u32,
    window: std::time::Duration,
}

struct RateBucket {
    count: u32,
    window_started: std::time::Instant,
}

impl RateLimiter {
    fn new(limit: u32, window: std::time::Duration) -> Self {
        Self {
            inner: std::sync::Mutex::new(std::collections::HashMap::new()),
            limit,
            window,
        }
    }

    fn allow(&self, key: &str) -> bool {
        let mut map = match self.inner.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(), // poisoned mutex → keep serving rather than 500
        };
        let now = std::time::Instant::now();

        // Opportunistic eviction so the map doesn't grow forever.
        if map.len() > 4096 {
            map.retain(|_, b| now.duration_since(b.window_started) < self.window);
        }

        let bucket = map.entry(key.to_string()).or_insert_with(|| RateBucket {
            count: 0,
            window_started: now,
        });
        if now.duration_since(bucket.window_started) >= self.window {
            bucket.count = 0;
            bucket.window_started = now;
        }
        if bucket.count >= self.limit {
            return false;
        }
        bucket.count += 1;
        true
    }
}

// 20/min per IP: roomy enough that an honest customer typing their address
// across two fields (street + house, then postcode) plus a phone lookup
// never hits it; tight enough that a scraping loop is bored within a minute.
static LOOKUP_LIMITER: std::sync::LazyLock<RateLimiter> =
    std::sync::LazyLock::new(|| RateLimiter::new(20, std::time::Duration::from_secs(60)));
