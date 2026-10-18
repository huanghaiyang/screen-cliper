//! The persistent control window: settings, global hotkey, tray menu and the
//! capture → overlay child-process orchestration.

use crate::autostart;
use crate::capture;
use crate::config::{self, AfterCapture, Config, ImageFormat, ThemeMode, APP_NAME, APP_VERSION};
use crate::design::{self, ButtonKind, Palette};
use crate::fonts;
use crate::hotkey;
use crate::icons::Icon;
use crate::overlay::OverlayStatus;
use crate::shapes::Tool;
use crate::tray::{self, Tray, TrayCommand};
use crate::win;
use eframe::egui;
use egui::{Color32, RichText, Sense, Vec2};
use global_hotkey::hotkey::HotKey;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
enum UiEvent {
    Hotkey,
    Tray(TrayCommand),
    /// Sent by the capture worker thread when the overlay process has exited.
    CaptureDone {
        outcome: Result<Option<OverlayStatus>, String>,
        was_visible: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Level {
    Info,
    Warn,
    Error,
}

pub struct ControlApp {
    config: Config,
    pal: Palette,
    app_icon: Option<egui::TextureHandle>,
    hotkey_manager: Option<GlobalHotKeyManager>,
    registered: Option<HotKey>,
    hotkey_error: Option<String>,
    tray: Option<Tray>,
    tray_error: Option<String>,
    tx: Sender<UiEvent>,
    rx: Receiver<UiEvent>,
    capture_busy: bool,
    window_visible: bool,
    status: String,
    level: Level,
    autostart_enabled: bool,
    allow_exit: bool,
    dirty: bool,
    font_desc: String,
    monitor_desc: String,
    save_dir_buf: String,
    hotkey_buf: String,
    last_save: Option<String>,
    /// Diagnostic hook: `SC_AUTO_SHOT_MS=1500` fires one capture automatically,
    /// which makes the hide → grab → overlay path testable without a mouse.
    auto_shot_at: Option<Instant>,
    /// Our own native window handle, resolved once at start-up. Used for
    /// hide/restore so visibility never depends on the UI loop ticking.
    main_hwnd: Option<isize>,
}

impl ControlApp {
    pub fn new(cc: &eframe::CreationContext<'_>, config: Config, start_hidden: bool) -> Self {
        fonts::install_egui_fonts(&cc.egui_ctx);
        let pal = design::for_dark(config.theme == ThemeMode::Dark);
        design::apply(&cc.egui_ctx, pal);

        let app_icon = {
            let (rgba, w, h) = crate::icon::icon_rgba(64);
            let image =
                egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &rgba);
            Some(
                cc.egui_ctx
                    .load_texture("app-icon", image, egui::TextureOptions::LINEAR),
            )
        };

        let (tx, rx): (Sender<UiEvent>, Receiver<UiEvent>) = channel();

        let (tray, tray_error) = match tray::create() {
            Ok(t) => (Some(t), None),
            Err(err) => (None, Some(err)),
        };

        crate::applog::log(format!(
            "启动：托盘={} 托盘错误={:?}",
            tray.is_some(),
            tray_error
        ));
        let font_desc = fonts::describe();
        let monitors = capture::list_monitors();
        let monitor_desc = if monitors.is_empty() {
            "未检测到显示器".to_string()
        } else {
            monitors
                .iter()
                .map(|(name, x, y, w, h, scale, primary)| {
                    format!(
                        "{}{} {}×{} @{:.0}% ({},{})",
                        if *primary { "[主] " } else { "" },
                        name,
                        w / (*scale as u32).max(1),
                        h / (*scale as u32).max(1),
                        scale * 100.0,
                        x,
                        y
                    )
                })
                .collect::<Vec<_>>()
                .join("   ")
        };

        let save_dir_buf = config.resolved_save_dir().display().to_string();
        let hotkey_buf = config.hotkey.clone();
        let autostart_enabled = autostart::is_enabled();

        let mut app = Self {
            config,
            pal,
            app_icon,
            hotkey_manager: None,
            registered: None,
            hotkey_error: None,
            tray,
            tray_error,
            tx: tx.clone(),
            rx,
            capture_busy: false,
            window_visible: !start_hidden,
            status: "就绪".to_string(),
            level: Level::Info,
            autostart_enabled,
            allow_exit: false,
            dirty: false,
            font_desc,
            monitor_desc,
            save_dir_buf,
            hotkey_buf,
            last_save: None,
            auto_shot_at: std::env::var("SC_AUTO_SHOT_MS")
                .ok()
                .and_then(|v| v.trim().parse::<u64>().ok())
                .map(|ms| Instant::now() + Duration::from_millis(ms)),
            main_hwnd: win::find_main_window(),
        };
        crate::applog::log(format!("窗口句柄：{:?}", app.main_hwnd));
        app.apply_hotkey();
        if app.tray.is_none() {
            app.set_status(
                "托盘图标不可用 · 将以窗口模式运行",
                Level::Warn,
            );
            // No tray means no way back, so a "start hidden" request is dropped.
            if start_hidden {
                app.window_visible = true;
                if let Some(h) = app.main_hwnd {
                    win::restore_and_focus(h);
                }
            }
        }
        crate::applog::log(format!(
            "启动：快捷键已注册={} 错误={:?}",
            app.registered.is_some(),
            app.hotkey_error
        ));
        app.spawn_event_thread(cc.egui_ctx.clone(), tx);
        app
    }

    /// One background thread drains the global-hotkey and tray event queues and
    /// wakes the UI. Keeping this off the render thread means shortcuts still work
    /// while the window is hidden.
    fn spawn_event_thread(&self, ctx: egui::Context, tx: Sender<UiEvent>) {
        std::thread::Builder::new()
            .name("screencliper-events".into())
            .spawn(move || loop {
                std::thread::sleep(Duration::from_millis(45));
                let mut fired = false;
                loop {
                    match GlobalHotKeyEvent::receiver().try_recv() {
                        Ok(event) => {
                            if event.state == HotKeyState::Pressed {
                                let _ = tx.send(UiEvent::Hotkey);
                                fired = true;
                            }
                        }
                        Err(_) => break,
                    }
                }
                for cmd in tray::poll_events() {
                    let _ = tx.send(UiEvent::Tray(cmd));
                    fired = true;
                }
                if fired {
                    ctx.request_repaint();
                }
            })
            .ok();
    }

    fn set_status(&mut self, message: impl Into<String>, level: Level) {
        self.status = message.into();
        self.level = level;
    }

    // -- hotkey -------------------------------------------------------------

    fn apply_hotkey(&mut self) {
        if let (Some(manager), Some(previous)) = (&self.hotkey_manager, self.registered.take()) {
            let _ = manager.unregister(previous);
        }
        self.hotkey_error = None;
        if !self.config.hotkey_enabled {
            self.set_status("全局快捷键已停用", Level::Warn);
            return;
        }
        let Some(spec) = hotkey::parse(&self.config.hotkey) else {
            self.hotkey_error = Some(format!("快捷键「{}」格式无效", self.config.hotkey));
            return;
        };
        if self.hotkey_manager.is_none() {
            match GlobalHotKeyManager::new() {
                Ok(manager) => self.hotkey_manager = Some(manager),
                Err(err) => {
                    self.hotkey_error = Some(format!("快捷键管理器初始化失败：{err}"));
                    return;
                }
            }
        }
        let manager = self.hotkey_manager.as_ref().unwrap();
        let hk = spec.to_hotkey();
        match manager.register(hk.clone()) {
            Ok(()) => {
                self.registered = Some(hk);
                self.set_status(
                    format!("就绪 · 按 {} 开始截图", hotkey::format(spec)),
                    Level::Info,
                );
            }
            Err(err) => {
                self.hotkey_error = Some(format!(
                    "注册快捷键 {} 失败：{err}（可能已被其他程序占用）",
                    hotkey::format(spec)
                ));
            }
        }
    }

    // -- capture orchestration ---------------------------------------------

    /// Hide our own window, then hand the whole grab → overlay → wait cycle to a
    /// worker thread. The UI loop cannot be trusted to keep ticking while its
    /// window is hidden, and the thread explicitly wakes it when it is done.
    fn request_capture(&mut self, full: bool, ctx: &egui::Context) {
        if self.capture_busy {
            return;
        }
        self.capture_busy = true;
        let was_visible = self.window_visible;
        let hwnd = self.main_hwnd;
        if self.window_visible {
            if let Some(h) = hwnd {
                win::set_window_visible(h, false);
                self.window_visible = false;
            }
        }
        // A console window (debug builds) would end up inside the frame.
        win::hide_console_window();
        self.set_status("正在截取屏幕…", Level::Info);

        let tx = self.tx.clone();
        let ui_ctx = ctx.clone();
        let spawned = std::thread::Builder::new()
            .name("screencliper-capture".into())
            .spawn(move || {
                // Give the compositor time to actually remove our window first.
                std::thread::sleep(Duration::from_millis(220));
                let outcome = capture_and_wait(full);
                // Restore our own window from this thread: hiding it can stop
                // the UI loop from ticking, so the UI thread may be asleep.
                if was_visible {
                    if let Some(h) = hwnd {
                        win::restore_and_focus(h);
                    }
                }
                let _ = tx.send(UiEvent::CaptureDone {
                    outcome,
                    was_visible,
                });
                ui_ctx.request_repaint();
            });
        if let Err(err) = spawned {
            self.capture_busy = false;
            self.set_status(format!("启动截图线程失败：{err}"), Level::Error);
        }
    }

    fn finish_capture(
        &mut self,
        outcome: Result<Option<OverlayStatus>, String>,
        was_visible: bool,
        ctx: &egui::Context,
    ) {
        self.capture_busy = false;
        match outcome {
            Ok(Some(status)) => {
                crate::applog::log(format!("标注结果：{}", status.message));
                if status.saved_path.is_some() {
                    self.last_save = status.saved_path.clone();
                }
                let level = if status.ok { Level::Info } else { Level::Error };
                self.set_status(status.message, level);
            }
            Ok(None) => self.set_status("已取消本次截图", Level::Info),
            Err(err) => {
                crate::applog::log(format!("截图失败：{err}"));
                self.set_status(err, Level::Error);
            }
        }
        win::show_console_window();
        if was_visible {
            self.window_visible = true;
        }
        ctx.request_repaint();
    }

    fn handle_tray(&mut self, cmd: TrayCommand, ctx: &egui::Context) {
        match cmd {
            TrayCommand::Capture => self.request_capture(false, ctx),
            TrayCommand::CaptureFull => self.request_capture(true, ctx),
            TrayCommand::ShowWindow => {
                if let Some(h) = self.main_hwnd {
                    win::restore_and_focus(h);
                }
                self.window_visible = true;
                ctx.request_repaint();
            }
            TrayCommand::OpenSaveDir => {
                let dir = self.config.resolved_save_dir();
                let _ = std::fs::create_dir_all(&dir);
                win::open_folder(&dir);
            }
            TrayCommand::Quit => {
                self.allow_exit = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }

    fn sync_autostart(&mut self) {
        let actual = autostart::is_enabled();
        if actual != self.config.autostart {
            match autostart::set_enabled(self.config.autostart) {
                Ok(()) => {
                    self.autostart_enabled = autostart::is_enabled();
                    if self.autostart_enabled == self.config.autostart {
                        self.set_status(
                            if self.autostart_enabled {
                                "已开启开机自动启动"
                            } else {
                                "已关闭开机自动启动"
                            },
                            Level::Info,
                        );
                    } else {
                        self.set_status("注册表写入未生效，请检查权限", Level::Warn);
                    }
                }
                Err(err) => {
                    self.config.autostart = actual;
                    self.set_status(err, Level::Error);
                }
            }
        } else {
            self.autostart_enabled = actual;
        }
        self.dirty = true;
    }

    fn level_color(&self) -> Color32 {
        match self.level {
            Level::Info => self.pal.success,
            Level::Warn => self.pal.warn,
            Level::Error => self.pal.danger,
        }
    }

    // -- UI: header / hero --------------------------------------------------

    fn header_ui(&mut self, ui: &mut egui::Ui) {
        let p = self.pal;
        ui.horizontal(|ui| {
            let icon_size = 40.0;
            let (rect, _) = ui.allocate_exact_size(Vec2::splat(icon_size), Sense::hover());
            if let Some(tex) = &self.app_icon {
                ui.painter().image(
                    tex.id(),
                    rect,
                    egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
            ui.add_space(4.0);
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(APP_NAME).size(19.0).strong().color(p.text));
                    ui.add_space(2.0);
                    ui.label(
                        RichText::new(format!("v{APP_VERSION}"))
                            .size(10.5)
                            .color(p.text_faint),
                    );
                });
                ui.label(
                    RichText::new("Windows 截屏 · 框选标注 · 贴花与马赛克")
                        .size(11.5)
                        .color(p.text_faint),
                );
            });

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if design::icon_button(
                    ui,
                    p,
                    Icon::Close,
                    "退出程序",
                    false,
                    Vec2::splat(30.0),
                )
                .clicked()
                {
                    self.allow_exit = true;
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
                if design::icon_button(
                    ui,
                    p,
                    Icon::Folder,
                    "打开保存目录",
                    false,
                    Vec2::splat(30.0),
                )
                .clicked()
                {
                    let dir = self.config.resolved_save_dir();
                    let _ = std::fs::create_dir_all(&dir);
                    win::open_folder(&dir);
                }
            });
        });
    }

    fn hero_ui(&mut self, ui: &mut egui::Ui) {
        let p = self.pal;
        design::card(ui, p, |ui| {
            ui.horizontal(|ui| {
                let busy = self.capture_busy;
                let capture_label = if busy { "正在截取…" } else { "区域截图" };
                if design::button_icon(
                    ui,
                    p,
                    capture_label,
                    Some(Icon::Camera),
                    ButtonKind::Primary,
                    Vec2::new(172.0, 46.0),
                )
                .on_hover_text("拖动鼠标框选一块区域（快捷键可自定义）")
                .clicked()
                {
                    self.request_capture(false, ui.ctx());
                }
                if design::button_icon(
                    ui,
                    p,
                    "全屏截图",
                    Some(Icon::Monitor),
                    ButtonKind::Secondary,
                    Vec2::new(136.0, 46.0),
                )
                .on_hover_text("截取整个屏幕，进入编辑器后仍可调整选区")
                .clicked()
                {
                    self.request_capture(true, ui.ctx());
                }

                ui.add_space(8.0);
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("快捷键")
                                .size(11.0)
                                .color(p.text_faint),
                        );
                        if self.registered.is_some() {
                            design::keycap(ui, p, &self.config.hotkey);
                        } else {
                            ui.label(
                                RichText::new("未启用")
                                    .size(11.0)
                                    .color(p.text_faint),
                            );
                        }
                    });
                    ui.add_space(2.0);
                    let color = self.level_color();
                    let text = self.status.clone();
                    ui.add(
                        egui::Label::new(RichText::new(text).size(12.0).color(color))
                            .truncate(),
                    )
                    .on_hover_text(self.status.clone());
                });
            });
        });
    }

    // -- UI: settings cards -------------------------------------------------

    fn hotkey_card(&mut self, ui: &mut egui::Ui) {
        let p = self.pal;
        design::row(ui, p, "预设", |ui| {
            let current = self.config.hotkey.clone();
            egui::ComboBox::from_id_salt("hotkey_combo")
                .selected_text(current.clone())
                .width(150.0)
                .show_ui(ui, |ui| {
                    for preset in hotkey::PRESETS {
                        if ui
                            .selectable_label(current == *preset, *preset)
                            .clicked()
                        {
                            self.config.hotkey = (*preset).to_string();
                            self.hotkey_buf = (*preset).to_string();
                            self.apply_hotkey();
                            self.dirty = true;
                        }
                    }
                });
            ui.label(
                RichText::new("点选即生效")
                    .size(11.0)
                    .color(p.text_faint),
            );
        });

        design::row(ui, p, "自定义", |ui| {
            let resp = ui.add(
                egui::TextEdit::singleline(&mut self.hotkey_buf)
                    .desired_width(150.0)
                    .hint_text("如 Ctrl+Shift+A"),
            );
            let submit = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            let changed = self.hotkey_buf.trim() != self.config.hotkey;
            let apply = design::button(
                ui,
                p,
                "应用",
                if changed { ButtonKind::Primary } else { ButtonKind::Ghost },
                Vec2::new(58.0, 26.0),
            );
            if (apply.clicked() || submit) && changed {
                let candidate = self.hotkey_buf.trim().to_string();
                if crate::hotkey::is_valid(&candidate) {
                    self.config.hotkey = candidate.clone();
                    self.hotkey_buf = candidate;
                    self.apply_hotkey();
                    self.dirty = true;
                } else {
                    self.hotkey_error = Some(format!("「{candidate}」不是有效的快捷键"));
                }
            }
            if let Some(err) = &self.hotkey_error {
                ui.add(
                    egui::Label::new(RichText::new(err).size(11.0).color(p.danger))
                        .truncate(),
                )
                .on_hover_text(err.clone());
            }
        });

        design::row(ui, p, "启用", |ui| {
            if design::toggle(ui, p, &mut self.config.hotkey_enabled).changed() {
                self.apply_hotkey();
                self.dirty = true;
            }
            ui.label(
                RichText::new("关闭后只能点按钮截图")
                    .size(11.0)
                    .color(p.text_faint),
            );
        });
    }

    fn output_card(&mut self, ui: &mut egui::Ui) {
        let p = self.pal;
        design::row(ui, p, "保存目录", |ui| {
            let avail = (ui.available_width() - 132.0).clamp(120.0, 420.0);
            let resp = ui.add(
                egui::TextEdit::singleline(&mut self.save_dir_buf).desired_width(avail),
            );
            if resp.lost_focus() {
                self.config.save_dir = self.save_dir_buf.trim().to_string();
                self.dirty = true;
            }
            if design::button(ui, p, "应用", ButtonKind::Secondary, Vec2::new(52.0, 26.0))
                .clicked()
            {
                self.config.save_dir = self.save_dir_buf.trim().to_string();
                self.dirty = true;
            }
            if design::button(ui, p, "打开", ButtonKind::Ghost, Vec2::new(52.0, 26.0))
                .clicked()
            {
                let dir = self.config.resolved_save_dir();
                let _ = std::fs::create_dir_all(&dir);
                win::open_folder(&dir);
            }
        });

        design::row(ui, p, "文件名", |ui| {
            let avail = (ui.available_width() - 96.0).clamp(120.0, 420.0);
            ui.add(
                egui::TextEdit::singleline(&mut self.config.filename_pattern)
                    .desired_width(avail),
            );
            if design::button(ui, p, "默认", ButtonKind::Ghost, Vec2::new(52.0, 26.0)).clicked()
            {
                self.config.filename_pattern = "ScreenCliper_{date}_{time}".to_string();
                self.dirty = true;
            }
        });
        design::caption(
            ui,
            p,
            "可用占位符：{date} {time} {yyyy} {MM} {dd} {HH} {mm} {ss}",
        );

        design::row(ui, p, "图片格式", |ui| {
            egui::ComboBox::from_id_salt("format_combo")
                .selected_text(self.config.image_format.label())
                .width(130.0)
                .show_ui(ui, |ui| {
                    for f in [ImageFormat::Png, ImageFormat::Jpeg] {
                        if ui
                            .selectable_value(&mut self.config.image_format, f, f.label())
                            .clicked()
                        {
                            self.dirty = true;
                        }
                    }
                });
            if self.config.image_format == ImageFormat::Jpeg {
                if ui
                    .add(
                        egui::Slider::new(&mut self.config.jpeg_quality, 40..=100)
                            .prefix("质量 ")
                            .max_decimals(0),
                    )
                    .changed()
                {
                    self.dirty = true;
                }
            }
        });

        design::row(ui, p, "完成后", |ui| {
            egui::ComboBox::from_id_salt("after_combo")
                .selected_text(self.config.after_capture.label())
                .width(150.0)
                .show_ui(ui, |ui| {
                    for mode in [
                        AfterCapture::Copy,
                        AfterCapture::Save,
                        AfterCapture::CopyAndSave,
                    ] {
                        if ui
                            .selectable_value(&mut self.config.after_capture, mode, mode.label())
                            .clicked()
                        {
                            self.dirty = true;
                        }
                    }
                });
        });
    }

    fn startup_card(&mut self, ui: &mut egui::Ui) {
        let p = self.pal;
        design::row(ui, p, "开机自启动", |ui| {
            if design::toggle(ui, p, &mut self.config.autostart).changed() {
                self.sync_autostart();
            }
            let (text, color) = if self.autostart_enabled {
                ("已写入注册表启动项", p.success)
            } else {
                ("未开启", p.text_faint)
            };
            ui.label(RichText::new(text).size(11.0).color(color));
        });

        design::row(ui, p, "启动方式", |ui| {
            if design::toggle(ui, p, &mut self.config.start_minimized).changed() {
                self.dirty = true;
            }
            ui.label(
                RichText::new("开机后静默到托盘")
                    .size(11.0)
                    .color(p.text_faint),
            );
        });

        design::row(ui, p, "关闭行为", |ui| {
            let tray_ok = self.tray.is_some();
            ui.add_enabled_ui(tray_ok, |ui| {
                if design::toggle(ui, p, &mut self.config.close_to_tray).changed() {
                    self.dirty = true;
                }
            });
            let hint = if tray_ok {
                "点关闭时最小化到托盘"
            } else {
                "无托盘时不可用（直接退出）"
            };
            ui.label(RichText::new(hint).size(11.0).color(p.text_faint));
        });

        if let Some(err) = &self.tray_error {
            ui.add(
                egui::Label::new(RichText::new("托盘不可用（悬停查看原因）").size(11.0).color(p.warn))
                    .truncate(),
            )
            .on_hover_text(err.clone());
        }
    }

    fn editor_card(&mut self, ui: &mut egui::Ui) {
        let p = self.pal;
        design::row(ui, p, "默认工具", |ui| {
            egui::ComboBox::from_id_salt("tool_combo")
                .selected_text(Tool::from_key(&self.config.default_tool).label())
                .width(110.0)
                .show_ui(ui, |ui| {
                    for tool in Tool::ALL {
                        if ui
                            .selectable_label(
                                self.config.default_tool == tool.key(),
                                tool.label(),
                            )
                            .clicked()
                        {
                            self.config.default_tool = tool.key().to_string();
                            self.dirty = true;
                        }
                    }
                });
        });

        design::row(ui, p, "默认颜色", |ui| {
            for c in crate::config::PALETTE {
                let selected = self.config.pen_color == c;
                let (rect, resp) = ui.allocate_exact_size(Vec2::splat(20.0), Sense::click());
                let color = Color32::from_rgba_unmultiplied(c[0], c[1], c[2], 255);
                ui.painter().circle_filled(rect.center(), 8.0, color);
                ui.painter().circle_stroke(
                    rect.center(),
                    8.0,
                    egui::Stroke::new(1.0_f32, p.border),
                );
                if selected {
                    ui.painter().circle_stroke(
                        rect.center(),
                        10.0,
                        egui::Stroke::new(2.0_f32, p.text),
                    );
                } else if resp.hovered() {
                    ui.painter().circle_stroke(
                        rect.center(),
                        10.0,
                        egui::Stroke::new(1.0_f32, p.text_faint),
                    );
                }
                if resp.clicked() {
                    self.config.pen_color = c;
                    self.dirty = true;
                }
                if resp.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
            }
        });

        for (label, value, range, suffix) in [
            ("画笔粗细", &mut self.config.pen_width, 1.0..=30.0, " px"),
            ("贴花大小", &mut self.config.sticker_size, 16.0..=300.0, " px"),
            ("文字字号", &mut self.config.font_size, 8.0..=120.0, " px"),
        ] {
            design::row(ui, p, label, |ui| {
                if ui
                    .add(
                        egui::Slider::new(value, range)
                            .suffix(suffix)
                            .max_decimals(0),
                    )
                    .changed()
                {
                    self.dirty = true;
                }
            });
        }

        design::row(ui, p, "马赛克", |ui| {
            if ui
                .add(
                    egui::Slider::new(&mut self.config.mosaic_block, 3..=60)
                        .max_decimals(0),
                )
                .changed()
            {
                self.dirty = true;
            }
        });

        design::row(ui, p, "选框辅助", |ui| {
            if design::toggle(ui, p, &mut self.config.magnifier).changed() {
                self.dirty = true;
            }
            ui.label(
                RichText::new("放大镜")
                    .size(11.0)
                    .color(p.text_faint),
            );
            ui.add_space(6.0);
            if design::toggle(ui, p, &mut self.config.show_size_label).changed() {
                self.dirty = true;
            }
            ui.label(
                RichText::new("尺寸标签")
                    .size(11.0)
                    .color(p.text_faint),
            );
        });

        design::row(ui, p, "界面主题", |ui| {
            let dark_now = self.config.theme == ThemeMode::Dark;
            if let Some(pick_dark) = design::segmented(ui, p, ("深色", dark_now), ("浅色", !dark_now))
            {
                self.config.theme = if pick_dark {
                    ThemeMode::Dark
                } else {
                    ThemeMode::Light
                };
                self.dirty = true;
            }
        });
    }

    fn about_card(&mut self, ui: &mut egui::Ui) {
        let p = self.pal;
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!("{APP_NAME} v{APP_VERSION}"))
                    .size(12.5)
                    .strong()
                    .color(p.text),
            );
            ui.label(
                RichText::new("纯 Rust 实现 · egui + xcap + 自研光栅化")
                    .size(11.0)
                    .color(p.text_faint),
            );
        });
        for (label, value) in [
            ("界面字体", self.font_desc.clone()),
            ("显示器", self.monitor_desc.clone()),
            ("配置目录", pretty_path(&config::home_dir())),
            ("日志文件", pretty_path(&crate::applog::log_file())),
        ] {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(label)
                        .size(11.0)
                        .color(p.text_faint),
                );
                ui.add(
                    egui::Label::new(RichText::new(value).size(11.0).color(p.text_dim))
                        .truncate(),
                );
            });
        }
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if design::button(
                ui,
                p,
                "打开配置目录",
                ButtonKind::Secondary,
                Vec2::new(112.0, 26.0),
            )
            .clicked()
            {
                let dir = config::home_dir();
                let _ = std::fs::create_dir_all(&dir);
                win::open_folder(&dir);
            }
            if design::button(ui, p, "打开日志", ButtonKind::Secondary, Vec2::new(92.0, 26.0))
                .clicked()
            {
                crate::applog::open_log();
            }
            if design::button(
                ui,
                p,
                "重新注册快捷键",
                ButtonKind::Ghost,
                Vec2::new(122.0, 26.0),
            )
            .clicked()
            {
                self.apply_hotkey();
            }
        });
    }

    fn footer_ui(&mut self, ui: &mut egui::Ui) {
        let p = self.pal;
        ui.horizontal(|ui| {
            if self.capture_busy {
                ui.spinner();
                ui.label(
                    RichText::new("正在截图…")
                        .size(11.0)
                        .color(p.text_faint),
                );
            } else if let Some(path) = &self.last_save {
                ui.label(
                    RichText::new("最近保存")
                        .size(11.0)
                        .color(p.text_faint),
                );
                ui.add(
                    egui::Label::new(
                        RichText::new(path).size(11.0).color(p.accent),
                    )
                    .truncate(),
                )
                .on_hover_text(path.clone());
            } else {
                ui.label(
                    RichText::new("提示：按快捷键后拖动鼠标框选，回车完成")
                        .size(11.0)
                        .color(p.text_faint),
                );
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    RichText::new(format!("{APP_NAME} {APP_VERSION}"))
                        .size(10.5)
                        .color(p.text_faint),
                );
            });
        });
    }
}

impl eframe::App for ControlApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Re-assert the configured theme if eframe re-applied the OS one.
        let want_dark = self.config.theme == ThemeMode::Dark;
        if self.pal.is_dark != want_dark || ctx.style().visuals.dark_mode != want_dark {
            self.pal = design::for_dark(want_dark);
            design::apply(ctx, self.pal);
        }

        // Events from the background polling thread.
        loop {
            match self.rx.try_recv() {
                Ok(UiEvent::Hotkey) => self.request_capture(false, ctx),
                Ok(UiEvent::Tray(cmd)) => self.handle_tray(cmd, ctx),
                Ok(UiEvent::CaptureDone {
                    outcome,
                    was_visible,
                }) => self.finish_capture(outcome, was_visible, ctx),
                Err(_) => break,
            }
        }

        if let Some(at) = self.auto_shot_at {
            if Instant::now() >= at {
                self.auto_shot_at = None;
                self.request_capture(self.config.fullscreen_capture, ctx);
            } else {
                ctx.request_repaint_after(Duration::from_millis(100));
            }
        }

        // Close-to-tray handling.
        if ctx.input(|i| i.viewport().close_requested()) && !self.allow_exit {
            if self.config.close_to_tray && self.tray.is_some() {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                if let Some(h) = self.main_hwnd {
                    win::set_window_visible(h, false);
                }
                self.window_visible = false;
                self.set_status("已最小化到托盘，可从托盘菜单恢复", Level::Info);
            }
        }

        let p = self.pal;
        let margin = egui::Margin::symmetric(18.0, 14.0);

        egui::TopBottomPanel::top("header")
            .frame(
                egui::Frame::none()
                    .fill(p.bg)
                    .inner_margin(egui::Margin {
                        left: 18.0,
                        right: 18.0,
                        top: 14.0,
                        bottom: 10.0,
                    }),
            )
            .show(ctx, |ui| {
                self.header_ui(ui);
                ui.add_space(12.0);
                self.hero_ui(ui);
            });

        egui::TopBottomPanel::bottom("footer")
            .frame(
                egui::Frame::none()
                    .fill(p.bg)
                    .inner_margin(egui::Margin {
                        left: 18.0,
                        right: 18.0,
                        top: 8.0,
                        bottom: 10.0,
                    }),
            )
            .show(ctx, |ui| {
                design::divider(ui, p);
                ui.add_space(6.0);
                self.footer_ui(ui);
            });

        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(p.bg).inner_margin(margin))
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        design::section_header(ui, p, "快捷键", Icon::Sliders);
                        design::card(ui, p, |ui| self.hotkey_card(ui));

                        design::section_header(ui, p, "保存与输出", Icon::Save);
                        design::card(ui, p, |ui| self.output_card(ui));

                        design::section_header(ui, p, "启动与托盘", Icon::Monitor);
                        design::card(ui, p, |ui| self.startup_card(ui));

                        design::section_header(ui, p, "标注默认值", Icon::Palette);
                        design::card(ui, p, |ui| self.editor_card(ui));

                        design::section_header(ui, p, "关于", Icon::Info);
                        design::card(ui, p, |ui| self.about_card(ui));

                        ui.add_space(10.0);
                    });
            });

        if self.dirty {
            self.dirty = false;
            if let Err(err) = self.config.save() {
                self.set_status(err, Level::Error);
            }
        }

        // Keep a slow heartbeat so hidden-window states still tick.
        ctx.request_repaint_after(Duration::from_millis(700));
    }
}

/// Fold a path under a well-known Windows folder into its environment variable
/// form (`%APPDATA%\ScreenCliper`). Keeps the UI readable and avoids printing
/// the user's profile path.
fn pretty_path(path: &std::path::Path) -> String {
    let text = path.display().to_string();
    let lower = text.to_lowercase();
    for (var, token) in [
        ("APPDATA", "%APPDATA%"),
        ("LOCALAPPDATA", "%LOCALAPPDATA%"),
        ("USERPROFILE", "%USERPROFILE%"),
    ] {
        let Ok(value) = std::env::var(var) else {
            continue;
        };
        if value.is_empty() {
            continue;
        }
        let needle = value.to_lowercase();
        if lower.starts_with(&needle) {
            let rest = &text[value.len()..];
            if rest.is_empty() || rest.starts_with('\\') || rest.starts_with('/') {
                return format!("{token}{rest}");
            }
        }
    }
    text
}
/// Runs on the capture worker thread: grab the screen, launch the overlay
/// process, and block until the user finishes or cancels.
fn capture_and_wait(full: bool) -> Result<Option<OverlayStatus>, String> {
    let cap = capture::capture_at_cursor()?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let dir = config::temp_dir();
    let image_path = dir.join(format!("capture-{stamp}.png"));
    cap.image
        .save_with_format(&image_path, image::ImageFormat::Png)
        .map_err(|e| format!("写入临时截图失败：{e}"))?;
    let status_path = dir.join(format!("status-{stamp}.json"));
    let _ = std::fs::remove_file(&status_path);
    let (mean, lo, hi) = capture::luminance(&cap.image);
    crate::applog::log(format!(
        "截图：显示器 {} {}x{} @{:.0}% 后端={} 亮度={:.0}({:.0}..{:.0}) → {}",
        cap.name,
        cap.w,
        cap.h,
        cap.scale * 100.0,
        cap.backend,
        mean,
        lo,
        hi,
        image_path.display()
    ));

    let exe = std::env::current_exe().map_err(|e| format!("无法定位自身程序：{e}"))?;
    let mut cmd = std::process::Command::new(&exe);
    cmd.arg("--overlay")
        .arg(&image_path)
        .arg("--monitor")
        .arg(format!("{},{},{},{}", cap.x, cap.y, cap.w, cap.h))
        .arg("--scale")
        .arg(format!("{:.4}", cap.scale))
        .arg("--status")
        .arg(&status_path);
    if full {
        cmd.arg("--full");
    }

    // Capture the child's stderr: the release build has no console, so a panic or
    // an eframe error inside the overlay would otherwise vanish without a trace.
    cmd.stdout(std::process::Stdio::null());
    cmd.stderr(std::process::Stdio::piped());
    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(err) => {
            let _ = std::fs::remove_file(&image_path);
            return Err(format!("启动标注窗口失败：{err}"));
        }
    };
    let mut child_stderr = String::new();
    if let Some(mut pipe) = child.stderr.take() {
        use std::io::Read;
        // Reading to EOF also blocks until the overlay exits.
        let _ = pipe.read_to_string(&mut child_stderr);
    }
    let _ = child.wait();
    if !child_stderr.trim().is_empty() {
        crate::applog::log(format!("标注窗口 stderr：{}", child_stderr.trim()));
    }

    let status = std::fs::read_to_string(&status_path)
        .ok()
        .and_then(|text| serde_json::from_str::<OverlayStatus>(&text).ok());
    if status.is_none() {
        if !child_stderr.trim().is_empty() {
            // A panic or an eframe error: surface it instead of pretending the
            // user cancelled, otherwise the capture just seems to vanish.
            let first = child_stderr
                .lines()
                .find(|l| !l.trim().is_empty())
                .unwrap_or("未知错误")
                .trim();
            crate::applog::log("标注窗口退出但未写入状态文件（异常退出）");
            return Err(format!("标注窗口异常退出：{first}"));
        }
        crate::applog::log("标注窗口退出但未写入状态文件（用户取消）");
    }
    let _ = std::fs::remove_file(&image_path);
    let _ = std::fs::remove_file(&status_path);
    Ok(status)
}
