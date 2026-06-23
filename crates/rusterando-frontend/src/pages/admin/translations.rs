//! /admin/translations — per-shop translation gap report.
//!
//! Shows the shop owner which of THEIR menu phrases are still untranslated, per
//! locale. The heavy lifting runs in the management `.wasm`
//! (rusterando-i18n-mgmt): it carries the i18n pack's menu COVERAGE (which
//! (locale, german) pairs have a translation) and exposes `gap_report`. This
//! page loads that wasm (admin-only), feeds it the shop's German phrases (the
//! `name_source` / `description_source` from `list_admin_menu` + category
//! names), and renders the gaps. A download button exports the worklist.
//!
//! DB-first reminder: a phrase being "translated" means the PACK has it; the
//! shop can still override any single value by filling the `_<lang>` DB cell.
//! This report is about pack coverage, the bulk path.
//!
//! SSR renders a loading shell (the wasm only runs client-side). Hydrate loads
//! the mgmt wasm and computes the report.

use leptos::prelude::*;

use crate::pages::admin::shell::AdminShell;
use crate::pages::menu::list_admin_menu;

// Inlined loaders (see app.rs WASM_SPLIT_LOADER_JS — prod hashing 404s a
// `/pkg/*.js` src). The generic factory + the thin mgmt config.
const WASM_SPLIT_LOADER_JS: &str = include_str!("../../static/wasm-split-loader.js");
const I18N_MGMT_LOADER_JS: &str = include_str!("../../static/i18n-mgmt-loader.js");

#[component]
pub fn TranslationsAdminPage() -> impl IntoView {
    // The shop's menu (German base). Blocking so SSR has the data and the
    // hydrate side can compute the report as soon as the wasm is ready.
    let menu = Resource::new_blocking(|| (), |_| async move { list_admin_menu().await });

    // The rendered report (set on the hydrate side once the wasm returns).
    let report = RwSignal::new(String::new());
    let status = RwSignal::new(String::from("Lädt Verwaltungs-Modul …"));

    // Collect the shop's distinct German phrases from the loaded menu: item
    // name_source + description_source + category names. These are the strings
    // the gap report checks against the pack.
    let phrases = move || -> Vec<String> {
        let Some(Ok(payload)) = menu.get() else {
            return Vec::new();
        };
        let mut out: Vec<String> = Vec::new();
        for c in &payload.categories {
            if !c.name.trim().is_empty() {
                out.push(c.name.clone());
            }
        }
        for it in &payload.items {
            if !it.name_source.trim().is_empty() {
                out.push(it.name_source.clone());
            }
            if let Some(d) = &it.description_source {
                if !d.trim().is_empty() {
                    out.push(d.clone());
                }
            }
        }
        out
    };

    // Hydrate-only: load the mgmt wasm, run the gap report over the shop's
    // phrases, and stash the JSON result.
    #[cfg(feature = "hydrate")]
    {
        let phrases = phrases.clone();
        Effect::new(move |_| {
            let ph = phrases();
            if ph.is_empty() {
                return; // menu not loaded yet; the effect re-runs when it is
            }
            let report = report;
            let status = status;
            leptos::task::spawn_local(async move {
                match run_gap_report(&ph).await {
                    Ok(json) => {
                        report.set(json);
                        status.set(String::new());
                    }
                    Err(e) => status.set(format!("Fehler: {e}")),
                }
            });
        });
    }

    let download = move |_| {
        #[cfg(feature = "hydrate")]
        download_report(&report.get_untracked());
    };

    view! {
        <AdminShell>
            // Mgmt-wasm loaders, INLINED (see app.rs WASM_SPLIT_LOADER_JS — the
            // prod LEPTOS_HASH_FILES pass would 404 a hashed `/pkg/*.js` src).
            // Generic factory first, then the thin mgmt config. Only on this
            // admin page, so it never loads on the public site.
            <script>{WASM_SPLIT_LOADER_JS}</script>
            <script>{I18N_MGMT_LOADER_JS}</script>
            <section class="admin-section">
                <h1>"Übersetzungen — Lückenbericht"</h1>
                <p class="muted">
                    "Zeigt, welche Begriffe aus IHRER Speisekarte noch nicht in der "
                    "Übersetzungs-Bibliothek (i18n-Paket) vorhanden sind — pro Sprache. "
                    "Übersetzte Begriffe erscheinen automatisch; fehlende zeigt die App "
                    "auf Deutsch. Einzelne Werte können Sie jederzeit im jeweiligen "
                    "Editor selbst überschreiben."
                </p>

                <Suspense fallback=|| view! { <p>"Lädt Speisekarte …"</p> }>
                    {move || {
                        let n = phrases().len();
                        view! {
                            <p class="muted">{format!("{n} Begriffe aus der Speisekarte geprüft.")}</p>
                        }
                    }}
                </Suspense>

                {move || (!status.get().is_empty()).then(|| view! {
                    <p class="muted">{status.get()}</p>
                })}

                {move || {
                    let r = report.get();
                    (!r.is_empty()).then(|| view! {
                        <GapView json=r/>
                        <button class="btn" on:click=download>
                            "Lückenliste herunterladen (JSON)"
                        </button>
                    })
                }}
            </section>
        </AdminShell>
    }
}

/// Render the gap-report JSON as a readable per-locale summary. The JSON shape
/// is `{ "<locale>": ["missing phrase", …], …, "_summary": {…} }`. We parse it
/// with serde_json (available in both ssr/hydrate via the frontend deps).
#[component]
fn GapView(json: String) -> impl IntoView {
    use std::collections::BTreeMap;
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap_or(serde_json::Value::Null);

    // Build (locale, missing[]) rows, skipping the _summary key.
    let mut rows: BTreeMap<String, Vec<String>> = BTreeMap::new();
    if let Some(obj) = parsed.as_object() {
        for (k, v) in obj {
            if k == "_summary" {
                continue;
            }
            let list: Vec<String> = v
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            rows.insert(k.clone(), list);
        }
    }

    // Static once computed (no reactivity needed) — a plain map is simpler and
    // sidesteps the <For> key-closure tuple-pattern parsing quirk.
    let blocks = rows
        .into_iter()
        .map(|(loc, list)| {
            let count = list.len();
            let head = if count == 0 {
                format!("{loc}: vollständig übersetzt ✓")
            } else {
                format!("{loc}: {count} fehlend")
            };
            let items = list
                .into_iter()
                .map(|p| view! { <li>{p}</li> })
                .collect_view();
            view! {
                <details open=count > 0>
                    <summary>{head}</summary>
                    <ul>{items}</ul>
                </details>
            }
        })
        .collect_view();

    view! { <div class="translations-gaps">{blocks}</div> }
}

// ---------------------------------------------------------------------------
// Hydrate-side wasm interop.
// ---------------------------------------------------------------------------

/// Load the mgmt wasm (idempotent) and call its `gapReport` via the shared
/// split-wasm interop. Returns the JSON string. Empty locales arg → the wasm
/// checks all locales the pack covers.
#[cfg(feature = "hydrate")]
async fn run_gap_report(phrases: &[String]) -> Result<String, String> {
    let phrases_nl = phrases.join("\n");
    rusterando_wasm_split::interop::load_and_call(
        "__loadI18nMgmt",
        "__i18nMgmt",
        "gapReport",
        &[&phrases_nl, ""],
    )
    .await
}

/// Trigger a browser download of the gap-report JSON.
#[cfg(feature = "hydrate")]
fn download_report(json: &str) {
    use wasm_bindgen::{JsCast, JsValue};
    let Some(win) = web_sys::window() else { return };
    let Some(doc) = win.document() else { return };
    // data: URL keeps it dependency-free (no Blob/URL juggling).
    let encoded = js_sys::encode_uri_component(json);
    let href = format!(
        "data:application/json;charset=utf-8,{}",
        String::from(encoded)
    );
    if let Ok(a) = doc.create_element("a") {
        let _ = a.set_attribute("href", &href);
        let _ = a.set_attribute("download", "uebersetzungs-luecken.json");
        if let Ok(a) = a.dyn_into::<web_sys::HtmlElement>() {
            let _ = a.click();
            let _ = JsValue::from(&a); // keep `a` alive through the click
        }
    }
}
