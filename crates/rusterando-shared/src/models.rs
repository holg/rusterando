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
    pub line_total_cents: i64,
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
}

/// Format a cent amount as a German euro string: 1234 -> "12,34 €".
pub fn format_eur(cents: i64) -> String {
    let euros = cents / 100;
    let rest = (cents % 100).abs();
    format!("{euros},{rest:02} €")
}
