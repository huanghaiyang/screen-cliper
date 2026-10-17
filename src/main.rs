//! ScreenCliper — a snip & annotate screenshot tool written in Rust.
//!
//! Process model
//! -------------
//! * `screen-cliper`                  → control window (tray, hotkey, settings)
//! * `screen-cliper --overlay ...`    → short-lived full-screen capture overlay
//! * `screen-cliper --capture <out>`  → capture straight to a PNG (diagnostics)
//! * `screen-cliper --selftest <dir>` → offscreen rasterizer self-test
//!
//! The overlay runs in its own process so the control window can be hidden
//! *before* the frame is grabbed and can never end up inside the screenshot.

mod applog;
mod autostart;
mod capture;
mod config;
mod control;
mod design;
mod fonts;
mod hotkey;
mod icons;
mod icon;
mod overlay;
mod raster;
mod selftest;
mod shapes;
mod tray;
mod win;

use config::{Config, APP_NAME, APP_VERSION};
use std::path::PathBuf;
use std::sync::Arc;

fn main() {
    win::init_dpi();
    let args: Vec<String> = std::env::args().collect();
    let mode = args.get(1).map(|s| s.as_str()).unwrap_or("");

    match mode {
        "--overlay" => run_overlay(&args),
        "--capture" => run_capture_cli(&args),
        "--selftest" => run_selftest_cli(&args),
        "--autostart" => run_autostart_cli(&args),
        "--clipboard" => run_clipboard_cli(&args),
        "--synth" => run_synth_cli(&args),
        "--version" | "-V" => println!("{APP_NAME} {APP_VERSION}"),
        "--help" | "-h" => print_help(),
        _ => run_control(&args),
    }
}

fn print_help() {
    println!("{APP_NAME} {APP_VERSION} — 截图标注工具");
    println!();
    println!("用法：");
    println!("  screen-cliper                     启动主窗口（托盘 + 全局快捷键）");
    println!("  screen-cliper --minimized         启动后隐藏主窗口（供开机自启动使用）");
    println!("  screen-cliper --capture <out.png> 直接截取当前显示器到文件");
    println!("  screen-cliper --selftest <dir>    离屏渲染自检，输出标注示例图");
    println!("  screen-cliper --autostart <on|off|status>  开机自启动开关");
    println!("  screen-cliper --clipboard <png>   把图片写入剪贴板（诊断用）");
    println!("  screen-cliper --synth <out.png>   生成合成桌面图（用于文档截图）");
    println!("  screen-cliper --overlay <png> --monitor x,y,w,h --scale s [--full] [--status file]");
    println!("                                    内部使用：全屏框选标注覆盖层");
}

// ---------------------------------------------------------------------------
// argument helpers
// ---------------------------------------------------------------------------

fn arg_value(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn has_flag(args: &[String], name: &str) -> bool {
    args.iter().any(|a| a == name)
}

fn icon_data() -> egui::IconData {
    let (rgba, width, height) = icon::icon_rgba(64);
    egui::IconData {
        rgba,
        width,
        height,
    }
}

// ---------------------------------------------------------------------------
// modes
// ---------------------------------------------------------------------------

fn run_control(args: &[String]) {
    if !win::acquire_single_instance("ScreenCliperMutexV1") {
        eprintln!("{APP_NAME} 已在运行（同一时间只允许一个实例）。");
        std::process::exit(0);
    }

    let config = Config::load();
    let start_hidden = has_flag(args, "--minimized") || config.start_minimized;

    let viewport = egui::ViewportBuilder::default()
        .with_title(format!("{APP_NAME} · 截图标注工具"))
        .with_inner_size([620.0, 900.0])
        .with_min_inner_size([500.0, 440.0])
        .with_visible(!start_hidden)
        .with_icon(Arc::new(icon_data()))
        .with_app_id("ScreenCliper");

    let native_options = eframe::NativeOptions {
        viewport,
        vsync: true,
        centered: true,
        ..Default::default()
    };

    let result = eframe::run_native(
        APP_NAME,
        native_options,
        Box::new(move |cc| Ok(Box::new(control::ControlApp::new(cc, config, start_hidden)))),
    );
    if let Err(err) = result {
        eprintln!("主窗口启动失败：{err}");
        std::process::exit(1);
    }
}


fn run_overlay(args: &[String]) {
    let Some(image_path) = args.get(2).map(PathBuf::from) else {
        eprintln!("--overlay 需要一个截图文件路径");
        std::process::exit(2);
    };
    let monitor = arg_value(args, "--monitor")
        .and_then(|s| {
            let parts: Vec<i32> = s
                .split(',')
                .filter_map(|p| p.trim().parse::<i32>().ok())
                .collect();
            if parts.len() == 4 {
                Some([parts[0], parts[1], parts[2], parts[3]])
            } else {
                None
            }
        })
        .unwrap_or([0, 0, 0, 0]);
    let scale = arg_value(args, "--scale")
        .and_then(|s| s.parse::<f32>().ok())
        .filter(|s| *s > 0.1)
        .unwrap_or(1.0);
    let full = has_flag(args, "--full");
    let status_path = arg_value(args, "--status").map(PathBuf::from);
    let demo = has_flag(args, "--demo");

    let options = overlay::OverlayOptions {
        image_path,
        monitor,
        scale,
        full,
        status_path,
        demo,
    };

    let (mw, mh) = if monitor[2] > 0 && monitor[3] > 0 {
        (monitor[2] as f32, monitor[3] as f32)
    } else {
        let (w, h) = win::virtual_screen_size();
        (w as f32, h as f32)
    };

    let viewport = egui::ViewportBuilder::default()
        .with_title(APP_NAME)
        .with_decorations(false)
        .with_always_on_top()
        .with_resizable(false)
        .with_taskbar(false)
        .with_position([monitor[0] as f32 / scale, monitor[1] as f32 / scale])
        .with_inner_size([mw / scale, mh / scale])
        .with_fullscreen(true)
        .with_icon(Arc::new(icon_data()));

    let native_options = eframe::NativeOptions {
        viewport,
        vsync: true,
        ..Default::default()
    };

    let result = eframe::run_native(
        APP_NAME,
        native_options,
        Box::new(move |cc| Ok(Box::new(overlay::OverlayApp::new(cc, options)))),
    );
    match result {
        Ok(()) => std::process::exit(0),
        Err(err) => {
            eprintln!("标注窗口启动失败：{err}");
            std::process::exit(1);
        }
    }
}

fn run_capture_cli(args: &[String]) {
    let out = args
        .get(2)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("capture.png"));
    match capture::capture_at_cursor() {
        Ok(cap) => {
            if let Some(parent) = out.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            match cap.image.save_with_format(&out, image::ImageFormat::Png) {
                Ok(()) => {
                    println!(
                        "已截取显示器 {}（{}×{} @{:.0}%，虚拟坐标 {},{}）→ {}",
                        cap.name,
                        cap.w,
                        cap.h,
                        cap.scale * 100.0,
                        cap.x,
                        cap.y,
                        out.display()
                    );
                }
                Err(err) => {
                    eprintln!("保存失败：{err}");
                    std::process::exit(1);
                }
            }
        }
        Err(err) => {
            eprintln!("截图失败：{err}");
            std::process::exit(1);
        }
    }
}

fn run_selftest_cli(args: &[String]) {
    let out = args
        .get(2)
        .map(PathBuf::from)
        .unwrap_or_else(|| config::temp_dir().join("selftest"));
    if let Err(err) = selftest::run(&out) {
        eprintln!("自检失败：{err}");
        std::process::exit(1);
    }
}

fn run_synth_cli(args: &[String]) {
    let Some(path) = args.get(2).map(PathBuf::from) else {
        eprintln!("--synth 需要一个输出路径");
        std::process::exit(2);
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let image = selftest::synth_desktop(3840, 2160);
    match image.save_with_format(&path, image::ImageFormat::Png) {
        Ok(()) => println!("已生成合成桌面：{}", path.display()),
        Err(err) => {
            eprintln!("保存失败：{err}");
            std::process::exit(1);
        }
    }
}
fn run_clipboard_cli(args: &[String]) {
    let Some(path) = args.get(2).map(PathBuf::from) else {
        eprintln!("--clipboard 需要一个图片路径");
        std::process::exit(2);
    };
    let image = match image::open(&path) {
        Ok(img) => img.to_rgba8(),
        Err(err) => {
            eprintln!("读取图片失败：{err}");
            std::process::exit(1);
        }
    };
    match overlay::copy_to_clipboard(&image) {
        Ok(()) => println!(
            "已把 {}（{}×{}）写入剪贴板",
            path.display(),
            image.width(),
            image.height()
        ),
        Err(err) => {
            eprintln!("写入剪贴板失败：{err}");
            std::process::exit(1);
        }
    }
}
fn run_autostart_cli(args: &[String]) {
    match args.get(2).map(|s| s.as_str()).unwrap_or("status") {
        "on" | "enable" => match autostart::set_enabled(true) {
            Ok(()) => println!("已开启开机自启动：{}", autostart::command_line()),
            Err(err) => {
                eprintln!("{err}");
                std::process::exit(1);
            }
        },
        "off" | "disable" => match autostart::set_enabled(false) {
            Ok(()) => println!("已关闭开机自启动"),
            Err(err) => {
                eprintln!("{err}");
                std::process::exit(1);
            }
        },
        _ => {
            println!("开机自启动：{}", if autostart::is_enabled() { "已开启" } else { "已关闭" });
            println!("注册表命令：{}", autostart::command_line());
        }
    }
}
