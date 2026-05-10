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
}

impl axum::extract::FromRef<AppState> for LeptosOptions {
    fn from_ref(state: &AppState) -> Self {
        state.leptos_options.clone()
    }
}

#[tokio::main]
async fn main() {
    let _ = dotenvy::dotenv();
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
        .create_if_missing(true);

    let db = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(connect_opts)
        .await
        .expect("open sqlite pool");

    sqlx::migrate!("../../migrations")
        .run(&db)
        .await
        .expect("run migrations");
    tracing::info!("migrations applied");

    let admin_password = std::env::var("ADMIN_PASSWORD").unwrap_or_else(|_| {
        tracing::warn!("ADMIN_PASSWORD not set — defaulting to 'davids-pizza-admin' for local dev");
        "davids-pizza-admin".to_string()
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

    let state = AppState {
        leptos_options: leptos_options.clone(),
        db: db.clone(),
        admin_password: Arc::new(admin_password),
        notifier: notifier.clone(),
        apns,
        theme,
        branding,
    };

    let routes = generate_route_list(App);

    // Trait-erased push handle so server fns in the frontend crate
    // can broadcast without depending on davidspizzeria-server's
    // concrete ApnsHandle type.
    let broadcast_sink: rusterando_frontend::pages::push::BroadcastSinkHandle =
        state.apns.as_ref().map(|h| {
            std::sync::Arc::new(h.clone())
                as std::sync::Arc<dyn rusterando_frontend::pages::push::BroadcastSink>
        });

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
                move || {
                    provide_context(db.clone());
                    provide_context(pwd.clone());
                    provide_context(notifier.clone());
                    provide_context(apns.clone());
                    provide_context(sink.clone());
                    provide_context(theme.clone());
                    provide_context(branding.clone());
                }
            },
            {
                let opts = leptos_options.clone();
                move || shell(opts.clone())
            },
        )
        // Specific routes BEFORE the /api/{*fn_name} catch-all so the Leptos
        // server-fn dispatcher doesn't swallow them.
        .route(
            "/api/webhook/stripe",
            axum::routing::post(stripe_webhook_handler),
        )
        .route(
            "/api/webhooks/stripe",
            axum::routing::post(stripe_webhook_handler),
        )
        // Admin image upload — specific path, registered BEFORE the
        // server-fn catch-all so leptos_axum doesn't swallow it.
        // The handler enforces the admin cookie internally.
        .route(
            "/api/admin/upload_image",
            axum::routing::post(upload_image_handler),
        )
        // Serve uploaded photos from the persistent data dir (lives
        // outside target/site so cargo-leptos rebuilds don't wipe them
        // and the deploy script's data/ rsync preserves them across
        // releases).
        .nest_service(
            "/img/uploads",
            tower_http::services::ServeDir::new("data/uploads"),
        )
        .route("/api/{*fn_name}", axum::routing::any(server_fn_handler))
        .route("/qr.svg", axum::routing::get(qr_svg_handler))
        .route("/menu.pdf", axum::routing::get(menu_pdf_handler))
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
        // Rate limit must wrap EVERYTHING so it sees server-fn paths that
        // leptos_axum auto-registers (those bypass our /api/{*fn_name} catch-all).
        .layer(axum::middleware::from_fn(rate_limit_middleware))
        // Reject unauthenticated SSR requests to /admin/* (other than
        // /admin/login). API server-fns enforce auth themselves.
        .layer(axum::middleware::from_fn(admin_auth_middleware))
        .layer(CookieManagerLayer::new())
        .with_state(state);

    tracing::info!("davidspizzeria listening on http://{addr}");
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .await
    .unwrap();
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

/// Max upload size — keeps decompression bombs out and matches what a
/// pizzeria photo realistically needs (a 4K JPEG comfortably fits).
const MAX_UPLOAD_BYTES: usize = 5 * 1024 * 1024;

/// `/api/admin/upload_image` — multipart POST that admin uses to attach
/// a photo to the hero / an offer / a gallery item. Streams bytes,
/// validates magic bytes, content-hashes the result, writes to
/// `data/uploads/<hash>.<ext>`, returns `{ "path": "/img/uploads/<hash>.<ext>" }`.
///
/// Why content-hash names? Re-uploading the same photo dedupes; the URL
/// is cache-stable forever; we never overwrite a file someone is still
/// linking to.
async fn upload_image_handler(
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

    let dir = std::path::Path::new("data/uploads");
    if let Err(e) = std::fs::create_dir_all(dir) {
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
    >(
        "SELECT order_number, created_at, status, contact_name, contact_phone,
                contact_email, scheduled_for, total_cents
         FROM orders
         WHERE created_at BETWEEN ?1 AND ?2
         ORDER BY created_at ASC",
    )
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

async fn stripe_webhook_handler(
    axum::extract::State(state): axum::extract::State<AppState>,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> impl axum::response::IntoResponse {
    use axum::http::StatusCode;
    use stripe::{EventObject, EventType, Webhook};

    let secret = match std::env::var("STRIPE_WEBHOOK_SECRET") {
        Ok(s) => s,
        Err(_) => {
            tracing::error!("STRIPE_WEBHOOK_SECRET not set; rejecting webhook");
            return StatusCode::INTERNAL_SERVER_ERROR;
        }
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
                let method_label = lookup_payment_method_detail(&intent.id.to_string()).await;
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
    ) = sqlx::query_as(
        "SELECT order_number, contact_name, contact_phone, contact_email,
                scheduled_for, total_cents, order_type, subtotal_cents,
                delivery_fee_cents, delivery_address_json
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
async fn lookup_payment_method_detail(intent_id: &str) -> Option<String> {
    use stripe::{Client, Expandable, PaymentIntent, PaymentIntentId};

    let secret = std::env::var("STRIPE_SECRET_KEY").ok()?;
    let client = Client::new(secret);
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

async fn menu_pdf_handler(
    axum::extract::State(state): axum::extract::State<AppState>,
) -> impl axum::response::IntoResponse {
    use axum::http::header;
    use axum::http::StatusCode;
    use axum::response::IntoResponse;

    let site_url = std::env::var("PUBLIC_URL").unwrap_or_else(|_| "http://127.0.0.1:3001".into());

    let branding = state.branding.get();
    match rusterando_server::pdf::build_menu_pdf(&state.db, site_url, &branding).await {
        Ok(bytes) => (
            [
                (header::CONTENT_TYPE, "application/pdf".to_string()),
                (
                    header::CONTENT_DISPOSITION,
                    "inline; filename=\"davids-pizzeria-speisekarte.pdf\"".to_string(),
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

    leptos_axum::handle_server_fns_with_context(
        {
            let db = state.db.clone();
            let pwd = state.admin_password.clone();
            let notifier = state.notifier.clone();
            let apns = state.apns.clone();
            let theme = state.theme.clone();
            let branding = state.branding.clone();
            move || {
                provide_context(db.clone());
                provide_context(pwd.clone());
                provide_context(notifier.clone());
                provide_context(apns.clone());
                provide_context(sink.clone());
                provide_context(theme.clone());
                provide_context(branding.clone());
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
