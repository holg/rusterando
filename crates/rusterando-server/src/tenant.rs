//! Tenancy mode detection — single-tenant (Model A) vs in-process
//! multi-tenant (Model B). See `docs/multi_tenant.md`.
//!
//! Model A is the **fail-safe default**: a real shop (davidspizzeria.de) must
//! never be dragged into multi-tenant mode by accident. Model B requires an
//! unambiguous positive signal. This module owns the gate logic and produces
//! the [`DeploymentInfo`] snapshot the admin diagnostics panel reads.
//!
//! This first pass DETECTS + REPORTS the mode (and powers the loud boot log +
//! the admin panel). The in-process per-request tenant registry (Model B's
//! request routing) is a follow-up; until then a multi-tenant boot is surfaced
//! but the process still serves the single configured DB. The gate is built so
//! that wiring the registry later does not change the detection contract.

use rusterando_frontend::pages::settings::DeploymentInfo;

/// Resolve the runtime deployment snapshot from env + the working directory.
/// Pure except for the `.env*` glob + env reads — call once at boot.
pub fn detect() -> DeploymentInfo {
    let db_url =
        std::env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite:./data/rusterando.db".into());
    let tenant_slug = db_stem(&db_url);
    let service_name = std::env::var("LEPTOS_OUTPUT_NAME").unwrap_or_else(|_| tenant_slug.clone());
    let site_addr = std::env::var("LEPTOS_SITE_ADDR").unwrap_or_default();

    let env_file = std::env::var("ENV_FILE").ok().filter(|s| !s.is_empty());
    let kill_switch = std::env::var("MULTI_TENANT")
        .map(|v| v == "0")
        .unwrap_or(false);

    // The env files visible in the working directory. bare `.env` counts.
    let env_files = glob_env_files();

    // --- The 4-gate rule (first match wins; Model A is the default) ---------
    // 1. MULTI_TENANT=0 kill-switch  -> single-tenant, forced.
    // 2. ENV_FILE pinned             -> single-tenant, forced.
    // 3. count `.env*` <= 1          -> single-tenant.
    // 4. >=2 env files               -> multi-tenant candidate; the registry
    //    (follow-up) enforces distinct DBs and may fall back to single. For
    //    DETECTION we report multi-tenant here.
    let multi_tenant = !kill_switch && env_file.is_none() && env_files.len() >= 2;

    // What we report as "loaded": the pinned ENV_FILE, else the discovered set
    // (single-tenant collapses to the one file actually in effect).
    let reported_env_files = match (&env_file, multi_tenant) {
        (Some(f), _) => vec![f.clone()],
        (None, true) => env_files,
        (None, false) => {
            // Single-tenant: the bare `.env` if present, else whatever the one
            // discovered file was, else "(env injected)" for prod systemd
            // which sets the environment directly without a file on disk.
            if env_files.is_empty() {
                vec!["(env injected by systemd)".to_string()]
            } else {
                env_files
            }
        }
    };

    DeploymentInfo {
        multi_tenant,
        env_files: reported_env_files,
        tenant_slug,
        service_name,
        database_url: db_url,
        site_addr,
        // host + subdomain are per-request — filled in by the diagnostics
        // server fn from the live headers, not known at boot.
        host: String::new(),
        subdomain: String::new(),
    }
}

/// One-line summary for the boot log. Single-tenant is info-level; multi-tenant
/// is warn-level so a real shop that unexpectedly trips it is obvious in the
/// journal.
pub fn log_mode(info: &DeploymentInfo) {
    if info.multi_tenant {
        tracing::warn!(
            "MODE: multi-tenant — {} env files: {}",
            info.env_files.len(),
            info.env_files.join(", ")
        );
    } else {
        tracing::info!(
            "MODE: single-tenant ({}) — db {} — env {}",
            info.tenant_slug,
            info.database_url,
            info.env_files.join(", ")
        );
    }
}

/// `sqlite:./data/davidspizzeria.db?foo=1` -> `davidspizzeria`.
fn db_stem(db_url: &str) -> String {
    let path = db_url
        .trim_start_matches("sqlite:")
        .trim_start_matches("//")
        .split('?')
        .next()
        .unwrap_or("");
    std::path::Path::new(path)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "rusterando".to_string())
}

/// All `.env*` files in the current working directory (sorted, deduped).
/// Bare `.env` counts toward the multi-tenant trigger. Backup-ish siblings
/// (`.env.bak`, `.env.example`, `.env.*.example`) are excluded so a stray
/// template can't flip a real shop — only real profile files count.
fn glob_env_files() -> Vec<String> {
    let mut out: Vec<String> = glob::glob(".env*")
        .map(|paths| {
            paths
                .filter_map(Result::ok)
                .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
                .filter(|name| !is_non_profile_env(name))
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out.dedup();
    out
}

/// Exclude obvious non-profile files so they don't count toward the trigger:
/// `.env.example`, `.env.demo.example`, `.env.bak`, editor swap files.
fn is_non_profile_env(name: &str) -> bool {
    name.ends_with(".example")
        || name.ends_with(".bak")
        || name.ends_with('~')
        || name.ends_with(".swp")
        || name.contains(".example.")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn db_stem_strips_prefix_and_query() {
        assert_eq!(db_stem("sqlite:./data/davidspizzeria.db"), "davidspizzeria");
        assert_eq!(db_stem("sqlite://data/flizza.sqlite?x=1"), "flizza");
        assert_eq!(db_stem("sqlite:"), "rusterando");
    }

    #[test]
    fn non_profile_env_files_excluded() {
        assert!(is_non_profile_env(".env.example"));
        assert!(is_non_profile_env(".env.demo.example"));
        assert!(is_non_profile_env(".env.bak"));
        assert!(!is_non_profile_env(".env"));
        assert!(!is_non_profile_env(".env.davids"));
        assert!(!is_non_profile_env(".env.rusterando.flizza"));
    }
}
