//! /admin/vouchers — list + create + toggle (active) + delete.
//!
//! Three voucher kinds: percent (1-100), fixed (cents), free_delivery
//! (waives the delivery fee). All four validity dimensions are
//! editable: min_subtotal, first_order_only, per_phone_cap,
//! global_cap, valid_from / valid_until. The customer-facing redeem
//! logic lives in `pages::vouchers`; this page is purely CRUD.

use leptos::prelude::*;
use rusterando_shared::models::format_eur;
use serde::{Deserialize, Serialize};

use crate::pages::admin::shell::AdminShell;
#[cfg(feature = "ssr")]
use crate::pages::vouchers::{normalise_code, VoucherKind};

// ---------------------------------------------------------------------------
// DTOs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VoucherAdminRow {
    pub id: String,
    pub code: String,
    pub kind: String,
    pub percent_off: Option<i64>,
    pub amount_off_cents: Option<i64>,
    pub min_subtotal_cents: i64,
    pub first_order_only: bool,
    pub per_phone_cap: i64,
    pub global_cap: i64,
    pub valid_from: Option<String>,
    pub valid_until: Option<String>,
    pub label: Option<String>,
    pub active: bool,
    pub redemption_count: i64,
    pub redeemed_total_cents: i64,
    /// Bound phone (normalised). None = anonymous code.
    pub customer_phone: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateVoucherForm {
    pub code: String,
    pub kind: String,
    pub percent_off: Option<i64>,
    pub amount_off_cents: Option<i64>,
    pub min_subtotal_cents: i64,
    pub first_order_only: bool,
    pub per_phone_cap: i64,
    pub global_cap: i64,
    pub valid_from: Option<String>,
    pub valid_until: Option<String>,
    pub label: Option<String>,
    /// Raw (unnormalised) customer phone; empty = anonymous.
    pub customer_phone: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateVoucherForm {
    pub id: String,
    pub kind: String,
    pub percent_off: Option<i64>,
    pub amount_off_cents: Option<i64>,
    pub min_subtotal_cents: i64,
    pub first_order_only: bool,
    pub per_phone_cap: i64,
    pub global_cap: i64,
    pub valid_from: Option<String>,
    pub valid_until: Option<String>,
    pub label: Option<String>,
    pub customer_phone: Option<String>,
}

// ---------------------------------------------------------------------------
// Server fns
// ---------------------------------------------------------------------------

#[server(
    name = ListAdminVouchers,
    prefix = "/api",
    endpoint = "list_admin_vouchers"
)]
pub async fn list_admin_vouchers() -> Result<Vec<VoucherAdminRow>, ServerFnError> {
    use sqlx::{Row, SqlitePool};

    crate::pages::admin::require_admin().await?;

    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;

    let rows = sqlx::query(
        "SELECT v.id, v.code, v.kind, v.percent_off, v.amount_off_cents,
                v.min_subtotal_cents, v.first_order_only,
                v.per_phone_cap, v.global_cap,
                v.valid_from, v.valid_until, v.label, v.active,
                v.customer_phone,
                COALESCE(r.cnt, 0)   AS redemption_count,
                COALESCE(r.total, 0) AS redeemed_total_cents
         FROM vouchers v
         LEFT JOIN (
            SELECT voucher_id,
                   COUNT(*)              AS cnt,
                   SUM(discount_cents)   AS total
            FROM voucher_redemptions
            GROUP BY voucher_id
         ) r ON r.voucher_id = v.id
         ORDER BY v.active DESC, v.created_at DESC",
    )
    .fetch_all(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("load vouchers: {e}")))?;

    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        out.push(VoucherAdminRow {
            id: r.get("id"),
            code: r.get("code"),
            kind: r.get("kind"),
            percent_off: r.get("percent_off"),
            amount_off_cents: r.get("amount_off_cents"),
            min_subtotal_cents: r.get("min_subtotal_cents"),
            first_order_only: r.get::<i64, _>("first_order_only") != 0,
            per_phone_cap: r.get("per_phone_cap"),
            global_cap: r.get("global_cap"),
            valid_from: r.get("valid_from"),
            valid_until: r.get("valid_until"),
            label: r.get("label"),
            active: r.get::<i64, _>("active") != 0,
            customer_phone: r.get("customer_phone"),
            redemption_count: r.get("redemption_count"),
            redeemed_total_cents: r.get("redeemed_total_cents"),
        });
    }
    Ok(out)
}

#[server(
    name = CreateVoucher,
    prefix = "/api",
    endpoint = "create_voucher"
)]
pub async fn create_voucher(form: CreateVoucherForm) -> Result<String, ServerFnError> {
    use sqlx::SqlitePool;

    crate::pages::admin::require_admin().await?;

    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;

    let code = normalise_code(&form.code);
    if code.is_empty() {
        return Err(ServerFnError::new("Code darf nicht leer sein"));
    }
    let kind = VoucherKind::parse(&form.kind)
        .ok_or_else(|| ServerFnError::new("Unbekannter Voucher-Typ"))?;

    // Validate per-kind numeric inputs.
    let (percent, amount) = match kind {
        VoucherKind::Percent => {
            let p = form.percent_off.unwrap_or(0);
            if !(1..=100).contains(&p) {
                return Err(ServerFnError::new(
                    "Prozent muss zwischen 1 und 100 liegen",
                ));
            }
            (Some(p), None)
        }
        VoucherKind::Fixed => {
            let c = form.amount_off_cents.unwrap_or(0);
            if c <= 0 {
                return Err(ServerFnError::new(
                    "Fester Rabatt muss > 0 Cent sein",
                ));
            }
            (None, Some(c))
        }
        VoucherKind::FreeDelivery => (None, None),
    };

    // Normalise the bound phone (or treat as anonymous if empty).
    // Stored normalised so the redeem path's lookup hits exact equality
    // against the customer's normalised phone.
    let phone_norm = form
        .customer_phone
        .as_deref()
        .map(crate::pages::order::ssr::normalize_phone)
        .filter(|s| !s.is_empty());

    let id = uuid::Uuid::new_v4().to_string();
    let res = sqlx::query(
        "INSERT INTO vouchers
           (id, code, kind, percent_off, amount_off_cents,
            min_subtotal_cents, first_order_only,
            per_phone_cap, global_cap, valid_from, valid_until,
            label, customer_phone, active)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, 1)",
    )
    .bind(&id)
    .bind(&code)
    .bind(kind.as_str())
    .bind(percent)
    .bind(amount)
    .bind(form.min_subtotal_cents.max(0))
    .bind(if form.first_order_only { 1 } else { 0 })
    .bind(form.per_phone_cap.max(0))
    .bind(form.global_cap.max(0))
    .bind(form.valid_from.as_deref().filter(|s| !s.is_empty()))
    .bind(form.valid_until.as_deref().filter(|s| !s.is_empty()))
    .bind(form.label.as_deref().filter(|s| !s.is_empty()))
    .bind(phone_norm.as_deref())
    .execute(&db)
    .await;

    match res {
        Ok(_) => Ok(id),
        Err(sqlx::Error::Database(db_err)) if db_err.message().contains("UNIQUE") => Err(
            ServerFnError::new(format!("Code '{code}' existiert bereits.")),
        ),
        Err(e) => Err(ServerFnError::new(format!("INSERT voucher: {e}"))),
    }
}

#[server(
    name = UpdateVoucher,
    prefix = "/api",
    endpoint = "update_voucher"
)]
pub async fn update_voucher(form: UpdateVoucherForm) -> Result<(), ServerFnError> {
    use sqlx::SqlitePool;

    crate::pages::admin::require_admin().await?;

    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;

    let kind = VoucherKind::parse(&form.kind)
        .ok_or_else(|| ServerFnError::new("Unbekannter Voucher-Typ"))?;
    let (percent, amount) = match kind {
        VoucherKind::Percent => {
            let p = form.percent_off.unwrap_or(0);
            if !(1..=100).contains(&p) {
                return Err(ServerFnError::new(
                    "Prozent muss zwischen 1 und 100 liegen",
                ));
            }
            (Some(p), None)
        }
        VoucherKind::Fixed => {
            let c = form.amount_off_cents.unwrap_or(0);
            if c <= 0 {
                return Err(ServerFnError::new(
                    "Fester Rabatt muss > 0 Cent sein",
                ));
            }
            (None, Some(c))
        }
        VoucherKind::FreeDelivery => (None, None),
    };
    let phone_norm = form
        .customer_phone
        .as_deref()
        .map(crate::pages::order::ssr::normalize_phone)
        .filter(|s| !s.is_empty());

    // Note: code is NOT updatable. Customers may have it saved /
    // bookmarked / mailed; renaming would silently break their link.
    // Toggle/Delete cover the "make this code go away" flow.
    sqlx::query(
        "UPDATE vouchers
            SET kind = ?2, percent_off = ?3, amount_off_cents = ?4,
                min_subtotal_cents = ?5, first_order_only = ?6,
                per_phone_cap = ?7, global_cap = ?8,
                valid_from = ?9, valid_until = ?10, label = ?11,
                customer_phone = ?12,
                updated_at = CURRENT_TIMESTAMP
          WHERE id = ?1",
    )
    .bind(&form.id)
    .bind(kind.as_str())
    .bind(percent)
    .bind(amount)
    .bind(form.min_subtotal_cents.max(0))
    .bind(if form.first_order_only { 1 } else { 0 })
    .bind(form.per_phone_cap.max(0))
    .bind(form.global_cap.max(0))
    .bind(form.valid_from.as_deref().filter(|s| !s.is_empty()))
    .bind(form.valid_until.as_deref().filter(|s| !s.is_empty()))
    .bind(form.label.as_deref().filter(|s| !s.is_empty()))
    .bind(phone_norm.as_deref())
    .execute(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("UPDATE voucher: {e}")))?;
    Ok(())
}

#[server(
    name = ToggleVoucher,
    prefix = "/api",
    endpoint = "toggle_voucher"
)]
pub async fn toggle_voucher(id: String, active: bool) -> Result<(), ServerFnError> {
    use sqlx::SqlitePool;

    crate::pages::admin::require_admin().await?;

    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;

    sqlx::query(
        "UPDATE vouchers SET active = ?2, updated_at = CURRENT_TIMESTAMP WHERE id = ?1",
    )
    .bind(&id)
    .bind(if active { 1 } else { 0 })
    .execute(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("toggle voucher: {e}")))?;
    Ok(())
}

#[server(
    name = DeleteVoucher,
    prefix = "/api",
    endpoint = "delete_voucher"
)]
pub async fn delete_voucher(id: String) -> Result<(), ServerFnError> {
    use sqlx::SqlitePool;

    crate::pages::admin::require_admin().await?;

    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;

    // Reject delete if redemptions exist; admin should toggle off instead.
    let used: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM voucher_redemptions WHERE voucher_id = ?1",
    )
    .bind(&id)
    .fetch_one(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("count redemptions: {e}")))?;
    if used.0 > 0 {
        return Err(ServerFnError::new(
            "Code wurde bereits eingelöst — bitte deaktivieren statt löschen.",
        ));
    }

    sqlx::query("DELETE FROM vouchers WHERE id = ?1")
        .bind(&id)
        .execute(&db)
        .await
        .map_err(|e| ServerFnError::new(format!("delete voucher: {e}")))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// UI
// ---------------------------------------------------------------------------

#[component]
pub fn VouchersAdminPage() -> impl IntoView {
    let creator = ServerAction::<CreateVoucher>::new();
    let updater = ServerAction::<UpdateVoucher>::new();
    let toggler = ServerAction::<ToggleVoucher>::new();
    let deleter = ServerAction::<DeleteVoucher>::new();

    let list = Resource::new(
        move || {
            (
                creator.version().get(),
                updater.version().get(),
                toggler.version().get(),
                deleter.version().get(),
            )
        },
        |_| async move { list_admin_vouchers().await },
    );

    view! {
        <AdminShell>
            <section class="admin-vouchers">
                <header class="admin-bar">
                    <h1>"Gutscheine"</h1>
                </header>
                <p class="hint">
                    "Codes für % oder € Rabatt oder Gratis-Lieferung. "
                    "Kunden tippen den Code an der Kasse ein. "
                    "Auch via URL möglich: "<code>"/checkout?code=START10"</code>
                </p>

                <CreateVoucherCard creator/>

                <Suspense fallback=|| view! { <p class="loading">"Lädt…"</p> }>
                    {move || list.get().map(|res| match res {
                        Err(e) => view! { <p class="error">{format!("Fehler: {e}")}</p> }.into_any(),
                        Ok(rows) => view! { <VoucherTable rows updater toggler deleter/> }.into_any(),
                    })}
                </Suspense>
            </section>
        </AdminShell>
    }
}

#[component]
fn CreateVoucherCard(creator: ServerAction<CreateVoucher>) -> impl IntoView {
    let code = RwSignal::new(String::new());
    let kind = RwSignal::new("percent".to_string());
    let percent = RwSignal::new("10".to_string());
    let amount_eur = RwSignal::new("5".to_string());
    let min_subtotal_eur = RwSignal::new("0".to_string());
    let first_order_only = RwSignal::new(false);
    let per_phone_cap = RwSignal::new("1".to_string());
    let global_cap = RwSignal::new("0".to_string());
    let valid_from = RwSignal::new(String::new());
    let valid_until = RwSignal::new(String::new());
    let label = RwSignal::new(String::new());

    // Auto-fill from ?phone=… so /admin/customers/:id can deep-link
    // into the create card with a pre-bound phone. Use leptos_router's
    // reactive query map so CSR navigation (where window.location is
    // updated but the component isn't re-mounted on first hydrate)
    // still picks up the param.
    //
    // Seed the signal synchronously from the query at construction
    // time so SSR + the very first hydrate paint render the input
    // with the correct value (no SSR/CSR DOM mismatch — see
    // memory/feedback_hydration_pattern.md).
    let query = leptos_router::hooks::use_query_map();
    let initial_phone = query
        .read_untracked()
        .get("phone")
        .map(|s| s.to_string())
        .unwrap_or_default();
    let customer_phone = RwSignal::new(initial_phone);
    // Keep the field in sync if the URL changes via CSR navigation
    // (e.g. admin clicks a different "Code für …"-link without a
    // page reload).
    Effect::new(move |_| {
        let q = query.get();
        if let Some(p) = q.get("phone") {
            let p = p.to_string();
            if !p.is_empty() && customer_phone.get_untracked() != p {
                customer_phone.set(p);
            }
        }
    });

    let on_submit = move |_| {
        let parse_int = |s: String| s.trim().parse::<i64>().ok();
        let parse_eur = |s: String| -> Option<i64> {
            let trimmed = s.trim().replace(',', ".");
            trimmed.parse::<f64>().ok().map(|f| (f * 100.0).round() as i64)
        };

        let form = CreateVoucherForm {
            code: code.get(),
            kind: kind.get(),
            percent_off: parse_int(percent.get()),
            amount_off_cents: parse_eur(amount_eur.get()),
            min_subtotal_cents: parse_eur(min_subtotal_eur.get()).unwrap_or(0),
            first_order_only: first_order_only.get(),
            per_phone_cap: parse_int(per_phone_cap.get()).unwrap_or(0),
            global_cap: parse_int(global_cap.get()).unwrap_or(0),
            valid_from: Some(valid_from.get()).filter(|s| !s.is_empty()),
            valid_until: Some(valid_until.get()).filter(|s| !s.is_empty()),
            label: Some(label.get()).filter(|s| !s.is_empty()),
            customer_phone: Some(customer_phone.get()).filter(|s| !s.trim().is_empty()),
        };

        creator.dispatch(CreateVoucher { form });

        // Reset code + label after submit; keep other defaults so an
        // admin can stamp out multiple similar codes in a row.
        code.set(String::new());
        label.set(String::new());
    };

    view! {
        <details class="card create-voucher" open=true>
            <summary><strong>"Neuen Gutschein anlegen"</strong></summary>

            <div class="grid">
                <label>
                    <span>"Code"</span>
                    <input type="text" placeholder="WILLKOMMEN10"
                        prop:value=move || code.get()
                        on:input=move |ev| code.set(event_target_value(&ev))/>
                </label>

                <label>
                    <span>"Typ"</span>
                    <select prop:value=move || kind.get()
                            on:change=move |ev| kind.set(event_target_value(&ev))>
                        <option value="percent">"% Rabatt auf Subtotal"</option>
                        <option value="fixed">"Fester € Rabatt"</option>
                        <option value="free_delivery">"Gratis-Lieferung"</option>
                    </select>
                </label>

                {move || (kind.get() == "percent").then(|| view! {
                    <label>
                        <span>"Prozent (1-100)"</span>
                        <input type="number" min="1" max="100"
                            prop:value=move || percent.get()
                            on:input=move |ev| percent.set(event_target_value(&ev))/>
                    </label>
                })}

                {move || (kind.get() == "fixed").then(|| view! {
                    <label>
                        <span>"Rabatt in €"</span>
                        <input type="text" inputmode="decimal" placeholder="5"
                            prop:value=move || amount_eur.get()
                            on:input=move |ev| amount_eur.set(event_target_value(&ev))/>
                    </label>
                })}

                <label>
                    <span>"Mindestbestellwert in €"</span>
                    <input type="text" inputmode="decimal" placeholder="0"
                        prop:value=move || min_subtotal_eur.get()
                        on:input=move |ev| min_subtotal_eur.set(event_target_value(&ev))/>
                </label>

                <label class="checkbox">
                    <input type="checkbox"
                        prop:checked=move || first_order_only.get()
                        on:change=move |ev| first_order_only.set(event_target_checked(&ev))/>
                    <span>"Nur Erstbesteller (Lieferando-Killer)"</span>
                </label>

                <label>
                    <span>"Max. Einlösungen pro Kunde (0 = unlimitiert)"</span>
                    <input type="number" min="0"
                        prop:value=move || per_phone_cap.get()
                        on:input=move |ev| per_phone_cap.set(event_target_value(&ev))/>
                </label>

                <label>
                    <span>"Gesamt-Limit (0 = unlimitiert)"</span>
                    <input type="number" min="0"
                        prop:value=move || global_cap.get()
                        on:input=move |ev| global_cap.set(event_target_value(&ev))/>
                </label>

                <label>
                    <span>"Gültig ab (optional, YYYY-MM-DD HH:MM:SS)"</span>
                    <input type="text" placeholder="2026-06-01 00:00:00"
                        prop:value=move || valid_from.get()
                        on:input=move |ev| valid_from.set(event_target_value(&ev))/>
                </label>

                <label>
                    <span>"Gültig bis (optional, YYYY-MM-DD HH:MM:SS)"</span>
                    <input type="text" placeholder="2026-12-31 23:59:59"
                        prop:value=move || valid_until.get()
                        on:input=move |ev| valid_until.set(event_target_value(&ev))/>
                </label>

                <label class="wide">
                    <span>"Label (optional, intern + Beleg)"</span>
                    <input type="text" placeholder="Sommer-Aktion 2026"
                        prop:value=move || label.get()
                        on:input=move |ev| label.set(event_target_value(&ev))/>
                </label>

                <label class="wide">
                    <span>"Telefon-Bindung (optional — nur diese Nummer kann einlösen)"</span>
                    <input type="tel" placeholder="z.B. 0151 1234567"
                        prop:value=move || customer_phone.get()
                        on:input=move |ev| customer_phone.set(event_target_value(&ev))/>
                </label>
            </div>

            <div class="actions">
                <button class="btn primary" on:click=on_submit>"Anlegen"</button>
                {move || creator.value().get().and_then(|res| match res {
                    Ok(_) => Some(view! { <span class="ok">"✓ Gespeichert"</span> }.into_any()),
                    Err(e) => Some(view! { <span class="error">{format!("Fehler: {e}")}</span> }.into_any()),
                })}
            </div>
        </details>
    }
}

#[component]
fn VoucherTable(
    rows: Vec<VoucherAdminRow>,
    updater: ServerAction<UpdateVoucher>,
    toggler: ServerAction<ToggleVoucher>,
    deleter: ServerAction<DeleteVoucher>,
) -> impl IntoView {
    if rows.is_empty() {
        return view! { <p class="empty">"Noch keine Gutscheine angelegt."</p> }.into_any();
    }
    view! {
        <table class="admin-vouchers-table">
            <thead>
                <tr>
                    <th>"Code"</th>
                    <th>"Typ"</th>
                    <th class="num">"Min."</th>
                    <th>"Regeln"</th>
                    <th>"Telefon"</th>
                    <th>"Gültig"</th>
                    <th class="num">"Genutzt"</th>
                    <th class="num">"Rabatt-Summe"</th>
                    <th></th>
                </tr>
            </thead>
            <tbody>
                {rows.into_iter().map(|r| view! { <VoucherRowView r updater toggler deleter/> }).collect_view()}
            </tbody>
        </table>
    }
    .into_any()
}

#[component]
fn VoucherRowView(
    r: VoucherAdminRow,
    updater: ServerAction<UpdateVoucher>,
    toggler: ServerAction<ToggleVoucher>,
    deleter: ServerAction<DeleteVoucher>,
) -> impl IntoView {
    let type_label = match r.kind.as_str() {
        "percent" => format!("{}%", r.percent_off.unwrap_or(0)),
        "fixed" => format!("{} fest", format_eur(r.amount_off_cents.unwrap_or(0))),
        "free_delivery" => "Gratis-Lieferung".to_string(),
        other => other.to_string(),
    };
    let mut rules: Vec<String> = Vec::new();
    if r.first_order_only {
        rules.push("Erstbesteller".to_string());
    }
    if r.per_phone_cap > 0 {
        rules.push(format!("{}× / Kunde", r.per_phone_cap));
    }
    if r.global_cap > 0 {
        rules.push(format!("max. {}× global", r.global_cap));
    }
    if rules.is_empty() {
        rules.push("—".to_string());
    }
    let rules_joined = rules.join(", ");

    let phone_label = r
        .customer_phone
        .clone()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "—".to_string());

    let valid_span = match (r.valid_from.as_deref(), r.valid_until.as_deref()) {
        (None, None) => "—".to_string(),
        (Some(f), None) => format!("ab {f}"),
        (None, Some(u)) => format!("bis {u}"),
        (Some(f), Some(u)) => format!("{f} → {u}"),
    };

    let id_toggle = r.id.clone();
    let id_delete = r.id.clone();
    let active_now = r.active;
    let on_toggle = move |_| {
        toggler.dispatch(ToggleVoucher {
            id: id_toggle.clone(),
            active: !active_now,
        });
    };
    let on_delete = move |_| {
        deleter.dispatch(DeleteVoucher {
            id: id_delete.clone(),
        });
    };

    // Open/close the inline edit panel.
    let editing = RwSignal::new(false);
    let on_edit = move |_| editing.update(|b| *b = !*b);

    let row_class = if r.active { "active" } else { "inactive" };
    let r_for_edit = r.clone();
    view! {
        <tr class=row_class>
            <td>
                <strong>{r.code.clone()}</strong>
                {r.label.clone().map(|l| view! { <p class="muted">{l}</p> })}
            </td>
            <td>{type_label}</td>
            <td class="num">{if r.min_subtotal_cents > 0 { format_eur(r.min_subtotal_cents) } else { "—".to_string() }}</td>
            <td><span class="muted">{rules_joined}</span></td>
            <td><span class="muted">{phone_label}</span></td>
            <td><span class="muted">{valid_span}</span></td>
            <td class="num">{r.redemption_count}</td>
            <td class="num">{format_eur(r.redeemed_total_cents)}</td>
            <td class="actions">
                <button class="btn small" on:click=on_edit>
                    {move || if editing.get() { "Schließen" } else { "Bearbeiten" }}
                </button>
                <button class="btn small" on:click=on_toggle>
                    {if r.active { "Deaktivieren" } else { "Aktivieren" }}
                </button>
                <button class="btn small danger" on:click=on_delete>"Löschen"</button>
            </td>
        </tr>
        {move || editing.get().then(|| view! {
            <tr class="edit-row">
                <td colspan="9">
                    <VoucherEditPanel r=r_for_edit.clone() updater editing/>
                </td>
            </tr>
        })}
    }
}

#[component]
fn VoucherEditPanel(
    r: VoucherAdminRow,
    updater: ServerAction<UpdateVoucher>,
    editing: RwSignal<bool>,
) -> impl IntoView {
    // Pre-fill every signal from the existing row. Code is read-only —
    // customers may have it saved; renaming silently would break links.
    let id = r.id.clone();
    let kind = RwSignal::new(r.kind.clone());
    let percent = RwSignal::new(r.percent_off.map(|p| p.to_string()).unwrap_or_default());
    let amount_eur = RwSignal::new(
        r.amount_off_cents
            .map(|c| format!("{:.2}", c as f64 / 100.0))
            .unwrap_or_default(),
    );
    let min_subtotal_eur = RwSignal::new(format!("{:.2}", r.min_subtotal_cents as f64 / 100.0));
    let first_order_only = RwSignal::new(r.first_order_only);
    let per_phone_cap = RwSignal::new(r.per_phone_cap.to_string());
    let global_cap = RwSignal::new(r.global_cap.to_string());
    let valid_from = RwSignal::new(r.valid_from.clone().unwrap_or_default());
    let valid_until = RwSignal::new(r.valid_until.clone().unwrap_or_default());
    let label = RwSignal::new(r.label.clone().unwrap_or_default());
    let customer_phone = RwSignal::new(r.customer_phone.clone().unwrap_or_default());

    let on_save = move |_| {
        let parse_int = |s: String| s.trim().parse::<i64>().ok();
        let parse_eur = |s: String| -> Option<i64> {
            let trimmed = s.trim().replace(',', ".");
            trimmed
                .parse::<f64>()
                .ok()
                .map(|f| (f * 100.0).round() as i64)
        };
        updater.dispatch(UpdateVoucher {
            form: UpdateVoucherForm {
                id: id.clone(),
                kind: kind.get(),
                percent_off: parse_int(percent.get()),
                amount_off_cents: parse_eur(amount_eur.get()),
                min_subtotal_cents: parse_eur(min_subtotal_eur.get()).unwrap_or(0),
                first_order_only: first_order_only.get(),
                per_phone_cap: parse_int(per_phone_cap.get()).unwrap_or(0),
                global_cap: parse_int(global_cap.get()).unwrap_or(0),
                valid_from: Some(valid_from.get()).filter(|s| !s.is_empty()),
                valid_until: Some(valid_until.get()).filter(|s| !s.is_empty()),
                label: Some(label.get()).filter(|s| !s.is_empty()),
                customer_phone: Some(customer_phone.get()).filter(|s| !s.trim().is_empty()),
            },
        });
        editing.set(false);
    };

    view! {
        <div class="voucher-edit-panel">
            <div class="grid">
                <label>
                    <span>"Typ"</span>
                    <select prop:value=move || kind.get()
                            on:change=move |ev| kind.set(event_target_value(&ev))>
                        <option value="percent">"% Rabatt"</option>
                        <option value="fixed">"Fester € Rabatt"</option>
                        <option value="free_delivery">"Gratis-Lieferung"</option>
                    </select>
                </label>

                {move || (kind.get() == "percent").then(|| view! {
                    <label>
                        <span>"Prozent (1-100)"</span>
                        <input type="number" min="1" max="100"
                            prop:value=move || percent.get()
                            on:input=move |ev| percent.set(event_target_value(&ev))/>
                    </label>
                })}

                {move || (kind.get() == "fixed").then(|| view! {
                    <label>
                        <span>"Rabatt in €"</span>
                        <input type="text" inputmode="decimal"
                            prop:value=move || amount_eur.get()
                            on:input=move |ev| amount_eur.set(event_target_value(&ev))/>
                    </label>
                })}

                <label>
                    <span>"Mindestbestellwert in €"</span>
                    <input type="text" inputmode="decimal"
                        prop:value=move || min_subtotal_eur.get()
                        on:input=move |ev| min_subtotal_eur.set(event_target_value(&ev))/>
                </label>

                <label class="checkbox">
                    <input type="checkbox"
                        prop:checked=move || first_order_only.get()
                        on:change=move |ev| first_order_only.set(event_target_checked(&ev))/>
                    <span>"Nur Erstbesteller"</span>
                </label>

                <label>
                    <span>"Pro Kunde max."</span>
                    <input type="number" min="0"
                        prop:value=move || per_phone_cap.get()
                        on:input=move |ev| per_phone_cap.set(event_target_value(&ev))/>
                </label>

                <label>
                    <span>"Gesamt-Limit"</span>
                    <input type="number" min="0"
                        prop:value=move || global_cap.get()
                        on:input=move |ev| global_cap.set(event_target_value(&ev))/>
                </label>

                <label>
                    <span>"Gültig ab"</span>
                    <input type="text" placeholder="2026-06-01 00:00:00"
                        prop:value=move || valid_from.get()
                        on:input=move |ev| valid_from.set(event_target_value(&ev))/>
                </label>

                <label>
                    <span>"Gültig bis"</span>
                    <input type="text" placeholder="2026-12-31 23:59:59"
                        prop:value=move || valid_until.get()
                        on:input=move |ev| valid_until.set(event_target_value(&ev))/>
                </label>

                <label class="wide">
                    <span>"Label (intern + Beleg)"</span>
                    <input type="text"
                        prop:value=move || label.get()
                        on:input=move |ev| label.set(event_target_value(&ev))/>
                </label>

                <label class="wide">
                    <span>"Telefon-Bindung (leer = anonym, jeder kann einlösen)"</span>
                    <input type="tel"
                        prop:value=move || customer_phone.get()
                        on:input=move |ev| customer_phone.set(event_target_value(&ev))/>
                </label>
            </div>
            <div class="actions">
                <button class="btn primary small" on:click=on_save>"Speichern"</button>
                <button class="btn small" on:click=move |_| editing.set(false)>"Abbrechen"</button>
                {move || updater.value().get().and_then(|res| match res {
                    Err(e) => Some(view! { <span class="error">{format!("Fehler: {e}")}</span> }.into_any()),
                    _ => None,
                })}
            </div>
        </div>
    }
}

