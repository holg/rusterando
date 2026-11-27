//! The Speisekarte as a CSV file for the till (e.g. a register with a
//! Swissbit TSE), so its prices can be brought up to date without typing
//! them: `/admin/speisekarte.csv`.
//!
//! One row per price (a pizza in two sizes is two articles there), the
//! extras and paid options too, and the staged new prices ("Neue Preise")
//! beside the current ones. German Excel conventions: UTF-8 with BOM, `;`
//! between fields, decimal comma.

use sqlx::SqlitePool;

pub struct Row {
    pub category: String,
    pub number: String,
    pub name: String,
    pub size: String,
    pub price_cents: i64,
    /// Staged in "Neue Preise" and different from the current one.
    pub next_cents: Option<i64>,
    pub allergens: String,
    pub additives: String,
    pub status: &'static str,
}

const HEADER: [&str; 9] = [
    "Kategorie",
    "Nr",
    "Artikel",
    "Größe",
    "Preis",
    "Preis neu",
    "Allergene",
    "Zusatzstoffe",
    "Status",
];

/// Everything on the menu, in menu order, then extras and options.
pub async fn rows(db: &SqlitePool) -> anyhow::Result<Vec<Row>> {
    #[allow(clippy::type_complexity)]
    let items: Vec<(
        String,         // category
        i64,            // category active
        Option<String>, // number
        String,         // name
        Option<String>, // small label
        i64,            // small price
        Option<i64>,    // small next
        Option<String>, // large label
        Option<i64>,    // large price
        Option<i64>,    // large next
        Option<String>, // allergens
        Option<String>, // additives
        i64,            // available
        i64,            // listed
    )> = sqlx::query_as(
        "SELECT c.name, c.is_active, i.menu_number, i.name,
                i.size_small_label, i.price_small_cents, i.next_price_small_cents,
                i.size_large_label, i.price_large_cents, i.next_price_large_cents,
                i.allergen_codes, i.additive_codes, i.is_available, i.is_listed
         FROM menu_items i JOIN menu_categories c ON c.id = i.category_id
         ORDER BY c.sort_order, i.sort_order,
                  CAST(COALESCE(i.menu_number, '') AS INTEGER), i.menu_number",
    )
    .fetch_all(db)
    .await?;
    let mut out = Vec::new();
    for (
        cat,
        cat_active,
        nr,
        name,
        s_label,
        s_price,
        s_next,
        l_label,
        l_price,
        l_next,
        all,
        add,
        avail,
        listed,
    ) in items
    {
        let status = if cat_active == 0 {
            "Kategorie aus"
        } else if listed == 0 {
            "nicht gelistet"
        } else if avail == 0 {
            "ausverkauft"
        } else {
            "aktiv"
        };
        let mut push = |size: Option<String>, price: i64, next: Option<i64>| {
            out.push(Row {
                category: cat.clone(),
                number: nr.clone().unwrap_or_default(),
                name: name.clone(),
                size: size.unwrap_or_default(),
                price_cents: price,
                next_cents: next.filter(|n| *n != price),
                allergens: codes(all.as_deref()),
                additives: codes(add.as_deref()),
                status,
            })
        };
        push(s_label, s_price, s_next);
        if let Some(large) = l_price {
            push(l_label, large, l_next);
        }
    }

    let extras: Vec<(String, i64, Option<i64>, i64)> = sqlx::query_as(
        "SELECT label, price_cents, next_price_cents, is_available FROM pizza_extras ORDER BY sort_order, label",
    )
    .fetch_all(db)
    .await?;
    for (label, price, next, avail) in extras {
        out.push(Row {
            category: "Extras".into(),
            number: String::new(),
            name: label,
            size: String::new(),
            price_cents: price,
            next_cents: next.filter(|n| *n != price),
            allergens: String::new(),
            additives: String::new(),
            status: if avail == 0 { "ausverkauft" } else { "aktiv" },
        });
    }

    let options: Vec<(String, String, i64, i64)> = sqlx::query_as(
        "SELECT g.label, o.label, o.price_cents, o.is_active * g.is_active
         FROM item_options o JOIN item_option_groups g ON g.id = o.group_id
         WHERE o.price_cents > 0
         ORDER BY g.sort_order, o.sort_order",
    )
    .fetch_all(db)
    .await?;
    for (group, label, price, active) in options {
        out.push(Row {
            category: format!("Optionen: {group}"),
            number: String::new(),
            name: label,
            size: String::new(),
            price_cents: price,
            next_cents: None,
            allergens: String::new(),
            additives: String::new(),
            status: if active == 0 { "aus" } else { "aktiv" },
        });
    }
    Ok(out)
}

/// The file: BOM, header, one line per row (CRLF, as Excel writes it).
pub fn to_csv(rows: &[Row]) -> String {
    let mut s = String::from("\u{feff}");
    s.push_str(&HEADER.join(";"));
    s.push_str("\r\n");
    for r in rows {
        let next = r.next_cents.map(eur).unwrap_or_default();
        let fields = [
            r.category.as_str(),
            &r.number,
            &r.name,
            &r.size,
            &eur(r.price_cents),
            &next,
            &r.allergens,
            &r.additives,
            r.status,
        ];
        s.push_str(
            &fields
                .iter()
                .map(|f| field(f))
                .collect::<Vec<_>>()
                .join(";"),
        );
        s.push_str("\r\n");
    }
    s
}

/// "5,00" — the amount without the € sign, as tills import it.
fn eur(cents: i64) -> String {
    let sign = if cents < 0 { "-" } else { "" };
    let c = cents.abs();
    format!("{sign}{},{:02}", c / 100, c % 100)
}

/// "2,6" → "2, 6": German Excel would read "2,6" as the number 2.6 (and
/// "1,10" as 1.1); with the space it stays text.
fn codes(raw: Option<&str>) -> String {
    raw.unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .collect::<Vec<_>>()
        .join(", ")
}

/// Quoted when it holds the separator, a quote or a line break.
fn field(s: &str) -> String {
    if s.contains([';', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(name: &str, size: &str, price: i64, next: Option<i64>) -> Row {
        Row {
            category: "Pizza".into(),
            number: "1".into(),
            name: name.into(),
            size: size.into(),
            price_cents: price,
            next_cents: next,
            allergens: "a, c, g".into(),
            additives: String::new(),
            status: "aktiv",
        }
    }

    #[test]
    fn csv_for_german_excel() {
        let csv = to_csv(&[
            row("Pizza Margherita", "22cm", 500, Some(550)),
            row("Pizza \"Spezial\"; groß", "30cm", 1000, None),
        ]);
        let mut lines = csv.split("\r\n");
        assert_eq!(
            lines.next(),
            Some(
                "\u{feff}Kategorie;Nr;Artikel;Größe;Preis;Preis neu;Allergene;Zusatzstoffe;Status"
            )
        );
        assert_eq!(
            lines.next(),
            Some("Pizza;1;Pizza Margherita;22cm;5,00;5,50;a, c, g;;aktiv")
        );
        assert_eq!(
            lines.next(),
            Some("Pizza;1;\"Pizza \"\"Spezial\"\"; groß\";30cm;10,00;;a, c, g;;aktiv")
        );
        assert_eq!(lines.next(), Some(""));
    }

    #[test]
    fn codes_stay_text_in_excel() {
        assert_eq!(codes(Some("2,6")), "2, 6");
        assert_eq!(codes(Some(" 1 ,10,")), "1, 10");
        assert_eq!(codes(Some("")), "");
        assert_eq!(codes(None), "");
    }
}
