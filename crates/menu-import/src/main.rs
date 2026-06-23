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
    /// Optional shop metadata (address/contact/hours) — present for
    /// Lieferando scrapes, absent for PDF menus. All fields best-effort.
    #[serde(default)]
    meta: Meta,
    categories: Vec<Category>,
}

#[derive(Debug, Default, Deserialize)]
struct Meta {
    #[serde(default)]
    address_street: String,
    #[serde(default)]
    postcode: String,
    #[serde(default)]
    city: String,
    lat: Option<f64>,
    lon: Option<f64>,
    #[serde(default)]
    phone: String,
    #[serde(default)]
    email: String,
    #[serde(default)]
    owner: String,
    #[serde(default)]
    vat_id: String,
    /// weekday name (lowercase) → list of "HH:MM-HH:MM" ranges; [] = closed.
    #[serde(default)]
    hours: std::collections::HashMap<String, Vec<String>>,
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
    let root = args
        .root
        .canonicalize()
        .with_context(|| format!("--root {} does not exist", args.root.display()))?;

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
        "Demo Shop".to_string()
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
    let db_path = args.db.clone().unwrap_or_else(|| root.join(&names.db_rel));

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
    let (n_cats, n_items, n_sizes) = import_menu(&pool, &menu, &shop_name).await?;
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
    let leptos_output_name = args.leptos_output_name.clone().unwrap_or(default_bundle);

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
    std::fs::write(env_path, body).with_context(|| format!("writing {}", env_path.display()))?;
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
        && v.chars()
            .any(|c| c.is_whitespace() || matches!(c, '<' | '>' | '#'));

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
    migrator.run(&pool).await.context("running migrations")?;
    tracing::info!("migrations up to date");

    Ok(pool)
}

// ---------------------------------------------------------------------------
// Step 3 — replace menu
// ---------------------------------------------------------------------------

async fn import_menu(
    pool: &SqlitePool,
    menu: &MenuFile,
    shop_name: &str,
) -> Result<(usize, usize, usize)> {
    let mut tx = pool.begin().await?;

    // Replace strategy: UPSERT, not wipe. A tenant DB may already hold orders
    // whose `order_items.menu_item_id` FK-references menu_items — a blanket
    // DELETE would fail (FK constraint) or destroy history. So we INSERT … ON
    // CONFLICT(id) DO UPDATE every category/item (ids are positional and
    // stable across re-imports of the same JSON), then prune stale rows that
    // (a) aren't in this import and (b) aren't referenced by any order.
    let mut kept_cat_ids: Vec<String> = Vec::new();
    let mut kept_item_ids: Vec<String> = Vec::new();

    let mut n_cats = 0usize;
    let mut n_items = 0usize;
    let mut n_sizes = 0usize;
    // menu_number is UNIQUE in the schema, but a print menu can reuse a
    // small number across sections (e.g. "6" as both a pizza topping and a
    // Pizzabrötchen). Keep the first occurrence's number; drop later
    // collisions to NULL so the insert doesn't fail.
    let mut seen_numbers: std::collections::HashSet<String> = std::collections::HashSet::new();

    // Auto-numbering: some shops (e.g. Pizzeria Rimini, Lieferando) print no
    // "Nr N" — every item's `id` is a UUID or empty, so none is a usable
    // menu_number. When NOT A SINGLE item carries a printed number, we assign
    // sequential 1..n in menu order. If the shop DOES print numbers, we keep
    // those verbatim and leave the un-numbered ones NULL (mirrors the
    // scraper's rule). The JSON `id` itself is never mutated — UUIDs stay in
    // the file for later comparison; this only decides the DB menu_number.
    let any_printed_number = menu
        .categories
        .iter()
        .flat_map(|c| &c.items)
        .any(|it| is_menu_number(it.id.trim()));
    let auto_number = !any_printed_number;
    let mut next_auto: i64 = 0;

    for (ci, cat) in menu.categories.iter().enumerate() {
        let name = cat.name.trim();
        if name.is_empty() || cat.items.is_empty() {
            continue;
        }
        let cat_id = format!("mc-{:03}", ci + 1);
        let cat_sort = (ci as i64) + 1;
        sqlx::query(
            "INSERT INTO menu_categories (id, name, sort_order, is_active) VALUES (?, ?, ?, 1)
             ON CONFLICT(id) DO UPDATE SET name = excluded.name,
                 sort_order = excluded.sort_order, is_active = 1",
        )
        .bind(&cat_id)
        .bind(name)
        .bind(cat_sort)
        .execute(&mut *tx)
        .await
        .with_context(|| format!("upserting category {name:?}"))?;
        kept_cat_ids.push(cat_id.clone());
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
            let menu_number: Option<String> = if auto_number {
                // No shop-printed numbers anywhere → assign 1..n in order.
                next_auto += 1;
                Some(next_auto.to_string())
            } else {
                // Shop prints numbers: keep this item's printed number (when
                // it has one), drop UUIDs/blanks to NULL, and de-dup.
                let n = item.id.trim();
                if !is_menu_number(n) {
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
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 0, 1, ?)
                 ON CONFLICT(id) DO UPDATE SET
                     category_id = excluded.category_id,
                     menu_number = excluded.menu_number,
                     name = excluded.name,
                     description = excluded.description,
                     item_type = excluded.item_type,
                     price_small_cents = excluded.price_small_cents,
                     price_large_cents = excluded.price_large_cents,
                     size_small_label = excluded.size_small_label,
                     size_large_label = excluded.size_large_label,
                     sort_order = excluded.sort_order,
                     is_available = 1,
                     updated_at = CURRENT_TIMESTAMP",
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
            .with_context(|| format!("upserting item {item_name:?}"))?;
            kept_item_ids.push(item_id.clone());

            n_items += 1;
            n_sizes += item.sizes.iter().filter(|s| s.price_eur.is_some()).count();
        }
    }

    // Sanity: don't commit an empty import — likely a parse/format mismatch.
    if n_items == 0 {
        tx.rollback().await.ok();
        bail!("no items imported — is the menu JSON in the expected shape?");
    }

    // Prune stale rows from a previous, larger import. Two safeguards:
    //   - menu_items referenced by an order (order_items.menu_item_id) are
    //     NEVER deleted — that would break history / the FK. Instead they're
    //     unlisted (is_listed=0, is_available=0) and their menu_number cleared
    //     so they vacate the UNIQUE(menu_number) slot for the new menu.
    //   - everything else not in this import is deleted.
    let kept_items = json_array(&kept_item_ids);
    let orphaned: Vec<(String,)> = sqlx::query_as(&format!(
        "SELECT id FROM menu_items WHERE id NOT IN ({kept_items})"
    ))
    .fetch_all(&mut *tx)
    .await?;
    for (sid,) in &orphaned {
        let referenced: i64 =
            sqlx::query("SELECT COUNT(*) AS c FROM order_items WHERE menu_item_id = ?1")
                .bind(sid)
                .fetch_one(&mut *tx)
                .await?
                .get("c");
        if referenced > 0 {
            sqlx::query(
                "UPDATE menu_items SET is_listed = 0, is_available = 0, menu_number = NULL,
                     updated_at = CURRENT_TIMESTAMP WHERE id = ?1",
            )
            .bind(sid)
            .execute(&mut *tx)
            .await?;
        } else {
            sqlx::query("DELETE FROM menu_items WHERE id = ?1")
                .bind(sid)
                .execute(&mut *tx)
                .await?;
        }
    }
    // Categories no longer in this import and now holding no items.
    let kept_cats = json_array(&kept_cat_ids);
    sqlx::query(&format!(
        "DELETE FROM menu_categories WHERE id NOT IN ({kept_cats})
         AND id NOT IN (SELECT DISTINCT category_id FROM menu_items)"
    ))
    .execute(&mut *tx)
    .await?;

    // Quick post-check inside the txn before commit.
    let count: i64 = sqlx::query("SELECT COUNT(*) AS c FROM menu_items")
        .fetch_one(&mut *tx)
        .await?
        .get("c");
    debug_assert_eq!(count as usize, n_items);

    // Branding: set the shop's display name from restaurant.name everywhere
    // it's stored, so a fresh tenant's site isn't branded with the David's
    // seed. Two sinks, both written here (in the same txn as the menu) and
    // logged so it's verifiable that it actually happened:
    //   - app_settings.shop_name  → header / branding name
    //   - home_hero.title (id=1)   → landing-page headline
    write_branding(&mut tx, shop_name).await?;
    let n_meta = write_meta(&mut tx, &menu.meta).await?;

    tx.commit().await?;
    tracing::info!(
        shop_name = %shop_name,
        meta_fields = n_meta,
        "set shop_name + home_hero.title + {n_meta} meta field(s)"
    );
    Ok((n_cats, n_items, n_sizes))
}

/// Map a lowercase English weekday name to the `opening_hours.weekday`
/// integer (0 = Sunday .. 6 = Saturday). Returns None for unknown names.
fn weekday_index(name: &str) -> Option<i64> {
    Some(match name.trim().to_lowercase().as_str() {
        "sunday" => 0,
        "monday" => 1,
        "tuesday" => 2,
        "wednesday" => 3,
        "thursday" => 4,
        "friday" => 5,
        "saturday" => 6,
        _ => return None,
    })
}

/// Write the scraped `meta` (address/contact/hours) into the tenant DB.
/// Only NON-EMPTY scalar fields are written, so a PDF import (no meta) or a
/// shop with a blank field never clobbers good admin-entered data. Returns
/// the count of fields written. Runs inside the import txn.
async fn write_meta(tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>, meta: &Meta) -> Result<usize> {
    // app_settings scalar fields: (key, value) — skip empties.
    let plz_city = {
        let pc = format!("{} {}", meta.postcode.trim(), meta.city.trim());
        pc.trim().to_string()
    };
    let pairs: Vec<(&str, String)> = vec![
        (
            "shop_address_street",
            meta.address_street.trim().to_string(),
        ),
        ("shop_address_plz_city", plz_city),
        ("shop_city", meta.city.trim().to_string()),
        ("shop_phone", meta.phone.trim().to_string()),
        ("shop_email", meta.email.trim().to_string()),
        ("shop_owner", meta.owner.trim().to_string()),
        ("shop_vat_id", meta.vat_id.trim().to_string()),
        (
            "shop_lat",
            meta.lat
                .filter(|v| *v != 0.0)
                .map(|v| v.to_string())
                .unwrap_or_default(),
        ),
        (
            "shop_lon",
            meta.lon
                .filter(|v| *v != 0.0)
                .map(|v| v.to_string())
                .unwrap_or_default(),
        ),
    ];

    let mut written = 0usize;
    for (key, value) in pairs {
        if value.is_empty() {
            continue;
        }
        upsert_setting(tx, key, &value).await?;
        written += 1;
    }

    // Opening hours (Lieferzeiten). Only when the scrape actually carried
    // any — otherwise leave the seeded/admin hours intact. Replace the whole
    // table so re-imports stay clean (opening_hours has no FK dependents).
    if !meta.hours.is_empty() {
        sqlx::query("DELETE FROM opening_hours")
            .execute(&mut **tx)
            .await?;
        for (day, ranges) in &meta.hours {
            let Some(wd) = weekday_index(day) else {
                continue;
            };
            if ranges.is_empty() {
                // Closed that day.
                sqlx::query(
                    "INSERT INTO opening_hours (id, weekday, open_time, close_time, is_closed)
                     VALUES (?1, ?2, '', '', 1)",
                )
                .bind(format!("oh-{wd}-closed"))
                .bind(wd)
                .execute(&mut **tx)
                .await?;
                continue;
            }
            for (i, range) in ranges.iter().enumerate() {
                let Some((open, close)) = range.split_once('-') else {
                    continue;
                };
                sqlx::query(
                    "INSERT INTO opening_hours (id, weekday, open_time, close_time, is_closed)
                     VALUES (?1, ?2, ?3, ?4, 0)",
                )
                .bind(format!("oh-{wd}-{i}"))
                .bind(wd)
                .bind(open.trim())
                .bind(close.trim())
                .execute(&mut **tx)
                .await?;
            }
        }
        written += 1;
    }

    Ok(written)
}

/// UPSERT a single app_settings row by key (value-only; preserves label/hint).
async fn upsert_setting(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    key: &str,
    value: &str,
) -> Result<()> {
    let n = sqlx::query(
        "UPDATE app_settings SET value = ?1, updated_at = CURRENT_TIMESTAMP WHERE key = ?2",
    )
    .bind(value)
    .bind(key)
    .execute(&mut **tx)
    .await?
    .rows_affected();
    if n == 0 {
        sqlx::query("INSERT INTO app_settings (key, value, label_de) VALUES (?1, ?2, ?1)")
            .bind(key)
            .bind(value)
            .execute(&mut **tx)
            .await?;
    }
    Ok(())
}

/// Write the shop name into every branding sink, robustly (UPDATE, then
/// INSERT if the seeded row is missing). Runs inside the import txn.
async fn write_branding(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    shop_name: &str,
) -> Result<()> {
    // app_settings.shop_name
    let n = sqlx::query(
        "UPDATE app_settings SET value = ?1, updated_at = CURRENT_TIMESTAMP WHERE key = 'shop_name'",
    )
    .bind(shop_name)
    .execute(&mut **tx)
    .await?
    .rows_affected();
    if n == 0 {
        sqlx::query(
            "INSERT INTO app_settings (key, value, label_de) VALUES ('shop_name', ?1, 'Name des Shops')",
        )
        .bind(shop_name)
        .execute(&mut **tx)
        .await?;
    }

    // home_hero.title (single row, CHECK id = 1). UPDATE first; if the seed
    // row is absent, INSERT it (title + subtitle are NOT NULL — give the
    // subtitle an empty string for the admin to fill in on /admin).
    let n =
        sqlx::query("UPDATE home_hero SET title = ?1, updated_at = CURRENT_TIMESTAMP WHERE id = 1")
            .bind(shop_name)
            .execute(&mut **tx)
            .await?
            .rows_affected();
    if n == 0 {
        sqlx::query("INSERT INTO home_hero (id, title, subtitle) VALUES (1, ?1, '')")
            .bind(shop_name)
            .execute(&mut **tx)
            .await?;
    }
    Ok(())
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
            (
                Some(eur_to_cents(one.price_eur.unwrap())),
                None,
                label,
                None,
            )
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
            let sl = non_empty(&first.label).or_else(|| is_pizza.then(|| "22cm".to_string()));
            let ll = non_empty(&last.label).or_else(|| is_pizza.then(|| "30cm".to_string()));
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

/// Render ids as a SQL `'a','b','c'` list for an IN-clause. Ids are
/// tool-generated (`mc-001-i001`), but we still escape single quotes
/// defensively. An empty input yields `''` (an IN-list that matches
/// nothing real), so `NOT IN ('')` correctly keeps no row by id.
fn json_array(ids: &[String]) -> String {
    if ids.is_empty() {
        return "''".to_string();
    }
    ids.iter()
        .map(|id| format!("'{}'", id.replace('\'', "''")))
        .collect::<Vec<_>>()
        .join(",")
}

/// Is `s` a usable printed menu number, e.g. "59" or "2a" — digits with an
/// optional trailing lowercase letter? UUIDs and empties are NOT (so a
/// Lieferando `id` of UUID form is treated as "no number").
fn is_menu_number(s: &str) -> bool {
    let s = s.trim();
    if s.is_empty() {
        return false;
    }
    let mut chars = s.chars();
    // at least one digit
    if !chars.next().is_some_and(|c| c.is_ascii_digit()) {
        return false;
    }
    let mut seen_letter = false;
    for c in chars {
        if c.is_ascii_digit() && !seen_letter {
            continue;
        } else if c.is_ascii_lowercase() && !seen_letter {
            seen_letter = true; // single trailing letter allowed (e.g. "2a")
        } else {
            return false;
        }
    }
    true
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
    } else if has("auflauf")
        || has("aufläuf")
        || has("überback")
        || has("ofen")
        || has("backofen")
        || has("gratin")
    {
        "oven"
    } else if has("salat") || has("insalata") || has("salad") {
        "salad"
    } else if has("getränk") || has("getraenk") || has("drink") || has("bevera") {
        "drink"
    } else if has("fleisch")
        || has("schnitzel")
        || has("hähnchen")
        || has("haehnchen")
        || has("bistecca")
        || has("fisch")
        || has("fish")
        || has("mare")
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
            Size {
                label: "".into(),
                price_eur: Some(5.0),
            },
            Size {
                label: "".into(),
                price_eur: Some(7.0),
            },
        ];
        let (s, l, sl, ll) = map_sizes(&sizes, "pizza");
        assert_eq!((s, l), (Some(500), Some(700)));
        assert_eq!(sl.as_deref(), Some("22cm"));
        assert_eq!(ll.as_deref(), Some("30cm"));
    }

    #[test]
    fn sizes_single_no_label() {
        let sizes = vec![Size {
            label: "".into(),
            price_eur: Some(12.5),
        }];
        let (s, l, sl, ll) = map_sizes(&sizes, "meat");
        assert_eq!((s, l, sl, ll), (Some(1250), None, None, None));
    }

    #[test]
    fn slug_basic() {
        assert_eq!(slugify("Pizza-Flizza 2025"), "pizza-flizza-2025");
        assert_eq!(slugify("  Davids Pizzeria  "), "davids-pizzeria");
    }

    #[test]
    fn menu_number_recognition() {
        // printed numbers (kept as menu_number)
        assert!(is_menu_number("59"));
        assert!(is_menu_number("1"));
        assert!(is_menu_number("2a")); // single trailing letter
        assert!(is_menu_number(" 200 ")); // trimmed
                                          // NOT menu numbers (UUID / empty / junk → auto-number or NULL)
        assert!(!is_menu_number(""));
        assert!(!is_menu_number("6986c2cc-e9e9-446a-a872-7d4ee59c1f68"));
        assert!(!is_menu_number("abc"));
        assert!(!is_menu_number("2ab")); // two letters
        assert!(!is_menu_number("a1")); // leading letter
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
        assert_eq!(
            n.email_from,
            "Pizza Flizza <bestellung@flizza.rusterando.de>"
        );
        assert!(n.site_addr.is_none()); // inherits shared service port
    }

    #[test]
    fn names_standalone_defaults() {
        let a = args_from(&[
            "--standalone",
            "--shop",
            "davids",
            "--domain",
            "davidspizzeria.de",
        ]);
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
            "--standalone",
            "--shop",
            "davids",
            "--domain",
            "davidspizzeria.de",
            "--app-name",
            "davidspizzeria-server",
            "--leptos-output-name",
            "davidspizzeria",
            "--port",
            "3005",
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
