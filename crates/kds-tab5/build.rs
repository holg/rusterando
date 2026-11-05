fn main() {
    embuild::espidf::sysenv::output();
    // Wi-Fi credentials come from tmp/tab5-wifi.env via
    // scripts/build_kds_tab5.sh; rebuild when they change.
    for n in ["", "2", "3", "4", "5"] {
        println!("cargo:rerun-if-env-changed=WIFI{n}_SSID");
        println!("cargo:rerun-if-env-changed=WIFI{n}_PASS");
    }
    println!("cargo:rerun-if-env-changed=KITCHEN_PASSWORD");
    println!("cargo:rerun-if-env-changed=SHOP_URL");
    println!("cargo:rerun-if-env-changed=KITCHEN_SLUG");
    // The JC1060P470's Wi-Fi hotspot (src/jc1060p470/net.rs).
    println!("cargo:rerun-if-env-changed=HOTSPOT_SSID");
    println!("cargo:rerun-if-env-changed=HOTSPOT_PASS");
    println!("cargo:rerun-if-env-changed=KDS_VERSION");
    println!("cargo:rerun-if-env-changed=KDS_DRY_RENDER");
    replay_job();
}

/// Diagnostic builds: `KDS_REPLAY_JOB=<file>` names one stored print job
/// (a `kitchen_outbox` payload as hex, e.g. from `sqlite3 … hex(payload)`),
/// compiled in as OUT_DIR/replay_job.bin and run once at start (printing.rs).
/// Without it the file is empty and nothing is replayed.
fn replay_job() {
    println!("cargo:rerun-if-env-changed=KDS_REPLAY_JOB");
    let out = std::path::Path::new(&std::env::var("OUT_DIR").unwrap()).join("replay_job.bin");
    let bytes = match std::env::var("KDS_REPLAY_JOB") {
        Ok(path) if !path.is_empty() => {
            println!("cargo:rerun-if-changed={path}");
            let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("KDS_REPLAY_JOB {path}: {e}"));
            let hex: String = text.chars().filter(|c| c.is_ascii_hexdigit()).collect();
            assert!(hex.len() % 2 == 0 && !hex.is_empty(), "KDS_REPLAY_JOB {path}: not a hex dump");
            (0..hex.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
                .collect()
        }
        _ => Vec::new(),
    };
    std::fs::write(out, bytes).unwrap();
}
