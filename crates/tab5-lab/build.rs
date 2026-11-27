fn main() {
    embuild::espidf::sysenv::output();
    // Commands run at start (src/main.rs).
    println!("cargo:rerun-if-env-changed=LAB_AUTORUN");
    // The phone terminal's key (src/remote.rs).
    println!("cargo:rerun-if-env-changed=LAB_KEY");
    // The shop server the LoRa events go to (src/report.rs).
    println!("cargo:rerun-if-env-changed=SHOP_URL");
}
