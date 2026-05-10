//! /admin/broadcast — manual push to staff or to everyone.
//!
//! Two buttons: "An Mitarbeiter senden" (admin/kitchen/driver) and
//! "An alle senden" (every active token, customer apps included).
//! Server fans out via APNs/FCM and writes an audit row capturing the
//! audience choice. The page below shows the recent sends so David can
//! see "did I already announce that, and to whom?".

use leptos::prelude::*;

use crate::pages::admin::shell::AdminShell;
use crate::pages::push::{list_broadcasts, BroadcastResult, BroadcastRow, SendBroadcast};

#[component]
pub fn BroadcastAdminPage() -> impl IntoView {
    let sender = ServerAction::<SendBroadcast>::new();
    let history = Resource::new(
        move || sender.version().get(),
        |_| async move { list_broadcasts().await },
    );

    // Reactive form state — one Title/Text pair, two submit buttons that
    // dispatch with different `audience` values.
    let title = RwSignal::new(String::new());
    let body = RwSignal::new(String::new());

    Effect::new(move |_| {
        if matches!(sender.value().get(), Some(Ok(_))) {
            title.set(String::new());
            body.set(String::new());
        }
    });

    let dispatch_send = move |audience: &'static str| {
        let t = title.get();
        let b = body.get();
        if t.trim().is_empty() || b.trim().is_empty() {
            return;
        }
        sender.dispatch(crate::pages::push::SendBroadcast {
            title: t,
            body: b,
            audience: audience.to_string(),
        });
    };

    view! {
        <AdminShell>
            <section class="broadcast-admin">
                <header class="admin-bar">
                    <h1>"Push-Nachricht senden"</h1>
                </header>

                <p class="hint">
                    "Wähle die Empfänger: nur Mitarbeiter (Admin, Küche, Fahrer) "
                    "oder alle Geräte inklusive Kunden-App. Maximal 80 Zeichen "
                    "für den Titel, 240 für den Text."
                </p>

                <div class="broadcast-form">
                    <label>
                        <span>"Titel"</span>
                        <input type="text" required maxlength="80"
                               placeholder="z.B. Küche schließt früher"
                               prop:value=move || title.get()
                               on:input=move |ev| title.set(event_target_value(&ev))/>
                    </label>
                    <label>
                        <span>"Text"</span>
                        <textarea required rows="3" maxlength="240"
                                  placeholder="z.B. Heute Annahme nur bis 21 Uhr."
                                  prop:value=move || body.get()
                                  on:input=move |ev| body.set(event_target_value(&ev))></textarea>
                    </label>

                    <div class="broadcast-actions">
                        <button type="button" class="btn primary"
                                disabled=move || sender.pending().get()
                                on:click=move |_| dispatch_send("staff")>
                            {move || if sender.pending().get() {
                                "Wird gesendet…".to_string()
                            } else {
                                "An Mitarbeiter senden".to_string()
                            }}
                        </button>
                        <button type="button" class="btn ghost"
                                disabled=move || sender.pending().get()
                                on:click=move |_| dispatch_send("all")>
                            "An alle senden"
                        </button>
                    </div>
                </div>

                {move || sender.value().get().map(|res| match res {
                    Err(e) => view! {
                        <p class="error">{format!("Fehler: {e}")}</p>
                    }.into_any(),
                    Ok(BroadcastResult { recipient_count, .. }) => view! {
                        <p class="ok">
                            "✓ Gesendet an " <strong>{recipient_count}</strong>
                            " Gerät(e)."
                        </p>
                    }.into_any(),
                })}

                <h2>"Letzte Sendungen"</h2>
                <Suspense fallback=|| view! { <p class="loading">"Lädt…"</p> }>
                    {move || history.get().map(|res| match res {
                        Err(e) => view! { <p class="error">{format!("Fehler: {e}")}</p> }.into_any(),
                        Ok(rows) => view! { <BroadcastList rows/> }.into_any(),
                    })}
                </Suspense>
            </section>
        </AdminShell>
    }
}

#[component]
fn BroadcastList(rows: Vec<BroadcastRow>) -> impl IntoView {
    if rows.is_empty() {
        return view! { <p class="empty">"Noch keine Push-Nachrichten gesendet."</p> }.into_any();
    }
    view! {
        <ul class="broadcast-history">
            {rows.into_iter().map(|r| {
                let audience_label = match r.audience.as_str() {
                    "all" => "Alle",
                    _ => "Mitarbeiter",
                };
                let audience_class = match r.audience.as_str() {
                    "all" => "audience-all",
                    _ => "audience-staff",
                };
                view! {
                    <li>
                        <header>
                            <time>{r.sent_at}</time>
                            <span class=format!("audience-tag {audience_class}")>{audience_label}</span>
                            <span class="muted">{format!("{} Gerät(e)", r.recipient_count)}</span>
                        </header>
                        <strong>{r.title}</strong>
                        <p>{r.body}</p>
                    </li>
                }
            }).collect_view()}
        </ul>
    }
    .into_any()
}
