//! Crisp vector icons drawn with egui primitives.
//!
//! Text glyphs are unreliable for tool symbols (tofu risk, font dependent) and
//! look blurry at small sizes, so every toolbar/settings icon is drawn from
//! paths here instead. All coordinates are normalised to the target rect.

use eframe::egui;
use egui::{Color32, Pos2, Rect, Stroke, Vec2};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    Select,
    Rect,
    Ellipse,
    Arrow,
    Pen,
    Highlighter,
    Mosaic,
    Blur,
    Text,
    Sticker,
    Number,
    Eraser,
    Undo,
    Redo,
    Paste,
    Copy,
    Save,
    Check,
    Close,
    Palette,
    Folder,
    Camera,
    Monitor,
    Info,
    Sliders,
    Plus,
    Minus,
}

fn pt(rect: Rect, x: f32, y: f32) -> Pos2 {
    Pos2::new(rect.min.x + rect.width() * x, rect.min.y + rect.height() * y)
}

fn v(rect: Rect, x: f32, y: f32) -> Vec2 {
    Vec2::new(rect.width() * x, rect.height() * y)
}

fn stroke(rect: Rect, width: f32) -> f32 {
    (rect.width().min(rect.height()) * width).max(1.1)
}

/// Draw `icon` inside `rect` (typically square, already inset by the caller).
pub fn paint(painter: &egui::Painter, rect: Rect, icon: Icon, color: Color32) {
    let s = Stroke::new(stroke(rect, 0.085), color);
    let thin = Stroke::new(stroke(rect, 0.06), color);
    match icon {
        Icon::Select => {
            // egui can only fill convex shapes, so the cursor is a head triangle
            // plus a tail quad instead of one concave outline.
            let head = [
                pt(rect, 0.29, 0.13),
                pt(rect, 0.29, 0.84),
                pt(rect, 0.62, 0.51),
            ];
            painter.add(egui::Shape::convex_polygon(
                head.to_vec(),
                color,
                Stroke::NONE,
            ));
            let tail = [
                pt(rect, 0.44, 0.56),
                pt(rect, 0.56, 0.51),
                pt(rect, 0.69, 0.86),
                pt(rect, 0.56, 0.91),
            ];
            painter.add(egui::Shape::convex_polygon(
                tail.to_vec(),
                color,
                Stroke::NONE,
            ));
        }
        Icon::Rect => {
            let r = Rect::from_min_max(pt(rect, 0.16, 0.24), pt(rect, 0.84, 0.76));
            painter.rect_stroke(r, egui::Rounding::same(rect.width() * 0.09), s);
        }
        Icon::Ellipse => {
            painter.circle_stroke(rect.center(), rect.width() * 0.32, s);
        }
        Icon::Arrow => {
            painter.line_segment([pt(rect, 0.20, 0.80), pt(rect, 0.62, 0.38)], s);
            let head = [
                pt(rect, 0.84, 0.16),
                pt(rect, 0.52, 0.30),
                pt(rect, 0.70, 0.48),
            ];
            painter.add(egui::Shape::convex_polygon(
                head.to_vec(),
                color,
                Stroke::NONE,
            ));
        }
        Icon::Pen => {
            // Body capsule + convex nib triangle + ink line.
            painter.line_segment(
                [pt(rect, 0.36, 0.66), pt(rect, 0.78, 0.24)],
                Stroke::new(stroke(rect, 0.20), color),
            );
            let nib = [
                pt(rect, 0.14, 0.88),
                pt(rect, 0.22, 0.62),
                pt(rect, 0.40, 0.78),
            ];
            painter.add(egui::Shape::convex_polygon(
                nib.to_vec(),
                color,
                Stroke::NONE,
            ));
        }
        Icon::Highlighter => {
            let tint = color.gamma_multiply(0.45);
            painter.line_segment(
                [pt(rect, 0.26, 0.74), pt(rect, 0.56, 0.44)],
                Stroke::new(stroke(rect, 0.26), tint),
            );
            painter.line_segment([pt(rect, 0.56, 0.44), pt(rect, 0.82, 0.18)], s);
            painter.line_segment(
                [pt(rect, 0.30, 0.88), pt(rect, 0.64, 0.88)],
                Stroke::new(stroke(rect, 0.10), color),
            );
        }
        Icon::Mosaic => {
            for row in 0..3 {
                for col in 0..3 {
                    let a = if (row + col) % 2 == 0 { 1.0 } else { 0.42 };
                    let x = 0.16 + col as f32 * 0.24;
                    let y = 0.16 + row as f32 * 0.24;
                    let r = Rect::from_min_max(
                        pt(rect, x, y),
                        pt(rect, x + 0.19, y + 0.19),
                    );
                    painter.rect_filled(r, egui::Rounding::same(1.5), color.gamma_multiply(a));
                }
            }
        }
        Icon::Blur => {
            for (i, r) in [0.34f32, 0.22, 0.10].iter().enumerate() {
                let a = 0.35 + 0.22 * i as f32;
                painter.circle_stroke(
                    rect.center(),
                    rect.width() * r,
                    Stroke::new(stroke(rect, 0.06), color.gamma_multiply(a)),
                );
            }
            painter.circle_filled(rect.center(), rect.width() * 0.045, color);
        }
        Icon::Text => {
            painter.line_segment(
                [pt(rect, 0.22, 0.24), pt(rect, 0.78, 0.24)],
                Stroke::new(stroke(rect, 0.10), color),
            );
            painter.line_segment(
                [pt(rect, 0.50, 0.24), pt(rect, 0.50, 0.80)],
                Stroke::new(stroke(rect, 0.10), color),
            );
        }
        Icon::Sticker => {
            // A star is concave; fan it out into 10 convex triangles from the centre
            // so the fill is correct without relying on concave-path tessellation.
            let pts = star(rect.center(), rect.width() * 0.38, rect.width() * 0.17, 5);
            for pair in pts.chunks(2) {
                if pair.len() == 2 {
                    let tri = [rect.center(), pair[0], pair[1]];
                    painter.add(egui::Shape::convex_polygon(
                        tri.to_vec(),
                        color,
                        Stroke::NONE,
                    ));
                }
            }
        }
        Icon::Number => {
            painter.circle_stroke(rect.center(), rect.width() * 0.33, s);
            let one = [
                pt(rect, 0.43, 0.40),
                pt(rect, 0.53, 0.33),
                pt(rect, 0.53, 0.68),
            ];
            painter.add(egui::Shape::line(
                one.to_vec(),
                Stroke::new(stroke(rect, 0.09), color),
            ));
            painter.line_segment(
                [pt(rect, 0.41, 0.68), pt(rect, 0.64, 0.68)],
                Stroke::new(stroke(rect, 0.09), color),
            );
        }
        Icon::Eraser => {
            let body = [
                pt(rect, 0.36, 0.60),
                pt(rect, 0.60, 0.26),
                pt(rect, 0.84, 0.48),
                pt(rect, 0.60, 0.82),
            ];
            painter.add(egui::Shape::convex_polygon(
                body.to_vec(),
                color,
                Stroke::NONE,
            ));
            painter.line_segment(
                [pt(rect, 0.20, 0.84), pt(rect, 0.50, 0.84)],
                Stroke::new(stroke(rect, 0.08), color),
            );
        }
        Icon::Undo | Icon::Redo => {
            let flip = matches!(icon, Icon::Redo);
            let cx = |x: f32| if flip { 1.0 - x } else { x };
            let mut pts = Vec::new();
            for i in 0..=14 {
                let t = i as f32 / 14.0;
                let ang = std::f32::consts::PI * (1.0 + 0.92 * t);
                pts.push(pt(
                    rect,
                    cx(0.50 + 0.26 * ang.cos()),
                    0.56 + 0.26 * ang.sin(),
                ));
            }
            painter.add(egui::Shape::line(pts, Stroke::new(stroke(rect, 0.085), color)));
            let head = [
                pt(rect, cx(0.26), 0.34),
                pt(rect, cx(0.22), 0.62),
                pt(rect, cx(0.46), 0.52),
            ];
            painter.add(egui::Shape::convex_polygon(
                head.to_vec(),
                color,
                Stroke::NONE,
            ));
        }
        Icon::Paste => {
            let r = Rect::from_min_max(pt(rect, 0.22, 0.26), pt(rect, 0.78, 0.84));
            painter.rect_stroke(r, egui::Rounding::same(rect.width() * 0.08), thin);
            let clip = Rect::from_min_max(pt(rect, 0.38, 0.18), pt(rect, 0.62, 0.30));
            painter.rect_filled(clip, egui::Rounding::same(2.0), color);
            painter.line_segment(
                [pt(rect, 0.50, 0.42), pt(rect, 0.50, 0.68)],
                Stroke::new(stroke(rect, 0.08), color),
            );
            painter.line_segment(
                [pt(rect, 0.40, 0.58), pt(rect, 0.50, 0.70)],
                Stroke::new(stroke(rect, 0.08), color),
            );
            painter.line_segment(
                [pt(rect, 0.60, 0.58), pt(rect, 0.50, 0.70)],
                Stroke::new(stroke(rect, 0.08), color),
            );
        }
        Icon::Copy => {
            let back = Rect::from_min_max(pt(rect, 0.18, 0.16), pt(rect, 0.64, 0.62));
            painter.rect_stroke(back, egui::Rounding::same(rect.width() * 0.07), thin);
            let front = Rect::from_min_max(pt(rect, 0.36, 0.36), pt(rect, 0.84, 0.84));
            painter.rect_filled(front, egui::Rounding::same(rect.width() * 0.07), color.gamma_multiply(0.18));
            painter.rect_stroke(front, egui::Rounding::same(rect.width() * 0.07), thin);
        }
        Icon::Save => {
            let r = Rect::from_min_max(pt(rect, 0.22, 0.22), pt(rect, 0.78, 0.80));
            painter.rect_stroke(r, egui::Rounding::same(rect.width() * 0.08), thin);
            let top = Rect::from_min_max(pt(rect, 0.36, 0.22), pt(rect, 0.64, 0.40));
            painter.rect_filled(top, egui::Rounding::same(1.5), color);
            let bottom = Rect::from_min_max(pt(rect, 0.32, 0.56), pt(rect, 0.68, 0.80));
            painter.rect_stroke(bottom, egui::Rounding::same(1.5), thin);
        }
        Icon::Check => {
            let pts = [pt(rect, 0.22, 0.54), pt(rect, 0.42, 0.74), pt(rect, 0.78, 0.28)];
            painter.add(egui::Shape::line(
                pts.to_vec(),
                Stroke::new(stroke(rect, 0.13), color),
            ));
        }
        Icon::Close => {
            painter.line_segment(
                [pt(rect, 0.28, 0.28), pt(rect, 0.72, 0.72)],
                Stroke::new(stroke(rect, 0.12), color),
            );
            painter.line_segment(
                [pt(rect, 0.72, 0.28), pt(rect, 0.28, 0.72)],
                Stroke::new(stroke(rect, 0.12), color),
            );
        }
        Icon::Palette => {
            painter.circle_stroke(rect.center(), rect.width() * 0.32, thin);
            for (i, c) in [
                Color32::from_rgb(235, 59, 48),
                Color32::from_rgb(52, 199, 89),
                Color32::from_rgb(0, 122, 255),
            ]
            .iter()
            .enumerate()
            {
                let x = 0.34 + i as f32 * 0.16;
                painter.circle_filled(pt(rect, x, 0.42), rect.width() * 0.055, *c);
            }
            painter.circle_filled(
                rect.center(),
                rect.width() * 0.05,
                color.gamma_multiply(0.55),
            );
        }
        Icon::Folder => {
            let body = Rect::from_min_max(pt(rect, 0.18, 0.34), pt(rect, 0.82, 0.80));
            painter.rect_filled(
                body,
                egui::Rounding::same(rect.width() * 0.07),
                color.gamma_multiply(0.22),
            );
            let tab = Rect::from_min_max(pt(rect, 0.18, 0.26), pt(rect, 0.46, 0.38));
            painter.rect_filled(tab, egui::Rounding::same(rect.width() * 0.05), color.gamma_multiply(0.22));
            painter.rect_stroke(body, egui::Rounding::same(rect.width() * 0.07), thin);
        }
        Icon::Camera => {
            let body = Rect::from_min_max(pt(rect, 0.14, 0.34), pt(rect, 0.86, 0.78));
            painter.rect_stroke(body, egui::Rounding::same(rect.width() * 0.09), thin);
            painter.circle_stroke(
                Pos2::new(rect.center().x, rect.min.y + rect.height() * 0.58),
                rect.width() * 0.15,
                thin,
            );
            painter.rect_filled(
                Rect::from_min_max(pt(rect, 0.34, 0.24), pt(rect, 0.56, 0.36)),
                egui::Rounding::same(rect.width() * 0.04),
                color,
            );
        }
        Icon::Monitor => {
            let screen = Rect::from_min_max(pt(rect, 0.14, 0.22), pt(rect, 0.86, 0.66));
            painter.rect_stroke(screen, egui::Rounding::same(rect.width() * 0.06), thin);
            painter.line_segment(
                [pt(rect, 0.50, 0.66), pt(rect, 0.50, 0.82)],
                Stroke::new(stroke(rect, 0.07), color),
            );
            painter.line_segment(
                [pt(rect, 0.30, 0.82), pt(rect, 0.70, 0.82)],
                Stroke::new(stroke(rect, 0.07), color),
            );
        }
        Icon::Info => {
            painter.circle_stroke(rect.center(), rect.width() * 0.32, thin);
            painter.circle_filled(pt(rect, 0.50, 0.34), rect.width() * 0.045, color);
            painter.line_segment(
                [pt(rect, 0.50, 0.46), pt(rect, 0.50, 0.68)],
                Stroke::new(stroke(rect, 0.08), color),
            );
        }
        Icon::Sliders => {
            for (i, y) in [0.30f32, 0.50, 0.70].iter().enumerate() {
                painter.line_segment(
                    [pt(rect, 0.18, *y), pt(rect, 0.82, *y)],
                    Stroke::new(stroke(rect, 0.06), color.gamma_multiply(0.55)),
                );
                let x = 0.34 + (i as f32) * 0.18;
                painter.circle_filled(pt(rect, x, *y), rect.width() * 0.085, color);
            }
        }
        Icon::Plus | Icon::Minus => {
            painter.line_segment(
                [pt(rect, 0.26, 0.50), pt(rect, 0.74, 0.50)],
                Stroke::new(stroke(rect, 0.11), color),
            );
            if matches!(icon, Icon::Plus) {
                painter.line_segment(
                    [pt(rect, 0.50, 0.26), pt(rect, 0.50, 0.74)],
                    Stroke::new(stroke(rect, 0.11), color),
                );
            }
        }
    }
    let _ = v;
}

fn star(center: Pos2, outer: f32, inner: f32, points: usize) -> Vec<Pos2> {
    let mut out = Vec::with_capacity(points * 2);
    for i in 0..points * 2 {
        let r = if i % 2 == 0 { outer } else { inner };
        let ang = -std::f32::consts::FRAC_PI_2
            + (i as f32) * std::f32::consts::PI / points as f32;
        out.push(Pos2::new(
            center.x + ang.cos() * r,
            center.y + ang.sin() * r,
        ));
    }
    out
}
