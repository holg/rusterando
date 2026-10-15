//! `geo-import` — produce a shop's local address index from the NRW cadastre
//! export, driven by parameters so it works for any municipality in NRW.
//!
//! ```text
//! # one-time, anywhere: cut the 100 MB export into one file per municipality
//! geo-import split --download --out-dir ./data/geo/nrw
//! geo-import split --source gebref_EPSG25832_ASCII.zip --out-dir ./data/geo/nrw
//!
//! # a tenant, from its env file (GEO_* + DATABASE_URL)
//! geo-import import --env-file .env --download
//! geo-import import --tenant flizza --source ./data/geo/nrw/05558024.txt
//!
//! # any shop, purely from parameters, into any SQLite file
//! geo-import import --source ./data/geo/nrw/05558024.txt \
//!     --ags 05558024 --postcode-map 05558024:59348:Lüdinghausen \
//!     --center 51.7701,7.4419 --radius-m 6000 --db sqlite:./data/newshop.sqlite
//!
//! geo-import import --sample                 # built-in fixture, offline
//! ```
//!
//! `import` keeps only addresses inside the delivery area: the tenant's
//! zones (radius / polygon / default) when the DB has some, or the
//! `--center`/`--radius-m` circle given on the command line. Streets the
//! shop never delivers to are not stored at all. It exits non-zero without
//! writing when zero rows were kept or the centroid falls outside the
//! expected box. Restart the tenant (or "Index neu laden" in /admin/zones)
//! afterwards. Running `geo-import` with no subcommand means `import`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use clap::{Args, Parser, Subcommand};
use rusterando_geo::import::{
    download_nrw, nrw_zip_path, split_by_ags, AddressSource, CadastreNrw, ImportResult,
};
use rusterando_geo::{sample, store, GeoConfig, Geometry, GeometryKind, ZoneDef};

#[derive(Parser, Debug)]
#[command(
    name = "geo-import",
    about = "NRW cadastre → local address index for Rusterando shops"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Import one shop's addresses into its SQLite database.
    Import(ImportArgs),
    /// Cut the whole-NRW export into one file per municipality (AGS).
    Split(SplitArgs),
}

#[derive(Args, Debug)]
struct SourceArgs {
    /// Data source(s): the official zip, an extracted gebref.txt, or
    /// municipality files from `split` (repeat for several municipalities).
    #[arg(long, alias = "zip", num_args = 1..)]
    source: Vec<PathBuf>,
    /// Download the official export into the data dir first.
    #[arg(long)]
    download: bool,
    /// Download cache dir (default: GEO_DATA_DIR, else ./tmp/geo).
    #[arg(long)]
    data_dir: Option<PathBuf>,
}

#[derive(Args, Debug)]
struct ImportArgs {
    #[command(flatten)]
    src: SourceArgs,
    /// Tenant slug: resolves `.env.<slug>` / `.env.rusterando.<slug>` / `.env`.
    #[arg(long)]
    tenant: Option<String>,
    /// Explicit env file (overrides --tenant resolution).
    #[arg(long)]
    env_file: Option<PathBuf>,
    /// Target database URL (overrides DATABASE_URL), e.g. sqlite:./data/shop.sqlite
    #[arg(long)]
    db: Option<String>,
    /// Import the built-in fixture instead of a file (offline / smoke test).
    #[arg(long)]
    sample: bool,
    /// Municipality keys, comma-separated (overrides GEO_AGS).
    #[arg(long)]
    ags: Option<String>,
    /// AGS:PLZ:City triples, comma-separated (overrides GEO_POSTCODE_MAP).
    #[arg(long)]
    postcode_map: Option<String>,
    /// Delivery-area centre "lat,lon" — with --radius-m, keeps only
    /// addresses inside that circle (instead of the DB's zones).
    #[arg(long)]
    center: Option<String>,
    /// Radius in metres around --center.
    #[arg(long)]
    radius_m: Option<f64>,
    /// Import sanity box "lat_min,lat_max,lon_min,lon_max" (overrides GEO_EXPECT_BBOX).
    #[arg(long)]
    expect_bbox: Option<String>,
    /// Store every address of the configured municipalities, ignoring zones.
    #[arg(long)]
    no_zone_filter: bool,
    /// Parse + check only; do not write the database.
    #[arg(long)]
    dry_run: bool,
}

#[derive(Args, Debug)]
struct SplitArgs {
    #[command(flatten)]
    src: SourceArgs,
    /// Output directory for `<AGS>.txt` files + `index.csv`.
    #[arg(long)]
    out_dir: PathBuf,
}

fn resolve_env_file(tenant: Option<&str>, env_file: Option<&PathBuf>) -> Option<PathBuf> {
    if let Some(p) = env_file {
        return Some(p.clone());
    }
    let slug = tenant?;
    for cand in [
        format!(".env.{slug}"),
        format!(".env.rusterando.{slug}"),
        ".env".to_string(),
    ] {
        if Path::new(&cand).is_file() {
            return Some(PathBuf::from(cand));
        }
    }
    None
}

/// The files to read: `--source …`, or the (cached) download.
fn resolve_sources(src: &SourceArgs, data_dir_env: Option<&str>) -> anyhow::Result<Vec<PathBuf>> {
    if src.download {
        let dir = src
            .data_dir
            .clone()
            .or_else(|| data_dir_env.map(PathBuf::from))
            .unwrap_or_else(|| PathBuf::from("./tmp/geo"));
        let dest = nrw_zip_path(&dir);
        download_nrw(&dest, false, &mut |line: String| {
            println!("{}", line.trim_start_matches('\r'))
        })?;
        return Ok(vec![dest]);
    }
    anyhow::ensure!(
        !src.source.is_empty(),
        "pass --source <zip|txt>…, --download or --sample"
    );
    Ok(src.source.clone())
}

fn parse_latlon(s: &str) -> anyhow::Result<(f64, f64)> {
    let v: Vec<f64> = s
        .split(',')
        .map(|x| x.trim().parse::<f64>())
        .collect::<Result<_, _>>()?;
    anyhow::ensure!(v.len() == 2, "--center needs \"lat,lon\"");
    Ok((v[0], v[1]))
}

async fn open_pool(database_url: &str) -> anyhow::Result<sqlx::SqlitePool> {
    let opts: sqlx::sqlite::SqliteConnectOptions = database_url
        .parse::<sqlx::sqlite::SqliteConnectOptions>()?
        .create_if_missing(true)
        .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
        .busy_timeout(std::time::Duration::from_secs(10));
    Ok(sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(2)
        .connect_with(opts)
        .await?)
}

async fn cmd_import(a: ImportArgs) -> anyhow::Result<()> {
    // ---- config: env file, overridden by flags ------------------------------
    let env_file = resolve_env_file(a.tenant.as_deref(), a.env_file.as_ref());
    let mut vars: HashMap<String, String> = match &env_file {
        Some(p) => {
            println!("env file: {}", p.display());
            dotenvy::from_path_iter(p)
                .map_err(|e| anyhow::anyhow!("read {}: {e}", p.display()))?
                .filter_map(Result::ok)
                .collect()
        }
        None => HashMap::new(),
    };
    for (key, val) in [
        ("GEO_AGS", a.ags.clone()),
        ("GEO_POSTCODE_MAP", a.postcode_map.clone()),
        ("GEO_EXPECT_BBOX", a.expect_bbox.clone()),
        ("DATABASE_URL", a.db.clone()),
    ] {
        if let Some(v) = val {
            vars.insert(key.to_string(), v);
        }
    }
    if a.ags.is_some() && !vars.contains_key("GEO_BACKEND") {
        vars.insert("GEO_BACKEND".into(), "cadastre_nrw".into());
    }
    let cfg = match GeoConfig::from_vars(&vars) {
        Some(c) => c,
        None if a.sample => {
            println!("no GEO_* config — using the fixture's own (Lüdinghausen)");
            sample::config()
        }
        None => anyhow::bail!(
            "no GEO_BACKEND / --ags given — set GEO_* in the env file or pass --ags/--postcode-map"
        ),
    };
    anyhow::ensure!(!cfg.ags.is_empty(), "GEO_AGS / --ags is empty");
    anyhow::ensure!(
        cfg.backend == "cadastre_nrw",
        "unsupported GEO_BACKEND {:?}",
        cfg.backend
    );

    // ---- read -----------------------------------------------------------------
    let mut result: ImportResult = if a.sample {
        println!("source: built-in sample fixture");
        CadastreNrw::import_reader(&cfg, sample::SAMPLE_UTF8.as_bytes())?
    } else {
        let paths = resolve_sources(&a.src, cfg.data_dir.as_deref())?;
        let mut merged: Option<ImportResult> = None;
        for path in &paths {
            println!("source: {}", path.display());
            let r = CadastreNrw.build_index(&cfg, path)?;
            match &mut merged {
                None => merged = Some(r),
                Some(m) => m.merge(r),
            }
        }
        merged.expect("at least one source")
    };
    result.summary.check(cfg.expect_bbox)?;

    // ---- database -------------------------------------------------------------
    let database_url = vars.get("DATABASE_URL").cloned().unwrap_or_else(|| {
        let slug = a.tenant.clone().unwrap_or_else(|| "rusterando".into());
        format!("sqlite:./data/{slug}.sqlite")
    });
    println!("database: {database_url}");
    let pool = open_pool(&database_url).await?;
    sqlx::migrate!().run(&pool).await?;

    // ---- keep only the delivery area ------------------------------------------
    let (zones, overrides) = match (&a.center, a.radius_m) {
        (Some(c), Some(r)) => {
            let (lat, lon) = parse_latlon(c)?;
            anyhow::ensure!(r > 0.0, "--radius-m must be > 0");
            println!("filter: circle {lat:.5},{lon:.5} r={r:.0} m (from the command line)");
            (
                vec![ZoneDef {
                    id: "cli-radius".into(),
                    name: "cli-radius".into(),
                    priority: 0,
                    fee_cents: 0,
                    min_order_cents: None,
                    eta_minutes: 0,
                    active: true,
                    kind: GeometryKind::Radius,
                    geometry: Some(Geometry::Radius {
                        lat,
                        lon,
                        metres: r,
                    }),
                }],
                Vec::new(),
            )
        }
        (Some(_), None) | (None, Some(_)) => anyhow::bail!("--center and --radius-m go together"),
        (None, None) if a.no_zone_filter => (Vec::new(), Vec::new()),
        (None, None) => {
            let z = store::load_zones(&pool).await?;
            if z.0.iter().any(|z| z.active && z.kind != GeometryKind::None) {
                println!("filter: the database's delivery zones");
            } else {
                println!("filter: none (no zone geometry configured — storing all addresses)");
            }
            z
        }
    };
    result.retain_in_zones(&zones, &overrides);
    println!("\n{}", result.summary.report());
    anyhow::ensure!(
        result.summary.addresses > 0,
        "no address lies inside the delivery area — check the zone geometry / --center"
    );

    if a.dry_run {
        println!("dry run — database untouched");
        return Ok(());
    }
    let (ns, na) = store::replace_import(&pool, &result.streets).await?;
    println!("written: {ns} streets, {na} addresses → {database_url}");
    println!("done — restart the tenant or click „Index neu laden“ in /admin/zones");
    Ok(())
}

fn cmd_split(a: SplitArgs) -> anyhow::Result<()> {
    let paths = resolve_sources(&a.src, std::env::var("GEO_DATA_DIR").ok().as_deref())?;
    anyhow::ensure!(paths.len() == 1, "split takes exactly one source");
    let path = &paths[0];
    println!(
        "source: {}\nout:    {}",
        path.display(),
        a.out_dir.display()
    );
    let t0 = std::time::Instant::now();
    let s = split_by_ags(path, &a.out_dir)?;
    println!(
        "split {} rows into {} municipality files in {:.1} s — see {}",
        s.rows,
        s.municipalities.len(),
        t0.elapsed().as_secs_f64(),
        a.out_dir.join("index.csv").display()
    );
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();
    // Backwards compatible: `geo-import --env-file … --zip …` means `import`.
    let mut argv: Vec<String> = std::env::args().collect();
    match argv.get(1).map(String::as_str) {
        None | Some("-h") | Some("--help") | Some("-V") | Some("--version") | Some("import")
        | Some("split") => {}
        Some(_) => argv.insert(1, "import".to_string()),
    }
    match Cli::parse_from(argv).cmd {
        Cmd::Import(a) => cmd_import(a).await,
        Cmd::Split(a) => cmd_split(a),
    }
}
