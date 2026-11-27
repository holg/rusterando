//! Legal pages: /datenschutz (GDPR privacy policy) + /impressum (TMG §5
//! mandatory imprint for German commercial sites).
//!
//! The shop's details (name, owner, address, contact, VAT id) and which
//! outside services it really uses come from the server through
//! [`get_legal_info`], in a `OnceResource`: the page reads the same data on
//! the server and in the browser. (Read from the server's context alone,
//! a page opened by an in-app link rendered with blank defaults — "Mein
//! Restaurant", no address, and "Kleinunternehmer" for want of a VAT id.)
//!
//! The privacy policy lists only the services the shop uses: Nominatim
//! unless the address check runs locally, OpenRouteService only with a
//! route-planning key, Stripe only with payment keys, APNs and e-mail only
//! when configured.
//!
//! IMPORTANT: this is not legal advice. The text is tailored to what the
//! codebase actually does. Have a lawyer or e-recht24.de review the
//! wording.

use leptos::prelude::*;
use serde::{Deserialize, Serialize};

/// What the legal pages show.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegalInfo {
    pub company: String,
    pub owner: String,
    pub street: String,
    pub plz_city: String,
    pub phone: String,
    pub email: String,
    pub vat_id: String,
    /// Address checks go to OpenStreetMap Nominatim (not the local index).
    pub nominatim: bool,
    /// Address checks run on our server against the local address index.
    pub local_addresses: bool,
    /// Driver tours are optimised by OpenRouteService.
    pub ors: bool,
    pub stripe: bool,
    pub apns: bool,
    pub mail: bool,
    /// The SMTP server is IONOS's: then the e-mail section names IONOS as
    /// the processor; any other server gets the neutral wording.
    pub mail_ionos: bool,
}

#[server(
    name = GetLegalInfo,
    prefix = "/api",
    endpoint = "get_legal_info"
)]
pub async fn get_legal_info() -> Result<LegalInfo, ServerFnError> {
    let b = use_context::<crate::branding::BrandingHandle>()
        .map(|h| h.get())
        .unwrap_or_default();
    let set = |k: &str| std::env::var(k).is_ok_and(|v| !v.trim().is_empty());
    let local_addresses = match use_context::<sqlx::SqlitePool>() {
        Some(db) => crate::pages::geo::ssr::autocomplete_enabled(&db).await,
        None => false,
    };
    Ok(LegalInfo {
        company: b.display_name(),
        owner: b.shop_owner.clone(),
        street: b.shop_address_street.clone(),
        plz_city: b.shop_address_plz_city.clone(),
        phone: b.shop_phone.clone(),
        email: b.shop_email.clone(),
        vat_id: b.shop_vat_id.clone(),
        nominatim: !local_addresses,
        local_addresses,
        ors: set("ORS_API_KEY"),
        stripe: set("L_STRIPE_SECRET_KEY") || set("S_STRIPE_SECRET_KEY"),
        apns: set("APNS_KEY_ID"),
        mail: set("SMTP_HOST"),
        mail_ionos: std::env::var("SMTP_HOST")
            .is_ok_and(|h| h.trim().to_ascii_lowercase().ends_with("ionos.de")),
    })
}

#[component]
pub fn ImpressumPage() -> impl IntoView {
    let info = OnceResource::new(get_legal_info());
    view! {
        <article class="legal-page">
            <header>
                <h1>{move || crate::t!("header.imprint")}</h1>
                <p class="hint">
                    // German law requires this exact German wording on the
                    // Impressum page (§ 5 DDG). It MUST remain German on every
                    // locale — translating it would be legally non-compliant.
                    "Angaben gemäß § 5 DDG."
                </p>
            </header>
            <Suspense fallback=|| view! { <p class="loading">"Lädt…"</p> }>
                {move || info.get().map(|r| match r {
                    Ok(i) => view! { <ImpressumBody i/> }.into_any(),
                    Err(e) => view! { <p class="error">{format!("Fehler: {e}")}</p> }.into_any(),
                })}
            </Suspense>
            <p class="legal-footer">
                <a href="/">"← Zurück zur Startseite"</a>
                " · "
                <a href="/datenschutz">{move || crate::t!("header.privacy")}</a>
            </p>
        </article>
    }
}

#[component]
fn ImpressumBody(i: LegalInfo) -> impl IntoView {
    let phone_href = format!("tel:{}", i.phone);
    let email_href = format!("mailto:{}", i.email);
    view! {
        <section>
            <h2>"Anbieter"</h2>
            <p>
                {i.company.clone()} <br/>
                {i.street.clone()} <br/>
                {i.plz_city.clone()} <br/>
                "Deutschland"
            </p>
            {(!i.owner.is_empty()).then(|| view! {
                <p>"Inhaber: " {i.owner.clone()}</p>
            })}
        </section>

        <section>
            <h2>"Kontakt"</h2>
            <p>
                {(!i.phone.is_empty()).then(|| view! {
                    <>"Telefon: " <a href=phone_href>{i.phone.clone()}</a> <br/></>
                })}
                {(!i.email.is_empty()).then(|| view! {
                    <>"E-Mail: " <a href=email_href>{i.email.clone()}</a></>
                })}
            </p>
        </section>

        <section>
            <h2>"Umsatzsteuer-ID"</h2>
            {if i.vat_id.is_empty() {
                view! {
                    <p>
                        "Kleinunternehmer im Sinne von § 19 UStG. "
                        "Es wird keine Umsatzsteuer ausgewiesen."
                    </p>
                }.into_any()
            } else {
                view! { <p>"USt-IdNr.: " {i.vat_id.clone()}</p> }.into_any()
            }}
        </section>

        <section>
            <h2>"Verantwortlich für den Inhalt nach § 18 Abs. 2 MStV"</h2>
            <p>
                {if i.owner.is_empty() { i.company.clone() } else { i.owner.clone() }} <br/>
                {i.street.clone()} <br/>
                {i.plz_city.clone()}
            </p>
        </section>

        <section>
            <h2>"Streitschlichtung"</h2>
            <p>
                "Die Europäische Kommission stellt eine Plattform zur "
                "Online-Streitbeilegung (OS) bereit: "
                <a href="https://ec.europa.eu/consumers/odr/" target="_blank" rel="noopener">
                    "https://ec.europa.eu/consumers/odr/"
                </a>
                ". Unsere E-Mail-Adresse finden Sie oben."
            </p>
            <p>
                "Wir sind weder verpflichtet noch bereit, an einem "
                "Streitbeilegungsverfahren vor einer Verbraucher-"
                "schlichtungsstelle teilzunehmen."
            </p>
        </section>

        <section>
            <h2>"Haftung für Inhalte"</h2>
            <p>
                "Als Diensteanbieter sind wir gemäß § 7 Abs. 1 DDG für "
                "eigene Inhalte auf diesen Seiten nach den allgemeinen "
                "Gesetzen verantwortlich. Nach §§ 8 bis 10 DDG sind wir "
                "als Diensteanbieter jedoch nicht verpflichtet, "
                "übermittelte oder gespeicherte fremde Informationen zu "
                "überwachen oder nach Umständen zu forschen, die auf eine "
                "rechtswidrige Tätigkeit hinweisen."
            </p>
        </section>
    }
}

#[component]
pub fn DatenschutzPage() -> impl IntoView {
    let info = OnceResource::new(get_legal_info());
    view! {
        <article class="legal-page">
            <header>
                <h1>"Datenschutzerklärung"</h1>
                <p class="hint">
                    "Diese Erklärung beschreibt, welche personenbezogenen Daten "
                    "wir verarbeiten, wenn Sie unsere Website besuchen oder "
                    "über die App eine Bestellung aufgeben."
                </p>
            </header>
            <Suspense fallback=|| view! { <p class="loading">"Lädt…"</p> }>
                {move || info.get().map(|r| match r {
                    Ok(i) => view! { <DatenschutzBody i/> }.into_any(),
                    Err(e) => view! { <p class="error">{format!("Fehler: {e}")}</p> }.into_any(),
                })}
            </Suspense>
            <p class="legal-footer">
                <a href="/">"← Zurück zur Startseite"</a>
                " · "
                <a href="/impressum">{move || crate::t!("header.imprint")}</a>
            </p>
        </article>
    }
}

#[component]
fn DatenschutzBody(i: LegalInfo) -> impl IntoView {
    let phone_href = format!("tel:{}", i.phone);
    let email_href = format!("mailto:{}", i.email);
    let LegalInfo {
        nominatim,
        local_addresses,
        ors,
        stripe,
        apns,
        mail,
        mail_ionos,
        ..
    } = i.clone();
    view! {
        <section>
            <h2>"1. Verantwortlicher"</h2>
            <p>
                {i.company.clone()} <br/>
                {(!i.owner.is_empty()).then(|| view! { <>"Inhaber: " {i.owner.clone()} <br/></> })}
                {i.street.clone()} <br/>
                {i.plz_city.clone()} <br/>
                {(!i.phone.is_empty()).then(|| view! {
                    <>"Telefon: " <a href=phone_href.clone()>{i.phone.clone()}</a> <br/></>
                })}
                {(!i.email.is_empty()).then(|| view! {
                    <>"E-Mail: " <a href=email_href.clone()>{i.email.clone()}</a></>
                })}
            </p>
        </section>

        <section>
            <h2>"2. Welche Daten wir bei einer Bestellung erheben"</h2>
            <p>
                "Beim Aufgeben einer Bestellung erfassen wir die Daten, "
                "die für Vertragsabschluss und Lieferung notwendig sind:"
            </p>
            <ul>
                <li>"Name, Telefonnummer, E-Mail-Adresse"</li>
                <li>
                    "Bei Lieferung: Straße, Hausnummer, Postleitzahl, Ort "
                    "sowie ein optionaler Hinweis (z. B. „Klingel Müller“)."
                </li>
                <li>
                    "Aus der Adresse ermittelte Geo-Koordinaten "
                    "(Breitengrad/Längengrad), um das Liefergebiet und "
                    "die Route zu bestimmen."
                </li>
                <li>
                    "Inhalt Ihrer Bestellung (Artikel, Menge, Extras, "
                    "Wunschuhrzeit, Hinweise)."
                </li>
                {stripe.then(|| view! {
                    <li>
                        "Bei Online-Zahlung: eine Stripe-Transaktions-ID. "
                        "Kartendaten sehen wir nicht — diese werden "
                        "ausschließlich von Stripe verarbeitet."
                    </li>
                })}
            </ul>
            <p>
                "Rechtsgrundlage: Art. 6 Abs. 1 lit. b DSGVO (Vertragserfüllung) "
                "sowie Art. 6 Abs. 1 lit. c DSGVO (gesetzliche Aufbewahrungspflicht "
                "für Geschäftsunterlagen, § 257 HGB / § 147 AO)."
            </p>
            {local_addresses.then(|| view! {
                <p>
                    "Ihre Lieferadresse prüfen wir auf unserem eigenen Server "
                    "anhand amtlicher Adressdaten (Open Data des Landes "
                    "Nordrhein-Westfalen). Dafür wird nichts an Dritte übermittelt."
                </p>
            })}
        </section>

        <section>
            <h2>"3. Cookies"</h2>
            <p>
                "Wir verwenden ausschließlich technisch notwendige "
                "Cookies — keine Tracking- oder Werbe-Cookies. Konkret:"
            </p>
            <ul>
                <li>
                    <code>"dp_cart"</code>
                    " — verknüpft Ihren Warenkorb über mehrere Seitenaufrufe "
                    "hinweg. Lebensdauer 30 Tage."
                </li>
                <li>
                    <code>"dp_session"</code> ", " <code>"dp_auth"</code>
                    " — Login-Cookies für Mitarbeiter: Rolle und deren "
                    "fälschungssichere Signatur. Lebensdauer 1 Jahr. "
                    "Für Besteller nicht relevant."
                </li>
            </ul>
            <p>
                "Rechtsgrundlage: Art. 6 Abs. 1 lit. f DSGVO bzw. § 25 "
                "Abs. 2 Nr. 2 TDDDG (unbedingt erforderliche Cookies)."
            </p>
        </section>

        <section>
            <h2>"4. Externe Dienste"</h2>
            {stripe.then(|| view! {
                <h3>"Stripe (Zahlungsdienstleister)"</h3>
                <p>
                    "Wenn Sie online mit Karte oder Apple Pay/Google Pay "
                    "bezahlen, übermitteln wir Bestellsumme und Bestell-ID an "
                    "Stripe Payments Europe, Limited. Ihre Zahlungsdaten "
                    "geben Sie direkt bei Stripe ein; wir sehen sie nie. "
                    "Datenschutzerklärung: "
                    <a href="https://stripe.com/de/privacy" target="_blank" rel="noopener">
                        "https://stripe.com/de/privacy"
                    </a>
                </p>
            })}
            {nominatim.then(|| view! {
                <h3>"OpenStreetMap Nominatim (Adressprüfung)"</h3>
                <p>
                    "Zur Prüfung Ihrer Lieferadresse senden wir die Adresse "
                    "(Straße, Hausnummer, PLZ, Ort) an den Geocoding-Dienst "
                    "Nominatim der OpenStreetMap Foundation. Es werden keine "
                    "weiteren personenbezogenen Daten übermittelt. "
                    "Datenschutzerklärung: "
                    <a href="https://wiki.osmfoundation.org/wiki/Privacy_Policy" target="_blank" rel="noopener">
                        "https://wiki.osmfoundation.org/wiki/Privacy_Policy"
                    </a>
                </p>
            })}
            {ors.then(|| view! {
                <h3>"OpenRouteService (Tour-Planung)"</h3>
                <p>
                    "Zur Routenplanung für unsere Fahrer senden wir "
                    "Geo-Koordinaten der Lieferadressen an OpenRouteService "
                    "(HeiGIT, Universität Heidelberg). Namen oder andere "
                    "Kontaktdaten werden nicht übermittelt. "
                    "Datenschutzerklärung: "
                    <a href="https://openrouteservice.org/privacy/" target="_blank" rel="noopener">
                        "https://openrouteservice.org/privacy/"
                    </a>
                </p>
            })}
            {apns.then(|| view! {
                <h3>"Apple Push Notification Service (Mitarbeiter-App)"</h3>
                <p>
                    "Die Mitarbeiter-App (iOS) verwendet Apples APNs, um "
                    "Bestellbenachrichtigungen anzuzeigen. Dabei wird ein "
                    "anonymes Geräte-Token an Apple und an unseren Server "
                    "übertragen. Dieser Dienst betrifft nur Mitarbeiter, "
                    "nicht Besteller."
                </p>
            })}
            {(mail && mail_ionos).then(|| view! {
                <h3>"E-Mail (IONOS SE)"</h3>
                <p>
                    "Bestellbestätigungen versenden wir per E-Mail über "
                    "den Hosting-Dienstleister 1&1 IONOS SE. "
                    "Datenschutzerklärung: "
                    <a href="https://www.ionos.de/terms-gtc/terms-privacy" target="_blank" rel="noopener">
                        "https://www.ionos.de/terms-gtc/terms-privacy"
                    </a>
                </p>
            })}
            {(mail && !mail_ionos).then(|| view! {
                <h3>"E-Mail"</h3>
                <p>
                    "Bestellbestätigungen versenden wir per E-Mail über den "
                    "E-Mail-Dienstleister des Shops (Auftragsverarbeitung nach "
                    "Art. 28 DSGVO)."
                </p>
            })}
        </section>

        <section>
            <h2>"5. Speicherdauer"</h2>
            <ul>
                <li>
                    "Bestelldaten (inkl. Name, Adresse, Bestellinhalt): "
                    "10 Jahre, nach handels- und steuerrechtlicher "
                    "Aufbewahrungspflicht (§ 257 HGB / § 147 AO)."
                </li>
                <li>
                    "Kundenkonto-Daten (sofern Sie ein Konto angelegt "
                    "haben): bis zur Löschung durch Sie, längstens 24 "
                    "Monate nach Ihrer letzten Bestellung. Danach werden "
                    "Name, E-Mail und Telefonnummer anonymisiert; nur die "
                    "rechnungsrelevanten Felder bleiben gemäß HGB erhalten."
                </li>
                {apns.then(|| view! {
                    <li>
                        "Geräte-Tokens für Push-Benachrichtigungen: bis zum "
                        "Abmelden bzw. zur Deinstallation der App, "
                        "längstens 30 Tage Inaktivität."
                    </li>
                })}
            </ul>
        </section>

        <section>
            <h2>"6. Ihre Rechte"</h2>
            <p>"Sie haben jederzeit das Recht auf:"</p>
            <ul>
                <li>"Auskunft über Ihre gespeicherten Daten (Art. 15 DSGVO)"</li>
                <li>"Berichtigung unrichtiger Daten (Art. 16 DSGVO)"</li>
                <li>"Löschung (Art. 17 DSGVO), soweit keine gesetzliche Aufbewahrungspflicht besteht"</li>
                <li>"Einschränkung der Verarbeitung (Art. 18 DSGVO)"</li>
                <li>"Datenübertragbarkeit (Art. 20 DSGVO)"</li>
                <li>"Widerspruch gegen die Verarbeitung (Art. 21 DSGVO)"</li>
                <li>"Beschwerde bei einer Aufsichtsbehörde (Art. 77 DSGVO)"</li>
            </ul>
            {(!i.email.is_empty()).then(|| view! {
                <p>
                    "Eine Anfrage genügt formlos per E-Mail an "
                    <a href=email_href.clone()>{i.email.clone()}</a>
                    "; wir antworten in der Regel innerhalb weniger Tage."
                </p>
            })}
            <p>
                "Zuständige Aufsichtsbehörde ist die Landesbeauftragte "
                "für Datenschutz und Informationsfreiheit Nordrhein-"
                "Westfalen, Kavalleriestraße 2-4, 40213 Düsseldorf, "
                <a href="https://www.ldi.nrw.de" target="_blank" rel="noopener">"ldi.nrw.de"</a>
                "."
            </p>
        </section>

        <section>
            <h2>"7. Änderungen dieser Erklärung"</h2>
            <p>
                "Wir behalten uns vor, diese Datenschutzerklärung "
                "anzupassen, wenn wir neue Funktionen einführen oder "
                "rechtliche Vorgaben sich ändern. Die jeweils aktuelle "
                "Version finden Sie immer unter "
                <a href="/datenschutz">"/datenschutz"</a>
                "."
            </p>
            <p class="muted">"Stand: 13. November 2026"</p>
        </section>
    }
}
