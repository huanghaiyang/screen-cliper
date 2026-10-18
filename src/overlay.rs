//! Full-screen capture overlay: region selection + the annotation editor.
//!
//! Runs as its own short-lived process (spawned by the control window). It shows
//! the frozen screen capture with a borderless full-screen window, lets the user
//! drag a region, then annotates it. The on-screen preview is produced by the
//! same rasterizer used for the exported file, so what you see is exactly what
//! you get.

use crate::config::Config;
use crate::design::{self, ButtonKind, Palette};
use crate::fonts;
use crate::icons::Icon;
use crate::raster::{self, Region};
use crate::shapes::{Pt, RectF, Rgba, Shape, StickerKind, Tool};
use eframe::egui;
use egui::{Color32, Pos2, Rect, RichText, Sense, Stroke, Vec2};
use image::RgbaImage;
use std::path::PathBuf;
use std::time::Instant;

const HANDLE_HIT_RADIUS: f32 = 9.0;
const MIN_SELECTION_AREA: f32 = 24.0;
const ACCENT: Color32 = Color32::from_rgb(0, 150, 255);

#[derive(Debug, Clone)]
pub struct OverlayOptions {
    pub image_path: PathBuf,
    pub monitor: [i32; 4],
    pub scale: f32,
    pub full: bool,
    pub status_path: Option<PathBuf>,
    /// `--demo`: preselect a region, seed sample annotations and stay open.
    /// Used to eyeball the editor chrome without a human at the mouse.
    pub demo: bool,
}

/// Written next to the temp capture so the parent window can report what happened.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct OverlayStatus {
    pub ok: bool,
    pub message: String,
    pub copied: bool,
    pub saved_path: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Choosing,
    Editing,
}

#[derive(Debug, Clone)]
enum Gesture {
    None,
    Creating {
        start: Pt,
    },
    Moving {
        grab: Pt,
        orig: RectF,
    },
    Resizing {
        handle: usize,
        orig: RectF,
    },
    Drawing {
        start: Pt,
        points: Vec<Pt>,
        tool: Tool,
    },
}

struct TextInput {
    pos: Pt,
    text: String,
    focus: bool,
}

#[derive(Clone, Copy)]
struct View {
    rect: Rect,
    /// points per image pixel
    scale: f32,
}

impl View {
    fn to_screen(&self, p: Pt) -> Pos2 {
        self.rect.min + egui::vec2(p[0] * self.scale, p[1] * self.scale)
    }

    fn to_img(&self, p: Pos2) -> Pt {
        let v = p - self.rect.min;
        [v.x / self.scale, v.y / self.scale]
    }

    fn rect_of(&self, r: &RectF) -> Rect {
        Rect::from_min_max(self.to_screen(r.min()), self.to_screen(r.max()))
    }
}

pub struct OverlayApp {
    opts: OverlayOptions,
    config: Config,
    pal: Palette,
    base: RgbaImage,
    base_tex: Option<egui::TextureHandle>,
    preview_tex: Option<egui::TextureHandle>,
    preview_dirty: bool,
    view: View,
    phase: Phase,
    sel: Option<RectF>,
    gesture: Gesture,
    tool: Tool,
    shapes: Vec<Shape>,
    redo: Vec<Shape>,
    live: Option<Shape>,
    color: Rgba,
    pen_width: f32,
    font_size: f32,
    sticker_kind: StickerKind,
    sticker_size: f32,
    mosaic_block: u32,
    next_number: u32,
    text_input: Option<TextInput>,
    toast: Option<(String, Instant)>,
    exit_requested: bool,
    cursor_img: Option<Pt>,
    hover_handle: Option<usize>,
    toolbar_size: Vec2,
    clipboard_image: Option<std::sync::Arc<RgbaImage>>,
    fatal: Option<String>,
    /// Frames painted so far; used to keep the window hidden until the capture is
    /// actually on screen.
    frames: u32,
    started: Instant,
    window_shown: bool,
}

impl OverlayApp {
    /// `base`/`fatal` are prepared by the caller *before* the window is created so
    /// that window creation to first frame is as short as possible.
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        opts: OverlayOptions,
        base: RgbaImage,
        fatal: Option<String>,
    ) -> Self {
        let started = Instant::now();
        fonts::install_egui_fonts(&cc.egui_ctx);
        design::apply(&cc.egui_ctx, design::dark());
        let config = Config::load();
        let tool = Tool::from_key(&config.default_tool);
        let mut app = Self {
            opts,
            color: config.pen_color,
            pen_width: config.pen_width,
            font_size: config.font_size,
            sticker_kind: StickerKind::Check,
            sticker_size: config.sticker_size,
            mosaic_block: config.mosaic_block,
            tool,
            config,
            pal: crate::design::dark(),
            base,
            base_tex: None,
            preview_tex: None,
            preview_dirty: true,
            view: View {
                rect: Rect::from_min_size(Pos2::ZERO, Vec2::splat(100.0)),
                scale: 1.0,
            },
            phase: Phase::Choosing,
            sel: None,
            gesture: Gesture::None,
            shapes: Vec::new(),
            redo: Vec::new(),
            live: None,
            next_number: 1,
            text_input: None,
            toast: None,
            exit_requested: false,
            cursor_img: None,
            hover_handle: None,
            toolbar_size: egui::vec2(640.0, 96.0),
            clipboard_image: None,
            fatal,
            frames: 0,
            started,
            window_shown: false,
        };
        if app.opts.full {
            let full = RectF::new(0.0, 0.0, app.base.width() as f32, app.base.height() as f32);
            app.sel = Some(full);
            app.phase = Phase::Editing;
        }
        if app.opts.demo {
            app.seed_demo();
        }
        app.spawn_first_frame_watchdog(cc.egui_ctx.clone());
        app
    }

    /// Safety net for the "window never becomes visible" failure mode: if the
    /// first frame does not make it to screen (driver stall, stuck event loop),
    /// the overlay would be invisible while the parent waits for it forever.
    fn spawn_first_frame_watchdog(&self, ctx: egui::Context) {
        std::thread::Builder::new()
            .name("overlay-watchdog".into())
            .spawn(move || {
                for _ in 0..4 {
                    std::thread::sleep(std::time::Duration::from_millis(700));
                    let Some(hwnd) = crate::win::find_main_window() else {
                        continue;
                    };
                    if crate::win::is_window_visible(hwnd) {
                        return;
                    }
                    crate::applog::log("标注窗口：首帧迟迟未显示，强制显示窗口");
                    crate::win::set_window_visible(hwnd, true);
                    crate::win::raise_above_all(hwnd);
                    ctx.request_repaint();
                    return;
                }
            })
            .ok();
    }

    /// Preselect a sensible region and fill it with one sample of every tool.
    fn seed_demo(&mut self) {
        let (w, h) = (self.base.width() as f32, self.base.height() as f32);
        let pad_x = w * 0.10;
        let pad_y = h * 0.10;
        let sel = RectF::new(pad_x, pad_y, w - pad_x * 2.0, h - pad_y * 2.0);
        self.sel = Some(sel);
        self.phase = Phase::Editing;

        let red = [235.0, 59.0, 48.0, 255.0];
        let green = [52.0, 199.0, 89.0, 255.0];
        let orange = [255.0, 149.0, 0.0, 255.0];
        let blue = [0.0, 199.0, 255.0, 255.0];
        let yellow = [255.0, 214.0, 10.0, 255.0];
        let px = |fx: f32| sel.x + sel.w * fx;
        let py = |fy: f32| sel.y + sel.h * fy;
        let scale = (sel.w.min(sel.h) / 600.0).clamp(0.6, 2.4);

        self.shapes.push(Shape::Rect {
            a: [px(0.04), py(0.06)],
            b: [px(0.40), py(0.34)],
            color: [red[0] as u8, red[1] as u8, red[2] as u8, 255],
            width: 4.0 * scale,
        });
        self.shapes.push(Shape::Ellipse {
            a: [px(0.44), py(0.06)],
            b: [px(0.72), py(0.34)],
            color: [green[0] as u8, green[1] as u8, green[2] as u8, 255],
            width: 4.0 * scale,
        });
        self.shapes.push(Shape::Arrow {
            a: [px(0.76), py(0.34)],
            b: [px(0.97), py(0.07)],
            color: [orange[0] as u8, orange[1] as u8, orange[2] as u8, 255],
            width: 5.0 * scale,
        });
        let mut zig = Vec::new();
        for i in 0..9 {
            let fx = 0.04 + i as f32 * 0.02;
            let fy = 0.46 + if i % 2 == 0 { 0.0 } else { -0.06 };
            zig.push([px(fx), py(fy)]);
        }
        self.shapes.push(Shape::Stroke {
            points: zig,
            color: [blue[0] as u8, blue[1] as u8, blue[2] as u8, 255],
            width: 4.0 * scale,
            alpha: 1.0,
        });
        self.shapes.push(Shape::Stroke {
            points: vec![[px(0.04), py(0.60)], [px(0.56), py(0.60)]],
            color: [yellow[0] as u8, yellow[1] as u8, yellow[2] as u8, 255],
            width: 18.0 * scale,
            alpha: 0.32,
        });
        self.shapes.push(Shape::Text {
            pos: [px(0.04), py(0.66)],
            text: "用五号粗体标注这一段文字".to_string(),
            color: [255, 255, 255, 255],
            size: 26.0 * scale,
        });
        self.shapes.push(Shape::Mosaic {
            rect: [px(0.62), py(0.42), sel.w * 0.34, sel.h * 0.22],
            block: 12,
        });
        self.shapes.push(Shape::Blur {
            rect: [px(0.62), py(0.68), sel.w * 0.34, sel.h * 0.18],
            radius: 9.0,
        });
        for (i, kind) in StickerKind::ALL.iter().enumerate() {
            self.shapes.push(Shape::Sticker {
                center: [px(0.06 + 0.045 * i as f32), py(0.87)],
                size: 54.0 * scale,
                kind: *kind,
                color: [255, 214, 10, 255],
            });
        }
        for i in 0..4u32 {
            self.shapes.push(Shape::Number {
                center: [px(0.45 + 0.05 * i as f32), py(0.87)],
                size: 42.0 * scale,
                n: i + 1,
                color: [255, 45, 85, 255],
            });
        }
        self.next_number = 5;
        self.preview_dirty = true;
    }

    // -- geometry helpers ---------------------------------------------------

    fn img_size(&self) -> (f32, f32) {
        (self.base.width() as f32, self.base.height() as f32)
    }

    fn clamp_img(&self, p: Pt) -> Pt {
        let (w, h) = self.img_size();
        [p[0].clamp(0.0, w), p[1].clamp(0.0, h)]
    }

    fn update_view(&mut self, ctx: &egui::Context) {
        // Map the capture to the *monitor* rect, not to the window rect: the
        // window is intentionally two pixels smaller than the monitor (see
        // run_overlay) so Windows keeps DWM composing it instead of switching
        // the display to SDR direct scan-out. Mapping to the monitor keeps the
        // capture at its exact pixel scale, and the inset simply clips the last
        // pixel row/column.
        let window = ctx.screen_rect();
        let monitor = &self.opts.monitor;
        let rect = if monitor[2] > 4 && monitor[3] > 4 && self.opts.scale > 0.1 {
            let size = egui::vec2(
                monitor[2] as f32 / self.opts.scale,
                monitor[3] as f32 / self.opts.scale,
            );
            egui::Rect::from_min_size(egui::Pos2::ZERO, size)
        } else {
            window
        };
        let (w, _h) = self.img_size();
        let scale = if w > 0.0 && rect.width() > 0.0 {
            rect.width() / w
        } else {
            // Degenerate window info: fall back to the monitor DPI the parent
            // process reported (points per image pixel = 1 / dpi scale).
            1.0 / self.opts.scale.clamp(0.5, 4.0)
        };
        self.view = View { rect, scale };
    }

    fn handle_positions(&self) -> [Pos2; 8] {
        let Some(sel) = self.sel else {
            return [Pos2::ZERO; 8];
        };
        let r = self.view.rect_of(&sel);
        [
            r.left_top(),
            Pos2::new(r.center().x, r.top()),
            r.right_top(),
            Pos2::new(r.right(), r.center().y),
            r.right_bottom(),
            Pos2::new(r.center().x, r.bottom()),
            r.left_bottom(),
            Pos2::new(r.left(), r.center().y),
        ]
    }

    fn hit_handle(&self, screen: Pos2) -> Option<usize> {
        if self.phase != Phase::Editing {
            return None;
        }
        let positions = self.handle_positions();
        let mut best: Option<(usize, f32)> = None;
        for (i, p) in positions.iter().enumerate() {
            let d = p.distance(screen);
            if d <= HANDLE_HIT_RADIUS && best.map(|(_, bd)| d < bd).unwrap_or(true) {
                best = Some((i, d));
            }
        }
        best.map(|(i, _)| i)
    }

    // -- history ------------------------------------------------------------

    fn push_shape(&mut self, shape: Shape) {
        self.shapes.push(shape);
        self.redo.clear();
        self.preview_dirty = true;
    }

    fn undo(&mut self) {
        if let Some(shape) = self.shapes.pop() {
            self.redo.push(shape);
            self.preview_dirty = true;
        }
    }

    fn redo(&mut self) {
        if let Some(shape) = self.redo.pop() {
            self.shapes.push(shape);
            self.preview_dirty = true;
        }
    }

    // -- preview ------------------------------------------------------------

    fn ensure_base_texture(&mut self, ctx: &egui::Context) {
        if self.base_tex.is_some() {
            return;
        }
        let (w, h) = (self.base.width() as usize, self.base.height() as usize);
        let color_image = egui::ColorImage::from_rgba_unmultiplied([w, h], self.base.as_raw());
        self.base_tex = Some(ctx.load_texture("base", color_image, egui::TextureOptions::NEAREST));
    }

    fn ensure_preview(&mut self, ctx: &egui::Context) {
        if self.phase != Phase::Editing || !self.preview_dirty {
            return;
        }
        let Some(sel) = self.sel else {
            return;
        };
        let region = Region::from_rect(&sel).clamped(self.base.width(), self.base.height());
        let rendered =
            raster::render_region(&self.base, region, &self.shapes, self.live.as_ref());
        let color_image = egui::ColorImage::from_rgba_unmultiplied(
            [region.w as usize, region.h as usize],
            rendered.as_raw(),
        );
        match &mut self.preview_tex {
            Some(handle) => handle.set(color_image, egui::TextureOptions::NEAREST),
            None => {
                self.preview_tex =
                    Some(ctx.load_texture("preview", color_image, egui::TextureOptions::NEAREST))
            }
        }
        self.preview_dirty = false;
    }

    // -- keyboard -----------------------------------------------------------

    fn handle_keys(&mut self, ctx: &egui::Context) {
        let mut want_exit = false;
        let mut want_finish = false;

        ctx.input(|i| {
            if i.key_pressed(egui::Key::Escape) {
                want_exit = true;
            }
            if i.key_pressed(egui::Key::Enter) {
                want_finish = true;
            }
        });

        if want_exit {
            if self.text_input.is_some() {
                self.text_input = None;
            } else {
                self.exit_requested = true;
            }
            return;
        }
        if want_finish {
            if self.phase == Phase::Editing && self.text_input.is_none() {
                self.finish_from_config();
            }
            return;
        }

        if self.phase == Phase::Editing {
            let ctrl = ctx.input(|i| i.modifiers.ctrl);
            let shift = ctx.input(|i| i.modifiers.shift);
            if ctrl && !shift && ctx.input(|i| i.key_pressed(egui::Key::Z)) {
                self.undo();
            }
            if (ctrl && ctx.input(|i| i.key_pressed(egui::Key::Y)))
                || (ctrl && shift && ctx.input(|i| i.key_pressed(egui::Key::Z)))
            {
                self.redo();
            }
            if ctrl && ctx.input(|i| i.key_pressed(egui::Key::C)) {
                self.finish_copy();
            }
            if ctrl && ctx.input(|i| i.key_pressed(egui::Key::S)) {
                self.finish_save();
            }
            if ctrl && ctx.input(|i| i.key_pressed(egui::Key::V)) {
                self.paste_clipboard();
            }
            // Tool shortcuts.
            let map = [
                (egui::Key::R, Tool::Rect),
                (egui::Key::O, Tool::Ellipse),
                (egui::Key::A, Tool::Arrow),
                (egui::Key::P, Tool::Pen),
                (egui::Key::H, Tool::Highlighter),
                (egui::Key::M, Tool::Mosaic),
                (egui::Key::B, Tool::Blur),
                (egui::Key::T, Tool::Text),
                (egui::Key::S, Tool::Sticker),
                (egui::Key::N, Tool::Number),
                (egui::Key::E, Tool::Eraser),
                (egui::Key::V, Tool::Select),
            ];
            if !ctrl {
                for (key, tool) in map {
                    if ctx.input(|i| i.key_pressed(key)) {
                        self.tool = tool;
                    }
                }
            }
        }
    }

    // -- interaction --------------------------------------------------------

    fn interact(&mut self, ctx: &egui::Context, resp: &egui::Response) {
        let hover = resp.hover_pos();
        let pointer = resp.interact_pointer_pos().or(hover);
        let pointer_img = pointer.map(|p| self.view.to_img(p));
        self.cursor_img = pointer_img.map(|p| self.clamp_img(p));

        // Sticker size / stroke width wheel control.
        if self.phase == Phase::Editing {
            let scroll = ctx.input(|i| i.raw_scroll_delta.y);
            if scroll.abs() > 0.1 {
                match self.tool {
                    Tool::Sticker => {
                        self.sticker_size = (self.sticker_size * (1.0 + scroll * 0.004))
                            .clamp(16.0, 400.0);
                    }
                    Tool::Pen | Tool::Highlighter | Tool::Rect | Tool::Ellipse | Tool::Arrow => {
                        self.pen_width = (self.pen_width + scroll * 0.05).clamp(1.0, 40.0);
                    }
                    Tool::Text => {
                        self.font_size = (self.font_size + scroll * 0.1).clamp(8.0, 160.0);
                    }
                    _ => {}
                }
            }
        }

        // Cursor icon.
        if self.phase == Phase::Choosing {
            ctx.set_cursor_icon(egui::CursorIcon::Crosshair);
        } else if let Some(h) = pointer.and_then(|p| self.hit_handle(p)) {
            ctx.set_cursor_icon(match h {
                0 | 4 => egui::CursorIcon::ResizeNwSe,
                2 | 6 => egui::CursorIcon::ResizeNeSw,
                1 | 5 => egui::CursorIcon::ResizeVertical,
                _ => egui::CursorIcon::ResizeHorizontal,
            });
        } else if self.tool == Tool::Select
            && pointer_img
                .map(|p| self.sel.map(|s| s.contains(p)).unwrap_or(false))
                .unwrap_or(false)
        {
            ctx.set_cursor_icon(egui::CursorIcon::Move);
        } else if self.tool == Tool::Text {
            ctx.set_cursor_icon(egui::CursorIcon::Text);
        } else {
            ctx.set_cursor_icon(egui::CursorIcon::Crosshair);
        }

        self.hover_handle = pointer.and_then(|p| self.hit_handle(p));

        match self.phase {
            Phase::Choosing => self.interact_choosing(resp, pointer_img),
            Phase::Editing => self.interact_editing(resp, pointer_img),
        }
    }

    fn interact_choosing(&mut self, resp: &egui::Response, pointer_img: Option<Pt>) {

        if resp.secondary_clicked() {
            self.exit_requested = true;
            return;
        }
        let Some(p) = pointer_img.map(|p| self.clamp_img(p)) else {
            return;
        };
        if resp.drag_started() {
            self.gesture = Gesture::Creating { start: p };
            self.sel = Some(RectF::new(p[0], p[1], 0.0, 0.0));
        } else if resp.dragged() {
            if let Gesture::Creating { start } = self.gesture {
                self.sel = Some(RectF::from_points(start, p));
            }
        } else if resp.drag_stopped() {
            if let Gesture::Creating { .. } = self.gesture {
                self.gesture = Gesture::None;
                let area = self.sel.map(|s| s.area()).unwrap_or(0.0);
                if area >= MIN_SELECTION_AREA {
                    self.phase = Phase::Editing;
                    self.preview_dirty = true;
                } else {
                    self.sel = None;
                }
            }
        }
    }

    fn interact_editing(&mut self, resp: &egui::Response, pointer_img: Option<Pt>) {
        let Some(sel) = self.sel else {
            return;
        };
        let Some(p) = pointer_img.map(|p| self.clamp_img(p)) else {
            return;
        };

        // Right click: drop the last shape, or go back to selecting.
        if resp.secondary_clicked() {
            if !self.shapes.is_empty() {
                self.undo();
            }
            return;
        }

        if resp.drag_started() {
            let handle = self.hover_handle;
            if let Some(h) = handle {
                self.gesture = Gesture::Resizing { handle: h, orig: sel };
            } else if self.tool == Tool::Select && sel.contains(p) {
                self.gesture = Gesture::Moving { grab: p, orig: sel };
            } else if self.tool.uses_drag() {
                self.gesture = Gesture::Drawing {
                    start: p,
                    points: vec![p],
                    tool: self.tool,
                };
                self.live = self.build_live(p, p, &[p]);
            }
        } else if resp.dragged() {
            match self.gesture.clone() {
                Gesture::Moving { grab, orig } => {
                    let dx = p[0] - grab[0];
                    let dy = p[1] - grab[1];
                    let (iw, ih) = self.img_size();
                    let x = (orig.x + dx).clamp(0.0, (iw - orig.w).max(0.0));
                    let y = (orig.y + dy).clamp(0.0, (ih - orig.h).max(0.0));
                    self.sel = Some(RectF::new(x, y, orig.w, orig.h));
                    self.preview_dirty = true;
                }
                Gesture::Resizing { handle, orig } => {
                    let mut l = orig.x;
                    let mut t = orig.y;
                    let mut r = orig.x + orig.w;
                    let mut b = orig.y + orig.h;
                    match handle {
                        0 => {
                            l = p[0];
                            t = p[1];
                        }
                        1 => t = p[1],
                        2 => {
                            r = p[0];
                            t = p[1];
                        }
                        3 => r = p[0],
                        4 => {
                            r = p[0];
                            b = p[1];
                        }
                        5 => b = p[1],
                        6 => {
                            l = p[0];
                            b = p[1];
                        }
                        _ => l = p[0],
                    }
                    let (iw, ih) = self.img_size();
                    let l = l.clamp(0.0, iw);
                    let t = t.clamp(0.0, ih);
                    let r = r.clamp(0.0, iw);
                    let b = b.clamp(0.0, ih);
                    let rect = RectF::from_points([l, t], [r, b]);
                    if rect.area() >= 4.0 {
                        self.sel = Some(rect);
                        self.preview_dirty = true;
                    }
                }
                Gesture::Drawing { start, mut points, tool } => {
                    let mut keep = false;
                    if matches!(tool, Tool::Pen | Tool::Highlighter) {
                        let last = *points.last().unwrap_or(&start);
                        if (p[0] - last[0]).abs() + (p[1] - last[1]).abs() > 0.8 {
                            points.push(p);
                            keep = true;
                        }
                    } else {
                        keep = true;
                    }
                    if keep {
                        self.live = self.build_live(start, p, &points);
                        self.gesture = Gesture::Drawing {
                            start,
                            points,
                            tool,
                        };
                        self.preview_dirty = true;
                    }
                }
                _ => {}
            }
        } else if resp.drag_stopped() {
            if let Some(shape) = self.live.take() {
                let valid = match &shape {
                    Shape::Stroke { points, .. } => points.len() >= 1,
                    Shape::Rect { a, b, .. } | Shape::Ellipse { a, b, .. } => {
                        let r = RectF::from_points(*a, *b);
                        r.area() >= 9.0
                    }
                    Shape::Arrow { a, b, .. } => {
                        ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt() >= 6.0
                    }
                    Shape::Mosaic { rect, .. } | Shape::Blur { rect, .. } => {
                        rect[2].abs() * rect[3].abs() >= 16.0
                    }
                    _ => true,
                };
                if valid {
                    self.push_shape(shape);
                }
            }
            self.gesture = Gesture::None;
            self.preview_dirty = true;
        } else if resp.clicked() {
            self.handle_click(p);
        }
    }

    fn handle_click(&mut self, p: Pt) {
        let (w, h) = self.img_size();
        match self.tool {
            Tool::Pen | Tool::Highlighter => {
                let alpha = if self.tool == Tool::Pen { 1.0 } else { 0.32 };
                let width = if self.tool == Tool::Pen {
                    self.pen_width
                } else {
                    self.pen_width * 3.5
                };
                self.push_shape(Shape::Stroke {
                    points: vec![p, [p[0] + 0.6, p[1] + 0.6]],
                    color: self.color,
                    width,
                    alpha,
                });
            }
            Tool::Sticker => {
                let half = self.sticker_size * 0.5;
                let center = [
                    p[0].clamp(half, w - half),
                    p[1].clamp(half, h - half),
                ];
                self.push_shape(Shape::Sticker {
                    center,
                    size: self.sticker_size,
                    kind: self.sticker_kind,
                    color: self.color,
                });
            }
            Tool::Number => {
                let radius = (self.sticker_size * 0.35).max(12.0);
                let center = [p[0].clamp(radius, w - radius), p[1].clamp(radius, h - radius)];
                let n = self.next_number;
                self.next_number += 1;
                self.push_shape(Shape::Number {
                    center,
                    size: radius * 2.0,
                    n,
                    color: self.color,
                });
            }
            Tool::Text => {
                self.text_input = Some(TextInput {
                    pos: p,
                    text: String::new(),
                    focus: true,
                });
            }
            Tool::Eraser => {
                // Remove the topmost shape whose bbox contains the click.
                if let Some(idx) = self
                    .shapes
                    .iter()
                    .rposition(|s| s.bbox().expand(4.0).contains(p))
                {
                    self.shapes.remove(idx);
                    self.preview_dirty = true;
                }
            }
            _ => {}
        }
    }

    fn build_live(&self, start: Pt, current: Pt, points: &[Pt]) -> Option<Shape> {
        let rect = RectF::from_points(start, current);
        match self.tool {
            Tool::Pen => Some(Shape::Stroke {
                points: points.to_vec(),
                color: self.color,
                width: self.pen_width,
                alpha: 1.0,
            }),
            Tool::Highlighter => Some(Shape::Stroke {
                points: points.to_vec(),
                color: self.color,
                width: self.pen_width * 3.5,
                alpha: 0.32,
            }),
            Tool::Rect => Some(Shape::Rect {
                a: start,
                b: current,
                color: self.color,
                width: self.pen_width,
            }),
            Tool::Ellipse => Some(Shape::Ellipse {
                a: start,
                b: current,
                color: self.color,
                width: self.pen_width,
            }),
            Tool::Arrow => Some(Shape::Arrow {
                a: start,
                b: current,
                color: self.color,
                width: self.pen_width,
            }),
            Tool::Mosaic => Some(Shape::Mosaic {
                rect: rect.as_xywh(),
                block: self.mosaic_block,
            }),
            Tool::Blur => Some(Shape::Blur {
                rect: rect.as_xywh(),
                radius: (self.mosaic_block as f32) * 0.8,
            }),
            _ => None,
        }
    }

    fn paste_clipboard(&mut self) {
        if self.clipboard_image.is_none() {
            match arboard::Clipboard::new().and_then(|mut cb| cb.get_image()) {
                Ok(img) => {
                    let w = img.width as u32;
                    let h = img.height as u32;
                    match RgbaImage::from_raw(w, h, img.bytes.into_owned()) {
                        Some(rgba) => self.clipboard_image = Some(std::sync::Arc::new(rgba)),
                        None => self.set_toast("剪贴板图片格式无效"),
                    }
                }
                Err(_) => self.set_toast("剪贴板里没有图片"),
            }
        }
        if let Some(img) = self.clipboard_image.clone() {
            let Some(sel) = self.sel else { return };
            // Fit inside the selection, keeping the aspect ratio.
            let scale = (sel.w / img.width() as f32)
                .min(sel.h / img.height() as f32)
                .min(1.0);
            let w = img.width() as f32 * scale;
            let h = img.height() as f32 * scale;
            let x = sel.x + (sel.w - w) * 0.5;
            let y = sel.y + (sel.h - h) * 0.5;
            self.push_shape(Shape::Bitmap {
                rect: [x, y, w, h],
                image: img,
            });
        }
    }

    // -- finishing ----------------------------------------------------------

    fn set_toast(&mut self, msg: impl Into<String>) {
        self.toast = Some((msg.into(), Instant::now()));
    }

    fn export(&self) -> Option<RgbaImage> {
        let sel = self.sel?;
        let region = Region::from_rect(&sel).clamped(self.base.width(), self.base.height());
        Some(raster::render_region(&self.base, region, &self.shapes, None))
    }

    fn finish_from_config(&mut self) {
        let mode = self.config.after_capture;
        let mut status = OverlayStatus::default();
        let mut ok = true;

        if mode.copies() {
            match self.export().map(|img| copy_to_clipboard(&img)) {
                Some(Ok(())) => {
                    status.copied = true;
                    status.message = "已复制到剪贴板".to_string();
                }
                Some(Err(err)) => {
                    ok = false;
                    status.message = err;
                }
                None => {
                    ok = false;
                    status.message = "没有可导出的选区".to_string();
                }
            }
        }

        if mode.saves() {
            match self.save_to_default_path() {
                Ok(path) => {
                    status.message = if status.copied {
                        format!("已复制并保存到 {path}")
                    } else {
                        format!("已保存到 {path}")
                    };
                    status.saved_path = Some(path);
                }
                Err(err) => {
                    ok = false;
                    status.message = err;
                }
            }
        }

        status.ok = ok;
        self.write_status(&status);
        if ok {
            self.exit_requested = true;
        } else if !status.message.is_empty() {
            // Keep the overlay alive so the user can retry or copy manually.
            self.set_toast(status.message);
        }
    }

    fn finish_copy(&mut self) {
        let mut status = OverlayStatus::default();
        match self.export().map(|img| copy_to_clipboard(&img)) {
            Some(Ok(())) => {
                status.ok = true;
                status.copied = true;
                status.message = "已复制到剪贴板".into();
                self.write_status(&status);
                self.exit_requested = true;
            }
            Some(Err(err)) => self.set_toast(err),
            None => {}
        }
    }

    fn finish_save(&mut self) {
        match self.save_to_default_path() {
            Ok(path) => {
                let status = OverlayStatus {
                    ok: true,
                    message: format!("已保存到 {path}"),
                    copied: false,
                    saved_path: Some(path),
                };
                self.write_status(&status);
                self.exit_requested = true;
            }
            Err(err) => self.set_toast(err),
        }
    }

    fn save_to_default_path(&self) -> Result<String, String> {
        let img = self.export().ok_or_else(|| "没有选区".to_string())?;
        let path = self.config.next_output_path()?;
        self.config.write_image(&img, &path)?;
        Ok(path.display().to_string())
    }

    fn write_status(&self, status: &OverlayStatus) {
        let Some(path) = &self.opts.status_path else {
            return;
        };
        if let Ok(text) = serde_json::to_string(status) {
            let _ = std::fs::write(path, text);
        }
    }

    // -- painting -----------------------------------------------------------

    /// Draw a rounded label whose left-middle edge sits at `anchor`.
    fn pill_left(
        &self,
        painter: &egui::Painter,
        anchor: Pos2,
        text: &str,
        fill: Color32,
        fg: Color32,
        font_size: f32,
    ) -> Rect {
        let font = egui::FontId::proportional(font_size);
        let galley = painter.layout_no_wrap(text.to_owned(), font, fg);
        let size = galley.size() + egui::vec2(18.0, 9.0);
        let rect = Rect::from_min_size(
            Pos2::new(anchor.x, anchor.y - size.y * 0.5),
            size,
        )
        .expand(0.0);
        painter.rect_filled(rect, egui::Rounding::same(6.0), fill);
        painter.galley(rect.center() - galley.size() * 0.5, galley, fg);
        rect
    }

    /// Centred rounded label (hints / toasts).
    fn pill_center(
        &self,
        painter: &egui::Painter,
        center: Pos2,
        text: &str,
        fill: Color32,
        fg: Color32,
        font_size: f32,
    ) -> Rect {
        let font = egui::FontId::proportional(font_size);
        let galley = painter.layout_no_wrap(text.to_owned(), font, fg);
        let size = galley.size() + egui::vec2(20.0, 10.0);
        let rect = Rect::from_center_size(center, size);
        painter.rect_filled(rect, egui::Rounding::same(7.0), fill);
        painter.galley(rect.center() - galley.size() * 0.5, galley, fg);
        rect
    }

    fn paint(&mut self, ctx: &egui::Context, painter: &egui::Painter) {
        let view = self.view;
        let screen = view.rect;
        let p = self.pal;
        let full_uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));

        // 1. Frozen screen, dimmed outside the selection.
        match &self.base_tex {
            Some(tex) => {
                painter.image(
                    tex.id(),
                    screen,
                    full_uv,
                    Color32::from_rgba_unmultiplied(255, 255, 255, 112),
                );
            }
            None => {
                painter.rect_filled(screen, 0.0, Color32::BLACK);
            }
        }

        // 2. Fully rendered preview inside the selection.
        if self.phase == Phase::Editing {
            if let (Some(sel), Some(tex)) = (self.sel, &self.preview_tex) {
                painter.image(tex.id(), view.rect_of(&sel), full_uv, Color32::WHITE);
            }
        }

        // 3. Selection chrome: two-tone border + circular handles.
        if let Some(sel) = self.sel {
            let r = view.rect_of(&sel);
            if r.width() > 1.0 && r.height() > 1.0 {
                painter.rect_stroke(
                    r.expand(0.5),
                    0.0,
                    Stroke::new(1.0_f32, Color32::from_black_alpha(100)),
                );
                painter.rect_stroke(r, 0.0, Stroke::new(1.5_f32, p.accent));
            }
            if self.phase == Phase::Editing {
                for (i, hp) in self.handle_positions().into_iter().enumerate() {
                    let hovered = self.hover_handle == Some(i);
                    let radius = if hovered { 6.5 } else { 5.0 };
                    painter.circle_filled(hp, radius + 1.0, Color32::from_black_alpha(70));
                    painter.circle_filled(hp, radius, Color32::WHITE);
                    painter.circle_stroke(hp, radius, Stroke::new(1.5_f32, p.accent));
                }
                if self.config.show_size_label {
                    let text = format!(
                        "{} × {}",
                        sel.w.round().max(0.0) as i64,
                        sel.h.round().max(0.0) as i64
                    );
                    let above = r.top() - 15.0;
                    let y = if above < screen.top() + 12.0 {
                        r.top() + 17.0
                    } else {
                        above
                    };
                    self.pill_left(
                        painter,
                        Pos2::new(r.left() + 2.0, y),
                        &text,
                        p.accent,
                        Color32::WHITE,
                        12.5,
                    );
                }
            }
        }

        // 4. Crosshair + magnifier while selecting.
        if self.phase == Phase::Choosing {
            if let Some(ip) = self.cursor_img {
                let pos = view.to_screen(ip);
                let hair = Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(255, 255, 255, 165));
                painter.line_segment(
                    [Pos2::new(screen.left(), pos.y), Pos2::new(pos.x - 6.0, pos.y)],
                    hair,
                );
                painter.line_segment(
                    [Pos2::new(pos.x + 6.0, pos.y), Pos2::new(screen.right(), pos.y)],
                    hair,
                );
                painter.line_segment(
                    [Pos2::new(pos.x, screen.top()), Pos2::new(pos.x, pos.y - 6.0)],
                    hair,
                );
                painter.line_segment(
                    [Pos2::new(pos.x, pos.y + 6.0), Pos2::new(pos.x, screen.bottom())],
                    hair,
                );
                if self.config.magnifier {
                    self.paint_magnifier(ctx, painter, ip, pos);
                }
                if let Some(sel) = self.sel {
                    if sel.area() > 1.0 {
                        let r = view.rect_of(&sel);
                        let text =
                            format!("{} × {}", sel.w.round() as i64, sel.h.round() as i64);
                        self.pill_left(
                            painter,
                            Pos2::new(r.left() + 2.0, (r.top() - 15.0).max(screen.top() + 12.0)),
                            &text,
                            p.accent,
                            Color32::WHITE,
                            12.5,
                        );
                    }
                }
            } else {
                self.pill_center(
                    painter,
                    Pos2::new(screen.center().x, screen.center().y),
                    "按住鼠标左键拖动框选区域　·　Esc 或右键取消",
                    Color32::from_black_alpha(200),
                    Color32::from_gray(235),
                    13.5,
                );
            }
        }

        // 5. Tool hint + toast + fatal error.
        if self.phase == Phase::Editing && self.shapes.is_empty() && self.toast.is_none() {
            self.paint_hint(painter, screen, self.tool.hint());
        }
        if let Some((msg, at)) = &self.toast {
            if at.elapsed().as_millis() < 2600 {
                self.pill_center(
                    painter,
                    Pos2::new(screen.center().x, screen.bottom() - 92.0),
                    msg,
                    Color32::from_black_alpha(225),
                    Color32::from_rgb(255, 226, 226),
                    13.0,
                );
                ctx.request_repaint_after(std::time::Duration::from_millis(200));
            } else {
                self.toast = None;
            }
        }
        if let Some(fatal) = &self.fatal {
            self.pill_center(
                painter,
                Pos2::new(screen.center().x, screen.center().y + 60.0),
                fatal,
                p.danger.gamma_multiply(0.9),
                Color32::WHITE,
                13.0,
            );
        }
    }

    fn paint_hint(&self, painter: &egui::Painter, screen: Rect, text: &str) {
        let p = self.pal;
        let center = Pos2::new(screen.center().x, screen.bottom() - 58.0);
        let font = egui::FontId::proportional(12.5);
        let galley = painter.layout_no_wrap(text.to_owned(), font, p.text_dim);
        let size = galley.size() + egui::vec2(34.0, 12.0);
        let rect = Rect::from_center_size(center, size);
        painter.rect_filled(
            rect.translate(egui::vec2(0.0, 3.0)),
            egui::Rounding::same(9.0),
            Color32::from_black_alpha(70),
        );
        painter.rect_filled(rect, egui::Rounding::same(9.0), Color32::from_black_alpha(185));
        painter.rect_stroke(
            rect,
            egui::Rounding::same(9.0),
            Stroke::new(1.0_f32, Color32::from_white_alpha(28)),
        );
        // little dot marker
        painter.circle_filled(
            Pos2::new(rect.left() + 13.0, rect.center().y),
            3.0,
            p.accent,
        );
        painter.galley(
            Pos2::new(rect.left() + 22.0, rect.center().y - galley.size().y * 0.5),
            galley,
            p.text_dim,
        );
    }

    fn paint_magnifier(
        &self,
        _ctx: &egui::Context,
        painter: &egui::Painter,
        p_img: Pt,
        pos: Pos2,
    ) {
        let Some(tex) = &self.base_tex else {
            return;
        };
        let pal = self.pal;
        let screen = self.view.rect;
        let size = 126.0;
        let offset = 26.0;
        let mut center = Pos2::new(pos.x + offset + size * 0.5, pos.y + offset + size * 0.5);
        if center.x + size * 0.5 > screen.right() - 12.0 {
            center.x = pos.x - offset - size * 0.5;
        }
        if center.y + size * 0.5 > screen.bottom() - 12.0 {
            center.y = pos.y - offset - size * 0.5;
        }
        let rect = Rect::from_center_size(center, Vec2::splat(size));

        // 26x26 image-pixel window around the cursor.
        let half_px = 13.0;
        let (iw, ih) = self.img_size();
        let u = (p_img[0] / iw).clamp(0.0, 1.0);
        let v = (p_img[1] / ih).clamp(0.0, 1.0);
        let du = half_px / iw;
        let dv = half_px / ih;
        let uv = Rect::from_min_max(
            Pos2::new((u - du).clamp(0.0, 1.0), (v - dv).clamp(0.0, 1.0)),
            Pos2::new((u + du).clamp(0.0, 1.0), (v + dv).clamp(0.0, 1.0)),
        );

        // Drop shadow + panel frame.
        painter.rect_filled(
            rect.translate(egui::vec2(0.0, 7.0)).expand(3.0),
            egui::Rounding::same(12.0),
            Color32::from_black_alpha(110),
        );
        painter.rect_filled(rect.expand(3.0), egui::Rounding::same(12.0), pal.panel);
        painter.image(tex.id(), rect, uv, Color32::WHITE);
        painter.rect_stroke(
            rect,
            egui::Rounding::same(10.0),
            Stroke::new(1.2_f32, pal.accent.gamma_multiply(0.85)),
        );
        painter.rect_stroke(
            rect.expand(3.0),
            egui::Rounding::same(12.0),
            Stroke::new(1.0_f32, pal.border),
        );

        // Centre crosshair.
        let c = rect.center();
        let hair = Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(255, 90, 90, 235));
        painter.line_segment([Pos2::new(c.x - 9.0, c.y), Pos2::new(c.x + 9.0, c.y)], hair);
        painter.line_segment([Pos2::new(c.x, c.y - 9.0), Pos2::new(c.x, c.y + 9.0)], hair);
        painter.circle_stroke(c, 9.0, Stroke::new(1.0_f32, Color32::from_white_alpha(120)));

        // Colour + coordinate readout.
        let ix = p_img[0].floor().clamp(0.0, iw - 1.0) as u32;
        let iy = p_img[1].floor().clamp(0.0, ih - 1.0) as u32;
        let px = self.base.get_pixel(ix, iy).0;
        let label = format!(
            "({}, {})  #{:02X}{:02X}{:02X}",
            ix as i32 + self.opts.monitor[0],
            iy as i32 + self.opts.monitor[1],
            px[0],
            px[1],
            px[2]
        );
        let font = egui::FontId::monospace(11.0);
        let galley = painter.layout_no_wrap(label, font, pal.text);
        let lr = Rect::from_min_size(
            Pos2::new(rect.left(), rect.bottom() + 7.0),
            galley.size() + egui::vec2(12.0, 6.0),
        );
        painter.rect_filled(lr, egui::Rounding::same(6.0), Color32::from_black_alpha(205));
        painter.rect_stroke(
            lr,
            egui::Rounding::same(6.0),
            Stroke::new(1.0_f32, Color32::from_white_alpha(25)),
        );
        painter.galley(lr.min + egui::vec2(6.0, 3.0), galley, pal.text);
    }

    // -- toolbar ------------------------------------------------------------

    fn toolbar_ui(&mut self, ctx: &egui::Context) {
        // Text entry lives in its own small area.
        self.text_input_ui(ctx);
        if self.phase != Phase::Editing {
            return;
        }
        let Some(sel) = self.sel else { return };
        let p = self.pal;
        let sel_screen = self.view.rect_of(&sel);
        let screen = self.view.rect;
        let estimated = self.toolbar_size;
        let gap = 12.0;

        // Prefer just below the selection, right-aligned with it.
        let mut pos = Pos2::new(
            sel_screen.right() - estimated.x,
            sel_screen.bottom() + gap,
        );
        if pos.y + estimated.y > screen.bottom() - 10.0 {
            pos.y = sel_screen.top() - gap - estimated.y;
        }
        if pos.y < 10.0 {
            pos.y = (screen.bottom() - estimated.y - 10.0).max(10.0);
        }
        pos.x = pos.x.clamp(10.0, (screen.right() - estimated.x - 10.0).max(10.0));

        let frame = egui::Frame::none()
            .fill(p.panel)
            .stroke(Stroke::new(1.0_f32, p.border))
            .rounding(egui::Rounding::same(12.0))
            .inner_margin(egui::Margin::symmetric(9.0, 7.0))
            .shadow(egui::epaint::Shadow {
                offset: egui::vec2(0.0, 10.0),
                blur: 26.0,
                spread: 0.0,
                color: Color32::from_black_alpha(140),
            });

        let mut measured = estimated;
        egui::Area::new(egui::Id::new("overlay_toolbar"))
            .fixed_pos(pos)
            .order(egui::Order::Foreground)
            .interactable(true)
            .show(ctx, |ui| {
                frame.show(ui, |ui| {
                    ui.set_max_width(screen.width() - 40.0);
                    self.toolbar_content(ui);
                    measured = ui.min_rect().size();
                });
            });
        self.toolbar_size = measured;
    }

    fn toolbar_content(&mut self, ui: &mut egui::Ui) {
        let p = self.pal;
        ui.spacing_mut().item_spacing = egui::vec2(3.0, 5.0);

        // Row 1: tools, history, export actions.
        ui.horizontal(|ui| {
            for tool in Tool::ALL {
                let selected = self.tool == tool;
                if design::icon_button(
                    ui,
                    p,
                    tool_icon(tool),
                    &format!("{} ({})", tool.label(), tool_key(tool)),
                    selected,
                    egui::vec2(32.0, 30.0),
                )
                .clicked()
                {
                    self.tool = tool;
                }
            }
            toolbar_sep(ui, p);
            if design::icon_button(ui, p, Icon::Paste, "贴图：粘贴剪贴板图片 (Ctrl+V)", false, egui::vec2(32.0, 30.0)).clicked() {
                self.paste_clipboard();
            }
            if design::icon_button(ui, p, Icon::Undo, "撤销 (Ctrl+Z)", false, egui::vec2(32.0, 30.0)).clicked() {
                self.undo();
            }
            if design::icon_button(ui, p, Icon::Redo, "重做 (Ctrl+Y)", false, egui::vec2(32.0, 30.0)).clicked() {
                self.redo();
            }
            toolbar_sep(ui, p);
            if design::icon_button(ui, p, Icon::Copy, "复制到剪贴板 (Ctrl+C)", false, egui::vec2(32.0, 30.0)).clicked() {
                self.finish_copy();
            }
            if design::icon_button(ui, p, Icon::Save, "保存文件 (Ctrl+S)", false, egui::vec2(32.0, 30.0)).clicked() {
                self.finish_save();
            }
            if design::button_icon(
                ui,
                p,
                "完成",
                Some(Icon::Check),
                ButtonKind::Success,
                egui::vec2(76.0, 30.0),
            )
            .on_hover_text("按当前设置完成（回车）")
            .clicked()
            {
                self.finish_from_config();
            }
            if design::icon_button(ui, p, Icon::Close, "放弃本次截图 (Esc)", false, egui::vec2(30.0, 30.0)).clicked() {
                self.exit_requested = true;
            }
        });

        // Row 2: colour, size, and context-specific options.
        ui.horizontal(|ui| {
            let swatch = Color32::from_rgba_unmultiplied(
                self.color[0],
                self.color[1],
                self.color[2],
                self.color[3],
            );
            let (rect, _) = ui.allocate_exact_size(egui::vec2(26.0, 24.0), Sense::hover());
            ui.painter().circle_filled(rect.center(), 8.5, swatch);
            ui.painter()
                .circle_stroke(rect.center(), 10.0, Stroke::new(1.0_f32, p.border));

            for c in crate::config::PALETTE {
                let selected = self.color == c;
                let color = Color32::from_rgba_unmultiplied(c[0], c[1], c[2], 255);
                let (r, resp) = ui.allocate_exact_size(egui::vec2(18.0, 24.0), Sense::click());
                ui.painter()
                    .circle_filled(r.center(), if selected { 7.5 } else { 6.5 }, color);
                if selected {
                    ui.painter()
                        .circle_stroke(r.center(), 9.5, Stroke::new(1.6_f32, p.text));
                } else if resp.hovered() {
                    ui.painter()
                        .circle_stroke(r.center(), 9.0, Stroke::new(1.0_f32, p.text_faint));
                }
                if resp.clicked() {
                    self.color = c;
                }
                if resp.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
            }

            toolbar_sep(ui, p);
            let width_of = |this: &Self| -> f32 {
                match this.tool {
                    Tool::Text => this.font_size,
                    Tool::Sticker => this.sticker_size,
                    _ => this.pen_width,
                }
            };
            if design::icon_button(ui, p, Icon::Minus, "变细", false, egui::vec2(26.0, 24.0)).clicked() {
                match self.tool {
                    Tool::Text => self.font_size = (self.font_size - 2.0).clamp(8.0, 160.0),
                    Tool::Sticker => self.sticker_size = (self.sticker_size - 6.0).clamp(16.0, 400.0),
                    _ => self.pen_width = (self.pen_width - 1.0).clamp(1.0, 40.0),
                }
            }
            let value_text = if matches!(self.tool, Tool::Text | Tool::Sticker) {
                format!("{:.0}", width_of(self))
            } else {
                format!("{:.0}px", width_of(self))
            };
            let (vr, _) = ui.allocate_exact_size(egui::vec2(38.0, 24.0), Sense::hover());
            ui.painter().text(
                vr.center(),
                egui::Align2::CENTER_CENTER,
                value_text,
                egui::FontId::proportional(12.0),
                p.text_dim,
            );
            if design::icon_button(ui, p, Icon::Plus, "变粗 / 变大", false, egui::vec2(26.0, 24.0)).clicked() {
                match self.tool {
                    Tool::Text => self.font_size = (self.font_size + 2.0).clamp(8.0, 160.0),
                    Tool::Sticker => self.sticker_size = (self.sticker_size + 6.0).clamp(16.0, 400.0),
                    _ => self.pen_width = (self.pen_width + 1.0).clamp(1.0, 40.0),
                }
            }

            toolbar_sep(ui, p);
            match self.tool {
                Tool::Sticker => {
                    for kind in StickerKind::ALL {
                        let selected = self.sticker_kind == kind;
                        if glyph_button(ui, p, kind.glyph(), kind.label(), selected).clicked() {
                            self.sticker_kind = kind;
                        }
                    }
                }
                Tool::Mosaic => {
                    ui.spacing_mut().slider_width = 92.0;
                    ui.add_space(2.0);
                    ui.label(
                        RichText::new("颗粒")
                            .size(11.5)
                            .color(p.text_dim),
                    );
                    if ui
                        .add(
                            egui::Slider::new(&mut self.mosaic_block, 3..=60)
                                .show_value(true)
                                .max_decimals(0),
                        )
                        .changed()
                    {
                        self.preview_dirty = true;
                    }
                }
                Tool::Blur => {
                    ui.spacing_mut().slider_width = 92.0;
                    ui.add_space(2.0);
                    ui.label(RichText::new("强度").size(11.5).color(p.text_dim));
                    ui.add(
                        egui::Slider::new(&mut self.mosaic_block, 2..=30)
                            .show_value(true)
                            .max_decimals(0),
                    );
                }
                Tool::Text => {
                    ui.spacing_mut().slider_width = 92.0;
                    ui.add_space(2.0);
                    ui.label(RichText::new("字号").size(11.5).color(p.text_dim));
                    ui.add(
                        egui::Slider::new(&mut self.font_size, 8.0..=120.0)
                            .show_value(true)
                            .max_decimals(0),
                    );
                }
                Tool::Number => {
                    ui.label(
                        RichText::new(format!("下一个序号：{}", self.next_number))
                            .size(11.5)
                            .color(p.text_dim),
                    );
                    if design::button(ui, p, "重置", ButtonKind::Ghost, egui::vec2(46.0, 24.0))
                        .clicked()
                    {
                        self.next_number = 1;
                    }
                }
                _ => {
                    ui.add_space(2.0);
                    ui.label(RichText::new("粗细").size(11.5).color(p.text_dim));
                    ui.add_space(2.0);
                    ui.spacing_mut().slider_width = 92.0;
                    ui.add(
                        egui::Slider::new(&mut self.pen_width, 1.0..=30.0)
                            .show_value(true)
                            .max_decimals(0),
                    );
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new("滚轮可调整大小")
                            .size(10.5)
                            .color(p.text_faint),
                    );
                }
            };
        });
    }

    fn text_input_ui(&mut self, ctx: &egui::Context) {
        let Some(mut input) = self.text_input.take() else {
            return;
        };
        let screen_pos = self.view.to_screen(input.pos);
        let mut commit = false;
        let mut cancel = false;
        egui::Area::new(egui::Id::new("overlay_text_input"))
            .fixed_pos(screen_pos)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::none()
                    .fill(Color32::from_black_alpha(220))
                    .stroke(Stroke::new(1.0_f32, ACCENT))
                    .rounding(egui::Rounding::same(4.0))
                    .inner_margin(egui::Margin::same(4.0))
                    .show(ui, |ui| {
                        let edit = egui::TextEdit::singleline(&mut input.text)
                            .font(egui::FontId::proportional(16.0))
                            .text_color(Color32::WHITE)
                            .desired_width(200.0)
                            .hint_text("输入文字后回车");
                        let resp = ui.add(edit);
                        if input.focus {
                            resp.request_focus();
                            input.focus = false;
                        }
                        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                            cancel = true;
                        } else if resp.lost_focus() {
                            // Enter, Tab or a click elsewhere all finish the text run.
                            commit = true;
                        }
                    });
            });
        if cancel {
            return;
        }
        if commit {
            if input.text.trim().is_empty() {
                return;
            }
            let shape = Shape::Text {
                pos: input.pos,
                text: input.text.trim().to_string(),
                color: self.color,
                size: self.font_size,
            };
            self.push_shape(shape);
            return;
        }
        self.text_input = Some(input);
    }
}

fn tool_key(tool: Tool) -> &'static str {
    match tool {
        Tool::Select => "V",
        Tool::Rect => "R",
        Tool::Ellipse => "O",
        Tool::Arrow => "A",
        Tool::Pen => "P",
        Tool::Highlighter => "H",
        Tool::Mosaic => "M",
        Tool::Blur => "B",
        Tool::Text => "T",
        Tool::Sticker => "S",
        Tool::Number => "N",
        Tool::Eraser => "E",
    }
}
fn tool_icon(tool: Tool) -> Icon {
    match tool {
        Tool::Select => Icon::Select,
        Tool::Rect => Icon::Rect,
        Tool::Ellipse => Icon::Ellipse,
        Tool::Arrow => Icon::Arrow,
        Tool::Pen => Icon::Pen,
        Tool::Highlighter => Icon::Highlighter,
        Tool::Mosaic => Icon::Mosaic,
        Tool::Blur => Icon::Blur,
        Tool::Text => Icon::Text,
        Tool::Sticker => Icon::Sticker,
        Tool::Number => Icon::Number,
        Tool::Eraser => Icon::Eraser,
    }
}

fn toolbar_sep(ui: &mut egui::Ui, p: Palette) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(9.0, 26.0), Sense::hover());
    let cx = rect.center().x;
    ui.painter().line_segment(
        [
            egui::Pos2::new(cx, rect.top() + 4.0),
            egui::Pos2::new(cx, rect.bottom() - 4.0),
        ],
        Stroke::new(1.0_f32, p.border),
    );
}

fn glyph_button(ui: &mut egui::Ui, p: Palette, glyph: &str, tip: &str, selected: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(28.0, 24.0), Sense::click());
    let fill = if selected {
        p.accent
    } else if resp.hovered() {
        p.card_hover
    } else {
        Color32::TRANSPARENT
    };
    if fill != Color32::TRANSPARENT {
        ui.painter().rect_filled(rect, egui::Rounding::same(7.0), fill);
    }
    let fg = if selected { p.on_accent } else { p.text };
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        glyph,
        egui::FontId::proportional(15.0),
        fg,
    );
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp.on_hover_text(tip)
}
impl eframe::App for OverlayApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // eframe may re-apply the OS (light) theme; the overlay is always dark.
        if !ctx.style().visuals.dark_mode {
            design::apply(ctx, self.pal);
        }
        self.update_view(ctx);
        self.ensure_base_texture(ctx);
        self.handle_keys(ctx);

        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(Color32::BLACK))
            .show(ctx, |ui| {
                let size = ui.available_size();
                let (resp, painter) = ui.allocate_painter(size, Sense::click_and_drag());

                self.interact(ctx, &resp);
                self.ensure_preview(ctx);
                self.paint(ctx, &painter);
            });

        self.toolbar_ui(ctx);

        if self.exit_requested {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if self.toast.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(120));
        }

        // The capture is only on screen once a frame has been presented; eframe
        // shows the window right after that. Doing the topmost/focus work on the
        // frame after keeps it from being applied before the first paint.
        self.frames = self.frames.saturating_add(1);
        if self.frames == 1 {
            crate::applog::log(format!(
                "标注窗口：首帧已绘制（进程内 {}ms）",
                self.started.elapsed().as_millis()
            ));
        }
        if self.frames >= 2 && !self.window_shown {
            self.window_shown = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            if let Some(hwnd) = crate::win::find_main_window() {
                if !crate::win::is_window_visible(hwnd) {
                    crate::applog::log("标注窗口：仍未显示，主动显示并置顶");
                    crate::win::set_window_visible(hwnd, true);
                }
                crate::win::raise_above_all(hwnd);
            }
            crate::applog::log("标注窗口：已显示并就绪");
        }
    }
}

pub fn copy_to_clipboard(img: &RgbaImage) -> Result<(), String> {
    let mut clipboard = arboard::Clipboard::new().map_err(|e| format!("打开剪贴板失败：{e}"))?;
    clipboard
        .set_image(arboard::ImageData {
            width: img.width() as usize,
            height: img.height() as usize,
            bytes: std::borrow::Cow::Borrowed(img.as_raw()),
        })
        .map_err(|e| format!("写入剪贴板失败：{e}"))
}
