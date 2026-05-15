//! /admin/pricing — sanity check for menu/extras price inversions.
//!
//! Background
//! ----------
//! If the price of "Extra Salami" is set to 1,00 €, then a customer can
//! order a Margherita (5,00 €) + Extra Salami (1,00 €) for 6,00 € and
//! get effectively the same pizza as "Pizza Salami della Casa" (5,80 €).
//! The Margherita-plus-extras path is then never *more expensive* than
//! the named pizza, and on large sizes it can be cheaper. Customers do
//! this math too, so the named pizzas don't sell.
//!
//! This page surfaces all such inversions so the admin can re-price
//! either the extra or the named pizza. It's read-only — the admin
//! takes action via /admin/menu or /admin/extras. We deliberately do
//! NOT auto-fix, because either side could be the "correct" price.
//!
//! Matching strategy (v1):
//!   1. Pick a single global "base" pizza — the listed pizza with the
//!      lowest small price across all categories. On Davids' menu that
//!      is "Pizza Margherita". Customers can order this base and add
//!      any extra, regardless of the named pizza's category.
//!   2. For each pizza_extra, search ALL listed pizzas/calzones for
//!      one whose name contains the extra's label (case-insensitive).
//!      Synonyms cover the de↔it translation:
//!      Schinken ↔ Prosciutto, Pilze ↔ Funghi, Thunfisch ↔ Tonno,
//!      Krabben ↔ Gamberi, Lachs ↔ Salmone, Zwiebeln ↔ Cipolla.
//!   3. If matched, compute combo_price = base_price + extra_price for
//!      both small and large sizes. Compare with named_pizza price; the
//!      inversion is `inv = named_price - combo_price` (positive = named
//!      pizza is more expensive, which is correct; negative or zero is
//!      the bug).
//!   4. Suggest a new extra price: enough to lift combo above named by
//!      a 0,50 € margin: `suggested = named_price - base_price + 50`.

use leptos::prelude::*;
use rusterando_shared::models::format_eur;
use serde::{Deserialize, Serialize};

use crate::pages::admin::shell::AdminShell;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PricingIssue {
    /// `"Extra Salami"` etc.
    pub extra_label: String,
    pub extra_id: String,
    pub extra_price_cents: i64,
    /// `"Pizza Salami della Casa"` — the named pizza this extra duplicates.
    pub matched_pizza_name: String,
    pub matched_pizza_id: String,
    /// Base item used for the combo (usually Pizza Margherita).
    pub base_pizza_name: String,
    pub base_small_cents: i64,
    pub base_large_cents: Option<i64>,
    pub matched_small_cents: i64,
    pub matched_large_cents: Option<i64>,
    /// Combo prices using base + extra.
    pub combo_small_cents: i64,
    pub combo_large_cents: Option<i64>,
    /// `matched - combo`. Negative or zero = the combo undercuts the
    /// named pizza on that size; that's the bug.
    pub inversion_small_cents: i64,
    pub inversion_large_cents: Option<i64>,
    /// What the named pizza's small price would have to become so that
    /// the combo (Margherita + extra) is at least 50 ct cheaper. This is
    /// the actionable suggestion: raise the named pizza's small to this
    /// value. None means small is already fine.
    pub suggested_named_small_cents: Option<i64>,
    /// Same idea for the large size. None means large is already fine
    /// (or the named pizza has no large variant).
    pub suggested_named_large_cents: Option<i64>,
}

impl PricingIssue {
    /// Worst-case (most negative) inversion across sizes. Used for
    /// sorting; smaller is more urgent.
    pub fn worst_inversion(&self) -> i64 {
        let mut w = self.inversion_small_cents;
        if let Some(l) = self.inversion_large_cents {
            if l < w {
                w = l;
            }
        }
        w
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PricingReport {
    pub issues: Vec<PricingIssue>,
    /// Extras with no matching pizza in the menu — these can't cause an
    /// inversion. Surfaced so the admin sees the matcher's coverage.
    pub unmatched_extras: Vec<String>,
}

#[server(name = AnalyzePricing, prefix = "/api", endpoint = "analyze_pricing")]
pub async fn analyze_pricing() -> Result<PricingReport, ServerFnError> {
    use sqlx::SqlitePool;
    crate::pages::admin::require_admin().await?;
    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;

    // Pizzas + calzones with their category. Only listed items so we
    // don't flag inversions on archived dishes.
    let pizzas: Vec<(String, String, String, i64, Option<i64>)> = sqlx::query_as(
        "SELECT id, name, category_id, price_small_cents, price_large_cents
         FROM menu_items
         WHERE is_listed = 1 AND item_type IN ('pizza','calzone')",
    )
    .fetch_all(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("load pizzas: {e}")))?;

    let extras: Vec<(String, String, i64)> = sqlx::query_as(
        "SELECT id, label, price_cents
         FROM pizza_extras
         WHERE is_available = 1 AND price_cents > 0",
    )
    .fetch_all(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("load extras: {e}")))?;

    Ok(analyze(pizzas, extras))
}

/// Pure-Rust core so we can unit-test it without a DB. Inputs are
/// (id, name, category_id, small, large) for pizzas and (id, label,
/// price) for extras.
#[cfg(feature = "ssr")]
fn analyze(
    pizzas: Vec<(String, String, String, i64, Option<i64>)>,
    extras: Vec<(String, String, i64)>,
) -> PricingReport {
    // Single global base: the cheapest listed pizza across all
    // categories. Customers can order Margherita and tack on any
    // extra regardless of where the named target sits in the menu.
    let base_idx = match pizzas
        .iter()
        .enumerate()
        .min_by_key(|(_, p)| p.3)
        .map(|(i, _)| i)
    {
        Some(i) => i,
        None => {
            return PricingReport {
                issues: vec![],
                unmatched_extras: extras.into_iter().map(|(_, l, _)| l).collect(),
            };
        }
    };

    let mut issues = Vec::new();
    let mut unmatched = Vec::new();
    let base = &pizzas[base_idx];

    for (eid, elabel, eprice) in &extras {
        let mut matched_any = false;
        for (pi, p) in pizzas.iter().enumerate() {
            if pi == base_idx {
                continue;
            }
            if !name_matches(&p.1, elabel) {
                continue;
            }
            matched_any = true;
            let combo_small = base.3 + eprice;
            let combo_large = base.4.zip(p.4).map(|(bl, _)| bl + eprice);
            let inv_small = p.3 - combo_small;
            let inv_large = p.4.zip(combo_large).map(|(pl, cs)| pl - cs);
            // Only flag if at least one size is inverted (<= 0).
            let small_bad = inv_small <= 0;
            let large_bad = inv_large.map(|l| l <= 0).unwrap_or(false);
            if !small_bad && !large_bad {
                continue;
            }
            // Suggested named-pizza prices: enough so combo is at least
            // 50 ct cheaper than named. We only suggest a bump for the
            // size(s) that are actually inverted — no point telling the
            // admin to raise a price that's already healthy.
            let suggested_small = small_bad.then_some(combo_small + 50);
            let suggested_large = match (large_bad, combo_large) {
                (true, Some(cl)) => Some(cl + 50),
                _ => None,
            };
            issues.push(PricingIssue {
                extra_label: elabel.clone(),
                extra_id: eid.clone(),
                extra_price_cents: *eprice,
                matched_pizza_name: p.1.clone(),
                matched_pizza_id: p.0.clone(),
                base_pizza_name: base.1.clone(),
                base_small_cents: base.3,
                base_large_cents: base.4,
                matched_small_cents: p.3,
                matched_large_cents: p.4,
                combo_small_cents: combo_small,
                combo_large_cents: combo_large,
                inversion_small_cents: inv_small,
                inversion_large_cents: inv_large,
                suggested_named_small_cents: suggested_small,
                suggested_named_large_cents: suggested_large,
            });
        }
        if !matched_any {
            unmatched.push(elabel.clone());
        }
    }

    // Sort worst-first so the admin sees the biggest gap at the top.
    issues.sort_by_key(|i| i.worst_inversion());

    PricingReport {
        issues,
        unmatched_extras: unmatched,
    }
}

/// Case-insensitive check whether a pizza name "matches" an extra label.
/// Either the extra word appears in the pizza name OR a synonym does.
#[cfg(feature = "ssr")]
fn name_matches(pizza_name: &str, extra_label: &str) -> bool {
    let pn = pizza_name.to_lowercase();
    let needle = clean_extra(extra_label).to_lowercase();
    if needle.is_empty() {
        return false;
    }
    if pn.contains(&needle) {
        return true;
    }
    // de ↔ it synonyms covering Davids' menu. Extendable.
    let syn: &[(&str, &[&str])] = &[
        ("schinken", &["prosciutto"]),
        ("pilze", &["funghi"]),
        ("thunfisch", &["tonno"]),
        ("krabben", &["gamberi"]),
        ("lachs", &["salmone"]),
        ("zwiebeln", &["cipolla", "cipolle"]),
        ("käse", &["formaggi", "quattro formaggi"]),
        ("knoblauch", &["aglio"]),
        ("salami", &["salami"]),
        ("peperoni", &["peperoni"]),
        ("artischocken", &["carciofi"]),
        ("oliven", &["olive"]),
        ("mais", &["mais"]),
        ("ananas", &["ananas", "hawaii"]),
    ];
    for (de, it_list) in syn {
        if &needle == de {
            for it in *it_list {
                if pn.contains(it) {
                    return true;
                }
            }
        }
    }
    false
}

/// Strip the "Extra " prefix and trim — "Extra Salami" → "Salami".
#[cfg(feature = "ssr")]
fn clean_extra(label: &str) -> String {
    let lower = label.trim().to_lowercase();
    let lower = lower.strip_prefix("extra ").unwrap_or(&lower);
    lower.trim().to_string()
}

// ---------------------------------------------------------------------------
// View
// ---------------------------------------------------------------------------

#[component]
pub fn PricingAdminPage() -> impl IntoView {
    let report = Resource::new(|| (), |_| async move { analyze_pricing().await });

    view! {
        <AdminShell>
            <section class="pricing-admin">
                <header class="admin-bar">
                    <h1>"Preis-Prüfung"</h1>
                </header>

                <p class="hint">
                    "Erkennt Fälle, in denen "
                    <b>"Margherita + Extra"</b>
                    " denselben (oder günstigeren) Preis hat als die "
                    "fertige Pizza mit der gleichen Zutat. Beispiel: "
                    "Margherita (5,00 €) + Extra Salami (1,00 €) = 6,00 € "
                    "ist günstiger als Pizza Salami della Casa (5,80 €). "
                    "Empfohlene Korrektur: Preis der fertigen Pizza in "
                    <a href="/admin/menu">"/admin/menu"</a>
                    " anheben — meist um 0,50–1,50 €. Sortiert nach "
                    "Dringlichkeit (größte Inversion zuerst)."
                </p>

                <Suspense fallback=|| view! { <p class="loading">"Lädt…"</p> }>
                    {move || report.get().map(|res| match res {
                        Err(e) => view! { <p class="error">{format!("Fehler: {e}")}</p> }.into_any(),
                        Ok(r) => view! { <ReportView r/> }.into_any(),
                    })}
                </Suspense>
            </section>
        </AdminShell>
    }
}

#[component]
fn ReportView(r: PricingReport) -> impl IntoView {
    let has_issues = !r.issues.is_empty();
    let unmatched = r.unmatched_extras.clone();
    view! {
        {if has_issues {
            view! {
                <table class="pricing-table">
                    <thead>
                        <tr>
                            <th>"Extra"</th>
                            <th>"Preis"</th>
                            <th>"Basis"</th>
                            <th>"Kombination"</th>
                            <th>"Fertige Pizza"</th>
                            <th>"Differenz (klein/groß)"</th>
                            <th>"Fertige Pizza anheben auf"</th>
                        </tr>
                    </thead>
                    <tbody>
                        {r.issues.into_iter().map(|i| view! { <IssueRow i/> }).collect_view()}
                    </tbody>
                </table>
            }.into_any()
        } else {
            view! {
                <p class="ok">"✓ Keine Inversionen gefunden. Preisstruktur ist konsistent."</p>
            }.into_any()
        }}

        {if unmatched.is_empty() {
            ().into_any()
        } else {
            view! {
                <details class="unmatched">
                    <summary>
                        {format!("Nicht zugeordnete Extras ({})", unmatched.len())}
                    </summary>
                    <p class="hint">
                        "Diese Extras haben keine namensgleiche fertige Pizza im Menü — "
                        "es kann also keine Preis-Inversion entstehen."
                    </p>
                    <ul>
                        {unmatched.into_iter().map(|e| view! { <li>{e}</li> }).collect_view()}
                    </ul>
                </details>
            }.into_any()
        }}
    }
}

#[component]
fn IssueRow(i: PricingIssue) -> impl IntoView {
    let combo_l = i
        .combo_large_cents
        .map(format_eur)
        .unwrap_or_else(|| "—".to_string());
    let matched_l = i
        .matched_large_cents
        .map(format_eur)
        .unwrap_or_else(|| "—".to_string());
    let inv_l = i
        .inversion_large_cents
        .map(format_eur_signed)
        .unwrap_or_else(|| "—".to_string());
    let row_cls = if i.worst_inversion() <= 0 {
        "row-bad"
    } else {
        ""
    };
    view! {
        <tr class=row_cls>
            <td><b>{i.extra_label}</b></td>
            <td>{format_eur(i.extra_price_cents)}</td>
            <td>
                <span class="muted">{i.base_pizza_name}</span>
                <br/>
                <span class="muted small">
                    {format_eur(i.base_small_cents)}
                    " / "
                    {i.base_large_cents.map(format_eur).unwrap_or_else(|| "—".to_string())}
                </span>
            </td>
            <td>
                <b>{format_eur(i.combo_small_cents)}</b>
                " / "
                <b>{combo_l}</b>
            </td>
            <td>
                <span>{i.matched_pizza_name}</span>
                <br/>
                <span class="muted small">
                    {format_eur(i.matched_small_cents)} " / " {matched_l}
                </span>
            </td>
            <td class="diff">
                {format_eur_signed(i.inversion_small_cents)}
                " / "
                {inv_l}
            </td>
            <td>
                {match (i.suggested_named_small_cents, i.suggested_named_large_cents) {
                    (Some(s), Some(l)) => view! {
                        <span>"klein → " <b>{format_eur(s)}</b></span>
                        <br/>
                        <span>"groß → " <b>{format_eur(l)}</b></span>
                    }.into_any(),
                    (Some(s), None) => view! {
                        <span>"klein → " <b>{format_eur(s)}</b></span>
                    }.into_any(),
                    (None, Some(l)) => view! {
                        <span>"groß → " <b>{format_eur(l)}</b></span>
                    }.into_any(),
                    (None, None) => view! { <span class="muted">"—"</span> }.into_any(),
                }}
            </td>
        </tr>
    }
}

/// Like `format_eur` but adds an explicit leading "+" for positive numbers.
/// Used in the inversion column so the admin can tell margin from inversion
/// at a glance.
fn format_eur_signed(c: i64) -> String {
    if c > 0 {
        format!("+{}", format_eur(c))
    } else {
        format_eur(c)
    }
}
