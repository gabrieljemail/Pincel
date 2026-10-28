//! Software rasterizer. Draws `DrawCmd`s into a `Texture`.

use crate::draw_cmd::{DrawCmd, Rect};
use crate::element::Image;
use crate::font::{FontKey, Fonts};
use crate::style::{Color, ImageFilter, TextShadow};

/// RGBA8 pixels, 4 bytes each, rows tightly packed with row 0 at the top.
/// Colors are premultiplied by alpha, so an opaque canvas background gives opaque pixels.
#[derive(Default)]
pub struct Texture {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

impl Texture {
    /// Resizes (contents become unspecified) and fills with `color`, replacing whatever was there.
    pub fn reset(&mut self, width: u32, height: u32, color: Color) {
        self.width = width;
        self.height = height;
        let pixel = premultiply(color);
        self.data.resize(width as usize * height as usize * 4, 0);

        // A clear covers the whole canvas, so it is worth doing a word at a time (or as a plain memset for the
        // common transparent background) rather than a pixel at a time.
        if pixel == [0; 4] {
            self.data.fill(0);
            return;
        }
        // SAFETY: any bytes are a valid u32, and `align_to_mut` gives back the parts that don't fit as bytes.
        let (head, words, tail) = unsafe { self.data.align_to_mut::<u32>() };
        if head.is_empty() && tail.is_empty() {
            words.fill(u32::from_ne_bytes(pixel));
        } else {
            // Not aligned to a pixel, which a Vec's allocation practically never is, so the simple way.
            for px in self.data.chunks_exact_mut(4) {
                px.copy_from_slice(&pixel);
            }
        }
    }

    /// The part of `rect` that is inside the texture, as `(x0, y0, x1, y1)`, or `None` if nothing is.
    fn clip(&self, rect: Rect) -> Option<(i32, i32, i32, i32)> {
        let x0 = rect.x.max(0);
        let y0 = rect.y.max(0);
        let x1 = rect.x.saturating_add(rect.w).min(self.width as i32);
        let y1 = rect.y.saturating_add(rect.h).min(self.height as i32);
        (x0 < x1 && y0 < y1).then_some((x0, y0, x1, y1))
    }

    fn pixel_mut(&mut self, x: i32, y: i32) -> &mut [u8] {
        let i = (y as usize * self.width as usize + x as usize) * 4;
        &mut self.data[i..i + 4]
    }

    /// Blends `color` over `rect`, clipped to the texture.
    pub fn fill_rect(&mut self, rect: Rect, color: Color) {
        let Some((x0, y0, x1, y1)) = self.clip(rect) else { return };
        if color.a <= 0.0 {
            return;
        }

        let src = premultiply(color);
        let inv = 1.0 - color.a.min(1.0);
        for y in y0..y1 {
            let row = (y as usize * self.width as usize + x0 as usize) * 4;
            for px in self.data[row..row + (x1 - x0) as usize * 4].chunks_exact_mut(4) {
                blend(px, &src, inv);
            }
        }
    }

    /// Draws the commands, each moved down by `dy` pixels (which is how scrolling is done, without laying out
    /// again). Commands that end up entirely above or below the texture are skipped.
    pub fn execute(&mut self, cmds: &[DrawCmd], dy: i32, fonts: &mut Fonts) {
        let view_h = self.height as i32;
        let off_screen = |y: i32, h: i32| y >= view_h || y.saturating_add(h) <= 0;

        for cmd in cmds {
            match cmd {
                DrawCmd::Box { rect, background, border, border_color, radius, clip } => {
                    let rect = Rect { y: rect.y + dy, ..*rect };
                    if !off_screen(rect.y, rect.h) {
                        let clip = clip.map(|c| Rect { y: c.y + dy, ..c });
                        self.draw_box(rect, *background, *border, *border_color, *radius, clip);
                    }
                }
                DrawCmd::Text { x, y, text, font, font_size, color, clip, shadow } => {
                    let y = y + dy;
                    // A shadow reaches past the line's box by its offset, so it can still be on screen when the line isn't.
                    let reach = shadow.map_or(0, |s| s.offset_y.abs());
                    if !off_screen(y - reach, fonts.line_metrics(*font, *font_size).height + 2 * reach) {
                        let clip = clip.map(|c| Rect { y: c.y + dy, ..c });
                        self.draw_text(fonts, *x, y, text, *font, *font_size, *color, clip, *shadow);
                    }
                }
                DrawCmd::Image { rect, image, radius, filter, clip } => {
                    let rect = Rect { y: rect.y + dy, ..*rect };
                    if !off_screen(rect.y, rect.h) {
                        let clip = clip.map(|c| Rect { y: c.y + dy, ..c });
                        self.draw_image(rect, image, *radius, *filter, clip);
                    }
                }
            }
        }
    }

    /// A box with square corners, restricted to `clip`. Square corners are exact on the pixel grid, so plain
    /// fills do. The border strips and the inner rect never overlap, so translucent colors blend once.
    fn fill_straight_box(&mut self, rect: Rect, clip: Rect, background: Color, border: [i32; 4], border_color: Color) {
        let [top, right, bottom, left] = border;
        let mid_h = rect.h - top - bottom;
        let fills = [
            (Rect { x: rect.x, y: rect.y, w: rect.w, h: top }, border_color),
            (Rect { x: rect.x, y: rect.y + rect.h - bottom, w: rect.w, h: bottom }, border_color),
            (Rect { x: rect.x, y: rect.y + top, w: left, h: mid_h }, border_color),
            (Rect { x: rect.x + rect.w - right, y: rect.y + top, w: right, h: mid_h }, border_color),
            (Rect { x: rect.x + left, y: rect.y + top, w: rect.w - left - right, h: mid_h }, background),
        ];
        for (area, color) in fills {
            self.fill_rect(area.intersect(clip), color);
        }
    }

    /// Draws a box with a border and rounded corners, only inside `clip` if there is one.
    fn draw_box(&mut self, rect: Rect, background: Color, border: [i32; 4], border_color: Color, radius: i32, clip: Option<Rect>) {
        // With no clip, everything is fair game (and the texture's own edges are applied further down).
        let bounds = clip.unwrap_or(Rect { x: i32::MIN / 2, y: i32::MIN / 2, w: i32::MAX, h: i32::MAX });
        // Borders can't be wider than the box.
        let [top, right, bottom, left] = border.map(|b| b.max(0));
        let top = top.min(rect.h);
        let bottom = bottom.min(rect.h - top);
        let left = left.min(rect.w);
        let right = right.min(rect.w - left);
        let border = [top, right, bottom, left];

        let radius = radius.min(rect.w.min(rect.h) / 2);
        if radius <= 0 {
            self.fill_straight_box(rect, rect.intersect(bounds), background, border, border_color);
            return;
        }

        // Only the four corner squares are curved. Everywhere else the edges are straight and the pixel grid
        // makes them exact, so the rest of the box is filled like a square one, in three rectangles. The curve
        // is costly per pixel, and a big box is mostly not corner.
        let r = radius;
        let straight = [
            Rect { x: rect.x + r, y: rect.y, w: rect.w - 2 * r, h: r },
            Rect { x: rect.x + r, y: rect.y + rect.h - r, w: rect.w - 2 * r, h: r },
            Rect { x: rect.x, y: rect.y + r, w: rect.w, h: rect.h - 2 * r },
        ];
        for region in straight {
            self.fill_straight_box(rect, region.intersect(bounds), background, border, border_color);
        }

        // In the corners, every pixel gets how much of it is inside the outer shape, and how much of that is inside
        // the inner shape (the border box minus the border, with a smaller radius). The ring between them is the border.
        // The inner shape's curve fits inside the corner squares too, because its radius is the outer one minus the
        // widest border.
        let outer = RoundRect::new(rect, radius);
        let inner_rect = Rect { x: rect.x + left, y: rect.y + top, w: rect.w - left - right, h: rect.h - top - bottom };
        let inner = (inner_rect.w > 0 && inner_rect.h > 0).then(|| {
            let widest = top.max(right).max(bottom).max(left);
            RoundRect::new(inner_rect, (radius - widest).max(0))
        });

        let corners = [
            Rect { x: rect.x, y: rect.y, w: r, h: r },
            Rect { x: rect.x + rect.w - r, y: rect.y, w: r, h: r },
            Rect { x: rect.x, y: rect.y + rect.h - r, w: r, h: r },
            Rect { x: rect.x + rect.w - r, y: rect.y + rect.h - r, w: r, h: r },
        ];
        for corner in corners {
            let Some((x0, y0, x1, y1)) = self.clip(corner.intersect(bounds)) else { continue };
            for y in y0..y1 {
                for x in x0..x1 {
                    let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
                    let outside = outer.coverage(fx, fy);
                    if outside <= 0.0 {
                        continue;
                    }
                    let inside = inner.as_ref().map_or(0.0, |i| i.coverage(fx, fy)).min(outside);
                    let px = self.pixel_mut(x, y);
                    blend_color(px, border_color, outside - inside);
                    blend_color(px, background, inside);
                }
            }
        }
    }

    /// Draws one line of text. `(x, y)` is the top-left of the line's box.
    /// Glyphs come from the atlas as coverage, which scales the color's alpha.
    /// Nothing is drawn outside `clip`, if there is one.
    /// A shadow, if there is one, is drawn first: the same glyphs moved by its offset, in its color.
    #[allow(clippy::too_many_arguments)]
    fn draw_text(&mut self, fonts: &mut Fonts, x: i32, y: i32, text: &str, font: FontKey, font_size: u16, color: Color, clip: Option<Rect>, shadow: Option<TextShadow>) {
        let baseline = y + fonts.line_metrics(font, font_size).ascent;
        let (placed, _) = fonts.place(font, font_size, text);

        // The pixels that can be drawn on: the texture, or the part of it inside the clip.
        let texture_rect = Rect { x: 0, y: 0, w: self.width as i32, h: self.height as i32 };
        let area = clip.map_or(texture_rect, |c| c.intersect(texture_rect));

        if let Some(shadow) = shadow.filter(TextShadow::is_visible) {
            let shadow_color = Color { a: shadow.color.a * color.a, ..shadow.color };
            self.blit_glyphs(fonts, &placed, x + shadow.offset_x, baseline + shadow.offset_y, font, font_size, shadow_color, area);
        }
        self.blit_glyphs(fonts, &placed, x, baseline, font, font_size, color, area);
    }

    /// Draws already-positioned glyphs, with the line starting at `x` and sitting on `baseline`, only inside `area`.
    #[allow(clippy::too_many_arguments)]
    fn blit_glyphs(&mut self, fonts: &mut Fonts, placed: &[(char, f32)], x: i32, baseline: i32, font: FontKey, font_size: u16, color: Color, area: Rect) {
        let (min_x, min_y, max_x, max_y) = (area.x, area.y, area.x + area.w, area.y + area.h);

        for &(c, pen) in placed {
            // Glyphs are rasterized for a whole-pixel origin, so each is snapped there. The pen itself keeps its
            // fraction, so spacing doesn't drift.
            let glyph = fonts.glyph(font, c, font_size);
            let left = (x as f32 + pen).round() as i32 + glyph.left;
            let top = baseline - glyph.bottom - glyph.height as i32;

            for row in 0..glyph.height as i32 {
                let py = top + row;
                if py < min_y || py >= max_y {
                    continue;
                }
                for col in 0..glyph.width as i32 {
                    let px = left + col;
                    if px < min_x || px >= max_x {
                        continue;
                    }
                    let coverage = fonts.atlas.data[((glyph.y as i32 + row) as u32 * fonts.atlas.width + (glyph.x as i32 + col) as u32) as usize];
                    if coverage != 0 {
                        blend_color(self.pixel_mut(px, py), color, coverage as f32 / 255.0);
                    }
                }
            }
        }
    }

    /// Stretches `image` to fill `rect`, with its corners rounded by `radius`.
    ///
    /// Drawn at its own size, or with `Nearest`, it is a copy of the pixels (blended where they are translucent,
    /// and skipped where they are fully transparent), which is cheap. Otherwise upscaling is bilinear, and
    /// downscaling averages a grid of bilinear samples across each destination pixel's footprint in the source,
    /// so shrinking a large image doesn't alias.
    /// Only the part inside `clip`, if there is one, is drawn.
    fn draw_image(&mut self, rect: Rect, image: &Image, radius: i32, filter: ImageFilter, clip: Option<Rect>) {
        let visible = clip.map_or(rect, |c| rect.intersect(c));
        let Some((x0, y0, x1, y1)) = self.clip(visible) else { return };
        let mask = (radius > 0).then(|| RoundRect::new(rect, radius));

        let same_size = rect.w == image.width as i32 && rect.h == image.height as i32;
        if filter == ImageFilter::Nearest || same_size {
            let (iw, ih) = (i64::from(image.width), i64::from(image.height));
            for y in y0..y1 {
                // Which source row this destination row shows, taken from the pixel's center.
                let sy = (((2 * i64::from(y - rect.y) + 1) * ih) / (2 * i64::from(rect.h))).min(ih - 1) as usize;
                for x in x0..x1 {
                    let coverage = mask.as_ref().map_or(1.0, |m| m.coverage(x as f32 + 0.5, y as f32 + 0.5));
                    if coverage <= 0.0 {
                        continue;
                    }
                    let sx = (((2 * i64::from(x - rect.x) + 1) * iw) / (2 * i64::from(rect.w))).min(iw - 1) as usize;
                    let i = (sy * image.width as usize + sx) * 4;
                    let texel = &image.data[i..i + 4];
                    let alpha = texel[3];

                    if alpha == 255 && coverage >= 1.0 {
                        self.pixel_mut(x, y).copy_from_slice(&[texel[0], texel[1], texel[2], 255]);
                    } else if alpha != 0 {
                        let a = f32::from(alpha) / 255.0 * coverage;
                        blend_color(self.pixel_mut(x, y), Color { r: texel[0], g: texel[1], b: texel[2], a }, 1.0);
                    }
                }
            }
            return;
        }

        // How many source pixels one destination pixel covers along each axis, rounded up (at most 8 samples).
        let nx = ((image.width as f32 / rect.w as f32).ceil() as u32).clamp(1, 8);
        let ny = ((image.height as f32 / rect.h as f32).ceil() as u32).clamp(1, 8);

        for y in y0..y1 {
            for x in x0..x1 {
                let coverage = mask.as_ref().map_or(1.0, |m| m.coverage(x as f32 + 0.5, y as f32 + 0.5));
                if coverage <= 0.0 {
                    continue;
                }

                let mut sum = [0.0f32; 4];
                for sy in 0..ny {
                    for sx in 0..nx {
                        // Position inside the destination rect, then mapped to source pixel coordinates.
                        let dx = (x - rect.x) as f32 + (sx as f32 + 0.5) / nx as f32;
                        let dy = (y - rect.y) as f32 + (sy as f32 + 0.5) / ny as f32;
                        let u = dx / rect.w as f32 * image.width as f32 - 0.5;
                        let v = dy / rect.h as f32 * image.height as f32 - 0.5;
                        let sample = sample_bilinear(image, u, v);
                        for c in 0..4 {
                            sum[c] += sample[c];
                        }
                    }
                }

                let count = (nx * ny) as f32;
                let src = sum.map(|s| s / count * coverage);
                let inv = 1.0 - src[3] / 255.0;
                let px = self.pixel_mut(x, y);
                for c in 0..4 {
                    px[c] = (src[c] + px[c] as f32 * inv).round().min(255.0) as u8;
                }
            }
        }
    }
}

/// A rectangle with rounded corners, as a signed distance field, which gives anti-aliased edges for free.
struct RoundRect {
    cx: f32,
    cy: f32,
    /// Half the width and height, minus the radius.
    hx: f32,
    hy: f32,
    r: f32,
}

impl RoundRect {
    fn new(rect: Rect, radius: i32) -> RoundRect {
        let r = (radius.max(0) as f32).min(rect.w.min(rect.h) as f32 / 2.0);
        RoundRect {
            cx: rect.x as f32 + rect.w as f32 / 2.0,
            cy: rect.y as f32 + rect.h as f32 / 2.0,
            hx: rect.w as f32 / 2.0 - r,
            hy: rect.h as f32 / 2.0 - r,
            r,
        }
    }

    /// How much of the pixel centered at `(x, y)` is inside the shape, 0.0 to 1.0.
    /// The distance to the edge in pixels, offset by half a pixel, is a good estimate of the covered area.
    fn coverage(&self, x: f32, y: f32) -> f32 {
        let qx = (x - self.cx).abs() - self.hx;
        let qy = (y - self.cy).abs() - self.hy;
        let distance = qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - self.r;
        (0.5 - distance).clamp(0.0, 1.0)
    }
}

/// One texel as premultiplied `[r, g, b, a]`, each 0.0 to 255.0. Coordinates are clamped to the image.
fn texel(image: &Image, x: i32, y: i32) -> [f32; 4] {
    let x = x.clamp(0, image.width as i32 - 1) as usize;
    let y = y.clamp(0, image.height as i32 - 1) as usize;
    let i = (y * image.width as usize + x) * 4;
    let a = image.data[i + 3] as f32;
    let premul = |c: u8| c as f32 * a / 255.0;
    [premul(image.data[i]), premul(image.data[i + 1]), premul(image.data[i + 2]), a]
}

/// Bilinear sample at `(u, v)`, where texel centers are at whole numbers. Interpolating premultiplied
/// values means transparent texels don't bleed their color into their neighbors.
fn sample_bilinear(image: &Image, u: f32, v: f32) -> [f32; 4] {
    let (u0, v0) = (u.floor(), v.floor());
    let (fu, fv) = (u - u0, v - v0);
    let (x, y) = (u0 as i32, v0 as i32);
    let (a, b, c, d) = (texel(image, x, y), texel(image, x + 1, y), texel(image, x, y + 1), texel(image, x + 1, y + 1));
    let mut out = [0.0; 4];
    for i in 0..4 {
        let top = a[i] + (b[i] - a[i]) * fu;
        let bottom = c[i] + (d[i] - c[i]) * fu;
        out[i] = top + (bottom - top) * fv;
    }
    out
}

/// Premultiplied "over": result = src + dst * (1 - src alpha). `inv` is `1 - src alpha`.
fn blend(dst: &mut [u8], src: &[u8; 4], inv: f32) {
    if inv == 0.0 {
        dst.copy_from_slice(src);
        return;
    }
    for c in 0..4 {
        dst[c] = (src[c] as f32 + dst[c] as f32 * inv).round().min(255.0) as u8;
    }
}

/// Blends `color` over a pixel, with its alpha scaled by `coverage` (0.0 to 1.0).
fn blend_color(dst: &mut [u8], color: Color, coverage: f32) {
    let a = color.a.clamp(0.0, 1.0) * coverage.clamp(0.0, 1.0);
    if a > 0.0 {
        blend(dst, &premultiply(Color { a, ..color }), 1.0 - a);
    }
}

fn premultiply(c: Color) -> [u8; 4] {
    let a = c.a.clamp(0.0, 1.0);
    let ch = |v: u8| (v as f32 * a).round() as u8;
    [ch(c.r), ch(c.g), ch(c.b), (a * 255.0).round() as u8]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::FontFamily;
    use std::sync::Arc;

    fn pixel(t: &Texture, x: u32, y: u32) -> [u8; 4] {
        let i = (y * t.width + x) as usize * 4;
        t.data[i..i + 4].try_into().unwrap()
    }

    fn white(w: u32, h: u32) -> Texture {
        let mut t = Texture::default();
        t.reset(w, h, Color::WHITE);
        t
    }

    const RED: Color = Color { r: 255, g: 0, b: 0, a: 1.0 };
    const BLUE: Color = Color { r: 0, g: 0, b: 255, a: 1.0 };
    const WHITE: [u8; 4] = [255, 255, 255, 255];
    const RED_PIXEL: [u8; 4] = [255, 0, 0, 255];

    fn run(t: &mut Texture, cmds: &[DrawCmd]) {
        t.execute(cmds, 0, &mut Fonts::new());
    }

    #[test]
    fn box_fills_exactly_its_rect_with_border() {
        let mut t = white(10, 10);
        run(
            &mut t,
            &[DrawCmd::Box {
                rect: Rect { x: 2, y: 3, w: 5, h: 4 },
                background: RED,
                border: [1, 1, 1, 1],
                border_color: BLUE,
                radius: 0,
                clip: None,
            }],
        );

        assert_eq!(pixel(&t, 1, 3), WHITE); // just left of the box
        assert_eq!(pixel(&t, 2, 3), [0, 0, 255, 255]); // top-left corner is border
        assert_eq!(pixel(&t, 6, 6), [0, 0, 255, 255]); // bottom-right corner is border
        assert_eq!(pixel(&t, 3, 4), [255, 0, 0, 255]); // inside
        assert_eq!(pixel(&t, 7, 4), WHITE); // just right of the box
        assert_eq!(pixel(&t, 3, 7), WHITE); // just below the box
    }

    #[test]
    fn reset_fills_every_pixel_with_the_color_at_any_size() {
        let colors = [Color::WHITE, Color::TRANSPARENT, Color { r: 10, g: 20, b: 30, a: 0.5 }, RED];
        for color in colors {
            for (w, h) in [(1, 1), (3, 5), (7, 1), (64, 64), (1001, 3)] {
                let mut t = Texture::default();
                t.reset(w, h, color);
                let expected = premultiply(color);
                assert_eq!(t.data.len(), (w * h * 4) as usize);
                assert!(t.data.chunks_exact(4).all(|p| p == expected), "{color:?} at {w}x{h}");
            }
        }
        // And it replaces what was there, including when the size changes.
        let mut t = white(8, 8);
        t.fill_rect(Rect { x: 0, y: 0, w: 8, h: 8 }, RED);
        t.reset(4, 4, Color::TRANSPARENT);
        assert!(t.data.iter().all(|&b| b == 0) && t.data.len() == 64);
    }

    #[test]
    fn clips_to_the_texture() {
        let mut t = white(4, 4);
        t.fill_rect(Rect { x: -5, y: -5, w: 7, h: 100 }, RED);
        assert_eq!(pixel(&t, 1, 3), [255, 0, 0, 255]);
        assert_eq!(pixel(&t, 2, 3), WHITE);
    }

    #[test]
    fn translucent_color_blends_over_destination() {
        let mut t = white(1, 1);
        t.fill_rect(Rect { x: 0, y: 0, w: 1, h: 1 }, Color { r: 0, g: 0, b: 0, a: 0.5 });
        assert_eq!(pixel(&t, 0, 0), [128, 128, 128, 255]);
    }

    fn rounded(rect: Rect, radius: i32, border: i32) -> DrawCmd {
        DrawCmd::Box { rect, background: RED, border: [border; 4], border_color: BLUE, radius, clip: None }
    }

    #[test]
    fn rounded_box_cuts_its_corners_and_keeps_its_edges() {
        let mut t = white(40, 40);
        run(&mut t, &[rounded(Rect { x: 4, y: 4, w: 32, h: 32 }, 10, 0)]);

        assert_eq!(pixel(&t, 4, 4), WHITE, "the very corner is outside a radius-10 arc");
        assert_eq!(pixel(&t, 35, 35), WHITE);
        assert_eq!(pixel(&t, 20, 4), [255, 0, 0, 255], "the middle of the top edge is full");
        assert_eq!(pixel(&t, 4, 20), [255, 0, 0, 255]);
        assert_eq!(pixel(&t, 20, 20), [255, 0, 0, 255]);
        assert_eq!(pixel(&t, 20, 3), WHITE, "nothing outside the rect");
    }

    #[test]
    fn rounded_corner_edge_is_anti_aliased() {
        let mut t = white(40, 40);
        run(&mut t, &[rounded(Rect { x: 4, y: 4, w: 32, h: 32 }, 10, 0)]);
        // Red over white keeps red at 255 and lowers green and blue by how much of the pixel is covered.
        let partial = (4..14).flat_map(|y| (4..14).map(move |x| (x, y))).filter(|&(x, y)| {
            let p = pixel(&t, x, y);
            p[1] > 10 && p[1] < 245
        });
        assert!(partial.count() >= 4, "an arc has partially covered pixels along it");
    }

    #[test]
    fn rounded_box_with_border_has_a_ring_and_a_fill() {
        let mut t = white(40, 40);
        run(&mut t, &[rounded(Rect { x: 4, y: 4, w: 32, h: 32 }, 10, 3)]);

        assert_eq!(pixel(&t, 20, 4), [0, 0, 255, 255], "top edge is border");
        assert_eq!(pixel(&t, 20, 6), [0, 0, 255, 255]);
        assert_eq!(pixel(&t, 20, 7), [255, 0, 0, 255], "just inside the 3px border is background");
        assert_eq!(pixel(&t, 4, 4), WHITE);
    }

    /// The slow, obviously right way to draw a rounded box: ask the distance field about every pixel.
    fn reference_rounded_box(t: &mut Texture, rect: Rect, background: Color, border: [i32; 4], border_color: Color, radius: i32) {
        let [top, right, bottom, left] = border;
        let outer = RoundRect::new(rect, radius);
        let inner_rect = Rect { x: rect.x + left, y: rect.y + top, w: rect.w - left - right, h: rect.h - top - bottom };
        let inner = (inner_rect.w > 0 && inner_rect.h > 0).then(|| RoundRect::new(inner_rect, (radius - top.max(right).max(bottom).max(left)).max(0)));
        let (x0, y0, x1, y1) = t.clip(rect).unwrap();
        for y in y0..y1 {
            for x in x0..x1 {
                let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
                let outside = outer.coverage(fx, fy);
                if outside <= 0.0 {
                    continue;
                }
                let inside = inner.as_ref().map_or(0.0, |i| i.coverage(fx, fy)).min(outside);
                let px = t.pixel_mut(x, y);
                blend_color(px, border_color, outside - inside);
                blend_color(px, background, inside);
            }
        }
    }

    #[test]
    fn rounded_box_fast_path_matches_the_per_pixel_version() {
        let translucent = Color { r: 30, g: 200, b: 90, a: 0.6 };
        let cases = [
            // (rect, radius, border, background, border color)
            (Rect { x: 3, y: 2, w: 40, h: 30 }, 8, [0; 4], RED, BLUE),
            (Rect { x: 3, y: 2, w: 41, h: 31 }, 8, [2, 2, 2, 2], RED, BLUE), // Odd sizes.
            (Rect { x: 0, y: 0, w: 50, h: 50 }, 25, [3, 3, 3, 3], translucent, BLUE), // A circle.
            (Rect { x: 4, y: 4, w: 44, h: 36 }, 6, [10, 10, 10, 10], RED, translucent), // Border wider than the radius.
            (Rect { x: 4, y: 4, w: 44, h: 36 }, 12, [1, 5, 2, 7], translucent, BLUE), // Uneven borders.
            (Rect { x: -10, y: -6, w: 40, h: 30 }, 9, [2, 2, 2, 2], RED, BLUE), // Partly off the texture.
            (Rect { x: 30, y: 20, w: 40, h: 30 }, 9, [2, 2, 2, 2], RED, BLUE),
            (Rect { x: 5, y: 5, w: 7, h: 4 }, 100, [1, 1, 1, 1], RED, BLUE), // Radius bigger than the box.
        ];
        for (rect, radius, border, background, border_color) in cases {
            let radius = radius.min(rect.w.min(rect.h) / 2);
            let mut fast = white(60, 50);
            let mut slow = white(60, 50);
            fast.draw_box(rect, background, border, border_color, radius, None);
            reference_rounded_box(&mut slow, rect, background, border, border_color, radius);
            assert!(fast.data == slow.data, "rounded box differs for {rect:?} radius {radius} border {border:?}");
        }
    }

    #[test]
    fn oversized_radius_is_a_circle_not_a_crash() {
        let mut t = white(20, 20);
        run(&mut t, &[rounded(Rect { x: 0, y: 0, w: 20, h: 20 }, 9999, 0)]);
        assert_eq!(pixel(&t, 10, 10), [255, 0, 0, 255]);
        assert_eq!(pixel(&t, 0, 0), WHITE);
    }

    #[test]
    fn rounded_box_clips_to_the_texture() {
        let mut t = white(10, 10);
        run(&mut t, &[rounded(Rect { x: -20, y: -20, w: 40, h: 40 }, 8, 2)]);
        assert_eq!(pixel(&t, 5, 5), [255, 0, 0, 255]);
    }

    fn image(w: u32, h: u32, pixels: &[[u8; 4]]) -> Arc<Image> {
        Arc::new(Image::from_rgba(w, h, pixels.concat()).unwrap())
    }

    fn draw_image(t: &mut Texture, rect: Rect, image: &Arc<Image>, radius: i32) {
        run(t, &[DrawCmd::Image { rect, image: image.clone(), radius, filter: ImageFilter::Smooth, clip: None }]);
    }

    #[test]
    fn image_at_its_own_size_is_copied_exactly() {
        let pixels = [[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255], [10, 20, 30, 255]];
        let img = image(2, 2, &pixels);
        let mut t = white(4, 4);
        draw_image(&mut t, Rect { x: 1, y: 1, w: 2, h: 2 }, &img, 0);
        assert_eq!(pixel(&t, 1, 1), pixels[0]);
        assert_eq!(pixel(&t, 2, 1), pixels[1]);
        assert_eq!(pixel(&t, 1, 2), pixels[2]);
        assert_eq!(pixel(&t, 2, 2), pixels[3]);
        assert_eq!(pixel(&t, 0, 0), WHITE);
        assert_eq!(pixel(&t, 3, 3), WHITE);
    }

    #[test]
    fn upscaled_image_is_smooth_between_texels() {
        let img = image(2, 1, &[[0, 0, 0, 255], [255, 255, 255, 255]]);
        let mut t = white(8, 1);
        draw_image(&mut t, Rect { x: 0, y: 0, w: 8, h: 1 }, &img, 0);
        let row: Vec<u8> = (0..8).map(|x| pixel(&t, x, 0)[0]).collect();
        assert!(row.windows(2).all(|w| w[0] <= w[1]), "a left-to-right gradient never goes back: {row:?}");
        assert_eq!(row[0], 0);
        assert_eq!(row[7], 255);
        assert!(row[3] > 0 && row[3] < 255, "the middle is a blend: {row:?}");
    }

    #[test]
    fn downscaled_image_averages_instead_of_skipping() {
        // A black and white checkerboard shrunk to one pixel is gray, whichever texel nearest-neighbor would pick.
        let (b, w) = ([0, 0, 0, 255], [255, 255, 255, 255]);
        let img = image(4, 4, &[b, w, b, w, w, b, w, b, b, w, b, w, w, b, w, b]);
        let mut t = white(1, 1);
        draw_image(&mut t, Rect { x: 0, y: 0, w: 1, h: 1 }, &img, 0);
        let v = pixel(&t, 0, 0)[0];
        assert!((120..=135).contains(&v), "expected mid-gray, got {v}");
    }

    #[test]
    fn transparent_image_pixels_show_what_is_behind() {
        let img = image(2, 1, &[[255, 0, 0, 0], [0, 0, 255, 255]]);
        let mut t = white(2, 1);
        draw_image(&mut t, Rect { x: 0, y: 0, w: 2, h: 1 }, &img, 0);
        assert_eq!(pixel(&t, 0, 0), WHITE, "alpha 0 leaves the background alone");
        assert_eq!(pixel(&t, 1, 0), [0, 0, 255, 255]);
    }

    #[test]
    fn half_transparent_image_blends() {
        let img = image(1, 1, &[[0, 0, 0, 128]]);
        let mut t = white(1, 1);
        draw_image(&mut t, Rect { x: 0, y: 0, w: 1, h: 1 }, &img, 0);
        let p = pixel(&t, 0, 0);
        assert!((126..=128).contains(&p[0]) && p[3] == 255, "{p:?}");
    }

    #[test]
    fn rounded_image_is_masked_at_the_corners() {
        let img = image(1, 1, &[[255, 0, 0, 255]]);
        let mut t = white(20, 20);
        draw_image(&mut t, Rect { x: 0, y: 0, w: 20, h: 20 }, &img, 8);
        assert_eq!(pixel(&t, 0, 0), WHITE);
        assert_eq!(pixel(&t, 10, 0), [255, 0, 0, 255]);
        assert_eq!(pixel(&t, 10, 10), [255, 0, 0, 255]);
    }

    fn drawn_text(shadow: Option<TextShadow>, color: Color) -> Texture {
        let mut t = white(60, 40);
        let cmd = DrawCmd::Text { x: 10, y: 8, text: "I".into(), font: FontKey::regular(FontFamily::Mono), font_size: 24, color, clip: None, shadow };
        run(&mut t, &[cmd]);
        t
    }

    fn ink_columns(t: &Texture) -> Vec<u32> {
        (0..t.width).filter(|&x| (0..t.height).any(|y| pixel(t, x, y) != WHITE)).collect()
    }

    #[test]
    fn a_shadow_is_drawn_offset_from_the_text_and_behind_it() {
        let plain = drawn_text(None, BLUE);
        let shadow = Some(TextShadow { color: RED, offset_x: 6, offset_y: 0 });
        let with = drawn_text(shadow, BLUE);

        let (before, after) = (ink_columns(&plain), ink_columns(&with));
        assert_eq!(after.first(), before.first(), "the text itself is where it was");
        assert_eq!(after.last().copied(), before.last().map(|x| x + 6), "and the shadow reaches 6px further right");

        // Where the two overlap, the text is on top. Where only the shadow is, it is red.
        let stem = (0..60).flat_map(|x| (0..40).map(move |y| (x, y))).find(|&(x, y)| pixel(&plain, x, y) == [0, 0, 255, 255]).unwrap();
        assert_eq!(pixel(&with, stem.0, stem.1), [0, 0, 255, 255], "the text wins over its own shadow");
        let only_shadow = (0..60).flat_map(|x| (0..40).map(move |y| (x, y))).find(|&(x, y)| pixel(&plain, x, y) == WHITE && pixel(&with, x, y) == [255, 0, 0, 255]);
        assert!(only_shadow.is_some(), "some pixels are just the shadow, in its own color");
    }

    #[test]
    fn a_transparent_shadow_draws_nothing_extra() {
        let plain = drawn_text(None, BLUE);
        let clear = Some(TextShadow { color: Color { a: 0.0, ..RED }, offset_x: 6, offset_y: 6 });
        assert!(drawn_text(clear, BLUE).data == plain.data);
    }

    #[test]
    fn a_shadows_opacity_follows_the_texts() {
        let shadow = Some(TextShadow { color: Color { r: 0, g: 0, b: 0, a: 1.0 }, offset_x: 12, offset_y: 0 });
        let faint = Color { a: 0.25, ..BLUE };
        let t = drawn_text(shadow, faint);
        // The shadow is full black at full text opacity, but only a quarter dark here, so nothing is darker than that.
        let darkest = (0..60).flat_map(|x| (0..40).map(move |y| (x, y))).map(|(x, y)| pixel(&t, x, y)[0]).min().unwrap();
        assert!(darkest >= 255 - 70, "the shadow of 25% text is at most about 25% dark, not {darkest}");
    }

    #[test]
    fn a_shadow_past_the_edge_of_a_clip_is_cut_off_with_the_text() {
        let mut t = white(60, 40);
        let shadow = Some(TextShadow { color: RED, offset_x: 20, offset_y: 0 });
        let clip = Some(Rect { x: 0, y: 0, w: 22, h: 40 });
        let cmd = DrawCmd::Text { x: 10, y: 8, text: "I".into(), font: FontKey::regular(FontFamily::Mono), font_size: 24, color: BLUE, clip, shadow };
        run(&mut t, &[cmd]);
        assert!(ink_columns(&t).iter().all(|&x| x < 22));
    }

    #[test]
    fn boxes_are_cut_off_at_their_clip() {
        let mut t = white(20, 20);
        let clip = Some(Rect { x: 5, y: 5, w: 6, h: 6 });
        let cmd = |radius, border| DrawCmd::Box { rect: Rect { x: 0, y: 0, w: 20, h: 20 }, background: RED, border: [border; 4], border_color: BLUE, radius, clip };
        for (radius, border) in [(0, 0), (0, 2), (8, 0), (8, 3)] {
            let mut t = white(20, 20);
            run(&mut t, &[cmd(radius, border)]);
            for y in 0..20 {
                for x in 0..20 {
                    let inside = (5..11).contains(&x) && (5..11).contains(&y);
                    assert!(inside || pixel(&t, x, y) == WHITE, "radius {radius} border {border}: ({x}, {y}) is outside the clip but drawn");
                }
            }
            assert_ne!(pixel(&t, 7, 7), WHITE, "radius {radius} border {border}: and inside it is drawn");
        }
        run(&mut t, &[DrawCmd::Box { rect: Rect { x: 0, y: 0, w: 4, h: 4 }, background: RED, border: [0; 4], border_color: BLUE, radius: 0, clip }]);
        assert_eq!(pixel(&t, 1, 1), WHITE, "a box wholly outside its clip draws nothing");
    }

    #[test]
    fn the_clip_moves_with_scrolling() {
        // The same command drawn 10px higher: the clip has to go with it.
        let mut fonts = Fonts::new();
        let mut t = white(30, 30);
        let cmd = DrawCmd::Box { rect: Rect { x: 0, y: 10, w: 30, h: 30 }, background: RED, border: [0; 4], border_color: BLUE, radius: 0, clip: Some(Rect { x: 0, y: 10, w: 30, h: 8 }) };
        t.execute(&[cmd], -10, &mut fonts);
        assert_eq!(pixel(&t, 5, 7), RED_PIXEL);
        assert_eq!(pixel(&t, 5, 9), WHITE, "cut at the moved clip's bottom edge, which is at y = 8");
    }

    #[test]
    fn images_are_cut_off_at_their_clip() {
        let img = image(1, 1, &[[255, 0, 0, 255]]);
        for filter in [ImageFilter::Smooth, ImageFilter::Nearest] {
            let mut t = white(20, 20);
            let clip = Some(Rect { x: 4, y: 4, w: 5, h: 5 });
            run(&mut t, &[DrawCmd::Image { rect: Rect { x: 0, y: 0, w: 20, h: 20 }, image: img.clone(), radius: 6, filter, clip }]);
            assert_eq!(pixel(&t, 6, 6), [255, 0, 0, 255], "{filter:?}");
            assert_eq!(pixel(&t, 3, 6), WHITE, "{filter:?}");
            assert_eq!(pixel(&t, 9, 6), WHITE, "{filter:?}");
            assert_eq!(pixel(&t, 6, 10), WHITE, "{filter:?}");
        }
    }

    #[test]
    fn nearest_filter_keeps_enlarged_pixels_crisp() {
        let (b, w) = ([0, 0, 0, 255], [255, 255, 255, 255]);
        let img = image(2, 1, &[b, w]);
        let mut t = white(8, 2);
        run(&mut t, &[DrawCmd::Image { rect: Rect { x: 0, y: 0, w: 8, h: 2 }, image: img, radius: 0, filter: ImageFilter::Nearest, clip: None }]);
        let row: Vec<u8> = (0..8).map(|x| pixel(&t, x, 0)[0]).collect();
        assert_eq!(row, vec![0, 0, 0, 0, 255, 255, 255, 255], "no blending between the two pixels");
        assert_eq!(pixel(&t, 0, 1), pixel(&t, 0, 0), "and each source row is repeated");
    }

    #[test]
    fn nearest_filter_shrinks_without_inventing_colors() {
        let (r, g) = ([255, 0, 0, 255], [0, 255, 0, 255]);
        let img = image(4, 1, &[r, r, g, g]);
        let mut t = white(2, 1);
        run(&mut t, &[DrawCmd::Image { rect: Rect { x: 0, y: 0, w: 2, h: 1 }, image: img, radius: 0, filter: ImageFilter::Nearest, clip: None }]);
        assert_eq!((pixel(&t, 0, 0), pixel(&t, 1, 0)), (r, g));
    }

    #[test]
    fn a_translucent_image_pixel_blends_in_the_copy_path_too() {
        let img = image(2, 1, &[[0, 0, 0, 128], [0, 0, 0, 0]]);
        let mut t = white(2, 1);
        run(&mut t, &[DrawCmd::Image { rect: Rect { x: 0, y: 0, w: 2, h: 1 }, image: img, radius: 0, filter: ImageFilter::Nearest, clip: None }]);
        assert!((126..=129).contains(&pixel(&t, 0, 0)[0]));
        assert_eq!(pixel(&t, 1, 0), WHITE, "fully transparent is skipped");
    }

    #[test]
    fn text_is_cut_off_at_its_clip() {
        let mut fonts = Fonts::new();
        let mono = FontKey::regular(FontFamily::Mono);
        let clip = Rect { x: 10, y: 0, w: 12, h: 40 };
        let mut t = white(120, 40);
        let cmd = DrawCmd::Text { x: 4, y: 4, text: "WWWWWWWW".into(), font: mono, font_size: 20, color: BLUE, clip: Some(clip), shadow: None };
        t.execute(&[cmd], 0, &mut fonts);
        let inked = |x: u32| (0..40).any(|y| pixel(&t, x, y) != WHITE);
        assert!((10..22).any(inked), "ink inside the clip");
        assert!(!(0..10).any(inked) && !(22..120).any(inked), "none outside it");
    }

    #[test]
    fn image_clips_to_the_texture() {
        let img = image(1, 1, &[[255, 0, 0, 255]]);
        let mut t = white(4, 4);
        draw_image(&mut t, Rect { x: -10, y: -10, w: 100, h: 100 }, &img, 0);
        assert_eq!(pixel(&t, 3, 3), [255, 0, 0, 255]);
    }

    fn text_cmd(family: FontFamily, text: &str) -> DrawCmd {
        DrawCmd::Text { x: 10, y: 10, text: text.into(), font: FontKey::regular(family), font_size: 16, color: BLUE, clip: None, shadow: None }
    }

    #[test]
    fn text_draws_inside_its_line_only() {
        let mut fonts = Fonts::new();
        let metrics = fonts.line_metrics(FontKey::regular(FontFamily::Mono), 16);
        let width = fonts.text_width(FontKey::regular(FontFamily::Mono), 16, "MM").ceil() as u32;
        let (w, h) = (width + 20, metrics.height as u32 + 20);
        let mut t = white(w, h);
        t.execute(&[text_cmd(FontFamily::Mono, "MM")], 0, &mut fonts);

        // "Solid" means mostly blue: red and green have dropped well below the white background.
        let blue_in = |x0: u32, x1: u32| (0..h).any(|y| (x0..x1).any(|x| pixel(&t, x, y)[0] < 64));
        assert!(blue_in(10, 10 + width), "text should have solid pixels where it was drawn");
        assert!(!blue_in(0, 9) && !blue_in(10 + width + 1, w), "nothing outside the text");
        // Anti-aliased edges are partial blends of blue over white: red and green drop together, blue stays.
        let edge = (0..h).flat_map(|y| (0..w).map(move |x| (x, y))).map(|(x, y)| pixel(&t, x, y)).find(|p| p[0] > 64 && p[0] < 255);
        let edge = edge.expect("anti-aliased edge pixels");
        assert_eq!(edge[0], edge[1]);
        assert_eq!(edge[2], 255);
    }

    #[test]
    fn sans_text_is_drawn_narrower_than_mono() {
        let ink_width = |family| {
            let mut t = white(200, 40);
            t.execute(&[text_cmd(family, "iiiiiiii")], 0, &mut Fonts::new());
            let inked: Vec<u32> = (0..200).filter(|&x| (0..40).any(|y| pixel(&t, x, y)[0] < 200)).collect();
            inked.last().unwrap() - inked.first().unwrap()
        };
        assert!(ink_width(FontFamily::Sans) < ink_width(FontFamily::Mono));
    }

    /// The darkness of everything drawn, as the sum of how far each pixel is from white.
    fn ink(font: FontKey) -> (u32, Vec<u8>) {
        let mut t = white(120, 40);
        let cmd = DrawCmd::Text { x: 4, y: 4, text: "Hamburg".into(), font, font_size: 24, color: BLUE, clip: None, shadow: None };
        t.execute(&[cmd], 0, &mut Fonts::new());
        let total = t.data.chunks_exact(4).map(|p| 255 - u32::from(p[0])).sum();
        (total, t.data)
    }

    #[test]
    fn bold_text_has_more_ink_than_regular() {
        let regular = FontKey::regular(FontFamily::Sans);
        let bold = FontKey { bold: true, ..regular };
        assert!(ink(bold).0 > ink(regular).0 * 11 / 10, "bold strokes are noticeably heavier");
    }

    #[test]
    fn italic_text_is_drawn_differently_from_regular() {
        let regular = FontKey::regular(FontFamily::Sans);
        let italic = FontKey { italic: true, ..regular };
        assert_ne!(ink(italic).1, ink(regular).1);
        assert!(ink(italic).0 > 0);
    }
}
