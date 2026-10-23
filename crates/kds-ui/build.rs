fn main() {
    println!("cargo:rerun-if-env-changed=KDS_UI_SCALE");
    let mut config = slint_build::CompilerConfiguration::new();
    if std::env::var_os("CARGO_FEATURE_MCU").is_some() {
        // A microcontroller has no font rasterizer: pre-render the glyphs, at
        // the screen's fixed scale factor (the 480x272 layout scaled up). As
        // signed distance fields: one set per font, scaled to every size at
        // run time, instead of a bitmap set per size (3.8 MB on the Tab5).
        let scale: f32 = std::env::var("KDS_UI_SCALE")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(1.0);
        config = config
            .embed_resources(slint_build::EmbedResourcesKind::EmbedForSoftwareRenderer)
            .with_scale_factor(scale)
            .with_sdf_fonts(true);
    }
    slint_build::compile_with_config("ui/kitchen.slint", config).expect("compile ui/kitchen.slint");
}
