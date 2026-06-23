use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MenuCategory {
    pub id: String,
    pub name: String,
    pub sort_order: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MenuItem {
    pub id: String,
    pub category_id: String,
    pub menu_number: Option<String>,
    pub name: String,
    pub description: Option<String>,
    /// The canonical GERMAN base of `name`/`description` (the un-coalesced DB
    /// column). Carried so the client can ask the translation pack for an
    /// empty-cell fallback: `t_menu(name, name_source)`. Equals `name` /
    /// `description` on the German locale and on the admin path. Defaulted so
    /// older payloads / callers that don't set it still deserialize.
    #[serde(default)]
    pub name_source: String,
    #[serde(default)]
    pub description_source: Option<String>,
    pub item_type: String,
    pub price_small_cents: i64,
    pub price_large_cents: Option<i64>,
    pub size_small_label: Option<String>,
    pub size_large_label: Option<String>,
    pub allergen_codes: Option<String>,
    pub additive_codes: Option<String>,
    pub is_spicy: bool,
    pub is_available: bool,
    #[serde(default = "default_listed")]
    pub is_listed: bool,
    pub sort_order: i64,
    /// Number of pizza_extras the customer can pick at no charge before
    /// the per-extra fee starts. 0 means "no freebies, charge everything".
    /// See `flat_extra_price_cents`.
    #[serde(default)]
    pub included_extras_count: i64,
    /// Per-extra flat charge in cents. `None` means "fall back to the
    /// pizza_extras catalog price" (existing behaviour for normal pizzas).
    /// `Some(n)` means every extra costs `n` cents past the included
    /// count, regardless of catalog price (used by Pizzablech: 3 free,
    /// then €3 each; by 36 cm pizza: 0 free, €1 each).
    #[serde(default)]
    pub flat_extra_price_cents: Option<i64>,
    /// Whether the public-menu picker should show the global extras
    /// (pizza_extras) checkboxes for this item. Default true — drinks
    /// and a few salads opt out via /admin/menu. Hides the entire
    /// "Extras" block in the order modal when false.
    #[serde(default = "default_allow_extras")]
    pub allow_extras: bool,
    /// Option groups attached to this item (e.g. "Dressing" on a
    /// salad). The picker renders these as radios (when max_select=1)
    /// or checkboxes. The min_select bound enforces "required pick".
    /// Empty for items with no group attachments.
    #[serde(default)]
    pub option_groups: Vec<OptionGroup>,
    /// Ingredients the customer may leave OFF ("ohne X"), computed
    /// SERVER-SIDE from the description × extras catalog (matching each
    /// extra's label + its "aka" aliases). No parsing on the client, no
    /// hardcoded grammar. Empty when the item has no matched ingredients.
    /// The modal renders this directly.
    #[serde(default)]
    pub removable: Vec<RemovableIngredient>,
}

fn default_allow_extras() -> bool {
    true
}

fn default_listed() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LegendEntry {
    pub code: String,
    pub name_de: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MenuPayload {
    pub categories: Vec<MenuCategory>,
    pub items: Vec<MenuItem>,
    pub allergens: Vec<LegendEntry>,
    pub additives: Vec<LegendEntry>,
    /// Shop phone for the "Telefonisch bestellen" hint on the menu page.
    /// Baked into the payload (rather than read from `BrandingHandle` in
    /// the component) so SSR and hydrate render the same DOM.
    #[serde(default)]
    pub shop_phone: String,
    /// Per-shop toggle for the always-visible category list on the
    /// menu page. Read from `app_settings.menu_category_overlay`. Off
    /// by default; toggled in `/admin/settings`.
    #[serde(default)]
    pub category_overlay: bool,
}

/// Which size was chosen for a pizza when adding to the cart. `Single` for
/// non-pizza items (no size split).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum SizeChoice {
    Small,
    Large,
    #[default]
    Single,
}

impl SizeChoice {
    pub fn label(self) -> &'static str {
        match self {
            SizeChoice::Small => "small",
            SizeChoice::Large => "large",
            SizeChoice::Single => "single",
        }
    }
    pub fn parse(s: &str) -> Self {
        match s {
            "small" => SizeChoice::Small,
            "large" => SizeChoice::Large,
            _ => SizeChoice::Single,
        }
    }
}

/// One line in the cart, decorated with the joined item name + computed prices
/// so the UI can render without re-fetching the menu.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CartLine {
    pub id: String,
    pub menu_item_id: String,
    pub menu_number: Option<String>,
    pub name: String,
    pub size: SizeChoice,
    pub size_label: Option<String>, // "22 cm", "30 cm", or None
    pub quantity: i64,
    /// Base unit price (without extras).
    pub unit_price_cents: i64,
    /// Total contribution from extras for ONE pizza of this line.
    /// Multiplied by `quantity` already in `line_total_cents`.
    #[serde(default)]
    pub extras_unit_cents: i64,
    /// Snapshotted picked extras for receipt history. Empty when no extras.
    #[serde(default)]
    pub extras: Vec<CartExtra>,
    /// Snapshotted picks from required-choice option groups (e.g. salad
    /// dressing). Receipts read this directly. Empty when the item has
    /// no option groups attached. Like `extras`, the labels + prices
    /// are snapshot at add-to-cart time.
    #[serde(default)]
    pub selected_options: Vec<CartSelectedOption>,
    /// Snapshotted ingredients the customer left OFF ("ohne X"). Bare
    /// labels, always free. Empty when none. Mirrors `extras`.
    #[serde(default)]
    pub removals: Vec<CartRemoval>,
    pub line_total_cents: i64,
    /// `true` when this line is the free giveaway (Gratis-Pizzabrötchen):
    /// priced at 0 and added only because the order qualified. Lets the UI
    /// badge it "Gratis" and the kitchen ticket flag it, without inferring
    /// "free" from a 0 price. Server-enforced at `place_order`.
    #[serde(default)]
    pub is_giveaway: bool,
}

/// One picked extra on a cart/order line. Snapshotted with label + price so
/// price changes in the catalog later don't rewrite history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CartExtra {
    pub id: String,
    pub label: String,
    pub price_cents: i64,
}

/// Catalog row served to the menu page. The picker shows these checkboxes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PizzaExtra {
    pub id: String,
    pub label: String,
    pub price_cents: i64,
    pub sort_order: i64,
    /// Alternate names ("aka") this ingredient may appear under in a menu
    /// description — across languages and inflections: e.g. Pilze →
    /// ["Pilzen", "Champignons"], Käse → ["Goudakäse", "Gouda"]. The "ohne
    /// X" matcher tries these in addition to the label. Explicit DATA per
    /// ingredient — no grammar rules, no per-language code.
    #[serde(default)]
    pub aliases: Vec<String>,
}

/// An ingredient the customer can LEAVE OFF an item ("ohne X"). Derived by
/// matching the item's free-text description against the extras catalog
/// (see [`match_removable_ingredients`]). `bare_label` is the catalog label
/// with a leading "Extra " stripped, so "Extra Käse" → "Käse": removals
/// read "ohne Käse", never "ohne Extra Käse".
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RemovableIngredient {
    /// The catalog (`pizza_extras`) id this was matched from.
    pub id: String,
    /// Bare ingredient name shown to the customer + printed on the ticket.
    pub bare_label: String,
}

/// One snapshot of a removed ingredient on a cart/order line. Always free;
/// `label` is the bare ingredient name. Mirror of [`CartExtra`] minus price.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CartRemoval {
    pub id: String,
    pub label: String,
}

/// Strip a leading "Extra " (case-insensitive) from a catalog label to get
/// the bare ingredient name. "Extra Käse" → "Käse"; "Thunfisch" → "Thunfisch".
pub fn bare_ingredient_label(label: &str) -> &str {
    let t = label.trim();
    for prefix in ["Extra ", "extra "] {
        if let Some(rest) = t.strip_prefix(prefix) {
            return rest.trim();
        }
    }
    t
}

/// Given an item's free-text `description` (e.g. "mit Salami, Thunfisch,
/// Paprika und Zwiebeln") and the extras `catalog`, return the ingredients
/// the customer may leave off — every catalog extra whose BARE name
/// ([`bare_ingredient_label`]) appears as a whole word in the description.
///
/// Conservative by design: only catalog matches are offered, so we never
/// propose removing something we can't cleanly name. Preserves catalog
/// order; de-dups by bare label (so "Käse" and "Extra Käse" don't both
/// appear).
pub fn match_removable_ingredients(
    description: &str,
    catalog: &[PizzaExtra],
) -> Vec<RemovableIngredient> {
    let desc = description.to_lowercase();
    let mut out: Vec<RemovableIngredient> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    for extra in catalog {
        let bare = bare_ingredient_label(&extra.label).to_string();
        if bare.is_empty() {
            continue;
        }
        let key = bare.to_lowercase();
        if seen.contains(&key) {
            continue;
        }
        // An ingredient is removable if the description contains its bare
        // name OR any of its configured "aka" aliases (Pilze → "Pilzen",
        // "Champignons"; Käse → "Goudakäse") as a whole word. Explicit
        // data per extra — no grammar, no per-language code.
        let matched = contains_word(&desc, &key)
            || extra
                .aliases
                .iter()
                .map(|a| a.trim().to_lowercase())
                .filter(|a| !a.is_empty())
                .any(|a| contains_word(&desc, &a));
        if matched {
            seen.insert(key);
            out.push(RemovableIngredient {
                id: extra.id.clone(),
                bare_label: bare,
            });
        }
    }
    out
}

/// Whole-word (boundary-guarded) containment: `needle` must appear in
/// `haystack` not flanked by alphanumerics, so "mais" matches "… mais …"
/// but NOT "maispoulet". Both args are expected lowercased. Word chars are
/// Unicode-alphanumeric (so umlauts in "käse" count).
fn contains_word(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    let mut start = 0;
    while let Some(pos) = haystack[start..].find(needle) {
        let i = start + pos;
        let before_ok = haystack[..i]
            .chars()
            .next_back()
            .map(|c| !c.is_alphanumeric())
            .unwrap_or(true);
        let after = &haystack[i + needle.len()..];
        let after_ok = after
            .chars()
            .next()
            .map(|c| !c.is_alphanumeric())
            .unwrap_or(true);
        if before_ok && after_ok {
            return true;
        }
        start = i + needle.len();
        if start >= haystack.len() {
            break;
        }
    }
    false
}

/// One snapshot of a chosen option from a required-choice group
/// (Dressing, Beilage, …) on a cart/order line. Same role as
/// `CartExtra` but distinguished by the group it came from so the
/// receipt can label it as "Joghurt-Dressing" under the "Dressing"
/// caption.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CartSelectedOption {
    pub group_id: String,
    pub group_label: String,
    pub option_id: String,
    pub option_label: String,
    pub price_cents: i64,
}

/// Catalog row for the menu page: an option group attached to an item.
/// The frontend renders a radio (max_select=1) or checkbox group based
/// on min/max bounds.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OptionGroup {
    pub id: String,
    pub label: String,
    pub min_select: i64,
    pub max_select: i64,
    pub sort_order: i64,
    /// When true, 0 € options in this group render WITHOUT a "gratis"
    /// label — used for pure variant pickers (drink flavour, etc.) where
    /// "gratis" would wrongly imply the item itself is free. Priced
    /// options still show their "+X,XX €". Default false.
    #[serde(default)]
    pub hide_zero_price: bool,
    pub options: Vec<OptionItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OptionItem {
    pub id: String,
    pub label: String,
    pub price_cents: i64,
    pub sort_order: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CartView {
    pub lines: Vec<CartLine>,
    pub subtotal_cents: i64,
    pub item_count: i64, // sum of quantities, for the cart icon badge
    /// Free-delivery promo threshold in cents (admin-set). 0 = disabled.
    /// Sent on every CartView so the cart drawer can show "noch X €
    /// bis kostenlose Lieferung" without a separate fetch.
    #[serde(default)]
    pub free_delivery_threshold_cents: i64,
    /// `true` when online ordering is currently off (manual pause or
    /// outside opening hours). Lets the cart drawer disable "Weiter zur
    /// Kasse" and show a note without a separate fetch. Mirrors the
    /// authoritative `place_order` gate; the checkout page repeats the
    /// check via CheckoutContext.
    #[serde(default)]
    pub orders_closed: bool,
    /// Giveaway (free-Pizzabrötchen) offer state for THIS cart, resolved
    /// server-side from the `giveaway_*` settings + the current subtotal and
    /// (when known) order type. Drives the cart's "noch X € bis zum gratis
    /// Pizzabrötchen" nudge and the "Jetzt sichern" offer button. None when
    /// the promo is disabled or the order type doesn't qualify.
    #[serde(default)]
    pub giveaway: Option<GiveawayOffer>,
}

/// The cart-facing view of the giveaway promo. Computed in `load_cart`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct GiveawayOffer {
    /// Qualifying subtotal threshold in cents (0 = every order).
    pub min_order_cents: i64,
    /// `true` once the paid subtotal (excluding the giveaway line itself)
    /// reaches the threshold — the customer may claim the free item.
    pub qualifies: bool,
    /// `true` when a giveaway line is already in the cart, so the UI shows
    /// "claimed" instead of the "Jetzt sichern" offer.
    pub claimed: bool,
    /// The menu item id of the free item (the Pizzabrötchen) so the cart can
    /// dispatch the existing add-to-cart action for the offer button.
    pub item_id: String,
}

/// A live event pushed to a customer's open `/orders/{id}` page over the
/// SSE channel. Keyed by `order_id` so the server can fan it out only to
/// the browser(s) watching that order. Serialized as JSON on the wire.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LiveEvent {
    pub order_id: String,
    pub kind: LiveKind,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "t", content = "v")]
pub enum LiveKind {
    /// A new restaurant→customer message (appended to the order's log).
    Message(OrderMessage),
    /// Order status changed (e.g. "preparing", "out_for_delivery").
    Status(String),
    /// A message was delivered (rendered) in a customer browser — flips
    /// the admin's view of that message to "✓ Zugestellt". Order-keyed.
    MessageAck { message_id: i64 },
    /// Global shop open/closed state changed (pause/snooze/force-open or
    /// a snooze expiring). Broadcast to all clients via `/api/live/shop`;
    /// `order_id` is empty/ignored for these. `level` drives the banner
    /// colour (green/amber/red); `reason` is the German caption.
    ShopStatus { level: ShopLevel, reason: String },
    /// The delivery driver's live position for THIS order, pushed periodically
    /// from the driver's phone (Geolocation API) while a delivery is active.
    /// Order-keyed → only the matching customer's `/api/live/orders/{id}`
    /// stream receives it. Coordinates are micro-degrees (degrees × 1e6) so the
    /// enum can stay `Eq` (raw f64 isn't); ~0.1 m precision, plenty for "driver
    /// is on the way". `at_unix` is the capture time (seconds) for staleness.
    DriverLocation {
        lat_e6: i32,
        lon_e6: i32,
        at_unix: i64,
    },
}

/// Three-level shop status, driving the colour of the customer banner and
/// the admin chip. Distinct from the binary order-gate (which only cares
/// about Open vs not-Open): a *temporary* closure that resolves on its own
/// (opens later today, or a timed/manual pause) is amber, not red — red is
/// reserved for a hard close with nothing more today (Ruhetag / past the
/// last window).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ShopLevel {
    /// Taking orders right now — green.
    #[default]
    Open,
    /// Closed at this moment but it resolves itself: opens later today, or
    /// a snooze/manual pause is in effect — amber.
    OpensLater,
    /// Hard closed with nothing more today (Ruhetag, or after the last
    /// window) — red.
    Closed,
}

impl ShopLevel {
    /// True when the shop isn't open at THIS moment (anything other than
    /// `Open`). This is the banner/chip's "is it green" question — it is
    /// NOT the same as "is ordering blocked" (see `blocks_ordering`):
    /// `OpensLater` is not-open-now but still allows pre-orders for a
    /// later slot today.
    pub fn is_closed(self) -> bool {
        !matches!(self, ShopLevel::Open)
    }
    /// Whether this level, on its own, definitively tells us ordering is
    /// blocked — for live UI (cart checkout button) that only has the
    /// SSE-pushed level, not the authoritative server gate.
    ///
    /// `Open` → `Some(false)` (ordering allowed). `Closed` → `Some(true)`
    /// (hard closed: Ruhetag / nothing more today). `OpensLater` → `None`,
    /// because it's AMBIGUOUS: it covers both "opens later today"
    /// (pre-orders allowed) AND "snooze/manual pause" (blocked), and the
    /// level alone can't tell them apart. Callers must then defer to the
    /// authoritative `shop_closed_state` value instead of guessing.
    pub fn blocks_ordering(self) -> Option<bool> {
        match self {
            ShopLevel::Open => Some(false),
            ShopLevel::Closed => Some(true),
            ShopLevel::OpensLater => None,
        }
    }
    /// CSS modifier class for the banner / chip.
    pub fn css_class(self) -> &'static str {
        match self {
            ShopLevel::Open => "open",
            ShopLevel::OpensLater => "opens-soon",
            ShopLevel::Closed => "closed",
        }
    }
}

/// One message in an order's shared chat thread, with its server-side
/// timestamp. `created_at` is a display-ready local string ("HH:MM" /
/// "DD.MM. HH:MM"). Either side can post: staff (admin/kitchen/driver) or a
/// VIP customer replying from their order page — `sender` says which.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OrderMessage {
    /// `order_messages.id` — used by the customer page to ack delivery.
    #[serde(default)]
    pub id: i64,
    pub body: String,
    pub created_at: String,
    /// Who wrote it: "admin" | "kitchen" | "driver" | "customer". Drives
    /// attribution + bubble alignment in every view. Defaults to "admin"
    /// so pre-`sender` rows (and any old serialized payloads) read correctly.
    #[serde(default = "default_sender")]
    pub sender: String,
    /// Delivered (rendered in a browser) yet? Drives the admin "✓
    /// Zugestellt" vs "gesendet" indicator.
    #[serde(default)]
    pub delivered: bool,
}

fn default_sender() -> String {
    "admin".to_string()
}

impl OrderMessage {
    /// True when the message came from the VIP customer (vs. any staff role).
    pub fn is_customer(&self) -> bool {
        self.sender == "customer"
    }

    /// German label + emoji for the sender, shown before the body. Unknown
    /// senders fall back to a neutral "Restaurant" so the log never breaks.
    pub fn sender_label_de(&self) -> &'static str {
        match self.sender.as_str() {
            "customer" => "🙋 Kunde",
            "kitchen" => "👨‍🍳 Küche",
            "driver" => "🛵 Fahrer",
            "admin" => "🏪 Restaurant",
            _ => "🏪 Restaurant",
        }
    }
}

/// Format a cent amount as a German euro string: 1234 -> "12,34 €".
pub fn format_eur(cents: i64) -> String {
    let euros = cents / 100;
    let rest = (cents % 100).abs();
    format!("{euros},{rest:02} €")
}

/// URL slug for SEO pages (category + delivery-area URLs). German-aware:
/// transliterates umlauts/ß to their ASCII digraphs FIRST (ü→ue, ö→oe,
/// ä→ae, ß→ss), so "Lüdinghausen" → "luedinghausen" rather than dropping
/// the umlaut. Then lowercases, keeps ASCII alphanumerics, and collapses
/// every other run into a single dash (no leading/trailing dash). Empty
/// input (or all-punctuation) yields "" — callers treat that as "no slug".
pub fn seo_slug(s: &str) -> String {
    // 1) Transliterate German specials into ASCII digraphs.
    let mut translit = String::with_capacity(s.len() + 4);
    for c in s.chars() {
        match c {
            'ä' => translit.push_str("ae"),
            'ö' => translit.push_str("oe"),
            'ü' => translit.push_str("ue"),
            'Ä' => translit.push_str("Ae"),
            'Ö' => translit.push_str("Oe"),
            'Ü' => translit.push_str("Ue"),
            'ß' => translit.push_str("ss"),
            other => translit.push(other),
        }
    }
    // 2) Lowercase ASCII alphanumerics; collapse the rest into single dashes.
    let mut out = String::with_capacity(translit.len());
    let mut prev_dash = false;
    for c in translit.chars() {
        if c.is_ascii_alphanumeric() {
            out.extend(c.to_lowercase());
            prev_dash = false;
        } else if !prev_dash && !out.is_empty() {
            out.push('-');
            prev_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::seo_slug;

    #[test]
    fn slug_transliterates_german() {
        assert_eq!(seo_slug("Lüdinghausen"), "luedinghausen");
        assert_eq!(seo_slug("Münster"), "muenster");
        assert_eq!(seo_slug("Straße"), "strasse");
        assert_eq!(seo_slug("Größer"), "groesser");
    }

    #[test]
    fn slug_basics() {
        assert_eq!(seo_slug("Pizza"), "pizza");
        assert_eq!(seo_slug("Pizza & Pasta"), "pizza-pasta");
        assert_eq!(seo_slug("  Seppenrade  "), "seppenrade");
        assert_eq!(seo_slug("Antipasti / Salate"), "antipasti-salate");
    }

    #[test]
    fn slug_empty_and_punctuation() {
        assert_eq!(seo_slug(""), "");
        assert_eq!(seo_slug("---"), "");
        assert_eq!(seo_slug("&&&"), "");
    }

    use super::{
        bare_ingredient_label, match_removable_ingredients, PizzaExtra, RemovableIngredient,
    };

    fn extra(i: usize, label: &str, aliases: &[&str]) -> PizzaExtra {
        PizzaExtra {
            id: format!("ex-{i}"),
            label: label.to_string(),
            price_cents: 100,
            sort_order: i as i64,
            aliases: aliases.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn catalog() -> Vec<PizzaExtra> {
        // Mirror of the real seed, with "aka" aliases on Käse + Pilze.
        vec![
            extra(0, "Extra Käse", &["Goudakäse", "Gouda"]),
            extra(1, "Extra Salami", &[]),
            extra(2, "Thunfisch", &[]),
            extra(3, "Mais", &[]),
            extra(4, "Knoblauch", &[]),
            extra(5, "Pilze", &["Pilzen", "Champignons"]),
        ]
    }

    fn bare(v: &[RemovableIngredient]) -> Vec<&str> {
        v.iter().map(|r| r.bare_label.as_str()).collect()
    }

    #[test]
    fn strips_extra_prefix() {
        assert_eq!(bare_ingredient_label("Extra Käse"), "Käse");
        assert_eq!(bare_ingredient_label("extra Salami"), "Salami");
        assert_eq!(bare_ingredient_label("Thunfisch"), "Thunfisch");
    }

    #[test]
    fn extra_prefixed_ingredient_is_removable_by_bare_name() {
        let got = match_removable_ingredients("mit Tomaten, Käse und Salami", &catalog());
        assert_eq!(bare(&got), vec!["Käse", "Salami"]);
        assert!(got.iter().all(|r| !r.bare_label.starts_with("Extra")));
    }

    #[test]
    fn matches_bare_catalog_names_and_skips_uncatalogued() {
        let got =
            match_removable_ingredients("mit Salami, Thunfisch, Paprika und Zwiebeln", &catalog());
        assert_eq!(bare(&got), vec!["Salami", "Thunfisch"]);
    }

    #[test]
    fn word_boundary_guard() {
        let got = match_removable_ingredients("mit Maispoulet", &catalog());
        assert!(bare(&got).is_empty());
        let got = match_removable_ingredients("mit Schinken und Mais", &catalog());
        assert_eq!(bare(&got), vec!["Mais"]);
    }

    #[test]
    fn dedups_and_preserves_catalog_order() {
        let got = match_removable_ingredients("Käse, Käse, Thunfisch", &catalog());
        assert_eq!(bare(&got), vec!["Käse", "Thunfisch"]);
    }

    #[test]
    fn alias_matches_german_inflection() {
        // The real bug: catalog "Pilze", description "Pilzen" — matched via
        // the alias "Pilzen", and the offered label stays the clean "Pilze".
        let got = match_removable_ingredients("mit Schinken, Salami und Pilzen", &catalog());
        assert!(bare(&got).contains(&"Pilze"), "got {:?}", bare(&got));
        // Boundary guard still holds.
        let got = match_removable_ingredients("mit Pilzragout", &catalog());
        assert!(!bare(&got).contains(&"Pilze"));
    }

    #[test]
    fn alias_matches_loanword_across_locales() {
        // A French menu line "avec Champignons" matches the German "Pilze"
        // via the alias — locale-agnostic, no per-language code.
        let got = match_removable_ingredients("avec jambon et champignons", &catalog());
        assert!(bare(&got).contains(&"Pilze"), "got {:?}", bare(&got));
    }

    #[test]
    fn alias_matches_specific_cheese() {
        // "Goudakäse" in the description → removable "Käse" via alias.
        let got = match_removable_ingredients("mit Tomaten und Goudakäse", &catalog());
        assert!(bare(&got).contains(&"Käse"), "got {:?}", bare(&got));
    }

    #[test]
    fn no_aliases_means_exact_only() {
        // Without an alias, "Pilzen" must NOT match "Pilze" — proving the
        // declension is DATA (the alias), not baked into the matcher.
        let plain = vec![extra(0, "Pilze", &[])];
        let got = match_removable_ingredients("mit Pilzen", &plain);
        assert!(bare(&got).is_empty(), "got {:?}", bare(&got));
        // Exact form still matches.
        let got = match_removable_ingredients("mit Pilze", &plain);
        assert!(bare(&got).contains(&"Pilze"));
    }
}
