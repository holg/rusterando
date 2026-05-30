//! Static legal pages: /datenschutz (GDPR privacy policy) + /impressum
//! (TMG §5 mandatory imprint for German commercial sites).
//!
//! Both pages are pure SSR — no resources, no signals, no interactivity.
//! They're plain markup with constants for the few values that might
//! change (street address, phone, contact email). Updates land via
//! deploy.
//!
//! IMPORTANT: this is not legal advice. The text is tailored to what
//! the codebase actually does (audited from migrations + code). For
//! the public App Store release, have a lawyer or e-recht24.de review
//! the wording.

use leptos::prelude::*;

use crate::branding::Branding;

/// SSR-side helper: pull `Branding` out of context. Falls back to the
/// `Default` impl (all blanks) if the handle isn't present, which only
/// happens during hydrate-side type checking.
#[cfg(feature = "ssr")]
fn current_branding() -> Branding {
    use_context::<crate::branding::BrandingHandle>()
        .map(|h| h.get())
        .unwrap_or_default()
}
#[cfg(not(feature = "ssr"))]
fn current_branding() -> Branding {
    Branding::default()
}

#[component]
pub fn ImpressumPage() -> impl IntoView {
    let b = current_branding();
    let company = b.display_name();
    let phone = b.shop_phone.clone();
    let phone_href = format!("tel:{phone}");
    let email = b.shop_email.clone();
    let email_href = format!("mailto:{email}");
    let ustid = b.shop_vat_id.clone();
    let owner = b.shop_owner.clone();
    let street = b.shop_address_street.clone();
    let plz_city = b.shop_address_plz_city.clone();
    view! {
        <article class="legal-page">
            <header>
                <h1>{crate::t!("header.imprint")}</h1>
                <p class="hint">
                    // German law requires this exact German wording on the
                    // Impressum page (§ 5 TMG). It MUST remain German on every
                    // locale — translating it would be legally non-compliant.
                    "Angaben gemäß § 5 TMG."
                </p>
            </header>

            <section>
                <h2>"Anbieter"</h2>
                <p>
                    {company.clone()} <br/>
                    {street.clone()} <br/>
                    {plz_city.clone()} <br/>
                    "Deutschland"
                </p>
                {(!owner.is_empty()).then(|| view! {
                    <p>"Inhaber: " {owner}</p>
                })}
            </section>

            <section>
                <h2>"Kontakt"</h2>
                <p>
                    {(!phone.is_empty()).then(|| view! {
                        <>"Telefon: " <a href=phone_href.clone()>{phone.clone()}</a> <br/></>
                    })}
                    {(!email.is_empty()).then(|| view! {
                        <>"E-Mail: " <a href=email_href.clone()>{email.clone()}</a></>
                    })}
                </p>
            </section>

            <section>
                <h2>"Umsatzsteuer-ID"</h2>
                {if ustid.is_empty() {
                    view! {
                        <p>
                            "Kleinunternehmer im Sinne von § 19 UStG. "
                            "Es wird keine Umsatzsteuer ausgewiesen."
                        </p>
                    }.into_any()
                } else {
                    view! {
                        <p>"USt-IdNr.: " {ustid}</p>
                    }.into_any()
                }}
            </section>

            <section>
                <h2>"Verantwortlich für den Inhalt nach § 55 Abs. 2 RStV"</h2>
                <p>
                    {company} <br/>
                    {street} <br/>
                    {plz_city}
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
                    "Als Diensteanbieter sind wir gemäß § 7 Abs. 1 TMG für "
                    "eigene Inhalte auf diesen Seiten nach den allgemeinen "
                    "Gesetzen verantwortlich. Nach §§ 8 bis 10 TMG sind wir "
                    "als Diensteanbieter jedoch nicht verpflichtet, "
                    "übermittelte oder gespeicherte fremde Informationen zu "
                    "überwachen oder nach Umständen zu forschen, die auf eine "
                    "rechtswidrige Tätigkeit hinweisen."
                </p>
            </section>

            <p class="legal-footer">
                <a href="/">"← Zurück zur Startseite"</a>
                " · "
                <a href="/datenschutz">{crate::t!("header.privacy")}</a>
            </p>
        </article>
    }
}

#[component]
pub fn DatenschutzPage() -> impl IntoView {
    let b = current_branding();
    let company = b.display_name();
    let phone = b.shop_phone.clone();
    let phone_href = format!("tel:{phone}");
    let email = b.shop_email.clone();
    let email_href = format!("mailto:{email}");
    let email_for_section6 = email.clone();
    let email_href_for_section6 = email_href.clone();
    let street = b.shop_address_street.clone();
    let plz_city = b.shop_address_plz_city.clone();
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

            <section>
                <h2>"1. Verantwortlicher"</h2>
                <p>
                    {company} <br/>
                    {street} <br/>
                    {plz_city} <br/>
                    {(!phone.is_empty()).then(|| view! {
                        <>"Telefon: " <a href=phone_href>{phone}</a> <br/></>
                    })}
                    {(!email.is_empty()).then(|| view! {
                        <>"E-Mail: " <a href=email_href>{email}</a></>
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
                        "sowie ein optionaler Hinweis (z.B. „Klingel Müller\")."
                    </li>
                    <li>
                        "Aus der Adresse berechnete Geo-Koordinaten "
                        "(Breitengrad/Längengrad), um das Liefergebiet und "
                        "die Route zu bestimmen."
                    </li>
                    <li>
                        "Inhalt Ihrer Bestellung (Artikel, Menge, Extras, "
                        "Wunschuhrzeit, Hinweise)."
                    </li>
                    <li>
                        "Bei Online-Zahlung: eine Stripe-Transaktions-ID. "
                        "Kreditkartendaten sehen wir nicht — diese werden "
                        "ausschließlich von Stripe verarbeitet."
                    </li>
                </ul>
                <p>
                    "Rechtsgrundlage: Art. 6 Abs. 1 lit. b DSGVO (Vertragserfüllung) "
                    "sowie Art. 6 Abs. 1 lit. c DSGVO (gesetzliche Aufbewahrungspflicht "
                    "für Geschäftsunterlagen, § 257 HGB / § 147 AO)."
                </p>
            </section>

            <section>
                <h2>"3. Cookies"</h2>
                <p>
                    "Wir verwenden ausschließlich technisch notwendige "
                    "Cookies — keine Tracking- oder Werbe-Cookies. "
                    "Konkret:"
                </p>
                <ul>
                    <li>
                        "<code>dp_cart</code> — verknüpft Ihren Warenkorb "
                        "über mehrere Seitenaufrufe hinweg. Lebensdauer 30 "
                        "Tage."
                    </li>
                    <li>
                        "<code>dp_session</code> — Login-Cookie für Mitarbeiter "
                        "(Admin/Küche/Fahrer). Lebensdauer 1 Jahr. Für "
                        "Besteller-Kunden nicht relevant."
                    </li>
                </ul>
                <p>
                    "Rechtsgrundlage: Art. 6 Abs. 1 lit. f DSGVO bzw. § 25 "
                    "Abs. 2 Nr. 2 TTDSG (unbedingt erforderliche Cookies)."
                </p>
            </section>

            <section>
                <h2>"4. Externe Dienste"</h2>
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

                <h3>"Apple Push Notification Service (Mitarbeiter-App)"</h3>
                <p>
                    "Die Mitarbeiter-App (iOS) verwendet Apples APNs, um "
                    "Bestellbenachrichtigungen anzuzeigen. Dabei wird ein "
                    "anonymes Geräte-Token an Apple und an unseren Server "
                    "übertragen. Dieser Dienst betrifft nur Mitarbeiter, "
                    "nicht Besteller-Kunden."
                </p>

                <h3>"E-Mail (IONOS SE)"</h3>
                <p>
                    "Bestellbestätigungen versenden wir per E-Mail über "
                    "den Hosting-Dienstleister 1&1 IONOS SE. "
                    "Datenschutzerklärung: "
                    <a href="https://www.ionos.de/terms-gtc/terms-privacy" target="_blank" rel="noopener">
                        "https://www.ionos.de/terms-gtc/terms-privacy"
                    </a>
                </p>
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
                    <li>
                        "Geräte-Tokens für Push-Benachrichtigungen: bis zum "
                        "Abmelden bzw. zur Deinstallation der App, "
                        "längstens 30 Tage Inaktivität."
                    </li>
                </ul>
            </section>

            <section>
                <h2>"6. Ihre Rechte"</h2>
                <p>
                    "Sie haben jederzeit das Recht auf:"
                </p>
                <ul>
                    <li>"Auskunft über Ihre gespeicherten Daten (Art. 15 DSGVO)"</li>
                    <li>"Berichtigung unrichtiger Daten (Art. 16 DSGVO)"</li>
                    <li>"Löschung (Art. 17 DSGVO), soweit keine gesetzliche Aufbewahrungspflicht besteht"</li>
                    <li>"Einschränkung der Verarbeitung (Art. 18 DSGVO)"</li>
                    <li>"Datenübertragbarkeit (Art. 20 DSGVO)"</li>
                    <li>"Widerspruch gegen die Verarbeitung (Art. 21 DSGVO)"</li>
                    <li>"Beschwerde bei einer Aufsichtsbehörde (Art. 77 DSGVO)"</li>
                </ul>
                <p>
                    "Eine Anfrage genügt formlos per E-Mail an "
                    <a href=email_href_for_section6>{email_for_section6}</a>
                    "; wir antworten in der Regel innerhalb weniger Tage."
                </p>
                <p>
                    "Zuständige Aufsichtsbehörde ist die Landesbeauftragte "
                    "für Datenschutz und Informationsfreiheit Nordrhein-"
                    "Westfalen, Kavalleriestraße 2-4, 40213 Düsseldorf, "
                    <a href="https://www.ldi.nrw.de" target="_blank" rel="noopener">
                        "ldi.nrw.de"
                    </a> "."
                </p>
            </section>

            <section>
                <h2>"7. Änderungen dieser Erklärung"</h2>
                <p>
                    "Wir behalten uns vor, diese Datenschutzerklärung "
                    "anzupassen, wenn wir neue Funktionen einführen oder "
                    "rechtliche Vorgaben sich ändern. Die jeweils aktuelle "
                    "Version finden Sie immer unter "
                    <a href="/datenschutz">"/datenschutz"</a> "."
                </p>
                <p class="muted">
                    "Stand: 9. Mai 2026"
                </p>
            </section>

            <p class="legal-footer">
                <a href="/">"← Zurück zur Startseite"</a>
                " · "
                <a href="/impressum">{crate::t!("header.imprint")}</a>
            </p>
        </article>
    }
}
