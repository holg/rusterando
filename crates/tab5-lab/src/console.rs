//! The terminal screen: 80×24 cells of ProFont 24 pt (16×29 px) on the
//! Tab5's panel turned to landscape (1280×720), drawn straight into the
//! panel's frame buffer — only cells that changed, then a cache write-back
//! of the panel rows they cover (ESP-IDF's DPI driver does no copy when
//! the buffer passed lies inside its frame buffer).
//!
//! Layout: row 0 status bar, rows 1–22 output (scrolling), row 23 input.
//!
//! The panel is 720×1280 portrait. Landscape (x, y) → panel:
//! normal `(719 − y, x)`, upside down `(y, 1279 − x)` — the same two
//! positions as the kitchen screen (imu.rs decides).

use anyhow::{ensure, Result};
use embedded_graphics::mono_font::MonoTextStyle;
use embedded_graphics::pixelcolor::BinaryColor;
use embedded_graphics::prelude::*;
use embedded_graphics::text::{Baseline, Text};
use esp_idf_svc::sys::{bsp, esp};

use crate::terminal::{Terminal, Tone};

pub const COLS: usize = 80;
pub const ROWS: usize = 24;
pub const OUTPUT_ROWS: usize = ROWS - 2;
const CELL_W: usize = 16;
const CELL_H: usize = 29;
const PANEL_W: usize = bsp::BSP_LCD_H_RES as usize; // 720
const PANEL_H: usize = bsp::BSP_LCD_V_RES as usize; // 1280
const LAND_H: usize = PANEL_W;
/// Top margin: 24 rows of 29 px leave 24 px.
const TOP: usize = (LAND_H - ROWS * CELL_H) / 2;

const fn rgb(r: u8, g: u8, b: u8) -> u16 {
    ((r as u16 >> 3) << 11) | ((g as u16 >> 2) << 5) | (b as u16 >> 3)
}

pub const BG: u16 = rgb(0x12, 0x14, 0x18);
const BAR_BG: u16 = rgb(0x1f, 0x4e, 0x86);
const BAR_FG: u16 = rgb(0xff, 0xff, 0xff);

/// The colour of a line's tone.
fn color(tone: Tone) -> u16 {
    match tone {
        Tone::Normal => rgb(0xd0, 0xd0, 0xd0),
        Tone::Dim => rgb(0x80, 0x88, 0x90),
        Tone::Echo => rgb(0xff, 0xff, 0xff),
        Tone::Ok => rgb(0x6f, 0xdc, 0x6f),
        Tone::Rx => rgb(0x5f, 0xd7, 0xff),
        Tone::Tx => rgb(0xff, 0xd7, 0x5f),
        Tone::Warn => rgb(0xff, 0xa0, 0x40),
        Tone::Err => rgb(0xff, 0x60, 0x60),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Cell {
    ch: char,
    fg: u16,
    bg: u16,
}

const BLANK: Cell = Cell {
    ch: ' ',
    fg: 0,
    bg: BG,
};

/// The 80×24 grid for a terminal state: status bar, the newest output
/// rows (lines wrapped at 80 columns), the input row with its cursor.
fn compose(term: &Terminal) -> Vec<[Cell; COLS]> {
    let mut grid = vec![[BLANK; COLS]; ROWS];
    // Status bar.
    grid[0] = [Cell {
        ch: ' ',
        fg: BAR_FG,
        bg: BAR_BG,
    }; COLS];
    for (i, c) in term.status.chars().take(COLS).enumerate() {
        grid[0][i].ch = c;
    }
    // Output, newest at the bottom: wrap from the newest line back until
    // the rows are full.
    let mut rows: Vec<Vec<Cell>> = Vec::new();
    for (tone, line) in term.lines().iter().rev() {
        let fg = color(*tone);
        let cells: Vec<Cell> = line.chars().map(|ch| Cell { ch, fg, bg: BG }).collect();
        let mut chunks: Vec<Vec<Cell>> = cells.chunks(COLS).map(|c| c.to_vec()).collect();
        if chunks.is_empty() {
            chunks.push(Vec::new());
        }
        for chunk in chunks.into_iter().rev() {
            rows.push(chunk);
        }
        if rows.len() >= OUTPUT_ROWS {
            break;
        }
    }
    rows.truncate(OUTPUT_ROWS);
    for (r, row) in rows.iter().rev().enumerate() {
        for (i, cell) in row.iter().enumerate() {
            grid[1 + r][i] = *cell;
        }
    }
    // Input: prompt, then the part of the line around the cursor.
    let row = ROWS - 1;
    let fg = color(Tone::Echo);
    let prompt: Vec<char> = term.prompt.chars().collect();
    let input: Vec<char> = term.input.chars().collect();
    let room = COLS.saturating_sub(prompt.len() + 1).max(1);
    let start = term.cursor.saturating_sub(room - 1).min(input.len());
    let mut col = 0;
    for c in &prompt {
        if col < COLS {
            grid[row][col] = Cell {
                ch: *c,
                fg: color(Tone::Ok),
                bg: BG,
            };
            col += 1;
        }
    }
    let cursor_col = col + (term.cursor - start);
    for c in input.iter().skip(start) {
        if col >= COLS {
            break;
        }
        grid[row][col] = Cell { ch: *c, fg, bg: BG };
        col += 1;
    }
    if cursor_col < COLS {
        let cell = &mut grid[row][cursor_col];
        *cell = Cell {
            ch: cell.ch,
            fg: BG,
            bg: fg,
        };
    }
    grid
}

pub struct Screen {
    panel: bsp::esp_lcd_panel_handle_t,
    fb: *mut u16,
    glyphs: Glyphs,
    shown: Vec<[Cell; COLS]>,
    flipped: bool,
}

impl Screen {
    /// Bring up panel and backlight (needs board::init first) and clear it.
    pub fn new(brightness: i32) -> Result<Self> {
        // SAFETY: plain C config struct, fully set below.
        let mut cfg: bsp::bsp_display_config_t = unsafe { core::mem::zeroed() };
        cfg.dsi_bus.phy_clk_src = 0; // driver default
        cfg.dsi_bus.lane_bit_rate_mbps = bsp::BSP_LCD_MIPI_DSI_LANE_BITRATE_MBPS;
        let mut panel = core::ptr::null_mut();
        let mut io = core::ptr::null_mut();
        let mut fb: *mut core::ffi::c_void = core::ptr::null_mut();
        // SAFETY: out-parameters receive valid handles on success; the DPI
        // panel has one frame buffer (CONFIG_BSP_LCD_DPI_BUFFER_NUMS=1).
        unsafe {
            esp!(bsp::bsp_display_new(&cfg, &mut panel, &mut io))?;
            esp!(bsp::esp_lcd_panel_disp_on_off(panel, true))?;
            esp!(bsp::esp_lcd_dpi_panel_get_frame_buffer(panel, 1, &mut fb))?;
            esp!(bsp::bsp_display_brightness_set(brightness))?;
        }
        ensure!(!fb.is_null(), "no frame buffer");
        let screen = Self {
            panel,
            fb: fb.cast(),
            glyphs: Glyphs::new(),
            shown: vec![
                [Cell {
                    ch: '\0',
                    fg: 0,
                    bg: 0
                }; COLS];
                ROWS
            ],
            flipped: false,
        };
        // SAFETY: the frame buffer holds PANEL_W × PANEL_H RGB565 pixels.
        unsafe { core::slice::from_raw_parts_mut(screen.fb, PANEL_W * PANEL_H) }.fill(BG);
        screen.flush(0, PANEL_H);
        log::info!("screen: {PANEL_W}x{PANEL_H} panel, {COLS}x{ROWS} terminal");
        Ok(screen)
    }

    /// Turn to the other landscape position (redraws everything).
    pub fn set_flipped(&mut self, flipped: bool, term: &mut Terminal) {
        if flipped != self.flipped {
            self.flipped = flipped;
            for row in &mut self.shown {
                row.fill(Cell {
                    ch: '\0',
                    fg: 0,
                    bg: 0,
                });
            }
            term.touch();
        }
    }

    /// Draw what changed since the last call.
    pub fn show(&mut self, term: &mut Terminal) {
        let grid = compose(term);
        term.drawn();
        // Landscape x range touched = panel row range to write back.
        let (mut x0, mut x1) = (usize::MAX, 0usize);
        for (r, row) in grid.iter().enumerate() {
            for (c, cell) in row.iter().enumerate() {
                if self.shown[r][c] != *cell {
                    self.draw_cell(c, r, *cell);
                    self.shown[r][c] = *cell;
                    x0 = x0.min(c * CELL_W);
                    x1 = x1.max((c + 1) * CELL_W);
                }
            }
        }
        if x0 < x1 {
            let (y0, y1) = if self.flipped {
                (PANEL_H - x1, PANEL_H - x0)
            } else {
                (x0, x1)
            };
            self.flush(y0, y1);
        }
    }

    fn draw_cell(&mut self, col: usize, row: usize, cell: Cell) {
        let mask = self.glyphs.get(cell.ch);
        // SAFETY: every index below stays inside the PANEL_W × PANEL_H buffer.
        let fb = unsafe { core::slice::from_raw_parts_mut(self.fb, PANEL_W * PANEL_H) };
        let lx0 = col * CELL_W;
        let ly0 = TOP + row * CELL_H;
        for gx in 0..CELL_W {
            let lx = lx0 + gx;
            for gy in 0..CELL_H {
                let ly = ly0 + gy;
                let (px, py) = if self.flipped {
                    (ly, PANEL_H - 1 - lx)
                } else {
                    (PANEL_W - 1 - ly, lx)
                };
                fb[py * PANEL_W + px] = if mask[gy * CELL_W + gx] != 0 {
                    cell.fg
                } else {
                    cell.bg
                };
            }
        }
    }

    /// Write back panel rows `y0..y1` from the cache, so the DSI DMA sees them.
    fn flush(&self, y0: usize, y1: usize) {
        // SAFETY: the buffer is the panel's own frame buffer: the driver only
        // writes back the cache for these rows (no copy).
        let r = unsafe {
            bsp::esp_lcd_panel_draw_bitmap(
                self.panel,
                0,
                y0 as i32,
                PANEL_W as i32,
                y1 as i32,
                self.fb.cast(),
            )
        };
        if r != 0 {
            log::warn!("screen: draw_bitmap {r}");
        }
    }
}

/// ProFont 24 pt rasterised once: one byte per pixel, 16×29 per glyph,
/// ASCII and Latin-1.
struct Glyphs {
    ascii: Vec<Box<[u8]>>,
    latin1: Vec<Box<[u8]>>,
    unknown: Box<[u8]>,
}

impl Glyphs {
    fn new() -> Self {
        let ascii = (0x20u8..=0x7e).map(|b| raster(b as char)).collect();
        let latin1 = (0xa0u32..=0xff)
            .map(|u| raster(char::from_u32(u).unwrap_or('?')))
            .collect();
        Self {
            ascii,
            latin1,
            unknown: raster('?'),
        }
    }

    fn get(&self, c: char) -> &[u8] {
        let u = c as u32;
        match u {
            0x20..=0x7e => &self.ascii[(u - 0x20) as usize],
            0xa0..=0xff => &self.latin1[(u - 0xa0) as usize],
            0 => &self.ascii[0],
            _ => &self.unknown,
        }
    }
}

fn raster(c: char) -> Box<[u8]> {
    let mut mask = Mask(vec![0u8; CELL_W * CELL_H]);
    let mut s = [0u8; 4];
    let style = MonoTextStyle::new(&profont::PROFONT_24_POINT, BinaryColor::On);
    let _ = Text::with_baseline(c.encode_utf8(&mut s), Point::zero(), style, Baseline::Top)
        .draw(&mut mask);
    mask.0.into_boxed_slice()
}

struct Mask(Vec<u8>);

impl OriginDimensions for Mask {
    fn size(&self) -> Size {
        Size::new(CELL_W as u32, CELL_H as u32)
    }
}

impl DrawTarget for Mask {
    type Color = BinaryColor;
    type Error = core::convert::Infallible;

    fn draw_iter<I: IntoIterator<Item = Pixel<BinaryColor>>>(
        &mut self,
        pixels: I,
    ) -> Result<(), Self::Error> {
        for Pixel(p, color) in pixels {
            if (0..CELL_W as i32).contains(&p.x)
                && (0..CELL_H as i32).contains(&p.y)
                && color.is_on()
            {
                self.0[p.y as usize * CELL_W + p.x as usize] = 1;
            }
        }
        Ok(())
    }
}
