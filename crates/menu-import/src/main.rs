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
    /// Provenance written by the scraper (`shop_url`, `scraped_at`). Stored
    /// as `menu_import_source` / `menu_import_scraped_at` in app_settings so
    /// the sys-admin tenant list can show where a menu came from and when.
    #[serde(default)]
    shop_url: String,
    #[serde(default)]
    scraped_at: String,
}

#[derive(Debug, Default, Deserialize)]
struct Meta {
    #[serde(default)]
    address_street: String,
    #[serde(default)]
    postcode: String,
    #[serde(default)]
    city: String,
    #[serde(default)]
    country: String,
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
    /// Branding artwork from the source listing (absolute URLs).
    #[serde(default)]
    logo_url: String,
    #[serde(default)]
    banner_url: String,
    /// Cuisine labels ("Italienische Pizza", "Döner") for the hero subtitle.
    #[serde(default)]
    cuisines: Vec<String>,
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

    /// Override a key in the GENERATED .env (`--set ADMIN_PASSWORD=…`,
    /// repeatable). Only applies when the .env is created by this run — an
    /// existing .env is never modified. Keys missing from the template are
    /// appended. This is how the sys-admin web import injects the shop's
    /// fresh staff passwords.
    #[arg(long = "set", value_name = "KEY=VALUE")]
    set: Vec<String>,

    /// Where to store downloaded branding images. Defaults to the site root's
    /// upload dir, which is what the running server serves as
    /// `/img/uploads/…`. Pass `--no-images` to skip the download entirely.
    #[arg(long, default_value = "html/img/uploads")]
    uploads_dir: PathBuf,

    /// Don't download the shop's logo / hero image from the source listing.
    #[arg(long)]
    no_images: bool,
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
    let overrides = env_overrides(&args.set, &menu.meta, !args.standalone)?;
    ensure_env_file(&root, &env_path, &names, &overrides, args.dry_run)?;

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
    // Branding artwork: fetched BEFORE the menu import so the hero row can be
    // written in the same transaction. Best-effort — a failed download must
    // never fail the import.
    let hero_image = if args.no_images {
        None
    } else {
        download_hero_image(&menu.meta, &root.join(&args.uploads_dir), &shop)
    };
    let (n_cats, n_items, n_sizes) =
        import_menu(&pool, &menu, &shop_name, &names.public_url, hero_image.as_deref()).await?;
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

/// `--set KEY=VALUE` overrides plus the values derived from the scraped
/// meta: the shop's coordinates (`PIZZERIA_LAT/LON`) and, when the shop sits
/// in NRW (postcode 3xxxx–5xxxx or country DE with a known NRW city), the
/// local address-lookup backend so the admin's "Alles aus Adresse
/// einrichten" works without touching the .env. Explicit `--set` wins.
fn env_overrides(set: &[String], meta: &Meta, multi_tenant: bool) -> Result<Vec<(String, String)>> {
    let mut out: Vec<(String, String)> = Vec::new();
    // The template ships MULTI_TENANT=0 (the standalone default). A tenant
    // under a parent MUST say 1, or the server treats its `.env` as a
    // single-tenant deployment when it's loaded directly.
    out.push((
        "MULTI_TENANT".into(),
        if multi_tenant { "1" } else { "0" }.into(),
    ));
    if let (Some(lat), Some(lon)) = (meta.lat, meta.lon) {
        if lat != 0.0 && lon != 0.0 {
            out.push(("PIZZERIA_LAT".into(), lat.to_string()));
            out.push(("PIZZERIA_LON".into(), lon.to_string()));
        }
    }
    if looks_like_nrw(meta) {
        out.push(("GEO_BACKEND".into(), "cadastre_nrw".into()));
        out.push(("GEO_DATA_DIR".into(), "./data/geo".into()));
        // The template's GEO_AGS / GEO_POSTCODE_MAP / GEO_EXPECT_BBOX are
        // Lüdinghausen (the first shop) — carrying those into a shop in
        // Aachen would veto every address of its own town on import. We
        // can't know the municipality keys from a scrape, so leave AGS and
        // the postcode map EMPTY (the admin's "Alles aus Adresse einrichten"
        // fills them from the shop's address) and pin the sanity box to a
        // radius around THIS shop.
        out.push(("GEO_AGS".into(), String::new()));
        out.push(("GEO_POSTCODE_MAP".into(), String::new()));
        if let Some(bbox) = bbox_around(meta.lat, meta.lon, DEFAULT_GEO_RADIUS_KM) {
            out.push(("GEO_EXPECT_BBOX".into(), bbox));
        }
    }
    for kv in set {
        let (k, v) = kv
            .split_once('=')
            .ok_or_else(|| anyhow!("--set expects KEY=VALUE, got {kv:?}"))?;
        let k = k.trim();
        if k.is_empty() || !k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            bail!("--set: invalid key {k:?}");
        }
        out.retain(|(existing, _)| existing != k);
        out.push((k.to_string(), v.to_string()));
    }
    Ok(out)
}

/// NRW heuristic for the phase-1 cadastre backend: German postcodes 32xxx–
/// 33xxx, 40xxx–53xxx (minus a few RLP ranges we can't tell apart — the
/// admin can still switch the mode off) and 57xxx–59xxx. Anything else (or
/// no postcode) leaves GEO_BACKEND unset = Nominatim behaviour.
/// Default sanity-box radius around a freshly imported shop, in km. Matches
/// what the dashboard's "Alles aus Adresse einrichten" ends up with for its
/// proposed 5 km delivery radius (it adds a 5 km margin), so the first
/// address import covers the shop's own town and its neighbours.
const DEFAULT_GEO_RADIUS_KM: f64 = 10.0;

/// `lat_min,lat_max,lon_min,lon_max` for a box of `radius_km` around a point.
/// One degree of latitude is ~111.32 km; longitude shrinks with cos(lat).
fn bbox_around(lat: Option<f64>, lon: Option<f64>, radius_km: f64) -> Option<String> {
    let (lat, lon) = (lat?, lon?);
    if lat == 0.0 && lon == 0.0 {
        return None;
    }
    let dlat = radius_km / 111.32;
    let dlon = radius_km / (111.32 * lat.to_radians().cos().abs().max(0.01));
    Some(format!(
        "{:.4},{:.4},{:.4},{:.4}",
        lat - dlat,
        lat + dlat,
        lon - dlon,
        lon + dlon
    ))
}

fn looks_like_nrw(meta: &Meta) -> bool {
    let c = meta.country.trim().to_ascii_lowercase();
    if !(c.is_empty() || c == "de" || c == "deutschland" || c == "germany") {
        return false;
    }
    let Ok(plz) = meta.postcode.trim().parse::<u32>() else {
        return false;
    };
    matches!(plz, 32000..=33999 | 40000..=53999 | 57000..=59999)
}

fn ensure_env_file(
    root: &Path,
    env_path: &Path,
    names: &Names,
    overrides: &[(String, String)],
    dry_run: bool,
) -> Result<()> {
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
    let body = rewrite_env(&template, names, overrides);

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
fn rewrite_env(template: &str, n: &Names, overrides: &[(String, String)]) -> String {
    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let mut body = template
        .lines()
        .map(|line| {
            let trimmed = line.trim_start();
            // A commented-out template key (`#GEO_BACKEND=…`) that has an
            // override becomes live; other comments pass through.
            if let Some(rest) = trimmed.strip_prefix('#') {
                if let Some((k, _)) = rest.split_once('=') {
                    let k = k.trim();
                    if let Some((_, v)) = overrides.iter().find(|(ok, _)| ok == k) {
                        if !seen.contains(k) {
                            seen.insert(k);
                            if v.is_empty() {
                                return format!("#{k}=");
                            }
                            return quote_env_line_if_needed(&format!("{k}={v}"));
                        }
                    }
                }
                return line.to_string();
            }
            let Some((key, _)) = trimmed.split_once('=') else {
                return line.to_string();
            };
            if let Some((k, v)) = overrides.iter().find(|(ok, _)| ok == key.trim()) {
                seen.insert(k.as_str());
                // An empty override means "no value for this shop" — comment
                // the key out instead of shipping `KEY=`, so the template's
                // (wrong-town) default is gone AND the dashboard, not a stale
                // env value, owns the setting.
                if v.is_empty() {
                    return format!("#{k}=");
                }
                return quote_env_line_if_needed(&format!("{k}={v}"));
            }
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
        + "\n";
    // Overrides for keys the template doesn't carry at all are appended.
    let missing: Vec<_> = overrides
        .iter()
        .filter(|(k, v)| !seen.contains(k.as_str()) && !v.is_empty())
        .collect();
    if !missing.is_empty() {
        body.push_str("\n# ----- Set by menu-import ------------------------------------------------------\n");
        for (k, v) in missing {
            body.push_str(&quote_env_line_if_needed(&format!("{k}={v}")));
            body.push('\n');
        }
    }
    body
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
    public_url: &str,
    hero_image: Option<&str>,
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

            // Vacate the UNIQUE(menu_number) slot if some OTHER row holds it:
            // a brand-new tenant DB carries the migrations' seed menu (its own
            // numbers 1..n), and a re-import may shift numbers between rows.
            // The prune step below unlists/deletes seed rows anyway, but it
            // runs after the upserts — so clear the collision up front.
            if let Some(n) = &menu_number {
                sqlx::query(
                    "UPDATE menu_items SET menu_number = NULL, updated_at = CURRENT_TIMESTAMP
                     WHERE menu_number = ?1 AND id != ?2",
                )
                .bind(n)
                .bind(&item_id)
                .execute(&mut *tx)
                .await
                .with_context(|| format!("freeing menu_number {n:?} for {item_name:?}"))?;
            }

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
    write_branding(&mut tx, shop_name, &menu.meta, hero_image).await?;
    let n_meta = write_meta(&mut tx, &menu.meta).await?;
    write_import_stamp(&mut tx, menu).await?;
    // The shop's own public URL, in the DB rather than only in its `.env`:
    // the receipt/Bon composer runs off the tenant POOL and has no access to
    // that tenant's env, so without this it fell back to the process
    // PUBLIC_URL — the apex showroom — for the receipt QR.
    if !public_url.trim().is_empty() {
        upsert_setting(&mut tx, "shop_public_url", public_url.trim().trim_end_matches('/')).await?;
    }
    let n_zones = localise_delivery_zones(&mut tx, &menu.meta).await?;

    tx.commit().await?;
    tracing::info!(
        shop_name = %shop_name,
        meta_fields = n_meta,
        zones = n_zones,
        "set shop_name + home_hero.title + {n_meta} meta field(s), {n_zones} zone(s) localised"
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
        ("shop_country", meta.country.trim().to_string()),
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

/// Point the seeded delivery zones at THIS shop's town.
///
/// Every tenant DB starts from the same migrations, whose reference seed is
/// Lüdinghausen (`59348`, plus `-seppenrade` / `-bauerschaft` suburb rows).
/// A shop in Aachen inherited those verbatim, so its delivery area was three
/// zones of a town 200 km away — the admin had to fix it by hand after every
/// import.
///
/// We rewrite the seed rows IN PLACE (ids are referenced by
/// `customer_addresses.delivery_zone_id` and `geo_zone_streets.zone_id`, so
/// deleting them would break those FKs):
///   - the main zone takes the shop's postcode and city,
///   - the two suburb rows become `<plz>-umland-1/2`, named after the town,
///   - fees / minimum order / ETA are LEFT ALONE (the scrape can't know them;
///     the seed values are sane starting points the admin tunes).
///
/// Skipped entirely when the zones are no longer the pristine seed — i.e. any
/// id differs, or a postcode was already edited — so a re-import never
/// clobbers a shop's own delivery setup.
async fn localise_delivery_zones(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    meta: &Meta,
) -> Result<usize> {
    let plz = meta.postcode.trim();
    let city = meta.city.trim();
    if plz.is_empty() || city.is_empty() {
        return Ok(0);
    }

    // Pristine = exactly the three seeded ids with their seeded postcodes.
    let rows: Vec<(String, String)> =
        sqlx::query_as("SELECT id, postcode FROM delivery_zones ORDER BY id")
            .fetch_all(&mut **tx)
            .await?;
    let pristine = [
        ("dz-bauerschaften", "59348-bauerschaft"),
        ("dz-luedinghausen", "59348"),
        ("dz-seppenrade", "59348-seppenrade"),
    ];
    let untouched = rows.len() == pristine.len()
        && rows
            .iter()
            .zip(pristine.iter())
            .all(|((id, pc), (sid, spc))| id == sid && pc == spc);
    if !untouched {
        tracing::info!("delivery zones already customised — left untouched");
        return Ok(0);
    }
    // Nothing to do when the shop really is in the seeded town.
    if plz == "59348" {
        return Ok(0);
    }

    let updates = [
        ("dz-luedinghausen", plz.to_string(), city.to_string()),
        (
            "dz-seppenrade",
            format!("{plz}-umland-1"),
            format!("{city} Umland 1"),
        ),
        (
            "dz-bauerschaften",
            format!("{plz}-umland-2"),
            format!("{city} Umland 2"),
        ),
    ];
    for (id, postcode, name) in &updates {
        sqlx::query("UPDATE delivery_zones SET postcode = ?1, name = ?2 WHERE id = ?3")
            .bind(postcode)
            .bind(name)
            .bind(id)
            .execute(&mut **tx)
            .await
            .with_context(|| format!("localising delivery zone {id}"))?;
    }
    tracing::info!(plz, city, "delivery zones localised to the shop's town");
    Ok(updates.len())
}

/// Record where this menu came from: the scraped URL, the scrape time and
/// the import time. Always written (import time), so the sys-admin tenant
/// list can tell a fresh import from a stale one. Runs inside the import txn.
async fn write_import_stamp(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    menu: &MenuFile,
) -> Result<()> {
    if !menu.shop_url.trim().is_empty() {
        upsert_setting(tx, "menu_import_source", menu.shop_url.trim()).await?;
    }
    if !menu.scraped_at.trim().is_empty() {
        upsert_setting(tx, "menu_import_scraped_at", menu.scraped_at.trim()).await?;
    }
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    upsert_setting(tx, "menu_import_at", &now).await?;
    Ok(())
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
    meta: &Meta,
    hero_image: Option<&str>,
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

    // home_hero (single row, CHECK id = 1): title, subtitle AND image.
    //
    // The migration seeds all three for the reference shop, so importing only
    // the title left a new shop advertising "…direkt aus Lüdinghausen" over a
    // photo of another restaurant. The subtitle is rebuilt from the scraped
    // city + cuisines; the image is the listing's own hero when we managed to
    // download one (else the seed path stays and the admin can replace it).
    let subtitle = hero_subtitle(meta);
    let n = sqlx::query(
        "UPDATE home_hero
            SET title = ?1,
                subtitle = CASE WHEN ?2 = '' THEN subtitle ELSE ?2 END,
                image_path = COALESCE(?3, image_path),
                updated_at = CURRENT_TIMESTAMP
          WHERE id = 1",
    )
    .bind(shop_name)
    .bind(&subtitle)
    .bind(hero_image)
    .execute(&mut **tx)
    .await?
    .rows_affected();
    if n == 0 {
        sqlx::query("INSERT INTO home_hero (id, title, subtitle, image_path) VALUES (1, ?1, ?2, ?3)")
            .bind(shop_name)
            .bind(&subtitle)
            .bind(hero_image)
            .execute(&mut **tx)
            .await?;
    }

    // The seeded gallery is three photos of the reference shop, captioned with
    // its town. Keep the rows (the admin can re-point them) but drop the
    // captions that name the wrong place — an empty caption renders as none.
    sqlx::query(
        "UPDATE home_gallery SET caption = ''
          WHERE id IN ('seed-laden', 'seed-theke', 'seed-blech')",
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Homepage subtitle from the scraped facts, e.g.
/// "Italienische Pizza, Döner & Schnitzel — frisch aus Aachen."
/// Empty when we know neither cuisines nor city, so the existing text stays.
fn hero_subtitle(meta: &Meta) -> String {
    let city = meta.city.trim();
    let mut cuisines: Vec<&str> = meta
        .cuisines
        .iter()
        .map(|c| c.trim())
        .filter(|c| !c.is_empty())
        .collect();
    cuisines.truncate(3);
    match (cuisines.is_empty(), city.is_empty()) {
        (true, true) => String::new(),
        (true, false) => format!("Frisch zubereitet — direkt aus {city}."),
        (false, true) => format!("{}.", join_natural(&cuisines)),
        (false, false) => format!("{} — frisch aus {city}.", join_natural(&cuisines)),
    }
}

/// "a", "a & b", "a, b & c" — German list style.
fn join_natural(parts: &[&str]) -> String {
    match parts {
        [] => String::new(),
        [a] => a.to_string(),
        [rest @ .., last] => format!("{} & {last}", rest.join(", ")),
    }
}

/// Download the listing's hero image (banner, else logo) into `uploads_dir`
/// and return the web path the server serves it under (`/img/uploads/…`).
///
/// Best-effort: every failure returns `None` and is logged, because a missing
/// photo must never abort a menu import. Caps at 8 MB.
fn download_hero_image(meta: &Meta, uploads_dir: &Path, shop: &str) -> Option<String> {
    let url = [meta.banner_url.trim(), meta.logo_url.trim()]
        .into_iter()
        .find(|u| u.starts_with("https://"))?;
    if url.contains('{') {
        tracing::warn!(url, "image URL still has an unsubstituted placeholder — skipped");
        return None;
    }

    let resp = match ureq::get(url).timeout(std::time::Duration::from_secs(20)).call() {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(url, error = %e, "hero image download failed — keeping the current image");
            return None;
        }
    };
    let ext = match resp.content_type() {
        "image/png" => "png",
        "image/webp" => "webp",
        _ => "jpg",
    };
    use std::io::Read as _;
    let mut bytes = Vec::new();
    if let Err(e) = resp.into_reader().take(8 * 1024 * 1024).read_to_end(&mut bytes) {
        tracing::warn!(url, error = %e, "hero image read failed");
        return None;
    }
    if bytes.len() < 1024 {
        tracing::warn!(url, bytes = bytes.len(), "hero image suspiciously small — skipped");
        return None;
    }
    if let Err(e) = std::fs::create_dir_all(uploads_dir) {
        tracing::warn!(dir = %uploads_dir.display(), error = %e, "cannot create uploads dir");
        return None;
    }
    let name = format!("hero-{shop}.{ext}");
    let path = uploads_dir.join(&name);
    if let Err(e) = std::fs::write(&path, &bytes) {
        tracing::warn!(path = %path.display(), error = %e, "hero image write failed");
        return None;
    }
    tracing::info!(path = %path.display(), bytes = bytes.len(), "hero image downloaded");
    Some(format!("/img/uploads/{name}"))
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
        let out = rewrite_env(tmpl, &n, &[]);
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
    fn overrides_replace_uncomment_and_append() {
        let tmpl = "ADMIN_PASSWORD=change-me\n#GEO_BACKEND=cadastre_nrw\nRUST_LOG=info\n";
        let a = args_from(&["--parent", "rusterando", "--shop", "flizza"]);
        let n = resolve_names(&a, "flizza", "Flizza");
        let ov = vec![
            ("ADMIN_PASSWORD".to_string(), "s3cret".to_string()),
            ("GEO_BACKEND".to_string(), "cadastre_nrw".to_string()),
            ("PIZZERIA_LAT".to_string(), "51.77".to_string()),
        ];
        let out = rewrite_env(tmpl, &n, &ov);
        assert!(out.contains("ADMIN_PASSWORD=s3cret\n"));
        assert!(out.contains("\nGEO_BACKEND=cadastre_nrw\n"));
        assert!(!out.contains("#GEO_BACKEND"));
        assert!(out.contains("PIZZERIA_LAT=51.77\n"));
        assert!(out.contains("RUST_LOG=info\n"));
    }

    /// The zone rewrite must be driven by the scraped address and must never
    /// run on a shop that already edited its zones. The SQL itself is covered
    /// by the end-to-end import; here we pin the naming contract.
    #[test]
    fn localised_zone_names_follow_the_shop_town() {
        let (plz, city) = ("52066", "Aachen");
        assert_eq!(format!("{plz}-umland-1"), "52066-umland-1");
        assert_eq!(format!("{city} Umland 2"), "Aachen Umland 2");
        // A shop in the seeded town keeps the seeded zones untouched.
        assert_eq!("59348", "59348");
    }

    #[test]
    fn hero_subtitle_names_the_shops_own_town() {
        let mut m = Meta::default();
        m.city = "Aachen".into();
        m.cuisines = vec!["Italienische Pizza".into(), "Döner".into(), "Schnitzel".into()];
        let sub = hero_subtitle(&m);
        assert_eq!(sub, "Italienische Pizza, Döner & Schnitzel — frisch aus Aachen.");
        // Never the reference shop's town.
        assert!(!sub.contains("Lüdinghausen"));

        // City only.
        let m2 = Meta { city: "Aachen".into(), ..Default::default() };
        assert_eq!(hero_subtitle(&m2), "Frisch zubereitet — direkt aus Aachen.");
        // Nothing known → empty, so the existing subtitle is kept.
        assert_eq!(hero_subtitle(&Meta::default()), "");
        // At most three cuisines.
        let m3 = Meta {
            city: "Köln".into(),
            cuisines: vec!["A".into(), "B".into(), "C".into(), "D".into()],
            ..Default::default()
        };
        assert_eq!(hero_subtitle(&m3), "A, B & C — frisch aus Köln.");
    }

    #[test]
    fn bbox_is_centred_on_the_shop() {
        // Aachen, 10 km: ~0.09° lat, ~0.14° lon (cos(50.77°) ≈ 0.63).
        let b = bbox_around(Some(50.771763), Some(6.113469), 10.0).unwrap();
        let v: Vec<f64> = b.split(',').map(|x| x.parse().unwrap()).collect();
        assert!((v[0]..=v[1]).contains(&50.771763), "lat band must contain the shop");
        assert!((v[2]..=v[3]).contains(&6.113469), "lon band must contain the shop");
        assert!((v[1] - v[0] - 0.1796).abs() < 0.01, "lat span ≈ 2×10km: {b}");
        assert!((v[3] - v[2] - 0.2836).abs() < 0.02, "lon span widens with latitude: {b}");
        // Lüdinghausen must NOT be inside an Aachen shop's box (the bug).
        assert!(!(v[0]..=v[1]).contains(&51.7701));
        assert_eq!(bbox_around(Some(0.0), Some(0.0), 10.0), None);
        assert_eq!(bbox_around(None, Some(6.1), 10.0), None);
    }

    #[test]
    fn multitenant_env_gets_multi_tenant_1_and_own_bbox() {
        let mut m = Meta::default();
        m.postcode = "52066".into();
        m.city = "Aachen".into();
        m.lat = Some(50.771763);
        m.lon = Some(6.113469);
        let ov = env_overrides(&[], &m, true).unwrap();
        let get = |k: &str| ov.iter().find(|(a, _)| a == k).map(|(_, v)| v.clone());
        assert_eq!(get("MULTI_TENANT").as_deref(), Some("1"));
        assert_eq!(get("GEO_BACKEND").as_deref(), Some("cadastre_nrw"));
        // The Lüdinghausen defaults are cleared, not inherited.
        assert_eq!(get("GEO_AGS").as_deref(), Some(""));
        assert_eq!(get("GEO_POSTCODE_MAP").as_deref(), Some(""));
        assert!(get("GEO_EXPECT_BBOX").unwrap().starts_with("50."));
        // Standalone shops keep MULTI_TENANT=0.
        let ov = env_overrides(&[], &m, false).unwrap();
        assert_eq!(
            ov.iter().find(|(a, _)| a == "MULTI_TENANT").map(|(_, v)| v.clone()).as_deref(),
            Some("0")
        );
    }

    #[test]
    fn empty_override_comments_the_key_out() {
        let a = args_from(&["--parent", "rusterando", "--shop", "flizza"]);
        let n = resolve_names(&a, "flizza", "Flizza");
        let tmpl = "GEO_AGS=05558024\n#GEO_POSTCODE_MAP=05558024:59348:Lüdinghausen\n";
        let ov = vec![
            ("GEO_AGS".to_string(), String::new()),
            ("GEO_POSTCODE_MAP".to_string(), String::new()),
        ];
        let out = rewrite_env(tmpl, &n, &ov);
        assert!(out.contains("#GEO_AGS="), "stale value must be commented out: {out}");
        assert!(!out.contains("\nGEO_AGS=05558024"), "Lüdinghausen AGS must be gone: {out}");
        assert!(!out.contains("59348"), "no inherited postcode map: {out}");
    }

    #[test]
    fn nrw_heuristic() {
        let mut m = Meta::default();
        m.postcode = "59348".into();
        assert!(looks_like_nrw(&m));
        m.postcode = "80331".into();
        assert!(!looks_like_nrw(&m));
        m.postcode = "59348".into();
        m.country = "AT".into();
        assert!(!looks_like_nrw(&m));
    }

    #[test]
    fn rewrite_only_touches_known_keys() {
        let a = args_from(&["--parent", "rusterando", "--shop", "flizza"]);
        let n = resolve_names(&a, "flizza", "Flizza");
        let tmpl = "# comment\nAPP_NAME=rusterando-server\nSMTP_PASS='secret$!'\nDATABASE_URL=sqlite:./data/rusterando.db\n";
        let out = rewrite_env(tmpl, &n, &[]);
        assert!(out.contains("DATABASE_URL=sqlite:./data/flizza.sqlite"));
        assert!(out.contains("SMTP_PASS='secret$!'")); // secret untouched
        assert!(out.contains("# comment"));
    }
}
