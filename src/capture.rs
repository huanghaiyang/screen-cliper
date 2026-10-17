//! Screen capture. Captures the monitor under the mouse cursor (the monitor the
//! user is looking at), together with its geometry so the overlay can map mouse
//! positions back to captured pixels exactly.
//!
//! Two backends are used:
//!
//! * **Windows Graphics Capture** (via `xcap`) — HDR-aware: the OS tone-maps an
//!   HDR/advanced-colour desktop down to the SDR frame we ask for, and it is not
//!   confused by GPU hardware overlays.
//! * **GDI `BitBlt`** (our own FFI) — a fallback for restricted processes where
//!   WGC is refused (`E_ACCESSDENIED`) or very old systems. It can come back
//!   black on HDR desktops, so its output is validated too.
//!
//! A grab that is essentially black or perfectly flat is treated as a failed
//! capture rather than shown to the user: before this check, an HDR/overlay grab
//! failure surfaced as a completely black screenshot with no explanation.

use image::RgbaImage;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// Some processes (restricted jobs, some remote sessions) are never allowed to use
/// Windows Graphics Capture. Remember that so later captures skip straight to the
/// fallback instead of paying for two doomed WGC attempts every time.
static WGC_BLOCKED: AtomicBool = AtomicBool::new(false);

/// `E_ACCESSDENIED` — the one WGC failure that retrying cannot fix.
const E_ACCESSDENIED: &str = "0x80070005";

#[derive(Debug, Clone)]
pub struct Capture {
    pub image: RgbaImage,
    /// Monitor top-left in virtual-screen physical pixels.
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
    /// Monitor DPI scale (1.0 = 96 DPI).
    pub scale: f32,
    pub name: String,
    /// Which backend produced this frame: "wgc" or "gdi".
    pub backend: &'static str,
}

#[derive(Debug, Clone)]
struct MonitorInfo {
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    scale: f32,
    primary: bool,
    name: String,
}

impl MonitorInfo {
    fn read(m: &xcap::Monitor) -> Result<Self, String> {
        let err = |what: &str, e: xcap::XCapError| format!("读取显示器{what}失败：{e}");
        Ok(MonitorInfo {
            x: m.x().map_err(|e| err("坐标", e))?,
            y: m.y().map_err(|e| err("坐标", e))?,
            w: m.width().map_err(|e| err("宽度", e))?,
            h: m.height().map_err(|e| err("高度", e))?,
            scale: m.scale_factor().map_err(|e| err("缩放比例", e))?,
            primary: m.is_primary().map_err(|e| err("主屏标记", e))?,
            name: m.name().unwrap_or_else(|_| "显示器".to_string()),
        })
    }

    fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x && x < self.x + self.w as i32 && y >= self.y && y < self.y + self.h as i32
    }
}

/// `(mean, min, max)` luminance over a coarse sample grid — enough to recognise a
/// black or flat frame without touching every pixel of a 4K image.
pub fn luminance(image: &RgbaImage) -> (f32, f32, f32) {
    let step_x = (image.width() / 64).max(1) as usize;
    let step_y = (image.height() / 64).max(1) as usize;
    let mut sum = 0.0f64;
    let mut count = 0u64;
    let mut lo = 255u8;
    let mut hi = 0u8;
    for y in (0..image.height()).step_by(step_y) {
        for x in (0..image.width()).step_by(step_x) {
            let px = image.get_pixel(x, y).0;
            let l = ((px[0] as u32 * 299 + px[1] as u32 * 587 + px[2] as u32 * 114) / 1000) as u8;
            sum += l as f64;
            count += 1;
            lo = lo.min(l);
            hi = hi.max(l);
        }
    }
    let mean = if count == 0 {
        0.0
    } else {
        (sum / count as f64) as f32
    };
    (mean, lo as f32, hi as f32)
}

/// A real desktop almost always has bright text somewhere; a frame that is both
/// flat and dark means the backend handed us nothing.
fn looks_broken(image: &RgbaImage) -> bool {
    let (mean, lo, hi) = luminance(image);
    (hi - lo) < 2.0 && mean < 4.0
}

fn capture_from(image: RgbaImage, info: &MonitorInfo, backend: &'static str) -> Capture {
    Capture {
        x: info.x,
        y: info.y,
        w: image.width(),
        h: image.height(),
        image,
        scale: if info.scale > 0.1 { info.scale } else { 1.0 },
        name: info.name.clone(),
        backend,
    }
}

/// Grab a monitor: Windows Graphics Capture first (HDR-correct), GDI as fallback.
fn grab(monitor: &xcap::Monitor, info: &MonitorInfo) -> Result<Capture, String> {
    let mut wgc_error: Option<String> = None;

    // WGC occasionally yields one blank frame while the capture item warms up,
    // so a blank result is retried before giving up on the backend.
    let skip_wgc = WGC_BLOCKED.load(Ordering::Relaxed);
    for attempt in 1..=2 {
        if skip_wgc {
            break;
        }
        match monitor.capture_image() {
            Ok(image) if !looks_broken(&image) => {
                let (mean, _, _) = luminance(&image);
                crate::applog::log(format!(
                    "抓屏：Windows Graphics Capture 成功（{}x{}，平均亮度 {:.0}）",
                    image.width(),
                    image.height(),
                    mean
                ));
                return Ok(capture_from(image, info, "wgc"));
            }
            Ok(image) => {
                let (mean, lo, hi) = luminance(&image);
                wgc_error = Some(format!("WGC 返回全黑画面（亮度 {mean:.0}，范围 {lo:.0}..{hi:.0}）"));
                crate::applog::log(format!(
                    "抓屏：WGC 第 {attempt} 次返回全黑画面（平均亮度 {mean:.0}，范围 {lo:.0}..{hi:.0}）"
                ));
            }
            Err(err) => {
                let text = err.to_string();
                let denied = text.contains(E_ACCESSDENIED);
                wgc_error = Some(format!("WGC 失败：{text}"));
                crate::applog::log(format!(
                    "抓屏：WGC 第 {attempt} 次失败：{text}{}",
                    if denied {
                        "（当前进程不允许使用 WGC，后续直接走 GDI）"
                    } else {
                        ""
                    }
                ));
                if denied {
                    WGC_BLOCKED.store(true, Ordering::Relaxed);
                    break;
                }
            }
        }
        std::thread::sleep(Duration::from_millis(90));
    }

    // Fallback: GDI BitBlt.
    match crate::win::capture_screen_gdi(info.x, info.y, info.w as i32, info.h as i32) {
        Ok(bitmap) => {
            let image = RgbaImage::from_raw(bitmap.width, bitmap.height, bitmap.rgba)
                .ok_or_else(|| "GDI 位图转换失败".to_string())?;
            if looks_broken(&image) {
                let (mean, lo, hi) = luminance(&image);
                crate::applog::log(format!(
                    "抓屏：GDI 备用方式同样全黑（平均亮度 {mean:.0}，范围 {lo:.0}..{hi:.0}）"
                ));
                Err(format!(
                    "抓屏得到全黑画面。已尝试 Windows Graphics Capture 与 GDI 两种方式；\
                     这通常发生在开启 HDR 的显示器或受保护内容上。原始错误：{}",
                    wgc_error.unwrap_or_else(|| "无".to_string())
                ))
            } else {
                let (mean, _, _) = luminance(&image);
                crate::applog::log(format!(
                    "抓屏：改用 GDI 备用方式成功（{}x{}，平均亮度 {:.0}）",
                    image.width(),
                    image.height(),
                    mean
                ));
                Ok(capture_from(image, info, "gdi"))
            }
        }
        Err(err) => Err(format!(
            "抓屏失败：{}；GDI 备用方式也失败：{err}",
            wgc_error.unwrap_or_else(|| "未知原因".to_string())
        )),
    }
}

/// Capture the monitor containing (x, y) in virtual-screen coordinates.
pub fn capture_monitor_at(x: i32, y: i32) -> Result<Capture, String> {
    let monitors = xcap::Monitor::all().map_err(|e| format!("枚举显示器失败：{e}"))?;
    if monitors.is_empty() {
        return Err("未检测到任何显示器".into());
    }
    let mut entries = Vec::with_capacity(monitors.len());
    for m in &monitors {
        entries.push((m, MonitorInfo::read(m)?));
    }
    let index = entries
        .iter()
        .position(|(_, info)| info.contains(x, y))
        .or_else(|| entries.iter().position(|(_, info)| info.primary))
        .unwrap_or(0);
    let (monitor, info) = &entries[index];
    grab(monitor, info)
}

pub fn capture_at_cursor() -> Result<Capture, String> {
    let (cx, cy) = crate::win::cursor_position();
    capture_monitor_at(cx, cy)
}

/// List monitors as (name, x, y, w, h, scale, primary) for diagnostics.
pub fn list_monitors() -> Vec<(String, i32, i32, u32, u32, f32, bool)> {
    match xcap::Monitor::all() {
        Ok(monitors) => monitors
            .iter()
            .filter_map(|m| {
                MonitorInfo::read(m)
                    .ok()
                    .map(|i| (i.name, i.x, i.y, i.w, i.h, i.scale, i.primary))
            })
            .collect(),
        Err(_) => Vec::new(),
    }
}
