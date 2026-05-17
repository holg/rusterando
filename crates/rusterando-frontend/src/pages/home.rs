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

    Ok(HomeDeliveryInfo {
        zones,
        free_delivery_threshold_cents: threshold,
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
fn site_url() -> String {
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
                    <tr><th>{crate::t!("home.day_mon")}</th><td>"11:30–14:30 · 17:00–22:00"</td></tr>
                    <tr><th>{crate::t!("home.day_tue")}</th><td>"11:30–14:30 · 17:00–22:00"</td></tr>
                    <tr><th>{crate::t!("home.day_wed")}</th><td class="closed">{crate::t!("home.day_closed")}</td></tr>
                    <tr><th>{crate::t!("home.day_thu")}</th><td>"11:30–14:30 · 17:00–22:00"</td></tr>
                    <tr><th>{crate::t!("home.day_fri")}</th><td>"16:00–22:00"</td></tr>
                    <tr><th>{crate::t!("home.day_sat")}</th><td>"16:00–22:00"</td></tr>
                    <tr><th>{crate::t!("home.day_sun")}</th><td>"11:30–14:30 · 17:00–22:00"</td></tr>
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
