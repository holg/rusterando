//! /admin/home — edit the public landing page content (hero, offers,
//! gallery). Backed by the `home_*` tables and the server fns in
//! `pages/home_content.rs`. Photo uploads go through the
//! `/api/admin/upload_image` route in main.rs which writes to
//! `data/uploads/` and returns a `/img/uploads/<hash>.<ext>` path.

use leptos::prelude::*;
#[cfg(feature = "hydrate")]
use leptos::task::spawn_local;

use crate::pages::admin::shell::AdminShell;
use crate::pages::home_content::{
    get_home_content, list_offers_admin, DeleteGalleryItem, DeleteOffer, GalleryItem, HomeContent,
    Offer, UpdateHero, UpsertGalleryItem, UpsertOffer,
};

#[component]
pub fn HomeAdminPage() -> impl IntoView {
    let saver_hero = ServerAction::<UpdateHero>::new();
    let saver_offer = ServerAction::<UpsertOffer>::new();
    let deleter_offer = ServerAction::<DeleteOffer>::new();
    let saver_gallery = ServerAction::<UpsertGalleryItem>::new();
    let deleter_gallery = ServerAction::<DeleteGalleryItem>::new();

    let content = Resource::new(
        move || {
            (
                saver_hero.version().get(),
                saver_offer.version().get(),
                deleter_offer.version().get(),
                saver_gallery.version().get(),
                deleter_gallery.version().get(),
            )
        },
        |_| async move { get_home_content().await },
    );
    let offers_admin = Resource::new(
        move || (saver_offer.version().get(), deleter_offer.version().get()),
        |_| async move { list_offers_admin().await },
    );

    view! {
        <AdminShell>
            <section class="home-admin">
                <header class="admin-bar"><h1>"Startseite bearbeiten"</h1></header>
                <p class="hint">
                    "Hier kannst du den Hero, die Angebots-Karten und die Galerie pflegen. "
                    "Bilder werden hochgeladen und unter /img/uploads gespeichert."
                </p>

                <Suspense fallback=|| view! { <p class="loading">"Lädt…"</p> }>
                    {move || content.get().map(|res| match res {
                        Err(e) => view! { <p class="error">{format!("Fehler: {e}")}</p> }.into_any(),
                        Ok(c) => view! {
                            <HeroEditor c=c.clone() saver=saver_hero/>
                        }.into_any(),
                    })}
                </Suspense>

                <h2>"Angebote"</h2>
                <Suspense fallback=|| view! { <p class="loading">"Lädt…"</p> }>
                    {move || offers_admin.get().map(|res| match res {
                        Err(e) => view! { <p class="error">{format!("Fehler: {e}")}</p> }.into_any(),
                        Ok(rows) => view! {
                            <OffersEditor rows saver=saver_offer deleter=deleter_offer/>
                        }.into_any(),
                    })}
                </Suspense>

                <h2>"Galerie"</h2>
                <Suspense fallback=|| view! { <p class="loading">"Lädt…"</p> }>
                    {move || content.get().map(|res| match res {
                        Err(_) => view! { <p class="error">"Galerie konnte nicht geladen werden."</p> }.into_any(),
                        Ok(c) => view! {
                            <GalleryEditor rows=c.gallery saver=saver_gallery deleter=deleter_gallery/>
                        }.into_any(),
                    })}
                </Suspense>
            </section>
        </AdminShell>
    }
}

#[component]
fn HeroEditor(c: HomeContent, saver: ServerAction<UpdateHero>) -> impl IntoView {
    let title = RwSignal::new(c.hero.title.clone());
    let subtitle = RwSignal::new(c.hero.subtitle.clone());
    let image_path = RwSignal::new(c.hero.image_path.clone().unwrap_or_default());

    let on_save = move |_| {
        saver.dispatch(UpdateHero {
            title: title.get(),
            subtitle: subtitle.get(),
            image_path: image_path.get(),
        });
    };

    view! {
        <div class="home-admin-card">
            <h3>"Hero"</h3>
            <label>
                <span>"Titel"</span>
                <input type="text" maxlength="60"
                    prop:value=move || title.get()
                    on:input=move |ev| title.set(event_target_value(&ev))/>
            </label>
            <label>
                <span>"Untertitel"</span>
                <textarea rows="2" maxlength="200"
                    prop:value=move || subtitle.get()
                    on:input=move |ev| subtitle.set(event_target_value(&ev))></textarea>
            </label>
            <HeroBackgroundPicker path=image_path/>
            <div class="row-actions">
                <button class="btn primary" on:click=on_save>"Speichern"</button>
                {move || saver.value().get().map(|res| match res {
                    Err(e) => view! { <span class="error">{format!("Fehler: {e}")}</span> }.into_any(),
                    Ok(_) => view! { <span class="ok">"✓ gespeichert"</span> }.into_any(),
                })}
            </div>
        </div>
    }
}

/// Hero-only background picker. Two tabs: pick a gradient/colour preset
/// (writes `gradient:<key>` into the signal) or upload/pick a photo
/// (delegates to the regular `ImagePicker`). Each preset renders a
/// swatch using the same CSS the public hero will use, so the admin
/// sees the exact result.
#[component]
fn HeroBackgroundPicker(path: RwSignal<String>) -> impl IntoView {
    use crate::pages::home_content::HERO_PRESETS;

    let is_gradient = move || path.get().starts_with("gradient:");
    let active_key = move || {
        path.get()
            .strip_prefix("gradient:")
            .map(|s| s.to_string())
            .unwrap_or_default()
    };

    view! {
        <div class="hero-bg-picker">
            <div class="hero-bg-tabs">
                <button type="button"
                    class:active=move || is_gradient()
                    on:click=move |_| {
                        // Switch to gradient mode if not already; pick the
                        // first preset as a sensible default.
                        if !path.get().starts_with("gradient:") {
                            if let Some(p) = HERO_PRESETS.first() {
                                path.set(format!("gradient:{}", p.key));
                            }
                        }
                    }>
                    "Farbe / Verlauf"
                </button>
                <button type="button"
                    class:active=move || !is_gradient()
                    on:click=move |_| {
                        if path.get().starts_with("gradient:") {
                            path.set(String::new());
                        }
                    }>
                    "Foto"
                </button>
            </div>

            {move || if is_gradient() {
                view! {
                    <div class="hero-preset-grid">
                        {HERO_PRESETS.iter().map(|p| {
                            let key = p.key.to_string();
                            let key_for_class = key.clone();
                            let key_for_click = key.clone();
                            let css = p.css.to_string();
                            let label = p.label_de.to_string();
                            view! {
                                <button type="button"
                                    class="hero-preset"
                                    class:active=move || active_key() == key_for_class
                                    on:click=move |_| path.set(format!("gradient:{}", key_for_click))>
                                    <span class="swatch" style=format!("background: {css};")></span>
                                    <span class="lbl">{label}</span>
                                </button>
                            }
                        }).collect_view()}
                    </div>
                }.into_any()
            } else {
                view! { <ImagePicker path/> }.into_any()
            }}
        </div>
    }
}

#[component]
fn OffersEditor(
    rows: Vec<Offer>,
    saver: ServerAction<UpsertOffer>,
    deleter: ServerAction<DeleteOffer>,
) -> impl IntoView {
    let new_offer_form = RwSignal::new(false);
    view! {
        <div class="home-admin-list">
            {rows.into_iter().map(|o| view! {
                <OfferRowEditor o saver deleter/>
            }).collect_view()}

            {move || if new_offer_form.get() {
                view! { <NewOfferForm saver close=move || new_offer_form.set(false)/> }.into_any()
            } else {
                view! {
                    <button class="btn ghost" on:click=move |_| new_offer_form.set(true)>
                        "+ Neues Angebot"
                    </button>
                }.into_any()
            }}
        </div>
    }
}

#[component]
fn OfferRowEditor(
    o: Offer,
    saver: ServerAction<UpsertOffer>,
    deleter: ServerAction<DeleteOffer>,
) -> impl IntoView {
    let id = o.id.clone();
    let id_for_delete = o.id.clone();
    let title = RwSignal::new(o.title);
    let price = RwSignal::new(o.price_label);
    let blurb = RwSignal::new(o.blurb);
    let add_on = RwSignal::new(o.add_on.unwrap_or_default());
    let image_path = RwSignal::new(o.image_path.unwrap_or_default());
    let active = RwSignal::new(o.active);
    let position = RwSignal::new(o.position);

    let on_save = move |_| {
        saver.dispatch(UpsertOffer {
            id: id.clone(),
            title: title.get(),
            price_label: price.get(),
            blurb: blurb.get(),
            add_on: add_on.get(),
            image_path: image_path.get(),
            active: active.get(),
            position: position.get(),
        });
    };
    let on_delete = move |_| {
        deleter.dispatch(DeleteOffer {
            id: id_for_delete.clone(),
        });
    };

    view! {
        <div class="home-admin-card">
            <label>
                <span>"Titel"</span>
                <input type="text" prop:value=move || title.get()
                    on:input=move |ev| title.set(event_target_value(&ev))/>
            </label>
            <label>
                <span>"Preis-Text"</span>
                <input type="text" prop:value=move || price.get()
                    on:input=move |ev| price.set(event_target_value(&ev))/>
            </label>
            <label>
                <span>"Beschreibung"</span>
                <input type="text" prop:value=move || blurb.get()
                    on:input=move |ev| blurb.set(event_target_value(&ev))/>
            </label>
            <label>
                <span>"Zusatzzeile (optional)"</span>
                <input type="text" prop:value=move || add_on.get()
                    on:input=move |ev| add_on.set(event_target_value(&ev))/>
            </label>
            <ImagePicker path=image_path/>
            <div class="row-toggles">
                <label>
                    <input type="checkbox" prop:checked=move || active.get()
                        on:change=move |ev| active.set(event_target_checked(&ev))/>
                    " Aktiv"
                </label>
                <label>
                    " Position "
                    <input type="number" style="width:5rem" prop:value=move || position.get().to_string()
                        on:input=move |ev| {
                            if let Ok(n) = event_target_value(&ev).parse::<i64>() {
                                position.set(n);
                            }
                        }/>
                </label>
            </div>
            <div class="row-actions">
                <button class="btn primary small" on:click=on_save>"Speichern"</button>
                <button class="btn ghost small" on:click=on_delete>"Löschen"</button>
            </div>
        </div>
    }
}

#[component]
fn NewOfferForm(
    saver: ServerAction<UpsertOffer>,
    close: impl Fn() + 'static + Copy + Send,
) -> impl IntoView {
    let title = RwSignal::new(String::new());
    let price = RwSignal::new(String::new());
    let blurb = RwSignal::new(String::new());
    let add_on = RwSignal::new(String::new());
    let image_path = RwSignal::new(String::new());

    let on_save = move |_| {
        saver.dispatch(UpsertOffer {
            id: String::new(),
            title: title.get(),
            price_label: price.get(),
            blurb: blurb.get(),
            add_on: add_on.get(),
            image_path: image_path.get(),
            active: true,
            position: 100,
        });
        close();
    };

    view! {
        <div class="home-admin-card">
            <h4>"Neues Angebot"</h4>
            <label><span>"Titel"</span>
                <input type="text" prop:value=move || title.get()
                    on:input=move |ev| title.set(event_target_value(&ev))/>
            </label>
            <label><span>"Preis"</span>
                <input type="text" prop:value=move || price.get()
                    on:input=move |ev| price.set(event_target_value(&ev))/>
            </label>
            <label><span>"Beschreibung"</span>
                <input type="text" prop:value=move || blurb.get()
                    on:input=move |ev| blurb.set(event_target_value(&ev))/>
            </label>
            <label><span>"Zusatzzeile"</span>
                <input type="text" prop:value=move || add_on.get()
                    on:input=move |ev| add_on.set(event_target_value(&ev))/>
            </label>
            <ImagePicker path=image_path/>
            <div class="row-actions">
                <button class="btn primary" on:click=on_save>"Anlegen"</button>
                <button class="btn ghost" on:click=move |_| close()>"Abbrechen"</button>
            </div>
        </div>
    }
}

#[component]
fn GalleryEditor(
    rows: Vec<GalleryItem>,
    saver: ServerAction<UpsertGalleryItem>,
    deleter: ServerAction<DeleteGalleryItem>,
) -> impl IntoView {
    let new_form = RwSignal::new(false);
    view! {
        <div class="home-admin-list">
            {rows.into_iter().map(|g| view! {
                <GalleryRowEditor g saver deleter/>
            }).collect_view()}
            {move || if new_form.get() {
                view! { <NewGalleryForm saver close=move || new_form.set(false)/> }.into_any()
            } else {
                view! {
                    <button class="btn ghost" on:click=move |_| new_form.set(true)>
                        "+ Neues Bild"
                    </button>
                }.into_any()
            }}
        </div>
    }
}

#[component]
fn GalleryRowEditor(
    g: GalleryItem,
    saver: ServerAction<UpsertGalleryItem>,
    deleter: ServerAction<DeleteGalleryItem>,
) -> impl IntoView {
    let id = g.id.clone();
    let id_for_delete = g.id.clone();
    let image_path = RwSignal::new(g.image_path);
    let caption = RwSignal::new(g.caption);
    let position = RwSignal::new(g.position);

    let on_save = move |_| {
        saver.dispatch(UpsertGalleryItem {
            id: id.clone(),
            image_path: image_path.get(),
            caption: caption.get(),
            position: position.get(),
        });
    };
    let on_delete = move |_| {
        deleter.dispatch(DeleteGalleryItem {
            id: id_for_delete.clone(),
        });
    };
    view! {
        <div class="home-admin-card">
            <ImagePicker path=image_path/>
            <label>
                <span>"Bildunterschrift"</span>
                <input type="text" prop:value=move || caption.get()
                    on:input=move |ev| caption.set(event_target_value(&ev))/>
            </label>
            <label>
                " Position "
                <input type="number" style="width:5rem" prop:value=move || position.get().to_string()
                    on:input=move |ev| {
                        if let Ok(n) = event_target_value(&ev).parse::<i64>() {
                            position.set(n);
                        }
                    }/>
            </label>
            <div class="row-actions">
                <button class="btn primary small" on:click=on_save>"Speichern"</button>
                <button class="btn ghost small" on:click=on_delete>"Löschen"</button>
            </div>
        </div>
    }
}

#[component]
fn NewGalleryForm(
    saver: ServerAction<UpsertGalleryItem>,
    close: impl Fn() + 'static + Copy + Send,
) -> impl IntoView {
    let image_path = RwSignal::new(String::new());
    let caption = RwSignal::new(String::new());

    let on_save = move |_| {
        saver.dispatch(UpsertGalleryItem {
            id: String::new(),
            image_path: image_path.get(),
            caption: caption.get(),
            position: 100,
        });
        close();
    };

    view! {
        <div class="home-admin-card">
            <h4>"Neues Bild"</h4>
            <ImagePicker path=image_path/>
            <label><span>"Bildunterschrift"</span>
                <input type="text" prop:value=move || caption.get()
                    on:input=move |ev| caption.set(event_target_value(&ev))/>
            </label>
            <div class="row-actions">
                <button class="btn primary" on:click=on_save>"Anlegen"</button>
                <button class="btn ghost" on:click=move |_| close()>"Abbrechen"</button>
            </div>
        </div>
    }
}

/// Image upload + path display. Lives entirely in the browser:
/// `<input type="file">` change handler → `fetch('/api/admin/upload_image')`
/// with multipart/form-data → response JSON → write `path` into the
/// signal so the parent form picks it up on submit.
#[component]
fn ImagePicker(path: RwSignal<String>) -> impl IntoView {
    let uploading = RwSignal::new(false);
    let err = RwSignal::new(Option::<String>::None);

    let on_change = move |ev: leptos::ev::Event| {
        #[cfg(feature = "hydrate")]
        {
            use wasm_bindgen::JsCast;
            let target = ev
                .target()
                .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok());
            let Some(input) = target else { return };
            let Some(files) = input.files() else { return };
            let Some(file) = files.get(0) else { return };

            uploading.set(true);
            err.set(None);
            spawn_local(async move {
                match upload_via_fetch(file).await {
                    Ok(p) => path.set(p),
                    Err(e) => err.set(Some(e)),
                }
                uploading.set(false);
            });
        }
        #[cfg(not(feature = "hydrate"))]
        {
            let _ = ev;
            let _ = uploading;
            let _ = err;
        }
    };

    view! {
        <div class="image-picker">
            <label>
                <span>"Bildpfad"</span>
                <input type="text" placeholder="/img/uploads/…"
                    prop:value=move || path.get()
                    on:input=move |ev| path.set(event_target_value(&ev))/>
            </label>
            <label class="upload-row">
                <span>"Hochladen"</span>
                <input type="file" accept="image/jpeg,image/png,image/webp"
                    on:change=on_change/>
                {move || if uploading.get() {
                    Some(view! { <span class="muted">"Wird hochgeladen…"</span> }.into_any())
                } else if let Some(e) = err.get() {
                    Some(view! { <span class="error">{e}</span> }.into_any())
                } else if !path.get().is_empty() {
                    Some(view! {
                        <img class="image-preview" src=path.get() alt=""
                            style="max-height:72px;border-radius:.4rem"/>
                    }.into_any())
                } else {
                    None
                }}
            </label>
        </div>
    }
}

#[cfg(feature = "hydrate")]
async fn upload_via_fetch(file: web_sys::File) -> Result<String, String> {
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::JsFuture;

    let form = web_sys::FormData::new().map_err(|_| "FormData unsupported".to_string())?;
    form.append_with_blob_and_filename("file", file.as_ref(), &file.name())
        .map_err(|_| "form append".to_string())?;

    let opts = web_sys::RequestInit::new();
    opts.set_method("POST");
    opts.set_body(form.as_ref());
    let req = web_sys::Request::new_with_str_and_init("/api/admin/upload_image", &opts)
        .map_err(|_| "request build".to_string())?;
    let win = web_sys::window().ok_or_else(|| "no window".to_string())?;
    let resp_val = JsFuture::from(win.fetch_with_request(&req))
        .await
        .map_err(|_| "fetch failed".to_string())?;
    let resp: web_sys::Response = resp_val
        .dyn_into()
        .map_err(|_| "not Response".to_string())?;
    if !resp.ok() {
        let status = resp.status();
        let text = JsFuture::from(resp.text().map_err(|_| "no text".to_string())?)
            .await
            .ok()
            .and_then(|v| v.as_string())
            .unwrap_or_default();
        return Err(format!("HTTP {status}: {text}"));
    }
    let json_promise = resp.json().map_err(|_| "no json".to_string())?;
    let json_val = JsFuture::from(json_promise)
        .await
        .map_err(|_| "json parse".to_string())?;
    let obj: js_sys::Object = json_val.dyn_into().map_err(|_| "not object".to_string())?;
    let path_val =
        js_sys::Reflect::get(&obj, &"path".into()).map_err(|_| "no path field".to_string())?;
    path_val
        .as_string()
        .ok_or_else(|| "path not string".to_string())
}
