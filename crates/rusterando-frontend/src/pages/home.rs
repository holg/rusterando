use leptos::prelude::*;
use rusterando_shared::models::format_eur;
use serde::{Deserialize, Serialize};

const DEFAULT_SITE_URL: &str = "http://127.0.0.1:3001";

/// What the home page renders in the "Wir liefern" section. Server-fed
/// so the values stay in sync with whatever David sets in
/// /admin/settings + delivery_zones.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct HomeDeliveryInfo {
    /// Zones in display order.
    pub zones: Vec<DeliveryZoneSummary>,
    /// Free-delivery threshold in cents. 0 = promo disabled.
    pub free_delivery_threshold_cents: i64,
    /// Three-level shop status driving the banner colour (green / amber /
    /// red). `OpensLater` (amber) = closed now but opens later today or a
    /// pause/snooze is on; `Closed` (red) = Ruhetag / nothing more today.
    #[serde(default)]
    pub shop_level: rusterando_shared::models::ShopLevel,
    /// Customer-facing reason shown in the banner when not plainly open.
    #[serde(default)]
    pub closed_reason: String,
    /// Weekly opening hours for the public table, Mon-first. Built from
    /// the DB so the displayed hours always match what gates ordering.
    #[serde(default)]
    pub hours_rows: Vec<DayHours>,
}

/// Localized weekday label for the public hours table. Maps the DB
/// weekday number (0=Sun..6=Sat) to the existing i18n `t!` key so the
/// table stays translated.
fn home_weekday_label(weekday: i64) -> String {
    match weekday {
        0 => crate::t!("home.day_sun"),
        1 => crate::t!("home.day_mon"),
        2 => crate::t!("home.day_tue"),
        3 => crate::t!("home.day_wed"),
        4 => crate::t!("home.day_thu"),
        5 => crate::t!("home.day_fri"),
        6 => crate::t!("home.day_sat"),
        _ => String::new(),
    }
}

/// One row of the public opening-hours table. `weekday` (0=Sun..6=Sat)
/// lets the view pick the localized label via `t!`; `windows` is the
/// collapsed time string ("11:30–14:30 · 17:00–22:00"), empty = closed.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DayHours {
    pub weekday: i64,
    pub windows: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeliveryZoneSummary {
    pub name: String,
    pub fee_cents: i64,
    pub min_order_cents: i64,
}

#[server(
    name = HomeDeliveryInfoFn,
    prefix = "/api",
    endpoint = "home_delivery_info"
)]
pub async fn home_delivery_info() -> Result<HomeDeliveryInfo, ServerFnError> {
    use sqlx::SqlitePool;

    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;

    let zones: Vec<(Option<String>, i64, i64)> = sqlx::query_as(
        "SELECT name, fee_cents, min_order_cents
         FROM delivery_zones
         WHERE is_active = 1
         ORDER BY fee_cents, id",
    )
    .fetch_all(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("zones: {e}")))?;

    let zones = zones
        .into_iter()
        .map(|(name, fee, min_order)| DeliveryZoneSummary {
            name: name.unwrap_or_else(|| "?".to_string()),
            fee_cents: fee,
            min_order_cents: min_order,
        })
        .collect();

    let threshold = crate::pages::settings::ssr::free_delivery_threshold_cents(&db).await;

    // Closed state mirrors the place_order gate (manual pause or outside
    // opening hours) so the home page can warn before the customer even
    // opens the menu.
    let (shop_level, closed_reason) = crate::pages::order::ssr::shop_open_state(&db).await;

    // Weekly hours for the public table — built from the DB so what's
    // shown always matches what gates ordering. Multiple shift rows per
    // weekday are collapsed into one "A–B · C–D" string; a day with no
    // open rows (or all closed) renders as Ruhetag (empty windows).
    let raw = sqlx::query_as::<_, (i64, String, String, i64)>(
        "SELECT weekday, open_time, close_time, is_closed
         FROM opening_hours ORDER BY weekday, open_time",
    )
    .fetch_all(&db)
    .await
    .unwrap_or_default();
    let week_order = [1_i64, 2, 3, 4, 5, 6, 0]; // Mon-first display
    let hours_rows = week_order
        .into_iter()
        .map(|wd| {
            let windows = raw
                .iter()
                .filter(|(w, _, _, closed)| *w == wd && *closed == 0)
                .map(|(_, o, c, _)| format!("{o}–{c}"))
                .collect::<Vec<_>>()
                .join(" · ");
            DayHours {
                weekday: wd,
                windows,
            }
        })
        .collect();

    Ok(HomeDeliveryInfo {
        zones,
        free_delivery_threshold_cents: threshold,
        shop_level,
        closed_reason,
        hours_rows,
    })
}

/// The public URL the QR code should encode. Reads `PUBLIC_URL` at SSR time
/// so dev/staging/prod each get the right link without a rebuild.
#[cfg(feature = "ssr")]
pub fn site_url() -> String {
    std::env::var("PUBLIC_URL").unwrap_or_else(|_| DEFAULT_SITE_URL.to_string())
}

/// Server-side: build the QR SVG once at render time. Tiny (<2 KB), inlined
/// into the SSR HTML so no extra round-trip is needed.
#[cfg(feature = "ssr")]
pub fn qr_svg(url: &str) -> String {
    use qrcode::render::svg;
    use qrcode::QrCode;

    match QrCode::new(url.as_bytes()) {
        Ok(code) => code
            .render::<svg::Color>()
            .min_dimensions(220, 220)
            .quiet_zone(true)
            .dark_color(svg::Color("#2a1a0f"))
            .light_color(svg::Color("#ffffff"))
            .build(),
        Err(_) => String::new(),
    }
}

#[cfg(not(feature = "ssr"))]
fn qr_svg(_url: &str) -> String {
    String::new()
}

#[cfg(not(feature = "ssr"))]
pub fn site_url() -> String {
    DEFAULT_SITE_URL.to_string()
}

#[component]
pub fn Home() -> impl IntoView {
    use crate::pages::home_content::get_home_content;

    let url = site_url();
    let display = url
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_end_matches('/')
        .to_string();
    let pdf_url = format!("{}/menu.pdf", url.trim_end_matches('/'));
    let qr_site = qr_svg(&url);
    let qr_pdf = qr_svg(&pdf_url);
    // Bundle both fetches into one resource so SSR and hydrate see
    // exactly the same await-point. Two `OnceResource`s consumed by
    // two `<Suspense>` blocks intermittently trips tachys's hydration
    // walker with `entered unreachable code` — collapsing into one
    // tuple-future avoids it (same pattern as MenuPage).
    //
    // MUST stay `OnceResource`: it owns the SSR→hydrate value handoff.
    // A plain `Resource` keyed on a client signal hydrates differently
    // and reintroduced the tachys hydration.rs:163 panic. The initial
    // open/closed banner comes from this resource (correct on load); the
    // LIVE flip comes from the SSE shop-status signal below, applied
    // reactively in the view — never altering SSR/hydrate DOM shape.
    let combined = OnceResource::new(async move {
        let h = get_home_content().await;
        let d = home_delivery_info().await;
        (h, d)
    });

    view! {
        <Suspense fallback=|| view! {
            <section class="hero hero-cinematic">
                <div class="hero-content"><h1 class="logo">"MEIN RESTAURANT"</h1></div>
            </section>
            <section id="delivery" class="delivery">
                <h2>{crate::t!("home.delivery_title")}</h2>
            </section>
        }>
            {move || combined.get().map(|(home_res, delivery_res)| {
                let hero = match home_res {
                    Err(_) => view! {
                        <section class="hero hero-cinematic">
                            <div class="hero-content">
                                <h1 class="logo">"MEIN RESTAURANT"</h1>
                                <p class="tag">"Konfiguriere Hero-Titel + Untertitel unter /admin/home."</p>
                            </div>
                        </section>
                    }.into_any(),
                    Ok(content) => view! { <HomeBody content/> }.into_any(),
                };
                // Prominent shop-status bar at the very top of the page (above
                // the hero), so 🟢 open / 🔴 closed / Heute Ruhetag is the first
                // thing seen. Initial state from the resource; the SSE
                // `shop_override` signal flips it live (read inside the closure
                // → reactive, no reload, hydration-safe).
                let (init_level, init_reason) = match &delivery_res {
                    Ok(info) => (info.shop_level, info.closed_reason.clone()),
                    Err(_) => (rusterando_shared::models::ShopLevel::Open, String::new()),
                };
                let top_status_bar = view! {
                    <crate::components::shop_status_bar::ShopStatusBar
                        init_level init_reason/>
                };
                let delivery_body = match delivery_res {
                    Err(_) => view! {
                        <p class="muted">{crate::t!("home.delivery_unavailable")}</p>
                    }.into_any(),
                    Ok(info) => {
                        let threshold = info.free_delivery_threshold_cents;
                        let banner = (threshold > 0).then(|| {
                            let label = format_eur(threshold);
                            let template = crate::t!("home.free_delivery_banner");
                            view! {
                                <p class="free-delivery-banner" inner_html=template.replace("{amount}", &format!("<strong>{label}</strong>"))></p>
                            }
                        });
                        view! {
                            {banner}
                            <ul>
                                {info.zones.into_iter().map(|z| view! {
                                    <li>
                                        <strong>{z.name}</strong>
                                        <span>{crate::t!("home.zone_min_order_prefix")} " " {format_eur(z.min_order_cents)} " · "</span>
                                        <span class="fee">"+ " {format_eur(z.fee_cents)} " " {crate::t!("home.zone_fee_suffix")}</span>
                                    </li>
                                }).collect_view()}
                            </ul>
                        }.into_any()
                    }
                };
                view! {
                    {top_status_bar}
                    {hero}
                    <section id="delivery" class="delivery">
                        <h2>{crate::t!("home.delivery_title")}</h2>
                        {delivery_body}
                    </section>
                }
            })}
        </Suspense>

        <section class="qr-card">
            <div class="qr-intro">
                <h2>{crate::t!("home.qr_title")}</h2>
                <p>
                    {crate::t!("home.qr_intro")} " "
                    <strong>{display}</strong>
                </p>
            </div>

            <div class="qr-pair">
                <figure class="qr-tile">
                    <div class="qr-svg" inner_html=qr_site></div>
                    <figcaption>
                        <strong>{crate::t!("home.qr_online_title")}</strong>
                        <span>{crate::t!("home.qr_online_sub")}</span>
                    </figcaption>
                </figure>
                <figure class="qr-tile">
                    <div class="qr-svg" inner_html=qr_pdf></div>
                    <figcaption>
                        <strong>{crate::t!("home.qr_pdf_title")}</strong>
                        <span>{crate::t!("home.qr_pdf_sub")}</span>
                    </figcaption>
                </figure>
            </div>

            <p class="qr-actions">
                <a class="btn ghost" href="/menu.pdf" target="_blank" rel="noopener">
                    {format!("📄 {}", crate::t!("home.pdf_download"))}
                </a>
                // Downloadable QR for the shop URL. Points at the cached
                // /qr.svg endpoint (Cache-Control 1h, defaults to site_url),
                // size=1024 so the saved vector has a large min-dimension for
                // print. `download` names the saved file.
                <a class="btn ghost" href="/qr.svg?size=1024" download="davidspizzeria-qr.svg">
                    {format!("⬇ {}", crate::t!("home.qr_download"))}
                </a>
            </p>
        </section>

        <section id="hours" class="hours">
            <h2>{crate::t!("header.opening_hours")}</h2>
            // Explicit <tbody>: browsers auto-insert one when parsing
            // <table><tr>... HTML, but Leptos's view tree doesn't emit
            // it. The SSR HTML therefore says <table><tr> while the
            // hydrated DOM is <table><tbody><tr>, and tachys's hydrate
            // walker panics at hydration.rs:248 ("hydration error
            // occurred while trying to hydrate an element defined at
            // pages/home.rs:217:22"). Adding <tbody> here makes both
            // sides agree.
            <table>
                <tbody>
                    // The rows come from `combined` (a resource). A resource
                    // read MUST happen inside <Suspense>, otherwise SSR (not
                    // yet resolved → empty) and hydrate (resolved → 7 rows)
                    // produce different DOM at this node and tachys panics at
                    // hydration.rs:163. The fallback emits nothing so the
                    // <tbody>'s child shape matches on both sides.
                    <Suspense fallback=|| ().into_view()>
                        {move || combined.get().map(|(_, delivery_res)| {
                            let rows = delivery_res.map(|info| info.hours_rows).unwrap_or_default();
                            rows.into_iter().map(|row| {
                                let label = home_weekday_label(row.weekday);
                                if row.windows.is_empty() {
                                    view! {
                                        <tr><th>{label}</th><td class="closed">{crate::t!("home.day_closed")}</td></tr>
                                    }.into_any()
                                } else {
                                    view! {
                                        <tr><th>{label}</th><td>{row.windows}</td></tr>
                                    }.into_any()
                                }
                            }).collect_view()
                        })}
                    </Suspense>
                </tbody>
            </table>
        </section>

        <footer class="site-footer">
            <a href="/admin">"Admin"</a>
            <span class="sep">"·"</span>
            <a href="/impressum">{crate::t!("header.imprint")}</a>
            <span class="sep">"·"</span>
            <a href="/datenschutz">{crate::t!("header.privacy")}</a>
        </footer>
    }
}

#[component]
fn HomeBody(content: crate::pages::home_content::HomeContent) -> impl IntoView {
    use crate::pages::home_content::{resolve_hero_background, HomeContent, Offer};
    let HomeContent {
        hero,
        offers,
        gallery,
    } = content;
    let title_upper = hero.title.to_uppercase();

    // Either an image URL (render <img>) or a CSS background string
    // (apply via inline style on .hero-bg). Falls back to the theme's
    // --hero-bg variable when both are None.
    let (hero_img, hero_css_bg) = resolve_hero_background(&hero.image_path);
    let bg_style = hero_css_bg
        .as_ref()
        .map(|css| format!("background: {css};"))
        .unwrap_or_default();

    view! {
        <section class="hero hero-cinematic">
            <div class="hero-bg" aria-hidden="true" style=bg_style>
                {hero_img.map(|src| view! {
                    <img src=src alt="" class="hero-photo" loading="eager"/>
                })}
                <div class="hero-vignette"></div>
            </div>
            <div class="hero-content">
                <h1 class="logo">{title_upper}</h1>
                <p class="tag">{hero.subtitle}</p>
                {render_meta()}
                <p class="cta">
                    <a class="btn pill big" href="/menu">{crate::t!("home.order_now")}</a>
                </p>
            </div>
            <a class="hero-scroll-cue" href="#offers" aria-label=crate::t!("home.scroll_more")>"↓"</a>
        </section>

        <section id="offers" class="offers">
            {offers.into_iter().map(|o: Offer| view! { <OfferCard o/> }).collect_view()}
        </section>

        <section class="gallery">
            {gallery.into_iter().map(|g| view! {
                <figure>
                    <img src=g.image_path alt=g.caption.clone() loading="lazy"/>
                    <figcaption>{g.caption}</figcaption>
                </figure>
            }).collect_view()}
        </section>
    }
}

#[component]
fn OfferCard(o: crate::pages::home_content::Offer) -> impl IntoView {
    let has_image = o.image_path.is_some();
    let class = if has_image {
        "offer offer-photo"
    } else {
        "offer"
    };
    let bg = o
        .image_path
        .as_deref()
        .map(|p| format!("background-image: url('{p}');"))
        .unwrap_or_default();

    view! {
        <article class=class style=bg>
            <h3>{o.title}</h3>
            <p class="price">{o.price_label}</p>
            <p>{o.blurb}</p>
            {o.add_on.map(|a| view! { <p class="add-on">{a}</p> })}
        </article>
    }
}

/// Hero meta block — address + phone, sourced from `BrandingHandle` so
/// every shop renders its own. Empty when neither is set (a fresh
/// install boots clean instead of "  ·  Tel. ").
fn render_meta() -> impl IntoView {
    #[cfg(feature = "ssr")]
    let b = use_context::<crate::branding::BrandingHandle>()
        .map(|h| h.get())
        .unwrap_or_default();
    #[cfg(not(feature = "ssr"))]
    let b = crate::branding::Branding::default();

    let address = b.full_address();
    let phone = b.shop_phone.clone();
    let phone_label = if phone.is_empty() {
        String::new()
    } else {
        format!("Tel. {phone}")
    };
    let nothing = address.is_empty() && phone.is_empty();
    if nothing {
        return view! { <p class="meta"></p> }.into_any();
    }
    view! {
        <p class="meta">
            {(!address.is_empty()).then(|| view! { <>{address}<br/></> })}
            {(!phone.is_empty()).then(|| view! { <>{phone_label}</> })}
        </p>
    }
    .into_any()
}
