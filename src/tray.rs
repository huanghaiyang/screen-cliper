//! System tray icon and menu.
//!
//! The tray is optional: Windows without an Explorer shell (headless sessions,
//! locked-down images) cannot host a notification-area icon, so failures here
//! degrade to window-only mode instead of aborting the app.

use crate::icon;
use tray_icon::menu::{Menu, MenuItem, PredefinedMenuItem};
use tray_icon::{TrayIcon, TrayIconBuilder};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayCommand {
    Capture,
    CaptureFull,
    ShowWindow,
    OpenSaveDir,
    Quit,
}

pub struct Tray {
    _icon: TrayIcon,
}

pub fn create() -> Result<Tray, String> {
    let (rgba, w, h) = icon::icon_rgba(32);
    let icon = tray_icon::Icon::from_rgba(rgba, w, h).map_err(|e| format!("托盘图标无效：{e}"))?;

    let menu = Menu::new();
    let add = |item: &dyn tray_icon::menu::IsMenuItem| -> Result<(), String> {
        menu.append(item).map_err(|e| format!("托盘菜单构建失败：{e}"))
    };
    add(&MenuItem::with_id(
        "capture",
        "区域截图",
        true,
        None,
    ))?;
    add(&MenuItem::with_id(
        "capture_full",
        "全屏截图",
        true,
        None,
    ))?;
    add(&PredefinedMenuItem::separator())?;
    add(&MenuItem::with_id(
        "show",
        "显示主窗口",
        true,
        None,
    ))?;
    add(&MenuItem::with_id(
        "save_dir",
        "打开保存目录",
        true,
        None,
    ))?;
    add(&PredefinedMenuItem::separator())?;
    add(&MenuItem::with_id(
        "quit",
        "退出 ScreenCliper",
        true,
        None,
    ))?;

    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip("ScreenCliper · 截图标注工具")
        .with_icon(icon)
        .build()
        .map_err(|e| format!("创建托盘图标失败：{e}"))?;

    Ok(Tray { _icon: tray })
}

/// Drain pending tray events (menu selections and left clicks).
pub fn poll_events() -> Vec<TrayCommand> {
    let mut out = Vec::new();
    while let Ok(event) = tray_icon::menu::MenuEvent::receiver().try_recv() {
        let id = event.id.0.as_str();
        let cmd = match id {
            "capture" => Some(TrayCommand::Capture),
            "capture_full" => Some(TrayCommand::CaptureFull),
            "show" => Some(TrayCommand::ShowWindow),
            "save_dir" => Some(TrayCommand::OpenSaveDir),
            "quit" => Some(TrayCommand::Quit),
            _ => None,
        };
        if let Some(cmd) = cmd {
            out.push(cmd);
        }
    }
    while let Ok(event) = tray_icon::TrayIconEvent::receiver().try_recv() {
        if let tray_icon::TrayIconEvent::Click {
            button: tray_icon::MouseButton::Left,
            button_state: tray_icon::MouseButtonState::Up,
            ..
        } = event
        {
            out.push(TrayCommand::Capture);
        }
    }
    out
}
