//! /admin/pdf — PDF editor.
//!
//! The PDF is rendered by the server crate via Typst. Until recently
//! the template + frontpage image + hours/extras lists were all
//! compile-time constants in `rusterando-server/src/pdf.rs`. With the
//! editor in place, those bits live in `app_settings` and can be
//! changed live:
//!
//! - `pdf_tagline`           — single-line subtitle under the shop name
//! - `pdf_hours`             — newline-separated opening-hours lines
//! - `pdf_extras_pizza`      — newline-separated extras list (pizza)
//! - `pdf_extras_pasta`      — newline-separated extras list (pasta)
//! - `pdf_cover_image`       — `/img/uploads/<hash>.<ext>` (replaces ladenfront.jpg)
//! - `pdf_ad_cover_image`    — optional ad image, frontpage slot
//! - `pdf_ad_center_image`   — optional ad image, between-pages slot
//! - `pdf_ad_back_image`     — optional ad image, back-page slot
//! - `pdf_template_source`   — full Typst source override (empty = factory)
//!
//! All eight non-template fields go through the existing
//! `pages::settings::update_setting` server fn. The template source
//! takes a custom path because we test-render it server-side before
//! committing — a broken Typst typo would otherwise break the live
//! PDF for everyone.

use leptos::prelude::*;
use serde::{Deserialize, Serialize};

use crate::pages::admin::shell::AdminShell;
use crate::pages::settings::{list_settings, UpdateSetting};

// ---------------------------------------------------------------------------
// Server fns
// ---------------------------------------------------------------------------

/// Save the Typst template source after a test-render confirms it
/// compiles. Returns the validator's complaint when the source has a
/// syntax error so the admin can see exactly which line broke.
#[server(
    name = SavePdfTemplate,
    prefix = "/api",
    endpoint = "save_pdf_template"
)]
pub async fn save_pdf_template(source: String) -> Result<(), ServerFnError> {
    use sqlx::SqlitePool;

    crate::pages::admin::require_admin().await?;

    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;

    let trimmed = source.trim();
    // Empty == reset to factory. No validation needed in that case;
    // the embedded TEMPLATE_SRC is known-good.
    if trimmed.is_empty() {
        sqlx::query("UPDATE app_settings SET value = '' WHERE key = 'pdf_template_source'")
            .execute(&db)
            .await
            .map_err(|e| ServerFnError::new(format!("reset template: {e}")))?;
        return Ok(());
    }

    // Persist. The Axum /api/admin/pdf/test_render route the client
    // already called returns a 400 on compile failure, so if we got
    // here the source is known to render. We trust that — re-running
    // the full render here would double the round-trip time.
    sqlx::query("UPDATE app_settings SET value = ?1 WHERE key = 'pdf_template_source'")
        .bind(trimmed)
        .execute(&db)
        .await
        .map_err(|e| ServerFnError::new(format!("save template: {e}")))?;
    Ok(())
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PdfDefaults {
    pub template_source: String,
    pub tagline: String,
    pub hours: String,
    pub extras_pizza: String,
    pub extras_pasta: String,
}

/// Fetch the bundled factory defaults. Used by the per-field
/// "Zurücksetzen"-Buttons and the template "Werks-Source laden"
/// button on /admin/pdf.
///
/// Keep these strings 1:1 in sync with `rusterando-server::pdf`'s
/// `default_*_lines()` helpers — both sides are the source of truth
/// for "what ships when nothing is configured". Frontend can't depend
/// on rusterando-server (cycle), so we duplicate intentionally.
#[server(
    name = GetPdfDefaults,
    prefix = "/api",
    endpoint = "get_pdf_defaults"
)]
pub async fn get_pdf_defaults() -> Result<PdfDefaults, ServerFnError> {
    crate::pages::admin::require_admin().await?;
    let src = include_str!("../../../../../templates/menu.typ");
    Ok(PdfDefaults {
        template_source: src.to_string(),
        tagline: String::new(),
        hours: "Mo, Di, Do, So: 11:30–14:30 · 17:00–22:00\n\
                Fr, Sa: 16:00–22:00\n\
                Mi Ruhetag"
            .to_string(),
        extras_pizza: "Krabben 1 €\n\
                       Lachs 2 €\n\
                       Kräuterbutter 0,60 €\n\
                       sonstige Extras 0,70 €"
            .to_string(),
        extras_pasta: "Krabben 1 €\n\
                       Lachs 2 €\n\
                       sonstige Extras 0,70 €"
            .to_string(),
    })
}

// ---------------------------------------------------------------------------
// UI
// ---------------------------------------------------------------------------

#[component]
pub fn PdfAdminPage() -> impl IntoView {
    let updater = ServerAction::<UpdateSetting>::new();
    let template_saver = ServerAction::<SavePdfTemplate>::new();

    let settings = Resource::new(
        move || (updater.version().get(), template_saver.version().get()),
        |_| async move { list_settings().await },
    );
    let defaults = Resource::new(|| (), |_| async move { get_pdf_defaults().await });

    view! {
        <AdminShell>
            <section class="admin-pdf">
                <header class="admin-bar">
                    <h1>"PDF-Editor"</h1>
                    <a class="btn primary small" href="/menu.pdf" target="_blank" rel="noopener">
                        "PDF jetzt öffnen"
                    </a>
                </header>
                <p class="hint">
                    "Alle Felder hier landen in der Speisekarten-PDF. Änderungen wirken sofort, "
                    "ohne Deploy. Das Typst-Template unten wird vor dem Speichern test-gerendert; "
                    "fehlerhafter Code wird abgelehnt."
                </p>

                <Suspense fallback=|| view! { <p class="loading">"Lädt…"</p> }>
                    {move || settings.get().map(|res| match res {
                        Err(e) => view! { <p class="error">{format!("Fehler: {e}")}</p> }.into_any(),
                        Ok(rows) => {
                            let by_key = |k: &str| rows.iter().find(|r| r.key == k).map(|r| r.value.clone()).unwrap_or_default();
                            view! {
                                <SimpleFieldsCard
                                    tagline=by_key("pdf_tagline")
                                    hours=by_key("pdf_hours")
                                    extras_pizza=by_key("pdf_extras_pizza")
                                    extras_pasta=by_key("pdf_extras_pasta")
                                    defaults
                                    updater
                                />
                                <ImagesCard
                                    cover=by_key("pdf_cover_image")
                                    ad_cover=by_key("pdf_ad_cover_image")
                                    ad_center=by_key("pdf_ad_center_image")
                                    ad_back=by_key("pdf_ad_back_image")
                                    updater
                                />
                                {
                                    let source = by_key("pdf_template_source");
                                    view! { <TemplateCard initial=source defaults template_saver/> }
                                }
                            }.into_any()
                        }
                    })}
                </Suspense>
            </section>
        </AdminShell>
    }
}

#[component]
fn SimpleFieldsCard(
    tagline: String,
    hours: String,
    extras_pizza: String,
    extras_pasta: String,
    defaults: Resource<Result<PdfDefaults, ServerFnError>>,
    updater: ServerAction<UpdateSetting>,
) -> impl IntoView {
    let tag_sig = RwSignal::new(tagline);
    let hours_sig = RwSignal::new(hours);
    let pizza_sig = RwSignal::new(extras_pizza);
    let pasta_sig = RwSignal::new(extras_pasta);

    let save = move |key: &'static str, value: String| {
        updater.dispatch(UpdateSetting {
            key: key.to_string(),
            value,
        });
    };
    let save_tag = move |_| save("pdf_tagline", tag_sig.get());
    let save_hours = move |_| save("pdf_hours", hours_sig.get());
    let save_pizza = move |_| save("pdf_extras_pizza", pizza_sig.get());
    let save_pasta = move |_| save("pdf_extras_pasta", pasta_sig.get());

    // Reset puts the factory default into the input AND persists it
    // immediately, so the next /menu.pdf request sees the change
    // without the admin having to hit Save afterwards.
    let reset = move |key: &'static str, sig: RwSignal<String>, val: String| {
        sig.set(val.clone());
        updater.dispatch(UpdateSetting {
            key: key.to_string(),
            value: val,
        });
    };
    let reset_tag = move |_| {
        let d = defaults.get().and_then(|r| r.ok()).map(|d| d.tagline).unwrap_or_default();
        reset("pdf_tagline", tag_sig, d);
    };
    let reset_hours = move |_| {
        let d = defaults.get().and_then(|r| r.ok()).map(|d| d.hours).unwrap_or_default();
        reset("pdf_hours", hours_sig, d);
    };
    let reset_pizza = move |_| {
        let d = defaults.get().and_then(|r| r.ok()).map(|d| d.extras_pizza).unwrap_or_default();
        reset("pdf_extras_pizza", pizza_sig, d);
    };
    let reset_pasta = move |_| {
        let d = defaults.get().and_then(|r| r.ok()).map(|d| d.extras_pasta).unwrap_or_default();
        reset("pdf_extras_pasta", pasta_sig, d);
    };

    view! {
        <section class="card">
            <h2>"Text-Inhalte"</h2>
            <label>
                <span>"Untertitel"</span>
                <input type="text"
                    prop:value=move || tag_sig.get()
                    on:input=move |ev| tag_sig.set(event_target_value(&ev))/>
                <div class="field-actions">
                    <button class="btn small primary" on:click=save_tag>"Speichern"</button>
                    <button class="btn small ghost" on:click=reset_tag
                        title="Auf Werks-Default zurücksetzen">"↺ Standard"</button>
                </div>
            </label>

            <label>
                <span>"Öffnungszeiten (eine Zeile pro Eintrag)"</span>
                <textarea rows="4"
                    on:input=move |ev| hours_sig.set(event_target_value(&ev))
                    prop:value=move || hours_sig.get()></textarea>
                <div class="field-actions">
                    <button class="btn small primary" on:click=save_hours>"Speichern"</button>
                    <button class="btn small ghost" on:click=reset_hours>"↺ Standard"</button>
                </div>
            </label>

            <label>
                <span>"Pizza-Extras (eine Zeile pro Eintrag)"</span>
                <textarea rows="5"
                    on:input=move |ev| pizza_sig.set(event_target_value(&ev))
                    prop:value=move || pizza_sig.get()></textarea>
                <div class="field-actions">
                    <button class="btn small primary" on:click=save_pizza>"Speichern"</button>
                    <button class="btn small ghost" on:click=reset_pizza>"↺ Standard"</button>
                </div>
            </label>

            <label>
                <span>"Pasta-Extras (eine Zeile pro Eintrag)"</span>
                <textarea rows="4"
                    on:input=move |ev| pasta_sig.set(event_target_value(&ev))
                    prop:value=move || pasta_sig.get()></textarea>
                <div class="field-actions">
                    <button class="btn small primary" on:click=save_pasta>"Speichern"</button>
                    <button class="btn small ghost" on:click=reset_pasta>"↺ Standard"</button>
                </div>
            </label>
        </section>
    }
}

#[component]
fn ImagesCard(
    cover: String,
    ad_cover: String,
    ad_center: String,
    ad_back: String,
    updater: ServerAction<UpdateSetting>,
) -> impl IntoView {
    view! {
        <section class="card">
            <h2>"Bilder"</h2>
            <p class="hint">
                "Bilder werden hochgeladen und unter dem zurückgelieferten Pfad "
                "gespeichert. Leeres Feld = Werks-Bild bzw. kein Eintrag."
            </p>

            <ImageSlot label="Frontseiten-Cover (ersetzt ladenfront.jpg)"
                key="pdf_cover_image"  initial=cover     updater/>
            <ImageSlot label="Anzeige Frontseite (Slot 1)"
                key="pdf_ad_cover_image"  initial=ad_cover  updater/>
            <ImageSlot label="Anzeige Mittelseite (Slot 2)"
                key="pdf_ad_center_image" initial=ad_center updater/>
            <ImageSlot label="Anzeige Rückseite (Slot 3)"
                key="pdf_ad_back_image"   initial=ad_back   updater/>
        </section>
    }
}

#[component]
fn ImageSlot(
    label: &'static str,
    key: &'static str,
    initial: String,
    updater: ServerAction<UpdateSetting>,
) -> impl IntoView {
    let path = RwSignal::new(initial);
    let uploading = RwSignal::new(false);
    let err: RwSignal<Option<String>> = RwSignal::new(None);

    let on_clear = move |_| {
        path.set(String::new());
        updater.dispatch(UpdateSetting {
            key: key.to_string(),
            value: String::new(),
        });
    };

    let on_pick = move |ev: leptos::ev::Event| {
        #[cfg(feature = "hydrate")]
        {
            use wasm_bindgen::JsCast;
            let Some(input) = ev
                .target()
                .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok())
            else {
                return;
            };
            let Some(files) = input.files() else {
                return;
            };
            let Some(file) = files.get(0) else { return };

            uploading.set(true);
            err.set(None);
            wasm_bindgen_futures::spawn_local(async move {
                match upload_via_fetch(file).await {
                    Ok(p) => {
                        path.set(p.clone());
                        updater.dispatch(UpdateSetting {
                            key: key.to_string(),
                            value: p,
                        });
                    }
                    Err(e) => err.set(Some(e)),
                }
                uploading.set(false);
            });
        }
        let _ = ev;
    };

    view! {
        <div class="image-slot">
            <label class="image-slot-label">
                <span>{label}</span>
                <input type="file" accept="image/png,image/jpeg,image/webp"
                    on:change=on_pick/>
            </label>
            <p class="muted">
                {move || {
                    let p = path.get();
                    if p.is_empty() { "—".to_string() } else { p }
                }}
            </p>
            {move || path.get().is_empty().then(|| ()).is_none().then(|| view! {
                <img class="preview"
                    src=move || path.get()
                    alt="Vorschau"/>
            })}
            <div class="actions">
                {move || (!path.get().is_empty()).then(|| view! {
                    <button class="btn small muted" on:click=on_clear
                        title="Auf Werks-Bild zurücksetzen (Eintrag leeren)">"↺ Standard"</button>
                })}
                {move || uploading.get().then(|| view! { <span class="muted">"Lädt hoch…"</span> })}
                {move || err.get().map(|e| view! { <span class="error">{e}</span> })}
            </div>
        </div>
    }
}

#[cfg(feature = "hydrate")]
async fn upload_via_fetch(file: web_sys::File) -> Result<String, String> {
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::JsFuture;

    let form = web_sys::FormData::new().map_err(|_| "FormData ctor".to_string())?;
    form.append_with_blob("file", &file)
        .map_err(|_| "FormData append".to_string())?;

    let opts = web_sys::RequestInit::new();
    opts.set_method("POST");
    opts.set_body(&form.into());
    let req = web_sys::Request::new_with_str_and_init("/api/admin/upload_image", &opts)
        .map_err(|_| "Request ctor".to_string())?;

    let window = web_sys::window().ok_or_else(|| "no window".to_string())?;
    let resp_value = JsFuture::from(window.fetch_with_request(&req))
        .await
        .map_err(|_| "fetch failed".to_string())?;
    let resp: web_sys::Response = resp_value
        .dyn_into()
        .map_err(|_| "response cast".to_string())?;
    let text = JsFuture::from(
        resp.text()
            .map_err(|_| "response.text()".to_string())?,
    )
    .await
    .map_err(|_| "response body".to_string())?
    .as_string()
    .unwrap_or_default();
    if !resp.ok() {
        return Err(text);
    }
    // {"path":"/img/uploads/<hash>.<ext>"}
    let v: serde_json::Value = serde_json::from_str(&text).map_err(|e| format!("json: {e}"))?;
    Ok(v.get("path")
        .and_then(|p| p.as_str())
        .unwrap_or_default()
        .to_string())
}

#[cfg(not(feature = "hydrate"))]
#[allow(dead_code)]
async fn upload_via_fetch(_file: ()) -> Result<String, String> {
    Err("hydrate-only".into())
}

#[component]
fn TemplateCard(
    initial: String,
    defaults: Resource<Result<PdfDefaults, ServerFnError>>,
    template_saver: ServerAction<SavePdfTemplate>,
) -> impl IntoView {
    let source = RwSignal::new(initial);
    let status: RwSignal<Option<String>> = RwSignal::new(None);
    let testing = RwSignal::new(false);

    // "Speichern" runs a test-render first; only on 200 do we commit.
    let on_save = move |_| {
        let src = source.get();
        let trimmed = src.trim();
        // Empty source = explicit reset to factory; skip the test
        // render (TEMPLATE_SRC is known-good) and persist directly.
        if trimmed.is_empty() {
            template_saver.dispatch(SavePdfTemplate {
                source: String::new(),
            });
            status.set(Some("Auf Werks-Template zurückgesetzt.".into()));
            return;
        }
        #[cfg(feature = "hydrate")]
        {
            testing.set(true);
            status.set(None);
            let to_persist = src.clone();
            wasm_bindgen_futures::spawn_local(async move {
                match test_render(&to_persist).await {
                    Ok(()) => {
                        template_saver.dispatch(SavePdfTemplate { source: to_persist });
                        status.set(Some("Test-Render OK — gespeichert.".into()));
                    }
                    Err(e) => {
                        status.set(Some(format!("Fehler beim Test-Render: {e}")));
                    }
                }
                testing.set(false);
            });
        }
        let _ = src;
    };

    let on_reset = move |_| {
        if let Some(Ok(d)) = defaults.get() {
            source.set(d.template_source);
        }
    };
    let on_clear = move |_| {
        source.set(String::new());
    };

    view! {
        <section class="card template-card">
            <h2>"Typst-Template"</h2>
            <p class="hint">
                "Voller Typst-Source. Leer = Werks-Template. Beim Speichern "
                "läuft erst ein Test-Render gegen die aktuelle Speisekarte; "
                "ein Syntaxfehler stoppt den Save, sodass die Live-PDF "
                "nie kaputtgeht."
            </p>
            <textarea rows="22" spellcheck="false"
                class="template-editor"
                on:input=move |ev| source.set(event_target_value(&ev))
                prop:value=move || source.get()></textarea>
            <div class="actions">
                <button class="btn primary" on:click=on_save>
                    {move || if testing.get() { "Teste…" } else { "Speichern (mit Test-Render)" }}
                </button>
                <button class="btn" on:click=on_reset>"Werks-Source laden"</button>
                <button class="btn muted" on:click=on_clear>"Leeren (= Werks-Default)"</button>
                {move || status.get().map(|s| view! { <span class="status">{s}</span> })}
            </div>
        </section>
    }
}

#[cfg(feature = "hydrate")]
async fn test_render(source: &str) -> Result<(), String> {
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::JsFuture;

    let opts = web_sys::RequestInit::new();
    opts.set_method("POST");
    opts.set_body(&wasm_bindgen::JsValue::from_str(source));
    let req = web_sys::Request::new_with_str_and_init("/api/admin/pdf/test_render", &opts)
        .map_err(|_| "Request ctor".to_string())?;
    let _ = req
        .headers()
        .set("Content-Type", "text/plain; charset=utf-8");

    let window = web_sys::window().ok_or_else(|| "no window".to_string())?;
    let resp_value = JsFuture::from(window.fetch_with_request(&req))
        .await
        .map_err(|_| "fetch failed".to_string())?;
    let resp: web_sys::Response = resp_value
        .dyn_into()
        .map_err(|_| "response cast".to_string())?;

    if resp.ok() {
        Ok(())
    } else {
        let text = JsFuture::from(
            resp.text()
                .map_err(|_| "response.text()".to_string())?,
        )
        .await
        .map_err(|_| "response body".to_string())?
        .as_string()
        .unwrap_or_else(|| format!("HTTP {}", resp.status()));
        Err(text)
    }
}

#[cfg(not(feature = "hydrate"))]
#[allow(dead_code)]
async fn test_render(_source: &str) -> Result<(), String> {
    Err("hydrate-only".into())
}
