//! /admin/customers — list + detail + light edit.
//!
//! Reads from the `customers` and `customer_addresses` tables (which
//! exist since the phase-1 recall migration; see
//! `migrations/20260508000007_phase1_customer_recall.sql`). Aggregates
//! order count + total revenue per customer via a single LEFT JOIN
//! at list time.
//!
//! Blacklist is a thin flag with consequences in `place_order`: an
//! order with `payment_method == 'cash'` is rejected for any
//! customer whose phone matches a row with `blacklisted_at IS NOT
//! NULL`. Online (card) orders pass through unchanged — Stripe has
//! already cleared the charge before the kitchen sees it.

use leptos::prelude::*;
use rusterando_shared::models::format_eur;
use serde::{Deserialize, Serialize};

use crate::pages::admin::shell::AdminShell;

// ---------------------------------------------------------------------------
// DTOs
// ---------------------------------------------------------------------------

/// One row in the customer list. `total_cents` and `order_count` are
/// aggregates over the `orders` table. `last_order_at_unix` is also
/// from orders so a customer whose recall-row predates their first
/// order still sorts plausibly.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CustomerRow {
    pub id: String,
    pub phone: String,
    pub name: Option<String>,
    pub email: Option<String>,
    pub notes: Option<String>,
    pub blacklisted: bool,
    pub order_count: i64,
    pub total_cents: i64,
    /// Unix seconds; 0 means "never ordered yet" (signed-up via
    /// place_order's customer-upsert with no completed order).
    pub last_order_at_unix: i64,
    /// Unix seconds; from `customers.last_seen_at`, which the
    /// recall path bumps on every checkout.
    pub last_seen_at_unix: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomerDetail {
    pub row: CustomerRow,
    pub addresses: Vec<CustomerAddressRow>,
    pub orders: Vec<CustomerOrderRow>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomerAddressRow {
    pub id: String,
    pub street: String,
    pub house_number: String,
    pub postcode: String,
    pub city: String,
    pub notes: Option<String>,
    pub successful_deliveries: i64,
    pub failed_deliveries: i64,
    pub last_used_at_unix: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomerOrderRow {
    pub id: String,
    pub order_number: String,
    pub status: String,
    pub order_type: String,
    pub payment_method: Option<String>,
    pub payment_status: String,
    pub total_cents: i64,
    pub created_at_unix: i64,
}

// ---------------------------------------------------------------------------
// Server fns
// ---------------------------------------------------------------------------

/// All customers + aggregates. Sorted server-side by last_seen DESC
/// for a sensible default ordering; the client component re-sorts
/// in-place when the admin clicks a column header.
#[server(
    name = ListAdminCustomers,
    prefix = "/api",
    endpoint = "list_admin_customers"
)]
pub async fn list_admin_customers() -> Result<Vec<CustomerRow>, ServerFnError> {
    use sqlx::{Row, SqlitePool};

    crate::pages::admin::require_admin().await?;

    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;

    let rows = sqlx::query(
        "SELECT
            c.id,
            c.phone,
            c.name,
            c.email,
            c.notes,
            CASE WHEN c.blacklisted_at IS NULL THEN 0 ELSE 1 END AS blacklisted,
            COALESCE(o.order_count, 0)              AS order_count,
            COALESCE(o.total_cents, 0)              AS total_cents,
            COALESCE(o.last_order_at_unix, 0)       AS last_order_at_unix,
            CAST(strftime('%s', c.last_seen_at) AS INTEGER) AS last_seen_at_unix
         FROM customers c
         LEFT JOIN (
            SELECT
                customer_id,
                COUNT(*)                              AS order_count,
                SUM(total_cents)                      AS total_cents,
                MAX(CAST(strftime('%s', created_at) AS INTEGER)) AS last_order_at_unix
            FROM orders
            WHERE customer_id IS NOT NULL
              AND status <> 'cancelled'
            GROUP BY customer_id
         ) o ON o.customer_id = c.id
         ORDER BY last_seen_at_unix DESC
         LIMIT 5000",
    )
    .fetch_all(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("load customers: {e}")))?;

    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        out.push(CustomerRow {
            id: r.get("id"),
            phone: r.get("phone"),
            name: r.get("name"),
            email: r.get("email"),
            notes: r.get("notes"),
            blacklisted: r.get::<i64, _>("blacklisted") != 0,
            order_count: r.get("order_count"),
            total_cents: r.get("total_cents"),
            last_order_at_unix: r.get("last_order_at_unix"),
            last_seen_at_unix: r.get("last_seen_at_unix"),
        });
    }
    Ok(out)
}

#[server(
    name = GetAdminCustomer,
    prefix = "/api",
    endpoint = "get_admin_customer"
)]
pub async fn get_admin_customer(id: String) -> Result<CustomerDetail, ServerFnError> {
    use sqlx::{Row, SqlitePool};

    crate::pages::admin::require_admin().await?;

    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;

    let c = sqlx::query(
        "SELECT
            c.id, c.phone, c.name, c.email, c.notes,
            CASE WHEN c.blacklisted_at IS NULL THEN 0 ELSE 1 END AS blacklisted,
            COALESCE(o.order_count, 0)              AS order_count,
            COALESCE(o.total_cents, 0)              AS total_cents,
            COALESCE(o.last_order_at_unix, 0)       AS last_order_at_unix,
            CAST(strftime('%s', c.last_seen_at) AS INTEGER) AS last_seen_at_unix
         FROM customers c
         LEFT JOIN (
            SELECT customer_id,
                   COUNT(*)                              AS order_count,
                   SUM(total_cents)                      AS total_cents,
                   MAX(CAST(strftime('%s', created_at) AS INTEGER)) AS last_order_at_unix
            FROM orders
            WHERE customer_id IS NOT NULL AND status <> 'cancelled'
            GROUP BY customer_id
         ) o ON o.customer_id = c.id
         WHERE c.id = ?1",
    )
    .bind(&id)
    .fetch_one(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("load customer: {e}")))?;

    let row = CustomerRow {
        id: c.get("id"),
        phone: c.get("phone"),
        name: c.get("name"),
        email: c.get("email"),
        notes: c.get("notes"),
        blacklisted: c.get::<i64, _>("blacklisted") != 0,
        order_count: c.get("order_count"),
        total_cents: c.get("total_cents"),
        last_order_at_unix: c.get("last_order_at_unix"),
        last_seen_at_unix: c.get("last_seen_at_unix"),
    };

    let addr_rows = sqlx::query(
        "SELECT id, street, house_number, postcode, city, notes,
                successful_deliveries, failed_deliveries,
                CAST(strftime('%s', last_used_at) AS INTEGER) AS last_used_at_unix
         FROM customer_addresses
         WHERE customer_id = ?1
         ORDER BY last_used_at DESC",
    )
    .bind(&id)
    .fetch_all(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("load addresses: {e}")))?;

    let addresses = addr_rows
        .into_iter()
        .map(|r| CustomerAddressRow {
            id: r.get("id"),
            street: r.get("street"),
            house_number: r.get("house_number"),
            postcode: r.get("postcode"),
            city: r.get("city"),
            notes: r.get("notes"),
            successful_deliveries: r.get("successful_deliveries"),
            failed_deliveries: r.get("failed_deliveries"),
            last_used_at_unix: r.get("last_used_at_unix"),
        })
        .collect();

    // Cap at the 50 most recent orders so the page stays responsive
    // for a customer with hundreds of orders. /admin/orders has the
    // full history for the few who need to dig deeper.
    let order_rows = sqlx::query(
        "SELECT id, order_number, status, order_type, payment_status,
                total_cents,
                CAST(strftime('%s', created_at) AS INTEGER) AS created_at_unix,
                payment_method_detail
         FROM orders
         WHERE customer_id = ?1
         ORDER BY created_at DESC
         LIMIT 50",
    )
    .bind(&id)
    .fetch_all(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("load orders: {e}")))?;

    let orders = order_rows
        .into_iter()
        .map(|r| CustomerOrderRow {
            id: r.get("id"),
            order_number: r.get("order_number"),
            status: r.get("status"),
            order_type: r.get("order_type"),
            payment_method: r.get("payment_method_detail"),
            payment_status: r.get("payment_status"),
            total_cents: r.get("total_cents"),
            created_at_unix: r.get("created_at_unix"),
        })
        .collect();

    Ok(CustomerDetail {
        row,
        addresses,
        orders,
    })
}

#[server(
    name = UpdateAdminCustomer,
    prefix = "/api",
    endpoint = "update_admin_customer"
)]
pub async fn update_admin_customer(
    id: String,
    name: String,
    email: String,
    notes: String,
) -> Result<(), ServerFnError> {
    use sqlx::SqlitePool;
    crate::pages::admin::require_admin().await?;

    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;

    // Empty string → NULL so the DB doesn't end up with rows full
    // of blank strings (which would, e.g., make `WHERE email = ''`
    // match every "no email given" customer).
    let opt = |s: String| {
        let trimmed = s.trim().to_string();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    };

    sqlx::query(
        "UPDATE customers
         SET name       = ?2,
             email      = ?3,
             notes      = ?4,
             updated_at = CURRENT_TIMESTAMP
         WHERE id = ?1",
    )
    .bind(&id)
    .bind(opt(name))
    .bind(opt(email))
    .bind(opt(notes))
    .execute(&db)
    .await
    .map_err(|e| ServerFnError::new(format!("update customer: {e}")))?;

    Ok(())
}

/// Toggle blacklist by passing the desired state. Idempotent — setting
/// the same state twice is a no-op. The server-side effect lives in
/// `place_order`, which checks `blacklisted_at` and refuses cash
/// orders for the customer's phone.
#[server(
    name = SetCustomerBlacklist,
    prefix = "/api",
    endpoint = "set_customer_blacklist"
)]
pub async fn set_customer_blacklist(id: String, blacklisted: bool) -> Result<(), ServerFnError> {
    use sqlx::SqlitePool;
    crate::pages::admin::require_admin().await?;

    let db = use_context::<SqlitePool>()
        .ok_or_else(|| ServerFnError::new("database pool missing from context"))?;

    if blacklisted {
        sqlx::query(
            "UPDATE customers
             SET blacklisted_at = CURRENT_TIMESTAMP,
                 updated_at     = CURRENT_TIMESTAMP
             WHERE id = ?1",
        )
        .bind(&id)
        .execute(&db)
        .await
        .map_err(|e| ServerFnError::new(format!("set blacklist: {e}")))?;
    } else {
        sqlx::query(
            "UPDATE customers
             SET blacklisted_at = NULL,
                 updated_at     = CURRENT_TIMESTAMP
             WHERE id = ?1",
        )
        .bind(&id)
        .execute(&db)
        .await
        .map_err(|e| ServerFnError::new(format!("clear blacklist: {e}")))?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Components
// ---------------------------------------------------------------------------

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
enum SortCol {
    LastSeen,
    Phone,
    Name,
    OrderCount,
    Total,
}

#[component]
pub fn AdminCustomersPage() -> impl IntoView {
    let updater = ServerAction::<UpdateAdminCustomer>::new();
    let blacklister = ServerAction::<SetCustomerBlacklist>::new();
    let customers = Resource::new(
        move || (updater.version().get(), blacklister.version().get()),
        |_| async move { list_admin_customers().await },
    );

    // Client-side state: search box + sort column + direction.
    let search = RwSignal::new(String::new());
    let sort_col = RwSignal::new(SortCol::LastSeen);
    let sort_desc = RwSignal::new(true);

    let click_header = move |col: SortCol| {
        // Same column → flip direction. Different column → set it
        // and default to DESC (most-of-most-first is what you want
        // for orders/total/last_seen; for Phone/Name it'll feel
        // backwards once but you flip back on the second click).
        if sort_col.get() == col {
            sort_desc.update(|d| *d = !*d);
        } else {
            sort_col.set(col);
            sort_desc.set(true);
        }
    };

    view! {
        <AdminShell>
            <section class="admin-customers">
                <div class="page-bar">
                    <h1>"Kunden"</h1>
                    <input
                        class="search"
                        type="search"
                        placeholder="Suche: Telefon, Name, E-Mail…"
                        prop:value=move || search.get()
                        on:input=move |ev| search.set(event_target_value(&ev))
                    />
                    <button class="btn ghost" on:click=move |_| customers.refetch()>
                        "Aktualisieren"
                    </button>
                </div>

                <Suspense fallback=|| view! { <p>"Lädt…"</p> }>
                    {move || customers.get().map(|res| match res {
                        Err(e) => view! { <p class="error">{format!("Fehler: {e}")}</p> }.into_any(),
                        Ok(rows) => view! {
                            <CustomerTable
                                rows
                                search
                                sort_col
                                sort_desc
                                click_header=Callback::new(click_header)
                                blacklister
                            />
                        }.into_any(),
                    })}
                </Suspense>
            </section>
        </AdminShell>
    }
}

#[component]
fn CustomerTable(
    rows: Vec<CustomerRow>,
    search: RwSignal<String>,
    sort_col: RwSignal<SortCol>,
    sort_desc: RwSignal<bool>,
    click_header: Callback<SortCol>,
    blacklister: ServerAction<SetCustomerBlacklist>,
) -> impl IntoView {
    let rows_sv = StoredValue::new(rows);

    let filtered = Memo::new(move |_| {
        let needle_raw = search.get();
        let needle = needle_raw.trim().to_lowercase();
        let col = sort_col.get();
        let desc = sort_desc.get();

        rows_sv.with_value(|rows| {
            let mut filtered: Vec<CustomerRow> = rows
                .iter()
                .filter(|r| {
                    if needle.is_empty() {
                        return true;
                    }
                    let phone_hit = r.phone.to_lowercase().contains(&needle);
                    let name_hit = r
                        .name
                        .as_deref()
                        .map(|s| s.to_lowercase().contains(&needle))
                        .unwrap_or(false);
                    let email_hit = r
                        .email
                        .as_deref()
                        .map(|s| s.to_lowercase().contains(&needle))
                        .unwrap_or(false);
                    phone_hit || name_hit || email_hit
                })
                .cloned()
                .collect();

            filtered.sort_by(|a, b| {
                let ord = match col {
                    SortCol::LastSeen => a.last_seen_at_unix.cmp(&b.last_seen_at_unix),
                    SortCol::Phone => a.phone.cmp(&b.phone),
                    SortCol::Name => a
                        .name
                        .as_deref()
                        .unwrap_or("")
                        .cmp(b.name.as_deref().unwrap_or("")),
                    SortCol::OrderCount => a.order_count.cmp(&b.order_count),
                    SortCol::Total => a.total_cents.cmp(&b.total_cents),
                };
                if desc {
                    ord.reverse()
                } else {
                    ord
                }
            });
            filtered
        })
    });

    let sort_marker = move |col: SortCol| {
        if sort_col.get() != col {
            ""
        } else if sort_desc.get() {
            " ▼"
        } else {
            " ▲"
        }
    };

    view! {
        <table class="admin-customers-table">
            <thead>
                <tr>
                    <th class="sortable" on:click=move |_| click_header.run(SortCol::Phone)>
                        "Telefon" {move || sort_marker(SortCol::Phone)}
                    </th>
                    <th class="sortable" on:click=move |_| click_header.run(SortCol::Name)>
                        "Name" {move || sort_marker(SortCol::Name)}
                    </th>
                    <th>"E-Mail"</th>
                    <th class="num sortable" on:click=move |_| click_header.run(SortCol::OrderCount)>
                        "#" {move || sort_marker(SortCol::OrderCount)}
                    </th>
                    <th class="num sortable" on:click=move |_| click_header.run(SortCol::Total)>
                        "Umsatz" {move || sort_marker(SortCol::Total)}
                    </th>
                    <th class="sortable" on:click=move |_| click_header.run(SortCol::LastSeen)>
                        "Zuletzt" {move || sort_marker(SortCol::LastSeen)}
                    </th>
                    <th></th>
                </tr>
            </thead>
            <tbody>
                {move || filtered.with(|rows| {
                    if rows.is_empty() {
                        return view! {
                            <tr><td colspan="7" class="empty">"Keine Kunden gefunden."</td></tr>
                        }.into_any();
                    }
                    rows.iter().map(|r| view! {
                        <CustomerRowView r=r.clone() blacklister/>
                    }).collect_view().into_any()
                })}
            </tbody>
        </table>
    }
}

#[component]
fn CustomerRowView(
    r: CustomerRow,
    blacklister: ServerAction<SetCustomerBlacklist>,
) -> impl IntoView {
    let detail_href = format!("/admin/customers/{}", r.id);
    let tel_href = format!("tel:{}", r.phone);
    let mailto_href = r.email.as_deref().map(|e| format!("mailto:{e}"));
    let id_for_toggle = r.id.clone();
    let blacklisted_now = r.blacklisted;
    let toggle = move |_| {
        blacklister.dispatch(SetCustomerBlacklist {
            id: id_for_toggle.clone(),
            blacklisted: !blacklisted_now,
        });
    };

    view! {
        <tr class:blacklisted=blacklisted_now>
            <td>
                <a href=tel_href.clone()>{r.phone.clone()}</a>
                {blacklisted_now.then(|| view! {
                    <span class="pill blacklist-pill">"BAR-SPERRE"</span>
                })}
            </td>
            <td>{r.name.clone().unwrap_or_default()}</td>
            <td>
                {match mailto_href {
                    Some(h) => view! {
                        <a href=h>{r.email.clone().unwrap_or_default()}</a>
                    }.into_any(),
                    None => view! { <span class="muted">"—"</span> }.into_any(),
                }}
            </td>
            <td class="num">{r.order_count}</td>
            <td class="num">{format_eur(r.total_cents)}</td>
            <td>{format_unix_short(r.last_seen_at_unix)}</td>
            <td class="row-actions">
                <a class="btn ghost small" href=detail_href>"Details"</a>
                <button class="btn ghost small" on:click=toggle>
                    {if blacklisted_now { "Sperre aufheben" } else { "Bar sperren" }}
                </button>
            </td>
        </tr>
    }
}

// ---------------------------------------------------------------------------
// /admin/customers/:id
// ---------------------------------------------------------------------------

#[component]
pub fn AdminCustomerDetailPage() -> impl IntoView {
    let params = leptos_router::hooks::use_params_map();
    let id = move || params.read().get("id").unwrap_or_default();

    let updater = ServerAction::<UpdateAdminCustomer>::new();
    let blacklister = ServerAction::<SetCustomerBlacklist>::new();

    let detail = Resource::new(
        move || (id(), updater.version().get(), blacklister.version().get()),
        |(id, _, _)| async move { get_admin_customer(id).await },
    );

    view! {
        <AdminShell>
            <section class="admin-customer-detail">
                <p class="back-link">
                    <a href="/admin/customers">"← Alle Kunden"</a>
                </p>
                <Suspense fallback=|| view! { <p>"Lädt…"</p> }>
                    {move || detail.get().map(|res| match res {
                        Err(e) => view! {
                            <p class="error">{format!("Fehler: {e}")}</p>
                        }.into_any(),
                        Ok(d) => view! {
                            <CustomerDetailView d updater blacklister/>
                        }.into_any(),
                    })}
                </Suspense>
            </section>
        </AdminShell>
    }
}

#[component]
fn CustomerDetailView(
    d: CustomerDetail,
    updater: ServerAction<UpdateAdminCustomer>,
    blacklister: ServerAction<SetCustomerBlacklist>,
) -> impl IntoView {
    let id_for_save = d.row.id.clone();
    let id_for_toggle = d.row.id.clone();
    let blacklisted_now = d.row.blacklisted;

    let name = RwSignal::new(d.row.name.clone().unwrap_or_default());
    let email = RwSignal::new(d.row.email.clone().unwrap_or_default());
    let notes = RwSignal::new(d.row.notes.clone().unwrap_or_default());

    let on_save = move |_| {
        updater.dispatch(UpdateAdminCustomer {
            id: id_for_save.clone(),
            name: name.get(),
            email: email.get(),
            notes: notes.get(),
        });
    };
    let on_toggle_blacklist = move |_| {
        blacklister.dispatch(SetCustomerBlacklist {
            id: id_for_toggle.clone(),
            blacklisted: !blacklisted_now,
        });
    };

    let tel_href = format!("tel:{}", d.row.phone);
    let mailto = d.row.email.as_deref().map(|e| format!("mailto:{e}"));
    // Deep-link into the voucher-create card with the phone pre-bound.
    // urlencoding is already a dep on the frontend crate.
    let voucher_href = format!(
        "/admin/vouchers?phone={}",
        urlencoding::encode(&d.row.phone)
    );

    view! {
        <header class="customer-head">
            <h1>{d.row.name.clone().unwrap_or_else(|| d.row.phone.clone())}</h1>
            <p class="contacts">
                <a href=tel_href.clone()>"📞 " {d.row.phone.clone()}</a>
                {mailto.as_ref().map(|m| view! { <a href=m.clone()>"✉ " {m.clone()}</a> })}
                <a class="btn small primary" href=voucher_href>"🎁 Gutschein für diesen Kunden"</a>
            </p>
            <p class="stats">
                {d.row.order_count}" Bestellungen · "
                <strong>{format_eur(d.row.total_cents)}</strong>" Umsatz · zuletzt "
                {format_unix_short(d.row.last_seen_at_unix)}
            </p>
            {blacklisted_now.then(|| view! {
                <p class="warning">
                    "⚠ Dieser Kunde ist für BAR-Bestellungen gesperrt. "
                    "Online-Zahlung (Karte) wird weiterhin akzeptiert."
                </p>
            })}
        </header>

        <section class="card">
            <h2>"Kontakt + Notizen"</h2>
            <label>
                <span>"Name"</span>
                <input type="text" prop:value=move || name.get()
                    on:input=move |ev| name.set(event_target_value(&ev))/>
            </label>
            <label>
                <span>"E-Mail"</span>
                <input type="email" prop:value=move || email.get()
                    on:input=move |ev| email.set(event_target_value(&ev))/>
            </label>
            <label>
                <span>"Notizen (intern, nur Admin sieht das)"</span>
                <textarea rows="3"
                    on:input=move |ev| notes.set(event_target_value(&ev))>
                    {notes.get_untracked()}
                </textarea>
            </label>
            <div class="row">
                <button class="btn primary" on:click=on_save>"Speichern"</button>
                <button class="btn ghost danger" on:click=on_toggle_blacklist>
                    {if blacklisted_now { "Sperre aufheben" } else { "Für Bar/Abholung sperren" }}
                </button>
            </div>
        </section>

        <section class="card">
            <h2>"Adressen"</h2>
            {if d.addresses.is_empty() {
                view! { <p class="empty">"Keine Adressen gespeichert."</p> }.into_any()
            } else {
                view! {
                    <ul class="address-list">
                        {d.addresses.into_iter().map(|a| {
                            let line1 = format!("{} {}", a.street, a.house_number);
                            let line2 = format!("{} {}", a.postcode, a.city);
                            view! {
                                <li>
                                    <p><strong>{line1}</strong></p>
                                    <p>{line2}</p>
                                    {a.notes.map(|n| view! { <p class="muted">{format!("Hinweis: {n}")}</p> })}
                                    <p class="muted">
                                        {a.successful_deliveries}" Lieferungen · "
                                        {a.failed_deliveries}" Fehlversuche · zuletzt "
                                        {format_unix_short(a.last_used_at_unix)}
                                    </p>
                                </li>
                            }
                        }).collect_view()}
                    </ul>
                }.into_any()
            }}
        </section>

        <section class="card">
            <h2>"Bestellungen (max. 50 neueste)"</h2>
            {if d.orders.is_empty() {
                view! { <p class="empty">"Keine Bestellungen."</p> }.into_any()
            } else {
                view! {
                    <table class="order-list">
                        <thead>
                            <tr>
                                <th>"Nummer"</th>
                                <th>"Datum"</th>
                                <th>"Typ"</th>
                                <th>"Status"</th>
                                <th>"Zahlung"</th>
                                <th class="num">"Total"</th>
                            </tr>
                        </thead>
                        <tbody>
                            {d.orders.into_iter().map(|o| {
                                let href = format!("/admin/orders/{}", o.id);
                                let payment = match (o.payment_status.as_str(), o.payment_method.as_deref()) {
                                    ("paid", Some(m)) => format!("✓ {m}"),
                                    ("paid", None)    => "✓ bezahlt".to_string(),
                                    ("pending", _)    => "offen".to_string(),
                                    (other, _)        => other.to_string(),
                                };
                                view! {
                                    <tr>
                                        <td><a href=href>{o.order_number}</a></td>
                                        <td>{format_unix_short(o.created_at_unix)}</td>
                                        <td>{if o.order_type == "delivery" { "🛵" } else { "🏪" }}</td>
                                        <td>{o.status}</td>
                                        <td>{payment}</td>
                                        <td class="num">{format_eur(o.total_cents)}</td>
                                    </tr>
                                }
                            }).collect_view()}
                        </tbody>
                    </table>
                }.into_any()
            }}
        </section>
    }
}

/// Render a Unix timestamp as "12.05. 14:23" — staff want at-a-glance
/// recency, not full RFC-3339. Returns "—" for 0.
fn format_unix_short(unix: i64) -> String {
    if unix == 0 {
        return "—".to_string();
    }
    use chrono::{DateTime, Local};
    DateTime::<Local>::from(std::time::UNIX_EPOCH + std::time::Duration::from_secs(unix as u64))
        .format("%d.%m. %H:%M")
        .to_string()
}
