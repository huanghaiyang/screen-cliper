//! Design system: palette, spacing, and the small set of custom widgets the
//! settings window and the capture overlay both use.
//!
//! Everything here is drawn by hand on purpose — egui's defaults look like a
//! developer tool, and the app is judged by how the settings window looks.

use crate::icons::{self, Icon};
use eframe::egui;
use egui::{
    Align2, Color32, FontFamily, FontId, Response, RichText, Rounding, Sense, Stroke, TextStyle,
    Ui, Vec2,
};

#[derive(Clone, Copy)]
pub struct Palette {
    pub is_dark: bool,
    pub bg: Color32,
    pub panel: Color32,
    pub card: Color32,
    pub card_hover: Color32,
    pub input: Color32,
    pub border: Color32,
    pub border_soft: Color32,
    pub text: Color32,
    pub text_dim: Color32,
    pub text_faint: Color32,
    pub accent: Color32,
    pub accent_hover: Color32,
    pub on_accent: Color32,
    pub success: Color32,
    pub danger: Color32,
    pub warn: Color32,
}

pub fn dark() -> Palette {
    Palette {
        is_dark: true,
        bg: Color32::from_rgb(15, 17, 21),
        panel: Color32::from_rgb(20, 23, 28),
        card: Color32::from_rgb(26, 30, 37),
        card_hover: Color32::from_rgb(33, 38, 47),
        input: Color32::from_rgb(14, 16, 20),
        border: Color32::from_rgb(45, 51, 61),
        border_soft: Color32::from_rgb(34, 39, 47),
        text: Color32::from_rgb(232, 236, 243),
        text_dim: Color32::from_rgb(154, 163, 177),
        text_faint: Color32::from_rgb(108, 117, 130),
        accent: Color32::from_rgb(76, 141, 255),
        accent_hover: Color32::from_rgb(104, 160, 255),
        on_accent: Color32::WHITE,
        success: Color32::from_rgb(51, 209, 122),
        danger: Color32::from_rgb(255, 93, 93),
        warn: Color32::from_rgb(255, 176, 32),
    }
}

pub fn light() -> Palette {
    Palette {
        is_dark: false,
        bg: Color32::from_rgb(243, 245, 248),
        panel: Color32::from_rgb(255, 255, 255),
        card: Color32::from_rgb(255, 255, 255),
        card_hover: Color32::from_rgb(246, 248, 252),
        input: Color32::from_rgb(241, 244, 249),
        border: Color32::from_rgb(210, 217, 226),
        border_soft: Color32::from_rgb(228, 233, 240),
        text: Color32::from_rgb(22, 30, 41),
        text_dim: Color32::from_rgb(88, 100, 116),
        text_faint: Color32::from_rgb(134, 145, 160),
        accent: Color32::from_rgb(31, 111, 235),
        accent_hover: Color32::from_rgb(24, 93, 205),
        on_accent: Color32::WHITE,
        success: Color32::from_rgb(21, 145, 82),
        danger: Color32::from_rgb(210, 58, 58),
        warn: Color32::from_rgb(190, 118, 8),
    }
}

pub fn for_dark(is_dark: bool) -> Palette {
    if is_dark {
        dark()
    } else {
        light()
    }
}

/// Apply the palette plus a consistent widget style to an egui context.
pub fn apply(ctx: &egui::Context, p: Palette) {
    ctx.set_theme(if p.is_dark {
        egui::ThemePreference::Dark
    } else {
        egui::ThemePreference::Light
    });

    let mut style = (*ctx.style()).clone();
    let mut v = if p.is_dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    v.dark_mode = p.is_dark;
    v.override_text_color = Some(p.text);
    v.panel_fill = p.bg;
    v.window_fill = p.panel;
    v.window_stroke = Stroke::new(1.0_f32, p.border);
    v.extreme_bg_color = p.input;
    v.faint_bg_color = p.card;
    v.hyperlink_color = p.accent;
    v.selection.bg_fill = p.accent.gamma_multiply(0.9);
    v.slider_trailing_fill = true;
    v.selection.stroke = Stroke::new(1.0_f32, p.on_accent);
    v.window_rounding = Rounding::same(12.0);
    v.menu_rounding = Rounding::same(9.0);
    let shadow = egui::epaint::Shadow {
        offset: Vec2::new(0.0, 8.0),
        blur: 22.0,
        spread: 0.0,
        color: Color32::from_black_alpha(if p.is_dark { 110 } else { 45 }),
    };
    v.popup_shadow = shadow;
    v.window_shadow = shadow;

    let widget = |bg: Color32, stroke_color: Color32, fg: Color32, rounding: f32| {
        egui::style::WidgetVisuals {
            bg_fill: bg,
            weak_bg_fill: bg,
            bg_stroke: Stroke::new(1.0_f32, stroke_color),
            rounding: Rounding::same(rounding),
            fg_stroke: Stroke::new(1.0_f32, fg),
            expansion: 0.0,
        }
    };
    v.widgets.noninteractive = widget(p.panel, p.border_soft, p.text_dim, 9.0);
    // Inputs and slider rails use the darker input colour so they stay visible
    // when they sit on a card of the same tone as the default widget fill.
    v.widgets.inactive = widget(p.input, p.border, p.text, 8.0);
    v.widgets.hovered = widget(p.card_hover, p.accent.gamma_multiply(0.8), p.text, 8.0);
    v.widgets.active = widget(p.accent.gamma_multiply(0.9), p.accent, p.on_accent, 8.0);
    v.widgets.open = widget(p.card_hover, p.border, p.text, 8.0);

    style.visuals = v;
    style.spacing.item_spacing = Vec2::new(8.0, 7.0);
    style.spacing.button_padding = Vec2::new(10.0, 5.0);
    style.spacing.interact_size = Vec2::new(24.0, 26.0);
    style.spacing.slider_width = 150.0;
    style.spacing.combo_width = 120.0;
    style.spacing.indent = 16.0;
    style.spacing.scroll = egui::style::ScrollStyle::solid();
    style.text_styles = [
        (TextStyle::Heading, FontId::new(22.0, FontFamily::Proportional)),
        (TextStyle::Body, FontId::new(13.5, FontFamily::Proportional)),
        (TextStyle::Button, FontId::new(13.5, FontFamily::Proportional)),
        (TextStyle::Small, FontId::new(11.5, FontFamily::Proportional)),
        (TextStyle::Monospace, FontId::new(12.5, FontFamily::Monospace)),
    ]
    .into();
    ctx.set_style(style);
}

// ---------------------------------------------------------------------------
// primitives
// ---------------------------------------------------------------------------

pub fn card<R>(ui: &mut Ui, p: Palette, add: impl FnOnce(&mut Ui) -> R) -> R {
    egui::Frame::none()
        .fill(p.card)
        .stroke(Stroke::new(1.0_f32, p.border_soft))
        .rounding(Rounding::same(12.0))
        .inner_margin(egui::Margin::symmetric(14.0, 11.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

pub fn section_header(ui: &mut Ui, p: Palette, title: &str, icon: Icon) {
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(15.0), Sense::hover());
        icons::paint(ui.painter(), rect.shrink(1.0), icon, p.accent);
        ui.add_space(2.0);
        ui.label(
            RichText::new(title)
                .size(12.5)
                .strong()
                .color(p.text_dim),
        );
    });
    ui.add_space(1.0);
}

/// A settings row: fixed-width dim label, then caller-provided controls.
pub fn row(ui: &mut Ui, p: Palette, label: &str, add: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(
            Vec2::new(84.0, 26.0),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.label(RichText::new(label).size(12.5).color(p.text_dim));
            },
        );
        add(ui);
    });
}

pub fn caption(ui: &mut Ui, p: Palette, text: &str) {
    ui.label(RichText::new(text).size(11.0).color(p.text_faint));
}

pub fn divider(ui: &mut Ui, p: Palette) {
    let (rect, _) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), 1.0),
        Sense::hover(),
    );
    ui.painter().rect_filled(rect, 0.0, p.border_soft);
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ButtonKind {
    Primary,
    Secondary,
    Ghost,
    Success,
}

/// Hand-painted button with real hover/press states (egui's `Button::fill` is
/// static, which makes flat primary buttons feel dead).
pub fn button(ui: &mut Ui, p: Palette, label: &str, kind: ButtonKind, size: Vec2) -> Response {
    button_icon(ui, p, label, None, kind, size)
}

pub fn button_icon(
    ui: &mut Ui,
    p: Palette,
    label: &str,
    icon: Option<Icon>,
    kind: ButtonKind,
    size: Vec2,
) -> Response {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let hovered = resp.hovered();
    let pressed = resp.is_pointer_button_down_on();

    let (base, hover, fg, border) = match kind {
        ButtonKind::Primary => (p.accent, p.accent_hover, p.on_accent, Color32::TRANSPARENT),
        ButtonKind::Success => (
            p.success.gamma_multiply(0.92),
            p.success,
            Color32::from_rgb(6, 24, 14),
            Color32::TRANSPARENT,
        ),

        ButtonKind::Secondary => (p.card_hover, p.border.gamma_multiply(1.15), p.text, p.border),
        ButtonKind::Ghost => (Color32::TRANSPARENT, p.card_hover, p.text_dim, Color32::TRANSPARENT),
    };
    let fill = if pressed {
        base.gamma_multiply(0.85)
    } else if hovered {
        hover
    } else {
        base
    };

    let painter = ui.painter();
    if fill != Color32::TRANSPARENT {
        painter.rect_filled(rect, Rounding::same(9.0), fill);
    }
    if border != Color32::TRANSPARENT {
        painter.rect_stroke(rect, Rounding::same(9.0), Stroke::new(1.0_f32, border));
    }

    let font = FontId::new(13.5, FontFamily::Proportional);
    let galley = painter.layout_no_wrap(label.to_owned(), font, fg);
    let icon_size = 16.0;
    let gap = if icon.is_some() { 7.0 } else { 0.0 };
    let total = galley.size().x + if icon.is_some() { icon_size + gap } else { 0.0 };
    let start = rect.center() - Vec2::new(total * 0.5, galley.size().y * 0.5);
    if let Some(icon) = icon {
        let icon_rect = egui::Rect::from_center_size(
            egui::Pos2::new(start.x + icon_size * 0.5, rect.center().y),
            Vec2::splat(icon_size),
        );
        icons::paint(painter, icon_rect, icon, fg);
    }
    painter.galley(
        egui::Pos2::new(start.x + if icon.is_some() { icon_size + gap } else { 0.0 }, start.y),
        galley,
        fg,
    );

    if hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
}

/// Square icon-only button used by toolbars.
pub fn icon_button(
    ui: &mut Ui,
    p: Palette,
    icon: Icon,
    tooltip: &str,
    selected: bool,
    size: Vec2,
) -> Response {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let hovered = resp.hovered();
    let fill = if selected {
        p.accent
    } else if hovered {
        p.card_hover
    } else {
        Color32::TRANSPARENT
    };
    let fg = if selected { p.on_accent } else { p.text };
    let painter = ui.painter();
    if fill != Color32::TRANSPARENT {
        painter.rect_filled(rect, Rounding::same(8.0), fill);
    }
    let inner = if size.x < 30.0 { 15.0 } else { 18.0 };
    icons::paint(
        painter,
        egui::Rect::from_center_size(rect.center(), Vec2::splat(inner)),
        icon,
        fg,
    );
    if hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let resp = resp.on_hover_text(tooltip);
    resp
}

/// Two-option segmented control.
pub fn segmented(ui: &mut Ui, p: Palette, left: (&str, bool), right: (&str, bool)) -> Option<bool> {
    let width = 92.0;
    let size = Vec2::new(width * 2.0 + 4.0, 26.0);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().rect_filled(rect, Rounding::same(8.0), p.input);
    let mut clicked = None;
    for (i, (label, selected)) in [left, right].into_iter().enumerate() {
        let seg = egui::Rect::from_min_size(
            egui::Pos2::new(rect.min.x + 2.0 + i as f32 * width, rect.min.y + 2.0),
            Vec2::new(width, size.y - 4.0),
        );
        let resp = ui.interact(seg, ui.id().with(("seg", i)), Sense::click());
        if selected {
            ui.painter()
                .rect_filled(seg, Rounding::same(6.0), p.accent.gamma_multiply(0.32));
            ui.painter()
                .rect_stroke(seg, Rounding::same(6.0), Stroke::new(1.0_f32, p.accent.gamma_multiply(0.55)));
        } else if resp.hovered() {
            ui.painter()
                .rect_filled(seg, Rounding::same(6.0), p.card_hover);
        }
        ui.painter().text(
            seg.center(),
            Align2::CENTER_CENTER,
            label,
            FontId::new(12.5, FontFamily::Proportional),
            if selected { p.text } else { p.text_dim },
        );
        if resp.clicked() {
            clicked = Some(i == 1);
        }
        if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
    }
    clicked
}

/// iOS-style toggle switch.
pub fn toggle(ui: &mut Ui, p: Palette, on: &mut bool) -> Response {
    let size = Vec2::new(40.0, 22.0);
    let (rect, mut resp) = ui.allocate_exact_size(size, Sense::click());
    let enabled = ui.is_enabled();
    if resp.clicked() && enabled {
        *on = !*on;
        resp.mark_changed();
    }
    let t = if *on { 1.0 } else { 0.0 };
    let track = if !enabled {
        p.border_soft
    } else if *on {
        if resp.hovered() {
            p.accent_hover
        } else {
            p.accent
        }
    } else if resp.hovered() {
        p.card_hover
    } else {
        p.input
    };
    let painter = ui.painter();
    painter.rect_filled(rect, Rounding::same(rect.height() * 0.5), track);
    if !*on {
        painter.rect_stroke(
            rect,
            Rounding::same(rect.height() * 0.5),
            Stroke::new(1.0_f32, p.border),
        );
    }
    let knob_x = rect.left() + rect.height() * 0.5 + (rect.width() - rect.height()) * t;
    painter.circle_filled(
        egui::Pos2::new(knob_x, rect.center().y),
        rect.height() * 0.5 - 3.0,
        if !enabled {
            p.text_faint
        } else if *on {
            Color32::WHITE
        } else {
            p.text_dim
        },
    );
    if resp.hovered() && enabled {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
}

pub fn keycap(ui: &mut Ui, p: Palette, text: &str) {
    let font = FontId::new(11.0, FontFamily::Monospace);
    let galley = ui.painter().layout_no_wrap(text.to_owned(), font, p.text_dim);
    let size = Vec2::new(galley.size().x + 12.0, galley.size().y + 6.0);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, Rounding::same(5.0), p.input);
    painter.rect_stroke(rect, Rounding::same(5.0), Stroke::new(1.0_f32, p.border));
    painter.galley(
        egui::Pos2::new(rect.left() + 6.0, rect.center().y - galley.size().y * 0.5),
        galley,
        p.text_dim,
    );
}
