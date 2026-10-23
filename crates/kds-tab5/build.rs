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
}
