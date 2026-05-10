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
}

/// Format a cent amount as a German euro string: 1234 -> "12,34 €".
pub fn format_eur(cents: i64) -> String {
    let euros = cents / 100;
    let rest = (cents % 100).abs();
    format!("{euros},{rest:02} €")
}
