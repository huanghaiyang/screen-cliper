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
