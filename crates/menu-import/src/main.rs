//! menu-import — provision a shop's `.env` + database and import its menu.
//!
//! Two deployment kinds, distinguished by `--standalone`:
//!
//!   STANDALONE (own service, own domain) — like David's:
//!       .env.<shop>            e.g. .env.davids
//!       PUBLIC_URL             https://<domain>            davidspizzeria.de
//!       APP_NAME / BIN_NAME    <shop>-server               davidspizzeria-server
//!       LEPTOS_OUTPUT_NAME     <shop>                      davidspizzeria
//!       DATABASE_URL           sqlite:./data/<shop>.db
//!
//!   MULTI-TENANT (rides one shared service, subdomain per shop) — default:
//!       .env.<parent>.<shop>   e.g. .env.rusterando.flizza
//!       PUBLIC_URL             https://<shop>.<parent-domain>   flizza.rusterando.de
//!       APP_NAME / BIN_NAME    <parent>-server  (inherited)     rusterando-server
//!       LEPTOS_OUTPUT_NAME     <parent>         (inherited)     rusterando
//!       DATABASE_URL           sqlite:./data/<shop>.sqlite
//!     The service identity is the PARENT's — every tenant shares one
//!     `rusterando` service that routes by Host header to its own sqlite.
//!     (That Host→DB routing lives in the server, not this tool.)
//!
//! Every identity field can be overridden (`--app-name`, `--bin-name`,
//! `--leptos-output-name`, `--domain`, `--port`) since future variants will
//! differ. An existing `.env` is never modified.
//!
//! Then it always: ensures the shop's sqlite exists + is migrated against
//! the workspace `migrations/`, and replaces the menu (wipe + insert in one
//! transaction) from a `menu.json` (the shape emitted by
//! `rusterando-scrape/scrape_lieferando.py` and `pdf_to_menu.py`).
//!
//! Run from the repo root so `migrations/`, `data/`, `.env.example` resolve:
//!
//!   # multi-tenant tenant under rusterando:
//!   cargo run -p menu-import -- --parent rusterando --shop flizza \
//!       --menu rusterando-scrape/menu-json/menu-speisekarte-pizza-flizza-2025.json
//!
//!   # standalone own-service shop:
//!   cargo run -p menu-import -- --standalone --shop davids --domain davidspizzeria.de \
//!       --menu …/menu-davids-pizzeria-ludinghausen.json
//!
//! Re-running replaces the menu in place (manual /admin/menu edits are lost).

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use clap::Parser;
use serde::Deserialize;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};

// ---------------------------------------------------------------------------
// JSON input shape — matches scrape_lieferando.py / pdf_to_menu.py output.
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct MenuFile {
    #[serde(default)]
    restaurant: Restaurant,
    categories: Vec<Category>,
}

#[derive(Debug, Default, Deserialize)]
struct Restaurant {
    #[serde(default)]
    name: String,
    #[serde(default)]
    slug: String,
}

#[derive(Debug, Deserialize)]
struct Category {
    #[serde(default)]
    name: String,
    #[serde(default)]
    items: Vec<Item>,
}

#[derive(Debug, Deserialize)]
struct Item {
    /// The parsed menu number (e.g. "200", "39") when available, else "".
    #[serde(default)]
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    sizes: Vec<Size>,
}

#[derive(Debug, Deserialize)]
struct Size {
    #[serde(default)]
    label: String,
    /// Euros as a float; `null` for items with no listed price.
    price_eur: Option<f64>,
}

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------

#[derive(Parser, Debug)]
#[command(about = "Provision a shop's .env + sqlite and import its menu.json")]
struct Args {
    /// Shop slug. Standalone → `.env.<shop>` + `data/<shop>.db`.
    /// Multi-tenant → `.env.<parent>.<shop>` + `data/<shop>.sqlite`.
    /// Defaults to the menu JSON's `restaurant.slug`.
    #[arg(long)]
    shop: Option<String>,

    /// Path to the menu.json to import.
    #[arg(long)]
    menu: PathBuf,

    /// Generate a standalone own-service shop (own domain + binary identity),
    /// like David's. Without this flag the shop is a multi-tenant tenant.
    #[arg(long)]
    standalone: bool,

    /// Multi-tenant parent service slug (ignored with --standalone).
    /// `.env.<parent>.<shop>`; subdomain `<shop>.<parent-domain>`; service
    /// identity (APP_NAME/BIN_NAME/LEPTOS_OUTPUT_NAME) inherits the parent.
    #[arg(long, default_value = "rusterando")]
    parent: String,

    /// Public domain. Standalone: the shop's own domain (e.g.
    /// davidspizzeria.de). Multi-tenant: the PARENT domain (e.g.
    /// rusterando.de) — the tenant becomes `<shop>.<that>`.
    /// Defaults to `<parent>.de` (multi-tenant) or `<shop>.example.com`
    /// (standalone).
    #[arg(long)]
    domain: Option<String>,

    /// Override APP_NAME. Defaults: `<shop>-server` (standalone) /
    /// `<parent>-server` (multi-tenant).
    #[arg(long)]
    app_name: Option<String>,

    /// Override BIN_NAME. Defaults to the same value as APP_NAME.
    #[arg(long)]
    bin_name: Option<String>,

    /// Override LEPTOS_OUTPUT_NAME (JS bundle namespace). Defaults: `<shop>`
    /// (standalone) / `<parent>` (multi-tenant).
    #[arg(long)]
    leptos_output_name: Option<String>,

    /// Override LEPTOS_SITE_ADDR port. Multi-tenant tenants normally leave
    /// this alone — the shared service already binds the parent's port.
    #[arg(long)]
    port: Option<u16>,

    /// Explicit path to the shop's .env (overrides the derived name).
    #[arg(long)]
    env: Option<PathBuf>,

    /// Explicit path to the shop sqlite (overrides the derived `data/…`).
    #[arg(long)]
    db: Option<PathBuf>,

    /// Workspace root (where `migrations/`, `data/`, `.env.example` live).
    /// Defaults to the current working directory.
    #[arg(long, default_value = ".")]
    root: PathBuf,

    /// Resolve all paths/values and validate the menu, but don't write the
    /// .env, create/migrate the db, or import.
    #[arg(long)]
    dry_run: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,sqlx=warn".into()),
        )
        .with_target(false)
        .init();

    let args = Args::parse();
    let root = args.root.canonicalize().with_context(|| {
        format!("--root {} does not exist", args.root.display())
    })?;

    let menu = load_menu(&args.menu)?;

    // Resolve the slug: --shop wins, else restaurant.slug from the JSON.
    let shop = args
        .shop
        .clone()
        .or_else(|| {
            let s = menu.restaurant.slug.trim();
            (!s.is_empty()).then(|| s.to_string())
        })
        .ok_or_else(|| {
            anyhow!("no --shop given and menu JSON has no restaurant.slug to fall back to")
        })?;
    let shop = slugify(&shop);
    let shop_name = if menu.restaurant.name.trim().is_empty() {
        shop.clone()
    } else {
        menu.restaurant.name.trim().to_string()
    };

    // Resolve naming for the chosen mode.
    let names = resolve_names(&args, &shop, &shop_name);
    tracing::info!(
        shop = %shop,
        mode = if args.standalone { "standalone" } else { "multi-tenant" },
        env = %names.env_file,
        db = %names.db_rel,
        public_url = %names.public_url,
        "provisioning shop"
    );

    // .env: --env wins, else the derived file name under root.
    let env_path = args
        .env
        .clone()
        .unwrap_or_else(|| root.join(&names.env_file));
    ensure_env_file(&root, &env_path, &names, args.dry_run)?;

    // sqlite: --db wins, else the derived data/<…> under root.
    let db_path = args
        .db
        .clone()
        .unwrap_or_else(|| root.join(&names.db_rel));

    if args.dry_run {
        tracing::info!(
            db = %db_path.display(),
            "[dry-run] would create+migrate db and import {} categories",
            menu.categories.len()
        );
        return Ok(());
    }

    let pool = open_and_migrate(&db_path, &root.join("migrations")).await?;

    // Replace the menu.
    let (n_cats, n_items, n_sizes) = import_menu(&pool, &menu).await?;
    pool.close().await;

    tracing::info!(
        "imported {n_cats} categories, {n_items} items, {n_sizes} size variants into {}",
        db_path.display()
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// .env + naming
// ---------------------------------------------------------------------------

/// The fully-resolved per-shop identity used to write a fresh `.env` and to
/// place the sqlite. Every field is concrete (defaults applied + overrides
/// merged) so `rewrite_env` is pure substitution.
struct Names {
    /// `.env.davids` (standalone) / `.env.rusterando.flizza` (multi-tenant).
    env_file: String,
    /// `data/davids.db` (standalone) / `data/flizza.sqlite` (multi-tenant),
    /// relative to root.
    db_rel: String,
    app_name: String,
    bin_name: String,
    leptos_output_name: String,
    /// `sqlite:./data/<…>` matching `db_rel`.
    database_url: String,
    /// `https://davidspizzeria.de` / `https://flizza.rusterando.de`.
    public_url: String,
    /// `/var/www/<host>` deploy base, host taken from `public_url`.
    deploy_remote_base: String,
    /// `"<shop name> <bestellung@<host>>"`.
    email_from: String,
    /// `Some("127.0.0.1:<port>")` only when a port override is given.
    site_addr: Option<String>,
}

/// Apply mode + defaults + overrides into a concrete `Names`.
///
/// Standalone: identity is the shop's own (`<shop>-server`, `<shop>` bundle,
/// own `<domain>`, `data/<shop>.db`).
///
/// Multi-tenant: the service identity is the PARENT's (`<parent>-server`,
/// `<parent>` bundle); only the DB (`data/<shop>.sqlite`) and the subdomain
/// (`<shop>.<parent-domain>`) are shop-specific. The shared service already
/// owns the port, so no LEPTOS_SITE_ADDR unless explicitly overridden.
fn resolve_names(args: &Args, shop: &str, shop_name: &str) -> Names {
    let (env_file, db_rel, default_app, default_bundle, host) = if args.standalone {
        let domain = args
            .domain
            .clone()
            .unwrap_or_else(|| format!("{shop}.example.com"));
        (
            format!(".env.{shop}"),
            format!("data/{shop}.db"),
            format!("{shop}-server"),
            shop.to_string(),
            domain,
        )
    } else {
        let parent = &args.parent;
        let parent_domain = args
            .domain
            .clone()
            .unwrap_or_else(|| format!("{parent}.de"));
        (
            format!(".env.{parent}.{shop}"),
            format!("data/{shop}.sqlite"),
            format!("{parent}-server"),
            parent.clone(),
            format!("{shop}.{parent_domain}"),
        )
    };

    let app_name = args.app_name.clone().unwrap_or(default_app);
    // BIN_NAME defaults to whatever APP_NAME resolved to.
    let bin_name = args.bin_name.clone().unwrap_or_else(|| app_name.clone());
    let leptos_output_name = args
        .leptos_output_name
        .clone()
        .unwrap_or(default_bundle);

    Names {
        env_file,
        database_url: format!("sqlite:./{db_rel}"),
        db_rel,
        app_name,
        bin_name,
        leptos_output_name,
        public_url: format!("https://{host}"),
        deploy_remote_base: format!("/var/www/{host}"),
        email_from: format!("{shop_name} <bestellung@{host}>"),
        site_addr: args.port.map(|p| format!("127.0.0.1:{p}")),
    }
}

fn ensure_env_file(root: &Path, env_path: &Path, names: &Names, dry_run: bool) -> Result<()> {
    if env_path.exists() {
        tracing::info!(env = %env_path.display(), "using existing .env (left untouched)");
        return Ok(());
    }

    let example = root.join(".env.example");
    let template = std::fs::read_to_string(&example).with_context(|| {
        format!(
            "cannot generate {} — template {} not found",
            env_path.display(),
            example.display()
        )
    })?;
    let body = rewrite_env(&template, names);

    if dry_run {
        tracing::info!(env = %env_path.display(), "[dry-run] would write new .env from .env.example");
        return Ok(());
    }
    std::fs::write(env_path, body)
        .with_context(|| format!("writing {}", env_path.display()))?;
    tracing::info!(
        env = %env_path.display(),
        "wrote new .env — fill in secrets (Stripe/SMTP/SSH/APNs) before deploy"
    );
    Ok(())
}

/// Substitute the shop-specific keys into the `.env.example` template,
/// leaving everything else (secrets, SSH host, Stripe/APNs) as placeholders.
fn rewrite_env(template: &str, n: &Names) -> String {
    template
        .lines()
        .map(|line| {
            let trimmed = line.trim_start();
            if trimmed.starts_with('#') {
                return line.to_string();
            }
            let Some((key, _)) = trimmed.split_once('=') else {
                return line.to_string();
            };
            match key.trim() {
                "DATABASE_URL" => format!("DATABASE_URL={}", n.database_url),
                "APP_NAME" => format!("APP_NAME={}", n.app_name),
                "BIN_NAME" => format!("BIN_NAME={}", n.bin_name),
                "LEPTOS_OUTPUT_NAME" => format!("LEPTOS_OUTPUT_NAME={}", n.leptos_output_name),
                "PUBLIC_URL" => format!("PUBLIC_URL={}", n.public_url),
                "DEPLOY_REMOTE_BASE" => format!("DEPLOY_REMOTE_BASE={}", n.deploy_remote_base),
                "EMAIL_FROM" => format!("EMAIL_FROM=\"{}\"", n.email_from),
                // Only rewrite the bind port when explicitly overridden;
                // otherwise keep the example's value (multi-tenant tenants
                // inherit the shared service's port).
                "LEPTOS_SITE_ADDR" if n.site_addr.is_some() => {
                    format!("LEPTOS_SITE_ADDR={}", n.site_addr.as_ref().unwrap())
                }
                // Pass through unknown keys, but quote any value that
                // wouldn't parse as bare dotenv. `.env.example` ships
                // placeholders like `<10-char team id>` (spaces!) which
                // dotenvy rejects unquoted — and a single bad line aborts
                // the whole file load. Quoting keeps a freshly-generated
                // .env loadable even before the operator fills secrets in.
                _ => quote_env_line_if_needed(line),
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}

/// If `line` is `KEY=value` with a value that needs quoting for dotenv
/// (contains whitespace, `<`, `>` or `#`) and isn't already quoted, return
/// `KEY="value"`. Otherwise return the line unchanged. Inline `# comments`
/// (a space-hash with text after) are preserved outside the quotes.
fn quote_env_line_if_needed(line: &str) -> String {
    // Leave comments / blanks / non-assignments alone.
    if line.trim_start().starts_with('#') {
        return line.to_string();
    }
    let Some((key, rest)) = line.split_once('=') else {
        return line.to_string();
    };

    // Split a trailing ` # comment` off the value so we don't quote it in.
    let (value, comment) = match rest.find(" #") {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, ""),
    };
    let v = value.trim_end();

    let already_quoted = (v.starts_with('"') && v.ends_with('"') && v.len() >= 2)
        || (v.starts_with('\'') && v.ends_with('\'') && v.len() >= 2);
    let needs_quote = !v.is_empty()
        && !already_quoted
        && v.chars().any(|c| c.is_whitespace() || matches!(c, '<' | '>' | '#'));

    if needs_quote {
        format!("{key}=\"{v}\"{comment}")
    } else {
        line.to_string()
    }
}

// ---------------------------------------------------------------------------
// sqlite create + migrate
// ---------------------------------------------------------------------------

async fn open_and_migrate(db_path: &Path, migrations_dir: &Path) -> Result<SqlitePool> {
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let existed = db_path.exists();

    let opts = SqliteConnectOptions::new()
        .filename(db_path)
        .create_if_missing(true)
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await
        .with_context(|| format!("opening sqlite at {}", db_path.display()))?;

    if existed {
        tracing::info!(db = %db_path.display(), "using existing db");
    } else {
        tracing::info!(db = %db_path.display(), "created new db");
    }

    // Run the workspace migrations. `Migrator::new` reads the directory at
    // runtime (vs the compile-time `sqlx::migrate!` macro) so this binary
    // doesn't need a baked-in copy of the migration set.
    let migrator = sqlx::migrate::Migrator::new(migrations_dir)
        .await
        .with_context(|| format!("loading migrations from {}", migrations_dir.display()))?;
    migrator
        .run(&pool)
        .await
        .context("running migrations")?;
    tracing::info!("migrations up to date");

    Ok(pool)
}

// ---------------------------------------------------------------------------
// Step 3 — replace menu
// ---------------------------------------------------------------------------

async fn import_menu(pool: &SqlitePool, menu: &MenuFile) -> Result<(usize, usize, usize)> {
    let mut tx = pool.begin().await?;

    // Replace: clear items first (FK to categories), then categories.
    // menu_options/extras FK to items with ON DELETE CASCADE where defined;
    // for safety clear menu_options explicitly if the table exists.
    sqlx::query("DELETE FROM menu_items").execute(&mut *tx).await?;
    sqlx::query("DELETE FROM menu_categories").execute(&mut *tx).await?;

    let mut n_cats = 0usize;
    let mut n_items = 0usize;
    let mut n_sizes = 0usize;
    // menu_number is UNIQUE in the schema, but a print menu can reuse a
    // small number across sections (e.g. "6" as both a pizza topping and a
    // Pizzabrötchen). Keep the first occurrence's number; drop later
    // collisions to NULL so the insert doesn't fail.
    let mut seen_numbers: std::collections::HashSet<String> = std::collections::HashSet::new();

    for (ci, cat) in menu.categories.iter().enumerate() {
        let name = cat.name.trim();
        if name.is_empty() || cat.items.is_empty() {
            continue;
        }
        let cat_id = format!("mc-{:03}", ci + 1);
        let cat_sort = (ci as i64) + 1;
        sqlx::query(
            "INSERT INTO menu_categories (id, name, sort_order, is_active) VALUES (?, ?, ?, 1)",
        )
        .bind(&cat_id)
        .bind(name)
        .bind(cat_sort)
        .execute(&mut *tx)
        .await
        .with_context(|| format!("inserting category {name:?}"))?;
        n_cats += 1;

        let item_type = infer_item_type(name);

        for (ii, item) in cat.items.iter().enumerate() {
            let item_name = item.name.trim();
            if item_name.is_empty() {
                tracing::warn!(category = %name, "skipping item with empty name");
                continue;
            }

            let (small, large, small_label, large_label) = map_sizes(&item.sizes, &item_type);
            let Some(small_cents) = small else {
                tracing::warn!(item = %item_name, "skipping item with no price");
                continue;
            };

            let item_id = format!("{cat_id}-i{:03}", ii + 1);
            let menu_number: Option<String> = {
                let n = item.id.trim();
                if n.is_empty() {
                    None
                } else if seen_numbers.insert(n.to_string()) {
                    Some(n.to_string())
                } else {
                    tracing::warn!(
                        number = %n,
                        item = %item_name,
                        "menu_number already used — storing this item without a number"
                    );
                    None
                }
            };
            let description = {
                let d = item.description.trim();
                (!d.is_empty()).then(|| d.to_string())
            };
            let item_sort = (ii as i64) + 1;

            sqlx::query(
                "INSERT INTO menu_items
                    (id, category_id, menu_number, name, description, item_type,
                     price_small_cents, price_large_cents, size_small_label, size_large_label,
                     is_spicy, is_available, sort_order)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 0, 1, ?)",
            )
            .bind(&item_id)
            .bind(&cat_id)
            .bind(menu_number)
            .bind(item_name)
            .bind(description)
            .bind(&item_type)
            .bind(small_cents)
            .bind(large)
            .bind(small_label)
            .bind(large_label)
            .bind(item_sort)
            .execute(&mut *tx)
            .await
            .with_context(|| format!("inserting item {item_name:?}"))?;

            n_items += 1;
            n_sizes += item.sizes.iter().filter(|s| s.price_eur.is_some()).count();
        }
    }

    // Sanity: don't commit an empty import — likely a parse/format mismatch.
    if n_items == 0 {
        tx.rollback().await.ok();
        bail!("no items imported — is the menu JSON in the expected shape?");
    }

    // Quick post-check inside the txn before commit.
    let count: i64 = sqlx::query("SELECT COUNT(*) AS c FROM menu_items")
        .fetch_one(&mut *tx)
        .await?
        .get("c");
    debug_assert_eq!(count as usize, n_items);

    tx.commit().await?;
    Ok((n_cats, n_items, n_sizes))
}

// ---------------------------------------------------------------------------
// Mapping helpers
// ---------------------------------------------------------------------------

fn eur_to_cents(eur: f64) -> i64 {
    (eur * 100.0).round() as i64
}

/// Map the `sizes[]` array onto the DB's two-price model.
///
///   0 priced sizes -> (None, …)             caller skips the item
///   1 size         -> small only, no labels (or its own label if set)
///   2 sizes        -> small + large; default 22cm/30cm if labels are blank
///   3+ sizes       -> small = first, large = last; a warning is logged
///                     (the schema only holds two price tiers)
fn map_sizes(
    sizes: &[Size],
    item_type: &str,
) -> (Option<i64>, Option<i64>, Option<String>, Option<String>) {
    let priced: Vec<&Size> = sizes.iter().filter(|s| s.price_eur.is_some()).collect();
    match priced.as_slice() {
        [] => (None, None, None, None),
        [one] => {
            let label = non_empty(&one.label);
            (Some(eur_to_cents(one.price_eur.unwrap())), None, label, None)
        }
        [first, .., last] => {
            if priced.len() > 2 {
                // Expected for pizzas printed with klein/mittel/normal — the
                // DB holds two tiers, so we keep cheapest + dearest. Debug,
                // not warn, so a normal import stays quiet.
                tracing::debug!(
                    n = priced.len(),
                    "item has {} price tiers; schema holds 2 — using first + last",
                    priced.len()
                );
            }
            let is_pizza = item_type == "pizza" || item_type == "calzone";
            let sl = non_empty(&first.label)
                .or_else(|| is_pizza.then(|| "22cm".to_string()));
            let ll = non_empty(&last.label)
                .or_else(|| is_pizza.then(|| "30cm".to_string()));
            (
                Some(eur_to_cents(first.price_eur.unwrap())),
                Some(eur_to_cents(last.price_eur.unwrap())),
                sl,
                ll,
            )
        }
    }
}

fn non_empty(s: &str) -> Option<String> {
    let t = s.trim();
    (!t.is_empty()).then(|| t.to_string())
}

/// Infer `item_type` (one of pizza/calzone/pasta/oven/meat/salad/drink/side)
/// from the German category name. Falls back to `side`.
fn infer_item_type(category: &str) -> &'static str {
    let c = category.to_lowercase();
    let has = |needle: &str| c.contains(needle);
    if has("calzone") {
        "calzone"
    } else if has("pizza") {
        "pizza"
    } else if has("nudel") || has("pasta") || has("spaghetti") || has("tortellini") {
        "pasta"
    } else if has("auflauf") || has("aufläuf") || has("überback") || has("ofen")
        || has("backofen") || has("gratin")
    {
        "oven"
    } else if has("salat") || has("insalata") || has("salad") {
        "salad"
    } else if has("getränk") || has("getraenk") || has("drink") || has("bevera") {
        "drink"
    } else if has("fleisch") || has("schnitzel") || has("hähnchen") || has("haehnchen")
        || has("bistecca") || has("fisch") || has("fish") || has("mare")
    {
        "meat"
    } else {
        "side"
    }
}

fn slugify(s: &str) -> String {
    s.trim()
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

fn load_menu(path: &Path) -> Result<MenuFile> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading menu json {}", path.display()))?;
    let menu: MenuFile = serde_json::from_str(&text)
        .with_context(|| format!("parsing menu json {}", path.display()))?;
    Ok(menu)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cents_rounding() {
        assert_eq!(eur_to_cents(10.20), 1020);
        assert_eq!(eur_to_cents(5.0), 500);
        assert_eq!(eur_to_cents(12.55), 1255);
    }

    #[test]
    fn item_type_inference() {
        assert_eq!(infer_item_type("Pizza Spezialitäten"), "pizza");
        assert_eq!(infer_item_type("Calzone Pizzen"), "calzone");
        assert_eq!(infer_item_type("Nudelgerichte aus der Pfanne"), "pasta");
        assert_eq!(infer_item_type("Salatspezialitäten"), "salad");
        assert_eq!(infer_item_type("Getränke außer Haus"), "drink");
        assert_eq!(infer_item_type("Fleisch Spezialitäten"), "meat");
        assert_eq!(infer_item_type("Broccoli Aufläufe"), "oven");
        assert_eq!(infer_item_type("Sonstiges"), "side");
    }

    #[test]
    fn sizes_two_pizza_default_labels() {
        let sizes = vec![
            Size { label: "".into(), price_eur: Some(5.0) },
            Size { label: "".into(), price_eur: Some(7.0) },
        ];
        let (s, l, sl, ll) = map_sizes(&sizes, "pizza");
        assert_eq!((s, l), (Some(500), Some(700)));
        assert_eq!(sl.as_deref(), Some("22cm"));
        assert_eq!(ll.as_deref(), Some("30cm"));
    }

    #[test]
    fn sizes_single_no_label() {
        let sizes = vec![Size { label: "".into(), price_eur: Some(12.5) }];
        let (s, l, sl, ll) = map_sizes(&sizes, "meat");
        assert_eq!((s, l, sl, ll), (Some(1250), None, None, None));
    }

    #[test]
    fn slug_basic() {
        assert_eq!(slugify("Pizza-Flizza 2025"), "pizza-flizza-2025");
        assert_eq!(slugify("  Davids Pizzeria  "), "davids-pizzeria");
    }

    // Build Args the way clap would, so default_value attrs (parent,
    // root) are applied just like at runtime.
    fn args_from(extra: &[&str]) -> Args {
        let mut argv = vec!["menu-import", "--menu", "x.json"];
        argv.extend_from_slice(extra);
        Args::parse_from(argv)
    }

    #[test]
    fn names_multitenant_defaults() {
        let a = args_from(&["--parent", "rusterando", "--shop", "flizza"]);
        let n = resolve_names(&a, "flizza", "Pizza Flizza");
        assert_eq!(n.env_file, ".env.rusterando.flizza");
        assert_eq!(n.db_rel, "data/flizza.sqlite");
        assert_eq!(n.app_name, "rusterando-server"); // parent identity
        assert_eq!(n.bin_name, "rusterando-server");
        assert_eq!(n.leptos_output_name, "rusterando");
        assert_eq!(n.public_url, "https://flizza.rusterando.de");
        assert_eq!(n.deploy_remote_base, "/var/www/flizza.rusterando.de");
        assert_eq!(n.email_from, "Pizza Flizza <bestellung@flizza.rusterando.de>");
        assert!(n.site_addr.is_none()); // inherits shared service port
    }

    #[test]
    fn names_standalone_defaults() {
        let a = args_from(&["--standalone", "--shop", "davids", "--domain", "davidspizzeria.de"]);
        let n = resolve_names(&a, "davids", "Davids Pizzeria");
        assert_eq!(n.env_file, ".env.davids");
        assert_eq!(n.db_rel, "data/davids.db");
        assert_eq!(n.app_name, "davids-server"); // own identity
        assert_eq!(n.leptos_output_name, "davids");
        assert_eq!(n.public_url, "https://davidspizzeria.de");
        assert_eq!(n.deploy_remote_base, "/var/www/davidspizzeria.de");
    }

    #[test]
    fn names_overrides_win() {
        // Reproduce the real .env.davids: brand-named identity via overrides.
        let a = args_from(&[
            "--standalone", "--shop", "davids", "--domain", "davidspizzeria.de",
            "--app-name", "davidspizzeria-server",
            "--leptos-output-name", "davidspizzeria",
            "--port", "3005",
        ]);
        let n = resolve_names(&a, "davids", "Davids");
        assert_eq!(n.app_name, "davidspizzeria-server");
        assert_eq!(n.bin_name, "davidspizzeria-server"); // BIN defaults to APP
        assert_eq!(n.leptos_output_name, "davidspizzeria");
        assert_eq!(n.site_addr.as_deref(), Some("127.0.0.1:3005"));
    }

    #[test]
    fn placeholder_values_get_quoted() {
        // The real .env.example breakers.
        assert_eq!(
            quote_env_line_if_needed("APNS_TEAM_ID=<10-char team id>"),
            "APNS_TEAM_ID=\"<10-char team id>\""
        );
        assert_eq!(
            quote_env_line_if_needed("ORS_API_KEY=<your ORS api key>"),
            "ORS_API_KEY=\"<your ORS api key>\""
        );
        // Already-fine values untouched.
        assert_eq!(
            quote_env_line_if_needed("APNS_TEAM_ID=U4GT5CZ7J7"),
            "APNS_TEAM_ID=U4GT5CZ7J7"
        );
        // Already-quoted untouched.
        assert_eq!(
            quote_env_line_if_needed("EMAIL_FROM=\"a b <c@d>\""),
            "EMAIL_FROM=\"a b <c@d>\""
        );
        // Inline comment preserved outside the quotes.
        assert_eq!(
            quote_env_line_if_needed("SSH_HOST=user@host.example.com"),
            "SSH_HOST=user@host.example.com"
        );
        // Comments + blanks left alone.
        assert_eq!(quote_env_line_if_needed("# a comment"), "# a comment");
        assert_eq!(quote_env_line_if_needed(""), "");
    }

    #[test]
    fn generated_env_parses_as_dotenv() {
        // The whole point: a freshly-generated env must load even with
        // placeholders unfilled. Build one from a template that contains
        // the exact .env.example breakers and assert every line is valid
        // `KEY=value` (no spaces in an unquoted value).
        let a = args_from(&["--parent", "rusterando", "--shop", "flizza"]);
        let n = resolve_names(&a, "flizza", "Flizza");
        let tmpl = "APP_NAME=rusterando-server\n\
                    APNS_TEAM_ID=<10-char team id>\n\
                    ORS_API_KEY=<your ORS api key>\n\
                    DATABASE_URL=sqlite:./data/rusterando.db\n";
        let out = rewrite_env(tmpl, &n);
        for line in out.lines() {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (_k, v) = line.split_once('=').expect("KEY=VALUE");
            let v = v.trim();
            let quoted = v.starts_with('"') || v.starts_with('\'');
            assert!(
                quoted || !v.contains(' '),
                "unquoted value with space would break dotenvy: {line:?}"
            );
        }
    }

    #[test]
    fn rewrite_only_touches_known_keys() {
        let a = args_from(&["--parent", "rusterando", "--shop", "flizza"]);
        let n = resolve_names(&a, "flizza", "Flizza");
        let tmpl = "# comment\nAPP_NAME=rusterando-server\nSMTP_PASS='secret$!'\nDATABASE_URL=sqlite:./data/rusterando.db\n";
        let out = rewrite_env(tmpl, &n);
        assert!(out.contains("DATABASE_URL=sqlite:./data/flizza.sqlite"));
        assert!(out.contains("SMTP_PASS='secret$!'")); // secret untouched
        assert!(out.contains("# comment"));
    }
}
