//! Software rasterizer: paints the annotation shapes into an `RgbaImage`.
//!
//! Design notes
//! ------------
//! * Everything is drawn in **image pixel space**. The overlay UI calls
//!   `render_region` for exactly the current selection rectangle and displays
//!   the result 1:1, which makes the on-screen preview byte-identical to the
//!   exported file — no duplicated drawing logic between preview and export.
//! * Anti-aliasing is done with a float coverage mask (max-combined per
//!   primitive, summed across polygon sub-scanlines) and a single composite
//!   step, so semi-transparent tools such as the highlighter never darken where
//!   their own segments overlap.
//! * Mosaic and blur always sample the *original* capture, never already
//!   painted pixels, which is what users expect when re-editing.

use crate::fonts;
use crate::shapes::{Pt, RectF, Rgba, Shape, StickerKind};
use ab_glyph::{Font, PxScale, ScaleFont};
use image::RgbaImage;

#[derive(Debug, Clone, Copy)]
pub struct Region {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

impl Region {
    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        let x0 = x.floor() as i32;
        let y0 = y.floor() as i32;
        let x1 = (x + w).ceil() as i32;
        let y1 = (y + h).ceil() as i32;
        Self {
            x: x0,
            y: y0,
            w: (x1 - x0).max(0) as u32,
            h: (y1 - y0).max(0) as u32,
        }
    }

    pub fn from_rect(r: &RectF) -> Self {
        Region::new(r.x, r.y, r.w, r.h)
    }

    pub fn clamped(&self, img_w: u32, img_h: u32) -> Region {
        if img_w == 0 || img_h == 0 {
            return Region {
                x: 0,
                y: 0,
                w: 1,
                h: 1,
            };
        }
        let x = self.x.clamp(0, img_w as i32 - 1);
        let y = self.y.clamp(0, img_h as i32 - 1);
        let w = self.w.min((img_w as i32 - x) as u32).max(1);
        let h = self.h.min((img_h as i32 - y) as u32).max(1);
        Region { x, y, w, h }
    }
}

// ---------------------------------------------------------------------------
// blending
// ---------------------------------------------------------------------------

#[inline]
fn blend_raw(buf: &mut [u8], w: u32, h: u32, x: i32, y: i32, color: Rgba, a: f32) {
    if a <= 0.0 || x < 0 || y < 0 || x >= w as i32 || y >= h as i32 {
        return;
    }
    let sa = (color[3] as f32 / 255.0) * a.clamp(0.0, 1.0);
    if sa <= 0.0 {
        return;
    }
    let idx = ((y as u32 * w + x as u32) * 4) as usize;
    let da = buf[idx + 3] as f32 / 255.0;
    let oa = sa + da * (1.0 - sa);
    if oa <= 0.0 {
        return;
    }
    for i in 0..3 {
        let sc = color[i] as f32;
        let dc = buf[idx + i] as f32;
        let v = (sc * sa + dc * da * (1.0 - sa)) / oa;
        buf[idx + i] = (v + 0.5).clamp(0.0, 255.0) as u8;
    }
    buf[idx + 3] = (oa * 255.0 + 0.5).clamp(0.0, 255.0) as u8;
}

#[inline]
fn dist_point_segment(p: Pt, a: Pt, b: Pt) -> f32 {
    let vx = b[0] - a[0];
    let vy = b[1] - a[1];
    let wx = p[0] - a[0];
    let wy = p[1] - a[1];
    let len2 = vx * vx + vy * vy;
    let t = if len2 <= 1e-6 {
        0.0
    } else {
        ((wx * vx + wy * vy) / len2).clamp(0.0, 1.0)
    };
    let dx = wx - vx * t;
    let dy = wy - vy * t;
    (dx * dx + dy * dy).sqrt()
}

// ---------------------------------------------------------------------------
// coverage mask
// ---------------------------------------------------------------------------

struct Mask {
    x0: i32,
    y0: i32,
    w: i32,
    h: i32,
    data: Vec<f32>,
}

impl Mask {
    fn new(x0: i32, y0: i32, w: i32, h: i32) -> Mask {
        Mask {
            x0,
            y0,
            w,
            h,
            data: vec![0.0; (w.max(0) * h.max(0)) as usize],
        }
    }

    /// Allocate a mask clipped to the surface; `None` when fully outside.
    fn for_bounds(
        min_x: f32,
        min_y: f32,
        max_x: f32,
        max_y: f32,
        surf_w: u32,
        surf_h: u32,
    ) -> Option<Mask> {
        let x0 = (min_x.floor() as i32).max(0);
        let y0 = (min_y.floor() as i32).max(0);
        let x1 = (max_x.ceil() as i32).min(surf_w as i32);
        let y1 = (max_y.ceil() as i32).min(surf_h as i32);
        if x1 <= x0 || y1 <= y0 {
            return None;
        }
        Some(Mask::new(x0, y0, x1 - x0, y1 - y0))
    }

    #[inline]
    fn raise(&mut self, x: i32, y: i32, v: f32) {
        let lx = x - self.x0;
        let ly = y - self.y0;
        if lx < 0 || ly < 0 || lx >= self.w || ly >= self.h {
            return;
        }
        let idx = (ly * self.w + lx) as usize;
        if v > self.data[idx] {
            self.data[idx] = v;
        }
    }

    fn add_capsule(&mut self, a: Pt, b: Pt, width: f32) {
        let r = (width * 0.5).max(0.35);
        let min_x = a[0].min(b[0]) - r - 1.0;
        let max_x = a[0].max(b[0]) + r + 1.0;
        let min_y = a[1].min(b[1]) - r - 1.0;
        let max_y = a[1].max(b[1]) + r + 1.0;
        let px0 = (min_x.floor() as i32).max(self.x0);
        let px1 = (max_x.ceil() as i32).min(self.x0 + self.w);
        let py0 = (min_y.floor() as i32).max(self.y0);
        let py1 = (max_y.ceil() as i32).min(self.y0 + self.h);
        for y in py0..py1 {
            let cy = y as f32 + 0.5;
            for x in px0..px1 {
                let cx = x as f32 + 0.5;
                let d = dist_point_segment([cx, cy], a, b);
                let cov = (r + 0.5 - d).clamp(0.0, 1.0);
                if cov > 0.0 {
                    self.raise(x, y, cov);
                }
            }
        }
    }

    fn add_circle(&mut self, c: Pt, radius: f32) {
        let px0 = ((c[0] - radius - 1.0).floor() as i32).max(self.x0);
        let px1 = ((c[0] + radius + 1.0).ceil() as i32).min(self.x0 + self.w);
        let py0 = ((c[1] - radius - 1.0).floor() as i32).max(self.y0);
        let py1 = ((c[1] + radius + 1.0).ceil() as i32).min(self.y0 + self.h);
        for y in py0..py1 {
            let cy = y as f32 + 0.5 - c[1];
            for x in px0..px1 {
                let cx = x as f32 + 0.5 - c[0];
                let d = (cx * cx + cy * cy).sqrt();
                let cov = (radius + 0.5 - d).clamp(0.0, 1.0);
                if cov > 0.0 {
                    self.raise(x, y, cov);
                }
            }
        }
    }

    fn add_polyline(&mut self, pts: &[Pt], width: f32, closed: bool) {
        if pts.is_empty() {
            return;
        }
        if pts.len() == 1 {
            self.add_circle(pts[0], (width * 0.5).max(0.35));
            return;
        }
        for i in 0..pts.len() - 1 {
            self.add_capsule(pts[i], pts[i + 1], width);
        }
        if closed {
            self.add_capsule(pts[pts.len() - 1], pts[0], width);
        }
    }

    /// Anti-aliased even-odd polygon fill with 4 sub-scanlines per pixel row.
    fn add_polygon(&mut self, pts: &[Pt]) {
        if pts.len() < 3 {
            return;
        }
        let y_min = pts.iter().map(|p| p[1]).fold(f32::MAX, f32::min);
        let y_max = pts.iter().map(|p| p[1]).fold(f32::MIN, f32::max);
        let py0 = (y_min.floor() as i32).max(self.y0);
        let py1 = (y_max.ceil() as i32).min(self.y0 + self.h);
        if py1 <= py0 {
            return;
        }
        let mut crossings: Vec<f32> = Vec::with_capacity(16);
        let mut row = vec![0.0f32; self.w.max(0) as usize];
        for y in py0..py1 {
            for v in row.iter_mut() {
                *v = 0.0;
            }
            for k in 0..4 {
                let sy = y as f32 + (k as f32 + 0.5) * 0.25;
                crossings.clear();
                for i in 0..pts.len() {
                    let a = pts[i];
                    let b = pts[(i + 1) % pts.len()];
                    if (a[1] <= sy) != (b[1] <= sy) {
                        let t = (sy - a[1]) / (b[1] - a[1]);
                        crossings.push(a[0] + t * (b[0] - a[0]));
                    }
                }
                if crossings.len() < 2 {
                    continue;
                }
                crossings.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                let mut i = 0;
                while i + 1 < crossings.len() {
                    let x0 = crossings[i];
                    let x1 = crossings[i + 1];
                    i += 2;
                    if x1 <= x0 {
                        continue;
                    }
                    let start = (x0.floor() as i32).max(self.x0);
                    let end = (x1.ceil() as i32).min(self.x0 + self.w);
                    for px in start..end {
                        let l = x0.max(px as f32);
                        let r = x1.min(px as f32 + 1.0);
                        if r > l {
                            let idx = (px - self.x0) as usize;
                            row[idx] += (r - l) * 0.25;
                        }
                    }
                }
            }
            for (lx, cov) in row.iter().enumerate() {
                if *cov > 0.001 {
                    let idx = ((y - self.y0) * self.w + lx as i32) as usize;
                    if *cov > self.data[idx] {
                        self.data[idx] = (*cov).min(1.0);
                    }
                }
            }
        }
    }

    fn composite(&self, buf: &mut [u8], w: u32, h: u32, color: Rgba, global_alpha: f32) {
        for y in 0..self.h {
            for x in 0..self.w {
                let cov = self.data[(y * self.w + x) as usize];
                if cov > 0.002 {
                    blend_raw(buf, w, h, self.x0 + x, self.y0 + y, color, cov * global_alpha);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// text
// ---------------------------------------------------------------------------

fn scaled_font(size: f32) -> Option<(&'static ab_glyph::FontVec, ab_glyph::PxScale)> {
    let font = fonts::glyph_font()?;
    let px = size.max(4.0);
    Some((font, PxScale::from(px)))
}

pub fn measure_text(text: &str, size: f32) -> (f32, f32) {
    let Some((font, px)) = scaled_font(size) else {
        return (text.chars().count() as f32 * size * 0.6, size);
    };
    let scaled = font.as_scaled(px);
    let mut w = 0.0f32;
    for ch in text.chars() {
        let gid = font.glyph_id(ch);
        w += scaled.h_advance(gid);
    }
    (w, size)
}

fn text_width(font: &ab_glyph::FontVec, px: ab_glyph::PxScale, text: &str) -> f32 {
    let scaled = font.as_scaled(px);
    text.chars()
        .map(|c| scaled.h_advance(font.glyph_id(c)))
        .sum()
}

fn ascent(font: &ab_glyph::FontVec, px: ab_glyph::PxScale) -> f32 {
    font.as_scaled(px).ascent()
}

/// Draw a single line of text with its top-left corner at `pos` (surface coords).
fn draw_text_at(
    buf: &mut [u8],
    w: u32,
    h: u32,
    pos: Pt,
    text: &str,
    size: f32,
    color: Rgba,
    alpha: f32,
) {
    let Some((font, px)) = scaled_font(size) else {
        return;
    };
    let baseline = pos[1] + ascent(&font, px);
    let mut caret = 0.0f32;
    for ch in text.chars() {
        let gid = font.glyph_id(ch);
        if ch == ' ' {
            caret += font.as_scaled(px).h_advance(gid);
            continue;
        }
        let glyph = gid.with_scale_and_position(px, ab_glyph::point(pos[0] + caret, baseline));
        if let Some(outlined) = font.outline_glyph(glyph) {
            let bb = outlined.px_bounds();
            let ox = bb.min.x as i32;
            let oy = bb.min.y as i32;
            outlined.draw(|gx, gy, cov| {
                blend_raw(buf, w, h, ox + gx as i32, oy + gy as i32, color, cov * alpha);
            });
        }
        caret += font.as_scaled(px).h_advance(gid);
    }
}

fn draw_text_centered(
    buf: &mut [u8],
    w: u32,
    h: u32,
    center: Pt,
    text: &str,
    size: f32,
    color: Rgba,
) {
    let Some((font, px)) = scaled_font(size) else {
        return;
    };
    let tw = text_width(&font, px, text);
    let scaled = font.as_scaled(px);
    let baseline_shift = (scaled.ascent() + scaled.descent()) * 0.5;
    let pos = [center[0] - tw * 0.5, center[1] - baseline_shift];
    draw_text_at(buf, w, h, pos, text, size, color, 1.0);
}

// ---------------------------------------------------------------------------
// helpers for effect shapes
// ---------------------------------------------------------------------------

fn base_pixel(base: &RgbaImage, x: i32, y: i32) -> Rgba {
    let (bw, bh) = (base.width() as i32, base.height() as i32);
    let cx = x.clamp(0, bw - 1);
    let cy = y.clamp(0, bh - 1);
    let p = base.get_pixel(cx as u32, cy as u32);
    [p[0], p[1], p[2], p[3]]
}

fn draw_mosaic(
    buf: &mut [u8],
    w: u32,
    h: u32,
    origin: (i32, i32),
    base: &RgbaImage,
    rect: RectF,
    block: u32,
) {
    let block = block.max(2) as f32;
    let x0 = rect.x.min(rect.x + rect.w);
    let y0 = rect.y.min(rect.y + rect.h);
    let x1 = rect.x + rect.w.abs();
    let y1 = rect.y + rect.h.abs();
    let bx0 = (x0 / block).floor() * block;
    let by0 = (y0 / block).floor() * block;
    let mut by = by0;
    while by < y1 {
        let mut bx = bx0;
        while bx < x1 {
            // Average the source block (clipped to the mosaic rect and image).
            let sx0 = bx.max(x0).max(0.0) as i32;
            let sy0 = by.max(y0).max(0.0) as i32;
            let sx1 = ((bx + block).min(x1).min(base.width() as f32)) as i32;
            let sy1 = ((by + block).min(y1).min(base.height() as f32)) as i32;
            if sx1 > sx0 && sy1 > sy0 {
                let mut acc = [0u64; 4];
                let mut n = 0u64;
                for yy in sy0..sy1 {
                    for xx in sx0..sx1 {
                        let p = base_pixel(base, xx, yy);
                        acc[0] += p[0] as u64;
                        acc[1] += p[1] as u64;
                        acc[2] += p[2] as u64;
                        acc[3] += p[3] as u64;
                        n += 1;
                    }
                }
                if n > 0 {
                    let avg = [
                        (acc[0] / n) as u8,
                        (acc[1] / n) as u8,
                        (acc[2] / n) as u8,
                        (acc[3] / n) as u8,
                    ];
                    // Fill the block area (clipped to the mosaic rect).
                    let fx0 = (bx.max(x0)) as i32 - origin.0;
                    let fy0 = (by.max(y0)) as i32 - origin.1;
                    let fx1 = ((bx + block).min(x1)) as i32 - origin.0;
                    let fy1 = ((by + block).min(y1)) as i32 - origin.1;
                    for yy in fy0..fy1 {
                        if yy < 0 || yy >= h as i32 {
                            continue;
                        }
                        for xx in fx0..fx1 {
                            if xx < 0 || xx >= w as i32 {
                                continue;
                            }
                            let idx = ((yy as u32 * w + xx as u32) * 4) as usize;
                            buf[idx] = avg[0];
                            buf[idx + 1] = avg[1];
                            buf[idx + 2] = avg[2];
                            buf[idx + 3] = avg[3];
                        }
                    }
                }
            }
            bx += block;
        }
        by += block;
    }
}

fn draw_blur(
    buf: &mut [u8],
    w: u32,
    h: u32,
    origin: (i32, i32),
    base: &RgbaImage,
    rect: RectF,
    radius: f32,
) {
    let x0 = rect.x.min(rect.x + rect.w).floor().max(0.0) as i32;
    let y0 = rect.y.min(rect.y + rect.h).floor().max(0.0) as i32;
    let x1 = (rect.x + rect.w.abs()).ceil().min(base.width() as f32) as i32;
    let y1 = (rect.y + rect.h.abs()).ceil().min(base.height() as f32) as i32;
    let (rw, rh) = (x1 - x0, y1 - y0);
    if rw <= 0 || rh <= 0 {
        return;
    }
    let r = radius.round().max(1.0) as i32;
    // Copy source region with a border margin so the blur does not sample garbage.
    let margin = r;
    let sx0 = x0 - margin;
    let sy0 = y0 - margin;
    let sw = rw + margin * 2;
    let sh = rh + margin * 2;
    let mut src = vec![0f32; (sw * sh * 3) as usize];
    for yy in 0..sh {
        for xx in 0..sw {
            let p = base_pixel(base, sx0 + xx, sy0 + yy);
            let i = ((yy * sw + xx) * 3) as usize;
            src[i] = p[0] as f32;
            src[i + 1] = p[1] as f32;
            src[i + 2] = p[2] as f32;
        }
    }
    // Separable box blur, two passes for a smoother falloff.
    let mut tmp = vec![0f32; src.len()];
    for _ in 0..2 {
        box_blur_h(&src, &mut tmp, sw, sh, r);
        box_blur_v(&tmp, &mut src, sw, sh, r);
    }
    for yy in 0..rh {
        for xx in 0..rw {
            let src_idx = (((yy + margin) * sw + (xx + margin)) * 3) as usize;
            let dx = (x0 + xx) - origin.0;
            let dy = (y0 + yy) - origin.1;
            if dx < 0 || dy < 0 || dx >= w as i32 || dy >= h as i32 {
                continue;
            }
            let idx = ((dy as u32 * w + dx as u32) * 4) as usize;
            buf[idx] = src[src_idx].clamp(0.0, 255.0) as u8;
            buf[idx + 1] = src[src_idx + 1].clamp(0.0, 255.0) as u8;
            buf[idx + 2] = src[src_idx + 2].clamp(0.0, 255.0) as u8;
            buf[idx + 3] = 255;
        }
    }
}

fn box_blur_h(src: &[f32], dst: &mut [f32], w: i32, h: i32, r: i32) {
    let win = (2 * r + 1) as f32;
    for y in 0..h {
        for ch in 0..3 {
            for x in 0..w {
                let mut sum = 0.0;
                for k in -r..=r {
                    let xx = (x + k).clamp(0, w - 1);
                    sum += src[((y * w + xx) * 3 + ch) as usize];
                }
                dst[((y * w + x) * 3 + ch) as usize] = sum / win;
            }
        }
    }
}

fn box_blur_v(src: &[f32], dst: &mut [f32], w: i32, h: i32, r: i32) {
    let win = (2 * r + 1) as f32;
    for x in 0..w {
        for ch in 0..3 {
            for y in 0..h {
                let mut sum = 0.0;
                for k in -r..=r {
                    let yy = (y + k).clamp(0, h - 1);
                    sum += src[((yy * w + x) * 3 + ch) as usize];
                }
                dst[((y * w + x) * 3 + ch) as usize] = sum / win;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// stickers
// ---------------------------------------------------------------------------

fn star_points(center: Pt, size: f32) -> Vec<Pt> {
    let outer = size * 0.5;
    let inner = outer * 0.45;
    let mut pts = Vec::with_capacity(10);
    for i in 0..10 {
        let r = if i % 2 == 0 { outer } else { inner };
        let angle = -std::f64::consts::FRAC_PI_2 + (i as f64) * std::f64::consts::PI / 5.0;
        pts.push([
            center[0] + (angle.cos() * r as f64) as f32,
            center[1] + (angle.sin() * r as f64) as f32,
        ]);
    }
    pts
}

fn heart_points(center: Pt, size: f32) -> Vec<Pt> {
    let s = size / 32.0;
    let mut pts = Vec::with_capacity(48);
    for i in 0..48 {
        let t = (i as f32) / 48.0 * std::f32::consts::TAU;
        let x = 16.0 * t.sin().powi(3);
        let y = 13.0 * t.cos() - 5.0 * (2.0 * t).cos() - 2.0 * (3.0 * t).cos() - (4.0 * t).cos();
        pts.push([center[0] + x * s, center[1] - y * s]);
    }
    pts
}

fn draw_sticker(
    buf: &mut [u8],
    w: u32,
    h: u32,
    origin: (i32, i32),
    center: Pt,
    size: f32,
    kind: StickerKind,
    color: Rgba,
) {
    let c = [center[0] - origin.0 as f32, center[1] - origin.1 as f32];
    let size = size.max(8.0);
    let half = size * 0.5;
    let stroke_w = (size * 0.17).max(1.5);

    match kind {
        StickerKind::Check => {
            let pts = [
                [c[0] - half * 0.62, c[1] + 0.02 * half],
                [c[0] - half * 0.16, c[1] + half * 0.48],
                [c[0] + half * 0.66, c[1] - half * 0.46],
            ];
            if let Some(mut mask) = Mask::for_bounds(
                c[0] - half,
                c[1] - half,
                c[0] + half,
                c[1] + half,
                w,
                h,
            ) {
                mask.add_polyline(&pts, stroke_w, false);
                mask.composite(buf, w, h, color, 1.0);
            }
        }
        StickerKind::Cross => {
            if let Some(mut mask) = Mask::for_bounds(
                c[0] - half,
                c[1] - half,
                c[0] + half,
                c[1] + half,
                w,
                h,
            ) {
                let d = half * 0.58;
                mask.add_capsule([c[0] - d, c[1] - d], [c[0] + d, c[1] + d], stroke_w);
                mask.add_capsule([c[0] + d, c[1] - d], [c[0] - d, c[1] + d], stroke_w);
                mask.composite(buf, w, h, color, 1.0);
            }
        }
        StickerKind::Star => {
            if let Some(mut mask) = Mask::for_bounds(
                c[0] - half,
                c[1] - half,
                c[0] + half,
                c[1] + half,
                w,
                h,
            ) {
                let pts = star_points(c, size);
                mask.add_polygon(&pts);
                mask.composite(buf, w, h, color, 1.0);
            }
        }
        StickerKind::Heart => {
            if let Some(mut mask) = Mask::for_bounds(
                c[0] - half,
                c[1] - half,
                c[0] + half,
                c[1] + half,
                w,
                h,
            ) {
                let pts = heart_points(c, size);
                mask.add_polygon(&pts);
                mask.composite(buf, w, h, color, 1.0);
            }
        }
        StickerKind::Exclaim => {
            if let Some(mut mask) = Mask::for_bounds(
                c[0] - half,
                c[1] - half,
                c[0] + half,
                c[1] + half,
                w,
                h,
            ) {
                mask.add_capsule(
                    [c[0], c[1] - half * 0.72],
                    [c[0], c[1] + half * 0.2],
                    size * 0.22,
                );
                mask.add_circle([c[0], c[1] + half * 0.62], size * 0.12);
                mask.composite(buf, w, h, color, 1.0);
            }
        }
        StickerKind::Question => {
            draw_text_centered(buf, w, h, c, "?", size, color);
        }
        StickerKind::Arrow => {
            let shaft_a = [c[0] - half * 0.78, c[1]];
            let shaft_b = [c[0] + half * 0.2, c[1]];
            let head = [
                [c[0] + half * 0.82, c[1]],
                [c[0] + half * 0.06, c[1] - half * 0.56],
                [c[0] + half * 0.06, c[1] + half * 0.56],
            ];
            if let Some(mut mask) = Mask::for_bounds(
                c[0] - half,
                c[1] - half,
                c[0] + half,
                c[1] + half,
                w,
                h,
            ) {
                mask.add_capsule(shaft_a, shaft_b, stroke_w);
                mask.add_polygon(&head);
                mask.composite(buf, w, h, color, 1.0);
            }
        }
        StickerKind::Circle => {
            if let Some(mut mask) = Mask::for_bounds(
                c[0] - half,
                c[1] - half,
                c[0] + half,
                c[1] + half,
                w,
                h,
            ) {
                let r = half * 0.82;
                let mut pts = Vec::with_capacity(48);
                for i in 0..48 {
                    let a = (i as f32) / 48.0 * std::f32::consts::TAU;
                    pts.push([c[0] + a.cos() * r, c[1] + a.sin() * r]);
                }
                mask.add_polyline(&pts, stroke_w, true);
                mask.composite(buf, w, h, color, 1.0);
            }
        }
    }
}

fn draw_number(
    buf: &mut [u8],
    w: u32,
    h: u32,
    origin: (i32, i32),
    center: Pt,
    size: f32,
    n: u32,
    color: Rgba,
) {
    let c = [center[0] - origin.0 as f32, center[1] - origin.1 as f32];
    let radius = (size * 0.5).max(8.0);
    if let Some(mut mask) = Mask::for_bounds(
        c[0] - radius - 1.0,
        c[1] - radius - 1.0,
        c[0] + radius + 1.0,
        c[1] + radius + 1.0,
        w,
        h,
    ) {
        mask.add_circle(c, radius);
        mask.composite(buf, w, h, color, 1.0);
    }
    let label = n.to_string();
    let font_size = (radius * 1.15).max(9.0);
    let text_color = if (color[0] as u32 * 299 + color[1] as u32 * 587 + color[2] as u32 * 114) / 1000
        > 150
    {
        [20, 20, 20, 255]
    } else {
        [255, 255, 255, 255]
    };
    draw_text_centered(buf, w, h, c, &label, font_size, text_color);
}

fn draw_bitmap(
    buf: &mut [u8],
    w: u32,
    h: u32,
    origin: (i32, i32),
    rect: RectF,
    image: &RgbaImage,
) {
    if image.width() == 0 || image.height() == 0 || rect.w <= 0.0 || rect.h <= 0.0 {
        return;
    }
    let x0 = (rect.x - origin.0 as f32).max(0.0) as i32;
    let y0 = (rect.y - origin.1 as f32).max(0.0) as i32;
    let x1 = ((rect.x + rect.w) - origin.0 as f32).min(w as f32) as i32;
    let y1 = ((rect.y + rect.h) - origin.1 as f32).min(h as f32) as i32;
    let sx = image.width() as f32 / rect.w;
    let sy = image.height() as f32 / rect.h;
    let raw = image.as_raw();
    for dy in y0..y1 {
        for dx in x0..x1 {
            let u = ((dx as f32 - (rect.x - origin.0 as f32)) * sx) as u32;
            let v = ((dy as f32 - (rect.y - origin.1 as f32)) * sy) as u32;
            let u = u.min(image.width() - 1);
            let v = v.min(image.height() - 1);
            let si = ((v * image.width() + u) * 4) as usize;
            let a = raw[si + 3] as f32 / 255.0;
            if a <= 0.0 {
                continue;
            }
            blend_raw(
                buf,
                w,
                h,
                dx,
                dy,
                [raw[si], raw[si + 1], raw[si + 2], raw[si + 3]],
                a,
            );
        }
    }
}

// ---------------------------------------------------------------------------
// shape dispatch
// ---------------------------------------------------------------------------

pub fn draw_shape(
    buf: &mut [u8],
    w: u32,
    h: u32,
    origin: (i32, i32),
    base: &RgbaImage,
    shape: &Shape,
) {
    let ox = origin.0 as f32;
    let oy = origin.1 as f32;
    match shape {
        Shape::Stroke {
            points,
            color,
            width,
            alpha,
        } => {
            if points.is_empty() {
                return;
            }
            let pts: Vec<Pt> = points.iter().map(|p| [p[0] - ox, p[1] - oy]).collect();
            let min = pts.iter().fold([f32::MAX, f32::MAX], |m, p| {
                [m[0].min(p[0]), m[1].min(p[1])]
            });
            let max = pts.iter().fold([f32::MIN, f32::MIN], |m, p| {
                [m[0].max(p[0]), m[1].max(p[1])]
            });
            let pad = width * 0.5 + 2.0;
            if let Some(mut mask) = Mask::for_bounds(
                min[0] - pad,
                min[1] - pad,
                max[0] + pad,
                max[1] + pad,
                w,
                h,
            ) {
                mask.add_polyline(&pts, *width, false);
                mask.composite(buf, w, h, *color, *alpha);
            }
        }
        Shape::Rect { a, b, color, width } => {
            let r = RectF::from_points(*a, *b);
            let (x0, y0) = (r.min()[0] - ox, r.min()[1] - oy);
            let (x1, y1) = (r.max()[0] - ox, r.max()[1] - oy);
            let pad = width * 0.5 + 2.0;
            if let Some(mut mask) =
                Mask::for_bounds(x0 - pad, y0 - pad, x1 + pad, y1 + pad, w, h)
            {
                let pts = [[x0, y0], [x1, y0], [x1, y1], [x0, y1]];
                mask.add_polyline(&pts, *width, true);
                mask.composite(buf, w, h, *color, 1.0);
            }
        }
        Shape::Ellipse { a, b, color, width } => {
            let r = RectF::from_points(*a, *b);
            let cx = r.center()[0] - ox;
            let cy = r.center()[1] - oy;
            let rx = (r.width() * 0.5).max(0.5);
            let ry = (r.height() * 0.5).max(0.5);
            let steps = (((rx + ry) * 1.6) as usize).clamp(24, 256);
            let mut pts = Vec::with_capacity(steps);
            for i in 0..steps {
                let t = (i as f32) / (steps as f32) * std::f32::consts::TAU;
                pts.push([cx + t.cos() * rx, cy + t.sin() * ry]);
            }
            let pad = width * 0.5 + 2.0;
            if let Some(mut mask) = Mask::for_bounds(
                cx - rx - pad,
                cy - ry - pad,
                cx + rx + pad,
                cy + ry + pad,
                w,
                h,
            ) {
                mask.add_polyline(&pts, *width, true);
                mask.composite(buf, w, h, *color, 1.0);
            }
        }
        Shape::Arrow { a, b, color, width } => {
            let p0 = [a[0] - ox, a[1] - oy];
            let p1 = [b[0] - ox, b[1] - oy];
            let dx = p1[0] - p0[0];
            let dy = p1[1] - p0[1];
            let len = (dx * dx + dy * dy).sqrt();
            if len < 1.0 {
                return;
            }
            let ux = dx / len;
            let uy = dy / len;
            // Head size grows with the arrow length so long arrows keep a visible head.
            let head_len = (len * 0.17)
                .clamp(width * 2.8, width * 7.5)
                .max(10.0)
                .min(len * 0.5);
            let head_w = head_len * 0.62;
            let base_pt = [p1[0] - ux * head_len, p1[1] - uy * head_len];
            // Slight overshoot so the shaft does not poke out of the head tip.
            let shaft_end = [p1[0] - ux * head_len * 0.6, p1[1] - uy * head_len * 0.6];
            let perp = [-uy, ux];
            let head = [
                p1,
                [base_pt[0] + perp[0] * head_w, base_pt[1] + perp[1] * head_w],
                [base_pt[0] - perp[0] * head_w, base_pt[1] - perp[1] * head_w],
            ];
            let pad = (width * 0.5).max(head_w) + 3.0;
            if let Some(mut mask) = Mask::for_bounds(
                p0[0].min(p1[0]) - pad,
                p0[1].min(p1[1]) - pad,
                p0[0].max(p1[0]) + pad,
                p0[1].max(p1[1]) + pad,
                w,
                h,
            ) {
                mask.add_capsule(p0, shaft_end, *width);
                mask.add_polygon(&head);
                mask.composite(buf, w, h, *color, 1.0);
            }
        }
        Shape::Text {
            pos,
            text,
            color,
            size,
        } => {
            draw_text_at(
                buf,
                w,
                h,
                [pos[0] - ox, pos[1] - oy],
                text,
                *size,
                *color,
                1.0,
            );
        }
        Shape::Mosaic { rect, block } => {
            draw_mosaic(buf, w, h, origin, base, RectF::from_xywh(*rect), *block);
        }
        Shape::Blur { rect, radius } => {
            draw_blur(buf, w, h, origin, base, RectF::from_xywh(*rect), *radius);
        }
        Shape::Sticker {
            center,
            size,
            kind,
            color,
        } => {
            draw_sticker(buf, w, h, origin, *center, *size, *kind, *color);
        }
        Shape::Number {
            center,
            size,
            n,
            color,
        } => {
            draw_number(buf, w, h, origin, *center, *size, *n, *color);
        }
        Shape::Bitmap { rect, image } => {
            draw_bitmap(buf, w, h, origin, RectF::from_xywh(*rect), image);
        }
    }
}

/// Render `shapes` (plus an optional in-progress shape) over the base capture,
/// producing exactly the pixels inside `region`.
pub fn render_region(
    base: &RgbaImage,
    region: Region,
    shapes: &[Shape],
    live: Option<&Shape>,
) -> RgbaImage {
    let region = region.clamped(base.width(), base.height());
    let n = (region.w * region.h * 4) as usize;
    let mut raw = vec![0u8; n];
    // Copy the base crop.
    let base_raw = base.as_raw();
    let bw = base.width() as usize;
    for row in 0..region.h as usize {
        let src_off = ((region.y as usize + row) * bw + region.x as usize) * 4;
        let dst_off = row * region.w as usize * 4;
        let len = region.w as usize * 4;
        raw[dst_off..dst_off + len].copy_from_slice(&base_raw[src_off..src_off + len]);
    }
    let origin = (region.x, region.y);
    for shape in shapes {
        draw_shape(&mut raw, region.w, region.h, origin, base, shape);
    }
    if let Some(shape) = live {
        draw_shape(&mut raw, region.w, region.h, origin, base, shape);
    }
    RgbaImage::from_raw(region.w, region.h, raw).unwrap_or_else(|| RgbaImage::new(1, 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: u32, h: u32, c: Rgba) -> RgbaImage {
        RgbaImage::from_pixel(w, h, image::Rgba(c))
    }

    #[test]
    fn text_metrics_grow_with_content_and_font_size() {
        let (w1, h1) = measure_text("中文", 40.0);
        let (w2, _) = measure_text("中文中文", 40.0);
        let (w3, _) = measure_text("中文", 80.0);
        assert!(w1 > 0.0 && h1 >= 40.0);
        assert!(w2 > w1 * 1.8, "more glyphs must be wider: {w1} vs {w2}");
        assert!(w3 > w1 * 1.8, "larger font must be wider: {w1} vs {w3}");
        // Measuring must agree with what the renderer advances per glyph.
        let font = fonts::glyph_font().expect("CJK font available");
        let px = ab_glyph::PxScale::from(40.0f32);
        let scaled = font.as_scaled(px);
        let expected: f32 = "中文".chars().map(|c| scaled.h_advance(font.glyph_id(c))).sum();
        assert!((w1 - expected).abs() < 0.01, "measure_text must match advances: {w1} vs {expected}");
    }
    #[test]
    fn render_region_is_identity_without_shapes() {
        let base = solid(64, 48, [10, 20, 30, 255]);
        let out = render_region(&base, Region::new(8.0, 4.0, 16.0, 12.0), &[], None);
        assert_eq!(out.width(), 16);
        assert_eq!(out.height(), 12);
        assert_eq!(out.get_pixel(0, 0).0, [10, 20, 30, 255]);
    }

    #[test]
    fn rect_stroke_paints_expected_pixels() {
        let base = solid(64, 64, [0, 0, 0, 255]);
        let shape = Shape::Rect {
            a: [8.0, 8.0],
            b: [56.0, 56.0],
            color: [255, 0, 0, 255],
            width: 4.0,
        };
        let out = render_region(&base, Region::new(0.0, 0.0, 64.0, 64.0), &[shape], None);
        // Top edge is painted, centre stays transparent-to-black.
        assert!(out.get_pixel(32, 8).0[0] > 200, "edge should be red");
        assert!(out.get_pixel(32, 32).0[0] < 20, "centre should stay black");
    }

    #[test]
    fn mosaic_uses_base_colour() {
        let mut base = solid(64, 64, [0, 0, 0, 255]);
        for y in 0..64 {
            for x in 0..64 {
                let c = if (x / 8 + y / 8) % 2 == 0 {
                    [200u8, 200, 200, 255]
                } else {
                    [40, 40, 40, 255]
                };
                base.put_pixel(x, y, image::Rgba(c));
            }
        }
        let shape = Shape::Mosaic {
            rect: [0.0, 0.0, 64.0, 64.0],
            block: 32,
        };
        let out = render_region(&base, Region::new(0.0, 0.0, 64.0, 64.0), &[shape], None);
        // With a 32px block the checkerboard averages to a flat mid grey.
        let a = out.get_pixel(2, 2).0[0];
        let b = out.get_pixel(40, 40).0[0];
        assert_eq!(a, b, "block average should be uniform");
    }

    #[test]
    fn highlighter_alpha_is_uniform_over_stroke() {
        let base = solid(64, 64, [0, 0, 0, 255]);
        let shape = Shape::Stroke {
            points: vec![[8.0, 32.0], [32.0, 32.0], [56.0, 32.0]],
            color: [255, 255, 0, 255],
            width: 10.0,
            alpha: 0.5,
        };
        let out = render_region(&base, Region::new(0.0, 0.0, 64.0, 64.0), &[shape], None);
        let start = out.get_pixel(10, 32).0[0];
        let joint = out.get_pixel(32, 32).0[0];
        assert!(start > 100, "stroke should brighten");
        assert!(
            (start as i32 - joint as i32).abs() < 12,
            "joint must not double-blend: {start} vs {joint}"
        );
    }
}
