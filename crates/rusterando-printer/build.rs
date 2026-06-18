// Capture the target triple at build time so the Pi can report its arch
// in the kitchen Hello (the server uses it to pick the right update
// artifact). cargo sets TARGET for build scripts; we re-export it as a
// compile-time env the binary reads via env!("BUILD_TARGET").
fn main() {
    let target = std::env::var("TARGET").unwrap_or_else(|_| "unknown".to_string());
    println!("cargo:rustc-env=BUILD_TARGET={target}");
}
