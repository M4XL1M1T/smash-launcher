//! Minecraft-inspired look: blocky square widgets, stone and dirt colors, green accents and small
//! pixel block icons. Everything is generated in code, no game assets are used.
use eframe::egui::{self, pos2, vec2, Color32, Id, Painter, Rect, Rounding, Stroke, TextureHandle, TextureOptions};

pub const TEXT: Color32 = Color32::from_rgb(0xec, 0xec, 0xec);
pub const WEAK: Color32 = Color32::from_rgb(0xa8, 0xa8, 0xa8);
pub const STONE: Color32 = Color32::from_rgb(0x2e, 0x2e, 0x30);
pub const STONE_DARK: Color32 = Color32::from_rgb(0x24, 0x24, 0x26);
pub const BTN: Color32 = Color32::from_rgb(0x4b, 0x4b, 0x4e);
pub const BTN_HOVER: Color32 = Color32::from_rgb(0x62, 0x63, 0x68);
pub const BTN_ACTIVE: Color32 = Color32::from_rgb(0x3a, 0x3a, 0x3d);
pub const OUTLINE: Color32 = Color32::from_rgb(0x10, 0x10, 0x11);
pub const GREEN: Color32 = Color32::from_rgb(0x4f, 0x9a, 0x2c);
pub const GREEN_DARK: Color32 = Color32::from_rgb(0x2f, 0x5f, 0x1a);
pub const GREEN_LIGHT: Color32 = Color32::from_rgb(0x7c, 0xc2, 0x4a);
pub const DANGER: Color32 = Color32::from_rgb(0xe6, 0x5a, 0x5a);

#[derive(Clone, Copy)]
pub enum Block {
    Grass,
    Diamond,
    Stone,
}

/// Cheap deterministic noise in 0..=255 for pixel textures.
fn noise(x: u32, y: u32, salt: u32) -> u32 {
    let mut h = x.wrapping_mul(374_761_393) ^ y.wrapping_mul(668_265_263) ^ salt.wrapping_mul(2_246_822_519);
    h = (h ^ (h >> 13)).wrapping_mul(1_274_126_177);
    (h ^ (h >> 16)) & 0xff
}

fn shade(c: (u8, u8, u8), d: i32) -> Color32 {
    let f = |v: u8| (v as i32 + d).clamp(0, 255) as u8;
    Color32::from_rgb(f(c.0), f(c.1), f(c.2))
}

/// Color of one pixel of a block texture on an n x n grid.
fn block_pixel(b: Block, x: u32, y: u32, n: u32) -> Color32 {
    let (gx, gy) = (x * 8 / n, y * 8 / n);
    let edge = gx == 0 || gy == 0 || gx == 7 || gy == 7;
    let d = (noise(gx, gy, 7) as i32 % 31) - 15;
    let c = match b {
        Block::Grass => {
            if gy < 3 || (gy == 3 && noise(gx, 3, 1) % 2 == 0) {
                shade((0x5d, 0x9b, 0x35), d)
            } else {
                shade((0x86, 0x5c, 0x3a), d)
            }
        }
        Block::Diamond => {
            if (gx + gy) % 3 == 0 {
                shade((0x2b, 0xc9, 0xc3), d / 2)
            } else {
                shade((0x7a, 0xf0, 0xe8), d / 2)
            }
        }
        Block::Stone => shade((0x7c, 0x7c, 0x7e), d),
    };
    if edge {
        shade((c.r(), c.g(), c.b()), -38)
    } else {
        c
    }
}

pub fn paint_block(p: &Painter, rect: Rect, b: Block) {
    let n = 8;
    let cell = rect.width() / n as f32;
    for gy in 0..n {
        for gx in 0..n {
            let r = Rect::from_min_size(rect.min + vec2(gx as f32 * cell, gy as f32 * cell), vec2(cell + 0.5, cell + 0.5));
            p.rect_filled(r, 0.0, block_pixel(b, gx * 2, gy * 2, 16));
        }
    }
    p.rect_stroke(rect, 0.0, Stroke::new(1.0_f32, OUTLINE));
}

pub fn app_icon() -> egui::IconData {
    let n = 32u32;
    let mut rgba = Vec::with_capacity((n * n * 4) as usize);
    for y in 0..n {
        for x in 0..n {
            let c = block_pixel(Block::Grass, x, y, n);
            rgba.extend_from_slice(&[c.r(), c.g(), c.b(), 255]);
        }
    }
    egui::IconData { rgba, width: n, height: n }
}

fn dirt_image() -> egui::ColorImage {
    let n = 16usize;
    let mut px = Vec::with_capacity(n * n);
    for y in 0..n as u32 {
        for x in 0..n as u32 {
            let r = noise(x, y, 3);
            let d = (r as i32 % 29) - 14;
            let mut c = shade((0x6b, 0x4a, 0x2e), d);
            if r % 11 == 0 {
                c = shade((0x4a, 0x4a, 0x4a), d / 2); // small stone specks
            }
            px.push(c);
        }
    }
    egui::ColorImage { size: [n, n], pixels: px }
}

/// Paints a tiled dirt texture with a dark overlay so that text on top stays readable.
pub fn paint_dirt(ctx: &egui::Context, painter: &Painter, rect: Rect) {
    let Some(tex) = ctx.data(|d| d.get_temp::<TextureHandle>(Id::new("smash_dirt"))) else { return };
    let tile = 64.0;
    let mut y = rect.top();
    while y < rect.bottom() {
        let mut x = rect.left();
        while x < rect.right() {
            let r = Rect::from_min_size(pos2(x, y), vec2(tile, tile)).intersect(rect);
            let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(r.width() / tile, r.height() / tile));
            painter.image(tex.id(), r, uv, Color32::WHITE);
            x += tile;
        }
        y += tile;
    }
    painter.rect_filled(rect, 0.0, Color32::from_black_alpha(170));
}

pub fn apply(ctx: &egui::Context) {
    let tex = ctx.load_texture("smash_dirt", dirt_image(), TextureOptions::NEAREST);
    ctx.data_mut(|d| d.insert_temp(Id::new("smash_dirt"), tex));

    let mut v = egui::Visuals::dark();
    v.panel_fill = STONE;
    v.window_fill = Color32::from_rgb(0x2f, 0x2f, 0x32);
    v.window_stroke = Stroke::new(2.0_f32, OUTLINE);
    v.window_rounding = Rounding::same(3.0);
    v.menu_rounding = Rounding::ZERO;
    v.extreme_bg_color = Color32::from_rgb(0x1a, 0x1a, 0x1b);
    v.faint_bg_color = Color32::from_rgb(0x36, 0x36, 0x39);
    v.selection.bg_fill = GREEN_DARK;
    v.selection.stroke = Stroke::new(1.0_f32, GREEN_LIGHT);
    v.hyperlink_color = GREEN_LIGHT;
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, TEXT);
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, Color32::from_rgb(0x1a, 0x1a, 0x1b));
    for (w, bg, stroke, fg) in [
        (&mut v.widgets.inactive, BTN, Stroke::new(2.0_f32, OUTLINE), TEXT),
        (&mut v.widgets.hovered, BTN_HOVER, Stroke::new(2.0_f32, Color32::from_rgb(0xdd, 0xdd, 0xe6)), Color32::WHITE),
        (&mut v.widgets.active, BTN_ACTIVE, Stroke::new(2.0_f32, GREEN_LIGHT), Color32::WHITE),
        (&mut v.widgets.open, BTN_ACTIVE, Stroke::new(2.0_f32, OUTLINE), TEXT),
    ] {
        w.bg_fill = bg;
        w.weak_bg_fill = bg;
        w.bg_stroke = stroke;
        w.fg_stroke = Stroke::new(1.0_f32, fg);
    }
    v.widgets.noninteractive.rounding = Rounding::ZERO;
    // Buttons and other interactive widgets get slightly rounded corners.
    for w in [&mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
        w.rounding = Rounding::same(3.0);
    }
    ctx.set_visuals(v);

    ctx.style_mut(|s| {
        s.spacing.button_padding = vec2(10.0, 6.0);
        s.spacing.item_spacing = vec2(8.0, 8.0);
        s.spacing.interact_size.y = 28.0;
        s.spacing.slider_width = 180.0;
        // Labels are plain text, not selectable with the mouse.
        s.interaction.selectable_labels = false;
        if let Some(h) = s.text_styles.get_mut(&egui::TextStyle::Heading) {
            h.size = 24.0;
        }
    });
}

#[cfg(test)]
mod icon_tests {
    use super::*;

    /// Writes assets/icon.ico (grass block, several sizes) that build.rs embeds into the exe.
    /// Run manually: cargo test write_icon_file -- --ignored
    #[test]
    #[ignore]
    fn write_icon_file() {
        let sizes = [16u32, 32, 48, 64];
        let mut images: Vec<Vec<u8>> = vec![];
        for &n in &sizes {
            let mut d = Vec::new();
            for v in [40u32, n, n * 2, 1 | (32 << 16), 0, 0, 0, 0, 0, 0] {
                d.extend_from_slice(&v.to_le_bytes());
            }
            for y in (0..n).rev() {
                for x in 0..n {
                    let c = block_pixel(Block::Grass, x, y, n);
                    d.extend_from_slice(&[c.b(), c.g(), c.r(), 255]);
                }
            }
            d.extend(std::iter::repeat(0u8).take((((n + 31) / 32) * 4 * n) as usize));
            images.push(d);
        }
        let mut out = vec![0u8, 0, 1, 0];
        out.extend_from_slice(&(sizes.len() as u16).to_le_bytes());
        let mut offset = 6 + 16 * sizes.len() as u32;
        for (n, img) in sizes.iter().zip(&images) {
            out.extend_from_slice(&[*n as u8, *n as u8, 0, 0, 1, 0, 32, 0]);
            out.extend_from_slice(&(img.len() as u32).to_le_bytes());
            out.extend_from_slice(&offset.to_le_bytes());
            offset += img.len() as u32;
        }
        for img in &images {
            out.extend_from_slice(img);
        }
        std::fs::create_dir_all("assets").unwrap();
        std::fs::write("assets/icon.ico", out).unwrap();
    }
}
