//! The README screenshots: the kitchen screen's three pages with the demo
//! orders, drawn the way the Tab5 draws them — Slint's software renderer,
//! 1280×720 at scale 2.647, the same pre-rendered fonts, the taller tabs:
//!
//! ```sh
//! KDS_UI_SCALE=2.647 cargo run --release --features mcu --example screenshots -- ../kds-tab5/screenshots
//! ```

use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use kds_ui::{Core, KitchenWindow};
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, WindowAdapter};
use slint::{ComponentHandle, PhysicalSize, Rgb8Pixel};

const W: u32 = 1280;
const H: u32 = 720;

struct Shots {
    window: Rc<MinimalSoftwareWindow>,
    start: Instant,
}

impl Platform for Shots {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.window.clone())
    }

    fn duration_since_start(&self) -> Duration {
        self.start.elapsed()
    }
}

/// What the Tab5 adds to every frame, with sample values.
struct Tab5Look;

impl kds_ui::Host for Tab5Look {
    fn decorate(&self, w: &KitchenWindow, _: &Core) {
        w.set_printer_ok(true);
        w.set_printer_text("Drucker".into());
        w.set_can_test_print(true);
        w.set_test_print_text("Testdruck (Epson)".into());
        w.set_has_saver(false);
        w.set_extra_text("Akku schonen: an".into());
        w.set_info_text("tab5-a1b2c3 · Akku 88 % · 8.14 V · geschont · WLAN".into());
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| ".".into()));
    std::fs::create_dir_all(&out)?;

    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    window.set_size(PhysicalSize::new(W, H));
    slint::platform::set_platform(Box::new(Shots {
        window: window.clone(),
        start: Instant::now(),
    }))
    .map_err(|e| format!("slint platform: {e:?}"))?;

    let kitchen = KitchenWindow::new()?;
    kitchen.set_header_height(32.0);
    kds_ui::wire(&kitchen, Rc::new(Tab5Look), |_| {});
    kds_ui::start_demo();
    kitchen.show()?;

    for (page, name) in [(0, "start"), (1, "kueche"), (2, "zeiten")] {
        kitchen.invoke_go_page(page);
        kds_ui::render(&kitchen);
        // Let colour transitions finish before the picture.
        std::thread::sleep(Duration::from_millis(600));
        slint::platform::update_timers_and_animations();
        window.request_redraw();

        let mut frame = vec![Rgb8Pixel::default(); (W * H) as usize];
        window.draw_if_needed(|renderer| {
            renderer.render(&mut frame, W as usize);
        });
        let bytes: Vec<u8> = frame.iter().flat_map(|p| [p.r, p.g, p.b]).collect();

        let path = out.join(format!("{name}.png"));
        let mut png =
            png::Encoder::new(std::io::BufWriter::new(std::fs::File::create(&path)?), W, H);
        png.set_color(png::ColorType::Rgb);
        png.set_depth(png::BitDepth::Eight);
        png.write_header()?.write_image_data(&bytes)?;
        println!("{}", path.display());
    }
    Ok(())
}
