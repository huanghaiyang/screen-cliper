//! Editor document model: tools, stickers and the annotate shapes themselves.
//!
//! Every coordinate in this module is expressed in *captured image pixels*
//! (physical screen pixels), never in egui points. Conversion to screen space
//! happens only inside the overlay UI.

/// A point in image-pixel space.
pub type Pt = [f32; 2];
/// Straight RGBA8, non-premultiplied.
pub type Rgba = [u8; 4];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Select,
    Pen,
    Highlighter,
    Rect,
    Ellipse,
    Arrow,
    Text,
    Mosaic,
    Blur,
    Sticker,
    Number,
    Eraser,
}

impl Tool {
    /// Every annotation tool, in toolbar order.
    pub const ALL: [Tool; 12] = [
        Tool::Select,
        Tool::Rect,
        Tool::Ellipse,
        Tool::Arrow,
        Tool::Pen,
        Tool::Highlighter,
        Tool::Mosaic,
        Tool::Blur,
        Tool::Text,
        Tool::Sticker,
        Tool::Number,
        Tool::Eraser,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Tool::Select => "select",
            Tool::Pen => "pen",
            Tool::Highlighter => "highlighter",
            Tool::Rect => "rect",
            Tool::Ellipse => "ellipse",
            Tool::Arrow => "arrow",
            Tool::Text => "text",
            Tool::Mosaic => "mosaic",
            Tool::Blur => "blur",
            Tool::Sticker => "sticker",
            Tool::Number => "number",
            Tool::Eraser => "eraser",
        }
    }

    pub fn from_key(key: &str) -> Tool {
        match key {
            "pen" => Tool::Pen,
            "highlighter" => Tool::Highlighter,
            "ellipse" => Tool::Ellipse,
            "arrow" => Tool::Arrow,
            "text" => Tool::Text,
            "mosaic" => Tool::Mosaic,
            "blur" => Tool::Blur,
            "sticker" => Tool::Sticker,
            "number" => Tool::Number,
            "eraser" => Tool::Eraser,
            "select" => Tool::Select,
            _ => Tool::Rect,
        }
    }

    /// Compact label used on the toolbar (2-3 CJK characters keep buttons small).
    pub fn label(self) -> &'static str {
        match self {
            Tool::Select => "选择",
            Tool::Pen => "画笔",
            Tool::Highlighter => "荧光",
            Tool::Rect => "矩形",
            Tool::Ellipse => "椭圆",
            Tool::Arrow => "箭头",
            Tool::Text => "文字",
            Tool::Mosaic => "马赛克",
            Tool::Blur => "模糊",
            Tool::Sticker => "贴花",
            Tool::Number => "序号",
            Tool::Eraser => "橡皮",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            Tool::Select => "拖动选区移动/缩放，双击完成",
            Tool::Pen => "按住左键自由绘制",
            Tool::Highlighter => "半透明荧光笔，可反复涂抹",
            Tool::Rect => "拖动画出矩形",
            Tool::Ellipse => "拖动画出椭圆",
            Tool::Arrow => "从起点拖到终点画出箭头",
            Tool::Text => "点击任意位置输入文字",
            Tool::Mosaic => "框选需要打码的区域",
            Tool::Blur => "框选需要模糊的区域",
            Tool::Sticker => "点击放置贴花，滚轮调整大小",
            Tool::Number => "点击放置递增序号标记",
            Tool::Eraser => "点击图形将其删除",
        }
    }

    pub fn uses_drag(self) -> bool {
        matches!(
            self,
            Tool::Pen | Tool::Highlighter | Tool::Rect | Tool::Ellipse | Tool::Arrow | Tool::Mosaic | Tool::Blur
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StickerKind {
    Check,
    Cross,
    Star,
    Heart,
    Exclaim,
    Question,
    Arrow,
    Circle,
}

impl StickerKind {
    pub const ALL: [StickerKind; 8] = [
        StickerKind::Check,
        StickerKind::Cross,
        StickerKind::Star,
        StickerKind::Heart,
        StickerKind::Exclaim,
        StickerKind::Question,
        StickerKind::Arrow,
        StickerKind::Circle,
    ];

    pub fn glyph(self) -> &'static str {
        match self {
            StickerKind::Check => "✓",
            StickerKind::Cross => "✗",
            StickerKind::Star => "★",
            StickerKind::Heart => "❤",
            StickerKind::Exclaim => "!",
            StickerKind::Question => "?",
            StickerKind::Arrow => "→",
            StickerKind::Circle => "○",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            StickerKind::Check => "对勾",
            StickerKind::Cross => "叉号",
            StickerKind::Star => "星星",
            StickerKind::Heart => "爱心",
            StickerKind::Exclaim => "感叹",
            StickerKind::Question => "问号",
            StickerKind::Arrow => "箭头",
            StickerKind::Circle => "圆圈",
        }
    }
}

#[derive(Debug, Clone)]
pub enum Shape {
    /// Freehand stroke; `alpha` distinguishes the pen (1.0) from the highlighter.
    Stroke {
        points: Vec<Pt>,
        color: Rgba,
        width: f32,
        alpha: f32,
    },
    Rect {
        a: Pt,
        b: Pt,
        color: Rgba,
        width: f32,
    },
    Ellipse {
        a: Pt,
        b: Pt,
        color: Rgba,
        width: f32,
    },
    Arrow {
        a: Pt,
        b: Pt,
        color: Rgba,
        width: f32,
    },
    Text {
        pos: Pt,
        text: String,
        color: Rgba,
        size: f32,
    },
    Mosaic {
        rect: [f32; 4],
        block: u32,
    },
    Blur {
        rect: [f32; 4],
        radius: f32,
    },
    Sticker {
        center: Pt,
        size: f32,
        kind: StickerKind,
        color: Rgba,
    },
    Number {
        center: Pt,
        size: f32,
        n: u32,
        color: Rgba,
    },
    /// A bitmap pasted from the clipboard (“贴图”). Stored at capture resolution.
    Bitmap {
        rect: [f32; 4],
        image: std::sync::Arc<image::RgbaImage>,
    },
}

#[derive(Debug, Clone, Copy)]
pub struct RectF {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl RectF {
    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    pub fn from_points(a: Pt, b: Pt) -> Self {
        Self {
            x: a[0].min(b[0]),
            y: a[1].min(b[1]),
            w: (a[0] - b[0]).abs(),
            h: (a[1] - b[1]).abs(),
        }
    }

    pub fn from_xywh(r: [f32; 4]) -> Self {
        Self {
            x: r[0],
            y: r[1],
            w: r[2],
            h: r[3],
        }
    }

    pub fn min(&self) -> Pt {
        [self.x, self.y]
    }

    pub fn max(&self) -> Pt {
        [self.x + self.w, self.y + self.h]
    }

    pub fn center(&self) -> Pt {
        [self.x + self.w * 0.5, self.y + self.h * 0.5]
    }

    pub fn width(&self) -> f32 {
        self.w.abs()
    }

    pub fn height(&self) -> f32 {
        self.h.abs()
    }

    pub fn area(&self) -> f32 {
        self.width() * self.height()
    }

    pub fn contains(&self, p: Pt) -> bool {
        p[0] >= self.x && p[0] <= self.x + self.w && p[1] >= self.y && p[1] <= self.y + self.h
    }

    pub fn expand(&self, m: f32) -> Self {
        Self::new(self.x - m, self.y - m, self.w + m * 2.0, self.h + m * 2.0)
    }

    pub fn as_xywh(&self) -> [f32; 4] {
        [self.x, self.y, self.w, self.h]
    }
}

impl Shape {
    /// Axis-aligned bounding box in image pixels, including stroke width.
    pub fn bbox(&self) -> RectF {
        match self {
            Shape::Stroke { points, width, .. } => {
                let mut rect = RectF::new(f32::MAX, f32::MAX, 0.0, 0.0);
                let (mut min_x, mut min_y) = (f32::MAX, f32::MAX);
                let (mut max_x, mut max_y) = (-f32::MAX, -f32::MAX);
                for p in points {
                    min_x = min_x.min(p[0]);
                    min_y = min_y.min(p[1]);
                    max_x = max_x.max(p[0]);
                    max_y = max_y.max(p[1]);
                }
                if min_x > max_x {
                    return rect;
                }
                rect = RectF::new(min_x, min_y, max_x - min_x, max_y - min_y);
                rect.expand(width * 0.5 + 1.0)
            }
            Shape::Rect { a, b, width, .. }
            | Shape::Ellipse { a, b, width, .. }
            | Shape::Arrow { a, b, width, .. } => RectF::from_points(*a, *b).expand(width * 0.5 + 1.0),
            Shape::Text {
                pos,
                text,
                size,
                ..
            } => {
                let w = crate::raster::measure_text(text, *size).0;
                RectF::new(pos[0], pos[1], w.max(*size * 0.5), *size * 1.3)
            }
            Shape::Mosaic { rect, .. } | Shape::Blur { rect, .. } => RectF::from_xywh(*rect),
            Shape::Sticker { center, size, .. } | Shape::Number { center, size, .. } => {
                RectF::new(center[0] - size * 0.5, center[1] - size * 0.5, *size, *size)
            }
            Shape::Bitmap { rect, .. } => RectF::from_xywh(*rect),
        }
    }


}

// ---------------------------------------------------------------------------
// freehand stroke smoothing
//
// Raw pointer samples were previously stitched together with straight capsules,
// which shows hard corners whenever the pointer moves fast (few samples per
// frame). The stroke is now fitted with a chain of cubic Bézier segments:
//
//   * the tangent at each sample is the central difference of its neighbours,
//     so the curve interpolates every sample and stays C1 continuous;
//   * control points sit one third of the chord away along those tangents
//     (the Catmull-Rom choice), which keeps the curve from overshooting or
//     looping on sharp direction changes;
//   * each Bézier is flattened into as many capsules as the bound on its
//     approximation error requires, so dense sampling costs nothing extra and
//     sparse sampling still comes out smooth.
// ---------------------------------------------------------------------------

/// Fit a smooth cubic-Bézier chain through `points` and return the flattened
/// polyline ready for the capsule rasteriser.
pub fn smooth_stroke(points: &[Pt]) -> Vec<Pt> {
    // Drop non-finite and duplicate samples: the pointer often reports the same
    // pixel several frames in a row, which would make the tangents undefined.
    let mut src: Vec<Pt> = Vec::with_capacity(points.len());
    for &p in points {
        if !p[0].is_finite() || !p[1].is_finite() {
            continue;
        }
        match src.last() {
            Some(&last) if len2(last, p) < 0.16 => {}
            _ => src.push(p),
        }
    }
    if src.len() <= 2 {
        return src;
    }

    // Clamping the control points into the samples' bounding box guarantees the
    // curve stays there too (a Bézier never leaves its control hull). Without it
    // a sparse sharp turn sends the central-difference tangent off to the side
    // and the stroke grows a visible loop outside the drawn area.
    let mut min = src[0];
    let mut max = src[0];
    for &p in &src {
        min = [min[0].min(p[0]), min[1].min(p[1])];
        max = [max[0].max(p[0]), max[1].max(p[1])];
    }

    let mut out: Vec<Pt> = Vec::with_capacity(src.len() * 2);
    out.push(src[0]);
    for i in 0..src.len() - 1 {
        // One-sided neighbours at the ends so the stroke starts and ends along
        // the actual drawing direction instead of bulging outwards.
        let p0 = if i == 0 { src[0] } else { src[i - 1] };
        let p1 = src[i];
        let p2 = src[i + 1];
        let p3 = if i + 2 < src.len() { src[i + 2] } else { src[i + 1] };
        let (c1, c2) = cubic_controls(p0, p1, p2, p3);
        let c1 = clamp_pt(c1, min, max);
        let c2 = clamp_pt(c2, min, max);
        flatten_cubic(p1, c1, c2, p2, &mut out);
    }
    out
}

fn clamp_pt(p: Pt, min: Pt, max: Pt) -> Pt {
    [p[0].clamp(min[0], max[0]), p[1].clamp(min[1], max[1])]
}

fn len2(a: Pt, b: Pt) -> f32 {
    let dx = b[0] - a[0];
    let dy = b[1] - a[1];
    dx * dx + dy * dy
}

fn dist(a: Pt, b: Pt) -> f32 {
    len2(a, b).sqrt()
}

/// Unit vector along `(dx, dy)`, falling back to `fallback` when it is degenerate.
fn unit(dx: f32, dy: f32, fallback: (f32, f32)) -> (f32, f32) {
    let len = (dx * dx + dy * dy).sqrt();
    if len < 1e-4 {
        fallback
    } else {
        (dx / len, dy / len)
    }
}

/// Catmull-Rom style control points for the segment `p1 -> p2`.
fn cubic_controls(p0: Pt, p1: Pt, p2: Pt, p3: Pt) -> (Pt, Pt) {
    let chord = dist(p1, p2);
    if chord < 1e-4 {
        return (p1, p2);
    }
    let chord_dir = unit(p2[0] - p1[0], p2[1] - p1[1], (1.0, 0.0));
    let t1 = unit(p2[0] - p0[0], p2[1] - p0[1], chord_dir);
    let t2 = unit(p3[0] - p1[0], p3[1] - p1[1], chord_dir);
    // Bound each handle by the adjacent segment length: uneven sample spacing
    // (fast pen movement next to a slow patch) otherwise produces long handles
    // that overshoot the drawn shape.
    let prev = dist(p0, p1);
    let next = dist(p2, p3);
    let k1 = if prev < 1e-3 { chord / 3.0 } else { chord.min(prev) / 3.0 };
    let k2 = if next < 1e-3 { chord / 3.0 } else { chord.min(next) / 3.0 };
    (
        [p1[0] + t1.0 * k1, p1[1] + t1.1 * k1],
        [p2[0] - t2.0 * k2, p2[1] - t2.1 * k2],
    )
}

fn point_cubic(p0: Pt, c1: Pt, c2: Pt, p3: Pt, t: f32) -> Pt {
    // Horner form of the cubic Bézier.
    let u = 1.0 - t;
    let a = u * u * u;
    let b = 3.0 * u * u * t;
    let c = 3.0 * u * t * t;
    let d = t * t * t;
    [
        a * p0[0] + b * c1[0] + c * c2[0] + d * p3[0],
        a * p0[1] + b * c1[1] + c * c2[1] + d * p3[1],
    ]
}

/// Perpendicular distance from `p` to the line through `a`..`b`.
fn chord_deviation(p: Pt, a: Pt, b: Pt) -> f32 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let len = (dx * dx + dy * dy).sqrt();
    if len < 1e-4 {
        return dist(p, a);
    }
    ((p[0] - a[0]) * dy - (p[1] - a[1]) * dx).abs() / len
}

/// Append `p3` (and intermediate points) approximating the cubic within ~0.2 px.
fn flatten_cubic(p0: Pt, c1: Pt, c2: Pt, p3: Pt, out: &mut Vec<Pt>) {
    // A cubic stays inside its control polygon, so the control points' distance
    // from the chord bounds how far the polyline can be from the real curve.
    let dev = chord_deviation(c1, p0, p3).max(chord_deviation(c2, p0, p3));
    // Midpoint error ~ 0.75 * dev; n subdivisions divide it by n^2.
    let steps = (0.75 * dev / 0.2).sqrt().ceil();
    let steps = if !steps.is_finite() {
        1
    } else {
        (steps as i32).clamp(1, 32) as usize
    };
    for i in 1..=steps {
        let t = i as f32 / steps as f32;
        out.push(point_cubic(p0, c1, c2, p3, t));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn max_gap(points: &[Pt]) -> f32 {
        points
            .windows(2)
            .map(|w| dist(w[0], w[1]))
            .fold(0.0f32, f32::max)
    }

    #[test]
    fn two_points_stay_a_straight_line() {
        let out = smooth_stroke(&[[0.0, 0.0], [10.0, 0.0]]);
        assert_eq!(out.len(), 2);
        assert!((out[1][0] - 10.0).abs() < 1e-5);
    }

    #[test]
    fn smoothing_passes_through_every_sample() {
        let input = [[0.0, 0.0], [10.0, 6.0], [24.0, 2.0], [30.0, 20.0]];
        let out = smooth_stroke(&input);
        assert!(out.len() > input.len());
        // Every input sample must be reproduced by the flattened curve, which is
        // the whole point of using an interpolating spline instead of a fit.
        for sample in input {
            let nearest = out
                .iter()
                .map(|p| dist(*p, sample))
                .fold(f32::MAX, f32::min);
            assert!(nearest < 0.01, "sample {sample:?} missed by {nearest}");
        }
    }

    #[test]
    fn sharp_turn_does_not_overshoot() {
        // A 90-degree corner drawn with sparse samples: the curve may round the
        // corner but must never leave the bounding box of the drawing, otherwise
        // fast strokes grow loops outside the painted area.
        let input = [[0.0, 0.0], [40.0, 0.0], [40.0, 40.0]];
        let out = smooth_stroke(&input);
        for p in &out {
            assert!(
                p[0] >= -1e-3 && p[0] <= 40.0 + 1e-3,
                "x out of box: {p:?}"
            );
            assert!(
                p[1] >= -1e-3 && p[1] <= 40.0 + 1e-3,
                "y out of box: {p:?}"
            );
        }
    }

    #[test]
    fn uneven_spacing_keeps_handles_short() {
        // Two close samples followed by a far one: a long handle would make the
        // curve bulge away from the drawn path.
        let input = [[0.0, 0.0], [2.0, 0.0], [60.0, 0.0], [62.0, 0.0]];
        let out = smooth_stroke(&input);
        for p in &out {
            assert!(p[1].abs() < 0.05, "straight stroke bent: {p:?}");
            assert!(p[0] >= -0.05 && p[0] <= 62.05, "overshoot: {p:?}");
        }
    }

    #[test]
    fn dense_samples_do_not_explode_the_point_count() {
        // 200 samples along a gentle arc: subdivision must stay near the input
        // density instead of multiplying the work the rasteriser has to do.
        let input: Vec<Pt> = (0..200)
            .map(|i| {
                let t = i as f32 * 0.01;
                [t * 100.0, (t * 1.4).sin() * 8.0]
            })
            .collect();
        let out = smooth_stroke(&input);
        assert!(
            out.len() < input.len() * 3,
            "flattened {} points for {} samples",
            out.len(),
            input.len()
        );
        assert!(max_gap(&out) < 6.0);
    }

    #[test]
    fn duplicate_and_nonfinite_samples_are_dropped() {
        let out = smooth_stroke(&[[5.0, 5.0], [5.0, 5.0], [f32::NAN, 1.0], [9.0, 5.0]]);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0], [5.0, 5.0]);
        assert_eq!(out[1], [9.0, 5.0]);
        assert!(smooth_stroke(&[]).is_empty());
        assert_eq!(smooth_stroke(&[[1.0, 2.0]]), vec![[1.0, 2.0]]);
    }
}