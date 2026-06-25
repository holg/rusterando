//! /admin/pdf — PDF editor.
//!
//! The PDF is rendered by the server crate via Typst. Editable bits
//! live in `app_settings` (simple text/image overrides) and, for the
//! cover image + Typst template, in dedicated library tables
//! (`pdf_cover_images`, `pdf_themes`) introduced in migration
//! `20260522000001_pdf_library.sql`.
//!
//! - `pdf_tagline`           — single-line subtitle under the shop name
//! - `pdf_hours`             — newline-separated opening-hours lines
//! - `pdf_extras_pizza`      — newline-separated extras list (pizza)
//! - `pdf_extras_pasta`      — newline-separated extras list (pasta)
//! - `pdf_ad_cover_image`    — optional ad image, frontpage slot
//! - `pdf_ad_center_image`   — optional ad image, between-pages slot
//! - `pdf_ad_back_image`     — optional ad image, back-page slot
//! - `pdf_cover_active_id`   — pointer into pdf_cover_images
//! - `pdf_theme_active_id`   — pointer into pdf_themes
//!
//! The simple text/ad-image fields go through the existing
//! `pages::settings::update_setting` server fn. The cover and theme
//! libraries have dedicated server fns below.

use leptos::prelude::*;
use serde::{Deserialize, Serialize};

use crate::pages::admin::shell::AdminShell;
use crate::pages::settings::{list_settings, UpdateSetting};

// ---------------------------------------------------------------------------
// Cover-image library: CRUD server fns
// ---------------------------------------------------------------------------

/// One row from `pdf_cover_images`, with the active flag joined in.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoverRow {
    pub id: i64,
    pub label: String,
    pub filename: String,
    pub mime_type: String,
    pub size_bytes: i64,
    pub is_active: bool,
}

#[server(
    name = ListPdfCovers,
    prefix = "/api",
    endpoint = "pdf_list_covers"
)]
pub async fn list_pdf_covers() -> Result<Vec<CoverRow>, ServerFnError> {
    use sqlx::SqlitePool;
    crate::pages::admin::require_admin().await?;
    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;
    let active: Option<String> =
        sqlx::query_scalar("SELECT value FROM app_settings WHERE key='pdf_cover_active_id'")
            .fetch_optional(&db)
            .await
            .map_err(|e| ServerFnError::new(format!("active id: {e}")))?;
    let active_id: Option<i64> = active.and_then(|s| s.trim().parse().ok());
    let rows: Vec<(i64, String, String, String, i64)> = sqlx::query_as(
        "SELECT id, label, filename, mime_type, size_bytes
         FROM pdf_cover_images
         ORDER BY created_at DESC, id DESC",
    )
    .fetch_all(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("list covers: {e}")))?;
    Ok(rows
        .into_iter()
        .map(|(id, label, filename, mime_type, size_bytes)| CoverRow {
            id,
            label,
            filename,
            mime_type,
            size_bytes,
            is_active: Some(id) == active_id,
        })
        .collect())
}

#[server(
    name = ActivatePdfCover,
    prefix = "/api",
    endpoint = "pdf_activate_cover"
)]
pub async fn activate_pdf_cover(id: i64) -> Result<(), ServerFnError> {
    use sqlx::SqlitePool;
    crate::pages::admin::require_admin().await?;
    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;
    // Guard: the row must exist.
    let exists: Option<i64> = sqlx::query_scalar("SELECT id FROM pdf_cover_images WHERE id = ?1")
        .bind(id)
        .fetch_optional(&db)
        .await
        .map_err(|e| ServerFnError::new(format!("lookup: {e}")))?;
    if exists.is_none() {
        return Err(ServerFnError::new("Cover existiert nicht."));
    }
    sqlx::query("UPDATE app_settings SET value = ?1 WHERE key = 'pdf_cover_active_id'")
        .bind(id.to_string())
        .execute(&db)
        .await
        .map_err(|e| ServerFnError::new(format!("activate cover: {e}")))?;
    Ok(())
}

#[server(
    name = DeletePdfCover,
    prefix = "/api",
    endpoint = "pdf_delete_cover"
)]
pub async fn delete_pdf_cover(id: i64) -> Result<(), ServerFnError> {
    use sqlx::SqlitePool;
    crate::pages::admin::require_admin().await?;
    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;

    // Refuse to delete the active cover — admin must activate another
    // first. Stops the renderer falling back to the bundled factory
    // unintentionally.
    let active: Option<String> =
        sqlx::query_scalar("SELECT value FROM app_settings WHERE key='pdf_cover_active_id'")
            .fetch_optional(&db)
            .await
            .map_err(|e| ServerFnError::new(format!("active lookup: {e}")))?;
    if active
        .and_then(|s| s.trim().parse::<i64>().ok())
        .is_some_and(|a| a == id)
    {
        return Err(ServerFnError::new(
            "Aktives Cover kann nicht gelöscht werden — bitte zuerst ein anderes aktivieren.",
        ));
    }

    // Get the filename so we can unlink the file too. If another
    // row references the same filename (content-hash dedupe), keep
    // the file on disk.
    let row: Option<(String,)> =
        sqlx::query_as("SELECT filename FROM pdf_cover_images WHERE id = ?1")
            .bind(id)
            .fetch_optional(&db)
            .await
            .map_err(|e| ServerFnError::new(format!("lookup row: {e}")))?;
    let Some((filename,)) = row else {
        return Err(ServerFnError::new("Cover existiert nicht."));
    };

    sqlx::query("DELETE FROM pdf_cover_images WHERE id = ?1")
        .bind(id)
        .execute(&db)
        .await
        .map_err(|e| ServerFnError::new(format!("delete cover: {e}")))?;

    let still_referenced: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM pdf_cover_images WHERE filename = ?1")
            .bind(&filename)
            .fetch_one(&db)
            .await
            .unwrap_or(0);
    if still_referenced == 0 && !filename.contains('/') && !filename.contains('\\') {
        // Resolve the uploads base from context (same dir the axum upload
        // handler writes to: <site_root>/img/uploads). Fall back to the
        // historical relative path if the context isn't present.
        let base = use_context::<crate::pages::settings::UploadsDir>()
            .map(|u| u.as_str().to_string())
            .unwrap_or_else(|| "data/uploads".to_string());
        let _ = std::fs::remove_file(format!("{base}/covers/{filename}"));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Typst-theme library: CRUD server fns
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThemeRow {
    pub id: i64,
    pub name: String,
    pub source: String,
    pub is_active: bool,
}

#[server(
    name = ListPdfThemes,
    prefix = "/api",
    endpoint = "pdf_list_themes"
)]
pub async fn list_pdf_themes() -> Result<Vec<ThemeRow>, ServerFnError> {
    use sqlx::SqlitePool;
    crate::pages::admin::require_admin().await?;
    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;
    let active: Option<String> =
        sqlx::query_scalar("SELECT value FROM app_settings WHERE key='pdf_theme_active_id'")
            .fetch_optional(&db)
            .await
            .map_err(|e| ServerFnError::new(format!("active id: {e}")))?;
    let active_id: Option<i64> = active.and_then(|s| s.trim().parse().ok());
    let rows: Vec<(i64, String, String)> =
        sqlx::query_as("SELECT id, name, source FROM pdf_themes ORDER BY updated_at DESC, id DESC")
            .fetch_all(&db)
            .await
            .map_err(|e| ServerFnError::new(format!("list themes: {e}")))?;
    Ok(rows
        .into_iter()
        .map(|(id, name, source)| ThemeRow {
            id,
            name,
            source,
            is_active: Some(id) == active_id,
        })
        .collect())
}

/// Insert (id=None) or update (id=Some) a theme row. Returns the row id.
/// Caller is expected to test-render the source via
/// `/api/admin/pdf/test_render` before calling this — broken Typst
/// is allowed to sit in the DB so the admin can keep iterating, but
/// switching to a broken theme would break /menu.pdf.
#[server(
    name = SavePdfTheme,
    prefix = "/api",
    endpoint = "pdf_save_theme"
)]
pub async fn save_pdf_theme(
    id: Option<i64>,
    name: String,
    source: String,
) -> Result<i64, ServerFnError> {
    use sqlx::SqlitePool;
    crate::pages::admin::require_admin().await?;
    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err(ServerFnError::new("Name darf nicht leer sein."));
    }
    match id {
        Some(id) => {
            sqlx::query(
                "UPDATE pdf_themes
                 SET name = ?1, source = ?2, updated_at = CURRENT_TIMESTAMP
                 WHERE id = ?3",
            )
            .bind(&name)
            .bind(&source)
            .bind(id)
            .execute(&db)
            .await
            .map_err(|e| ServerFnError::new(format!("update theme: {e}")))?;
            Ok(id)
        }
        None => {
            let row: (i64,) = sqlx::query_as(
                "INSERT INTO pdf_themes (name, source) VALUES (?1, ?2) RETURNING id",
            )
            .bind(&name)
            .bind(&source)
            .fetch_one(&db)
            .await
            .map_err(|e| ServerFnError::new(format!("insert theme: {e}")))?;
            Ok(row.0)
        }
    }
}

#[server(
    name = ActivatePdfTheme,
    prefix = "/api",
    endpoint = "pdf_activate_theme"
)]
pub async fn activate_pdf_theme(id: i64) -> Result<(), ServerFnError> {
    use sqlx::SqlitePool;
    crate::pages::admin::require_admin().await?;
    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;
    let exists: Option<i64> = sqlx::query_scalar("SELECT id FROM pdf_themes WHERE id = ?1")
        .bind(id)
        .fetch_optional(&db)
        .await
        .map_err(|e| ServerFnError::new(format!("lookup: {e}")))?;
    if exists.is_none() {
        return Err(ServerFnError::new("Vorlage existiert nicht."));
    }
    sqlx::query("UPDATE app_settings SET value = ?1 WHERE key = 'pdf_theme_active_id'")
        .bind(id.to_string())
        .execute(&db)
        .await
        .map_err(|e| ServerFnError::new(format!("activate theme: {e}")))?;
    Ok(())
}

#[server(
    name = DeletePdfTheme,
    prefix = "/api",
    endpoint = "pdf_delete_theme"
)]
pub async fn delete_pdf_theme(id: i64) -> Result<(), ServerFnError> {
    use sqlx::SqlitePool;
    crate::pages::admin::require_admin().await?;
    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;
    let active: Option<String> =
        sqlx::query_scalar("SELECT value FROM app_settings WHERE key='pdf_theme_active_id'")
            .fetch_optional(&db)
            .await
            .map_err(|e| ServerFnError::new(format!("active lookup: {e}")))?;
    if active
        .and_then(|s| s.trim().parse::<i64>().ok())
        .is_some_and(|a| a == id)
    {
        return Err(ServerFnError::new(
            "Aktive Vorlage kann nicht gelöscht werden — bitte zuerst eine andere aktivieren.",
        ));
    }
    sqlx::query("DELETE FROM pdf_themes WHERE id = ?1")
        .bind(id)
        .execute(&db)
        .await
        .map_err(|e| ServerFnError::new(format!("delete theme: {e}")))?;
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
    let cover_activator = ServerAction::<ActivatePdfCover>::new();
    let cover_deleter = ServerAction::<DeletePdfCover>::new();
    let theme_saver = ServerAction::<SavePdfTheme>::new();
    let theme_activator = ServerAction::<ActivatePdfTheme>::new();
    let theme_deleter = ServerAction::<DeletePdfTheme>::new();

    // Bump signal so the cover library refetches after a multipart
    // upload (which doesn't go through a ServerAction, so it can't be
    // versioned automatically).
    let cover_upload_bump: RwSignal<u32> = RwSignal::new(0);

    let settings = Resource::new(
        move || updater.version().get(),
        |_| async move { list_settings().await },
    );
    let defaults = Resource::new(|| (), |_| async move { get_pdf_defaults().await });
    let covers = Resource::new(
        move || {
            (
                cover_activator.version().get(),
                cover_deleter.version().get(),
                cover_upload_bump.get(),
            )
        },
        |_| async move { list_pdf_covers().await },
    );
    let themes = Resource::new(
        move || {
            (
                theme_saver.version().get(),
                theme_activator.version().get(),
                theme_deleter.version().get(),
            )
        },
        |_| async move { list_pdf_themes().await },
    );

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
                    "ohne Deploy. Cover-Bilder + Typst-Vorlagen sind Bibliotheken — "
                    "jede Änderung speichert eine neue Version; via Radio-Button wird ausgewählt, "
                    "welche aktiv ist."
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
                                <CoverLibraryCard
                                    cover_mode=by_key("pdf_cover_mode")
                                    covers
                                    cover_activator
                                    cover_deleter
                                    cover_upload_bump
                                    updater
                                />
                                <ImagesCard
                                    ad_cover=by_key("pdf_ad_cover_image")
                                    ad_center=by_key("pdf_ad_center_image")
                                    ad_back=by_key("pdf_ad_back_image")
                                    updater
                                />
                                <ThemeLibraryCard
                                    themes
                                    defaults
                                    theme_saver
                                    theme_activator
                                    theme_deleter
                                />
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
        let d = defaults
            .get()
            .and_then(|r| r.ok())
            .map(|d| d.tagline)
            .unwrap_or_default();
        reset("pdf_tagline", tag_sig, d);
    };
    let reset_hours = move |_| {
        let d = defaults
            .get()
            .and_then(|r| r.ok())
            .map(|d| d.hours)
            .unwrap_or_default();
        reset("pdf_hours", hours_sig, d);
    };
    let reset_pizza = move |_| {
        let d = defaults
            .get()
            .and_then(|r| r.ok())
            .map(|d| d.extras_pizza)
            .unwrap_or_default();
        reset("pdf_extras_pizza", pizza_sig, d);
    };
    let reset_pasta = move |_| {
        let d = defaults
            .get()
            .and_then(|r| r.ok())
            .map(|d| d.extras_pasta)
            .unwrap_or_default();
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
    ad_cover: String,
    ad_center: String,
    ad_back: String,
    updater: ServerAction<UpdateSetting>,
) -> impl IntoView {
    view! {
        <section class="card">
            <h2>"Anzeigen-Bilder"</h2>
            <p class="hint">
                "Optionale Anzeigen-Bilder für die drei PDF-Slots. Leeres Feld = "
                "kein Eintrag. Das Hauptcover wird oben unter „Cover-Bibliothek\" "
                "ausgewählt."
            </p>

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
            {move || path.get().is_empty().then_some(()).is_none().then(|| view! {
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
    let text = JsFuture::from(resp.text().map_err(|_| "response.text()".to_string())?)
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

// ---------------------------------------------------------------------------
// Cover-image library UI
// ---------------------------------------------------------------------------

#[component]
fn CoverLibraryCard(
    /// Current `pdf_cover_mode` setting: "text" (textual cover, default) or
    /// "image" (use the active uploaded cover). Seeds the toggle below.
    cover_mode: String,
    covers: Resource<Result<Vec<CoverRow>, ServerFnError>>,
    cover_activator: ServerAction<ActivatePdfCover>,
    cover_deleter: ServerAction<DeletePdfCover>,
    cover_upload_bump: RwSignal<u32>,
    /// Shared settings writer (also used by the other cards). Writing
    /// `pdf_cover_mode` flips text↔image; `update_setting` rebuilds the
    /// cached menu.pdf, so the change takes effect on the next open.
    updater: ServerAction<UpdateSetting>,
) -> impl IntoView {
    let new_label: RwSignal<String> = RwSignal::new(String::new());
    let uploading: RwSignal<bool> = RwSignal::new(false);
    let upload_err: RwSignal<Option<String>> = RwSignal::new(None);

    // The toggle: an empty/unknown value falls back to "text" (the default),
    // matching the renderer (pdf.rs reads anything ≠ "image" as text).
    let use_image = RwSignal::new(cover_mode == "image");
    let set_mode = move |image: bool| {
        use_image.set(image);
        updater.dispatch(UpdateSetting {
            key: "pdf_cover_mode".to_string(),
            value: if image { "image" } else { "text" }.to_string(),
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
            let label = new_label.get();
            uploading.set(true);
            upload_err.set(None);
            wasm_bindgen_futures::spawn_local(async move {
                match upload_cover_via_fetch(file, label).await {
                    Ok(_id) => {
                        new_label.set(String::new());
                        // Bump the resource to refetch the list.
                        cover_upload_bump.update(|n| *n += 1);
                    }
                    Err(e) => upload_err.set(Some(e)),
                }
                uploading.set(false);
            });
        }
        let _ = (ev, cover_upload_bump);
    };

    view! {
        <section class="card cover-library">
            <h2>"Cover-Bibliothek"</h2>
            <p class="hint">
                "Lade verschiedene Cover-Bilder hoch und wähle eines aktiv. "
                "Das aktive Cover wird in der PDF-Frontseite gerendert. "
                "Format: JPEG oder PNG, max. 5 MB."
            </p>

            // Text vs. Foto switch. This is the control that decides whether
            // the PDF uses the textual title page (default) or the active
            // uploaded cover — without it, uploading a cover changes nothing.
            <fieldset class="cover-mode">
                <legend>"Titelseite der PDF"</legend>
                <label class="cover-mode-opt">
                    <input type="radio" name="cover_mode"
                        prop:checked=move || !use_image.get()
                        on:change=move |_| set_mode(false)/>
                    <span>
                        <strong>"Textseite "</strong>
                        <span class="muted small">"(Standard) — gesetzte Titelseite aus den Shop-Daten"</span>
                    </span>
                </label>
                <label class="cover-mode-opt">
                    <input type="radio" name="cover_mode"
                        prop:checked=move || use_image.get()
                        on:change=move |_| set_mode(true)/>
                    <span>
                        <strong>"Foto-Titelbild "</strong>
                        <span class="muted small">"— verwendet das unten aktivierte Cover-Bild"</span>
                    </span>
                </label>
                {move || use_image.get().then(|| view! {
                    <p class="hint small">
                        "Foto-Modus aktiv: stelle sicher, dass unten ein Cover ausgewählt ist."
                    </p>
                })}
            </fieldset>

            <Suspense fallback=|| view! { <p class="muted">"Lädt…"</p> }>
                {move || covers.get().map(|res| match res {
                    Err(e) => view! { <p class="error">{format!("Fehler: {e}")}</p> }.into_any(),
                    Ok(list) if list.is_empty() => view! {
                        <p class="muted">"Noch keine Cover hochgeladen."</p>
                    }.into_any(),
                    Ok(list) => view! {
                        <ul class="cover-list">
                            {list.into_iter().map(|c| {
                                let id = c.id;
                                let is_active = c.is_active;
                                let src = format!("/img/uploads/covers/{}", c.filename);
                                let on_activate = move |_| {
                                    cover_activator.dispatch(ActivatePdfCover { id });
                                };
                                let on_delete = move |_| {
                                    cover_deleter.dispatch(DeletePdfCover { id });
                                };
                                view! {
                                    <li class=move || if is_active { "cover-row active" } else { "cover-row" }>
                                        <img class="cover-thumb" src=src alt=c.label.clone()/>
                                        <div class="cover-meta">
                                            <label class="cover-active-label">
                                                <input type="radio" name="cover_active"
                                                    checked=is_active
                                                    on:change=on_activate/>
                                                <strong>{c.label}</strong>
                                            </label>
                                            <span class="muted small">
                                                {c.mime_type.clone()} " · "
                                                {format!("{:.1} KB", (c.size_bytes as f64) / 1024.0)}
                                            </span>
                                        </div>
                                        <button class="btn small ghost danger"
                                            disabled=is_active
                                            title=if is_active { "Aktives Cover kann nicht gelöscht werden" } else { "Löschen" }
                                            on:click=on_delete>
                                            "Löschen"
                                        </button>
                                    </li>
                                }
                            }).collect_view()}
                        </ul>
                    }.into_any(),
                })}
            </Suspense>

            <div class="cover-upload">
                <label class="cover-upload-label">
                    <span>"Neues Cover hochladen"</span>
                    <input type="text" placeholder="Bezeichnung (z. B. Sommer-Aktion)"
                        on:input=move |ev| new_label.set(event_target_value(&ev))
                        prop:value=move || new_label.get()/>
                    <input type="file" accept="image/jpeg,image/png"
                        on:change=on_pick/>
                </label>
                {move || uploading.get().then(|| view! { <span class="muted">"Lädt hoch…"</span> })}
                {move || upload_err.get().map(|e| view! { <span class="error">{e}</span> })}
            </div>
        </section>
    }
}

#[cfg(feature = "hydrate")]
async fn upload_cover_via_fetch(file: web_sys::File, label: String) -> Result<i64, String> {
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::JsFuture;

    let form = web_sys::FormData::new().map_err(|_| "FormData ctor".to_string())?;
    form.append_with_blob("file", &file)
        .map_err(|_| "FormData append".to_string())?;
    let label_to_send = if label.trim().is_empty() {
        "Cover".to_string()
    } else {
        label
    };
    form.append_with_str("label", &label_to_send)
        .map_err(|_| "FormData label".to_string())?;

    let opts = web_sys::RequestInit::new();
    opts.set_method("POST");
    opts.set_body(&form.into());
    let req = web_sys::Request::new_with_str_and_init("/api/admin/pdf/cover/upload", &opts)
        .map_err(|_| "Request ctor".to_string())?;

    let window = web_sys::window().ok_or_else(|| "no window".to_string())?;
    let resp_value = JsFuture::from(window.fetch_with_request(&req))
        .await
        .map_err(|_| "fetch failed".to_string())?;
    let resp: web_sys::Response = resp_value
        .dyn_into()
        .map_err(|_| "response cast".to_string())?;
    let text = JsFuture::from(resp.text().map_err(|_| "response.text()".to_string())?)
        .await
        .map_err(|_| "response body".to_string())?
        .as_string()
        .unwrap_or_default();
    if !resp.ok() {
        return Err(text);
    }
    // {"id":42,"filename":"<hash>.<ext>"}
    let v: serde_json::Value = serde_json::from_str(&text).map_err(|e| format!("json: {e}"))?;
    v.get("id")
        .and_then(|x| x.as_i64())
        .ok_or_else(|| "missing id in response".to_string())
}

#[cfg(not(feature = "hydrate"))]
#[allow(dead_code)]
async fn upload_cover_via_fetch(_file: (), _label: String) -> Result<i64, String> {
    Err("hydrate-only".into())
}

// ---------------------------------------------------------------------------
// Theme library UI
// ---------------------------------------------------------------------------

#[component]
fn ThemeLibraryCard(
    themes: Resource<Result<Vec<ThemeRow>, ServerFnError>>,
    defaults: Resource<Result<PdfDefaults, ServerFnError>>,
    theme_saver: ServerAction<SavePdfTheme>,
    theme_activator: ServerAction<ActivatePdfTheme>,
    theme_deleter: ServerAction<DeletePdfTheme>,
) -> impl IntoView {
    // Selected = the theme currently being edited. None = "new theme draft".
    let selected_id: RwSignal<Option<i64>> = RwSignal::new(None);
    let name_sig: RwSignal<String> = RwSignal::new(String::new());
    let source_sig: RwSignal<String> = RwSignal::new(String::new());
    let status: RwSignal<Option<String>> = RwSignal::new(None);
    let testing: RwSignal<bool> = RwSignal::new(false);

    // When the themes list resolves, if no theme is currently selected
    // load the active one into the editor by default.
    Effect::new(move |_| {
        if selected_id.get().is_some() {
            return;
        }
        if let Some(Ok(list)) = themes.get() {
            if let Some(active) = list.iter().find(|t| t.is_active) {
                selected_id.set(Some(active.id));
                name_sig.set(active.name.clone());
                source_sig.set(active.source.clone());
            }
        }
    });

    // Switch the editor to a different theme row when the dropdown
    // changes.
    let switch_to = move |id: i64| {
        if let Some(Ok(list)) = themes.get() {
            if let Some(t) = list.iter().find(|t| t.id == id) {
                selected_id.set(Some(id));
                name_sig.set(t.name.clone());
                source_sig.set(t.source.clone());
                status.set(None);
            }
        }
    };

    let on_select_change = move |ev: leptos::ev::Event| {
        let v = event_target_value(&ev);
        if v == "__new__" {
            selected_id.set(None);
            name_sig.set(String::new());
            source_sig.set(String::new());
            status.set(None);
        } else if let Ok(id) = v.parse::<i64>() {
            switch_to(id);
        }
    };

    let on_duplicate = move |_| {
        // Keep source, clear id, prefix name.
        let cur_name = name_sig.get();
        let new_name = if cur_name.starts_with("Kopie von ") {
            cur_name
        } else {
            format!("Kopie von {cur_name}")
        };
        name_sig.set(new_name);
        selected_id.set(None);
        status.set(Some("Speichern legt eine neue Vorlage an.".into()));
    };

    let on_new = move |_| {
        selected_id.set(None);
        name_sig.set(String::new());
        source_sig.set(String::new());
        status.set(None);
    };

    let on_factory = move |_| {
        if let Some(Ok(d)) = defaults.get() {
            source_sig.set(d.template_source);
            status.set(Some(
                "Werks-Quelle geladen — bitte speichern, um zu übernehmen.".into(),
            ));
        }
    };

    let on_save = move |_| {
        let name = name_sig.get();
        let src = source_sig.get();
        let id = selected_id.get();
        if name.trim().is_empty() {
            status.set(Some("Bitte einen Namen vergeben.".into()));
            return;
        }
        #[cfg(feature = "hydrate")]
        {
            testing.set(true);
            status.set(None);
            let to_persist_name = name.clone();
            let to_persist_src = src.clone();
            wasm_bindgen_futures::spawn_local(async move {
                match test_render(&to_persist_src).await {
                    Ok(()) => {
                        theme_saver.dispatch(SavePdfTheme {
                            id,
                            name: to_persist_name,
                            source: to_persist_src,
                        });
                        status.set(Some("Test-Render OK — gespeichert.".into()));
                    }
                    Err(e) => {
                        status.set(Some(format!("Fehler beim Test-Render: {e}")));
                    }
                }
                testing.set(false);
            });
        }
        let _ = (name, src, id, theme_saver);
    };

    let on_test = move |_| {
        let src = source_sig.get();
        #[cfg(feature = "hydrate")]
        {
            testing.set(true);
            status.set(None);
            wasm_bindgen_futures::spawn_local(async move {
                match test_render(&src).await {
                    Ok(()) => status.set(Some("Test-Render OK.".into())),
                    Err(e) => status.set(Some(format!("Fehler: {e}"))),
                }
                testing.set(false);
            });
        }
        let _ = src;
    };

    let on_activate = move |_| {
        if let Some(id) = selected_id.get() {
            theme_activator.dispatch(ActivatePdfTheme { id });
            status.set(Some("Aktiviert.".into()));
        } else {
            status.set(Some(
                "Erst speichern, bevor eine neue Vorlage aktiviert werden kann.".into(),
            ));
        }
    };

    let on_delete = move |_| {
        if let Some(id) = selected_id.get() {
            theme_deleter.dispatch(DeletePdfTheme { id });
            status.set(Some("Gelöscht.".into()));
            selected_id.set(None);
            name_sig.set(String::new());
            source_sig.set(String::new());
        }
    };

    view! {
        <section class="card template-card theme-library">
            <h2>"Typst-Vorlagen"</h2>
            <p class="hint">
                "Mehrere Vorlagen können nebeneinander existieren. Wähle eine "
                "zum Bearbeiten aus dem Dropdown, oder „Neue Vorlage\" für ein "
                "leeres Editor-Feld. Beim Speichern wird zuerst test-gerendert; "
                "fehlerhafte Vorlagen werden abgelehnt. Nur die aktive Vorlage "
                "wird in der Live-PDF verwendet."
            </p>

            <div class="theme-toolbar">
                <Suspense fallback=|| view! { <span class="muted">"Lädt Vorlagen…"</span> }>
                    {move || themes.get().map(|res| match res {
                        Err(e) => view! { <span class="error">{format!("{e}")}</span> }.into_any(),
                        Ok(list) => view! {
                            <select class="theme-select"
                                on:change=on_select_change
                                prop:value=move || {
                                    selected_id.get().map(|i| i.to_string())
                                        .unwrap_or_else(|| "__new__".to_string())
                                }>
                                <option value="__new__">"— Neue Vorlage —"</option>
                                {list.into_iter().map(|t| {
                                    let label = if t.is_active {
                                        format!("● {}", t.name)
                                    } else {
                                        t.name.clone()
                                    };
                                    view! {
                                        <option value=t.id.to_string()>{label}</option>
                                    }
                                }).collect_view()}
                            </select>
                        }.into_any(),
                    })}
                </Suspense>

                <button class="btn small" on:click=on_new>"+ Neue Vorlage"</button>
                <button class="btn small" on:click=on_duplicate>"Duplizieren"</button>
            </div>

            <label>
                <span>"Name"</span>
                <input type="text" class="theme-name"
                    on:input=move |ev| name_sig.set(event_target_value(&ev))
                    prop:value=move || name_sig.get()/>
            </label>

            <textarea rows="22" spellcheck="false"
                class="template-editor"
                on:input=move |ev| source_sig.set(event_target_value(&ev))
                prop:value=move || source_sig.get()></textarea>

            <div class="actions">
                <button class="btn primary" on:click=on_save>
                    {move || if testing.get() { "Teste…" } else { "Speichern (mit Test-Render)" }}
                </button>
                <button class="btn" on:click=on_test>"Nur testen"</button>
                <button class="btn" on:click=on_activate>"Aktivieren"</button>
                <button class="btn" on:click=on_factory>"Werks-Quelle laden"</button>
                <button class="btn ghost danger" on:click=on_delete>"Löschen"</button>
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
        let text = JsFuture::from(resp.text().map_err(|_| "response.text()".to_string())?)
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
