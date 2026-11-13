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
//!   black on HDR desktops, so its output is validated too. It is tried twice:
//!   once with `CAPTUREBLT` (layered windows/cursor included) and once with a
//!   plain `SRCCOPY`, because some drivers answer only one of the two correctly.
//!
//! A grab that is essentially black is treated as a failed capture rather than
//! shown to the user: before this check, an HDR/overlay grab failure surfaced as
//! a completely black screenshot with no explanation.
//!
//! WGC refusals are *not* treated as permanent. Windows refuses capture while a
//! secure desktop (UAC prompt, lock screen) is up, while the session is being
//! switched, and for good in restricted/low-integrity processes. Remembering the
//! refusal for a short cooldown — instead of blacklisting WGC for the lifetime of
//! the process — keeps HDR-correct capture working for every later screenshot
//! after a one-off refusal.

use image::RgbaImage;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// How long to stay on the GDI fallback after Windows refused WGC. A refusal is
/// usually transient (secure desktop, session switch), so WGC is probed again
/// afterwards instead of being disabled for the rest of the process lifetime.
const WGC_DENIED_COOLDOWN: Duration = Duration::from_secs(15);
/// Engines where WGC does not exist at all: probing again cannot help, so the
/// cooldown is long (but still finite — the OS can be upgraded).
const WGC_UNSUPPORTED_COOLDOWN: Duration = Duration::from_secs(3600);

/// Instant (`now_ms() + cooldown`) at which WGC may be probed again; `0` = now.
static WGC_COOLDOWN_UNTIL: AtomicU64 = AtomicU64::new(0);

fn process_start() -> Instant {
    static START: OnceLock<Instant> = OnceLock::new();
    *START.get_or_init(Instant::now)
}

fn now_ms() -> u64 {
    process_start().elapsed().as_millis() as u64
}

fn wgc_cooldown_left() -> Duration {
    let until = WGC_COOLDOWN_UNTIL.load(Ordering::Relaxed);
    let now = now_ms();
    if until > now {
        Duration::from_millis(until - now)
    } else {
        Duration::ZERO
    }
}

fn wgc_cooldown_start(duration: Duration) {
    let until = now_ms().saturating_add(duration.as_millis() as u64);
    WGC_COOLDOWN_UNTIL.store(until, Ordering::Relaxed);
}

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

/// Luminance distribution of a frame, sampled on a coarse grid (fast even on 4K).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameStats {
    pub mean: f32,
    pub min: f32,
    pub max: f32,
    /// Median luminance: tells a genuinely dark desktop apart from a dead frame.
    pub median: f32,
    /// 99th percentile: if even this is black, nothing at all was captured.
    pub p99: f32,
}

impl FrameStats {
    pub fn describe(&self) -> String {
        format!(
            "平均亮度 {:.0}，中位 {:.0}，P99 {:.0}，范围 {:.0}..{:.0}",
            self.mean, self.median, self.p99, self.min, self.max
        )
    }
}

/// Cheap luminance histogram of a frame.
pub fn frame_stats(image: &RgbaImage) -> FrameStats {
    let (w, h) = (image.width(), image.height());
    if w == 0 || h == 0 {
        return FrameStats {
            mean: 0.0,
            min: 0.0,
            max: 0.0,
            median: 0.0,
            p99: 0.0,
        };
    }
    // At most ~128x128 samples: enough to recognise a dead frame, cheap on 4K.
    let step_x = (w / 128).max(1) as usize;
    let step_y = (h / 128).max(1) as usize;
    let mut samples: Vec<u8> = Vec::with_capacity(128 * 128);
    let mut sum = 0.0f64;
    let mut lo = 255u8;
    let mut hi = 0u8;
    let mut y = 0;
    while y < h {
        let mut x = 0;
        while x < w {
            let px = image.get_pixel(x, y).0;
            // Integer luma in the same weights the eye uses (Rec. 601).
            let l = ((px[0] as u32 * 299 + px[1] as u32 * 587 + px[2] as u32 * 114) / 1000) as u8;
            sum += l as f64;
            lo = lo.min(l);
            hi = hi.max(l);
            samples.push(l);
            x += step_x as u32;
        }
        y += step_y as u32;
    }
    samples.sort_unstable();
    let count = samples.len().max(1);
    let pick = |frac: f64| -> f32 {
        let idx = ((count - 1) as f64 * frac).round() as usize;
        samples[idx.min(count - 1)] as f32
    };
    FrameStats {
        mean: (sum / count as f64) as f32,
        min: lo as f32,
        max: hi as f32,
        median: pick(0.5),
        p99: pick(0.99),
    }
}

/// A frame that carries no usable picture.
///
/// The old check only rejected frames that were *both* perfectly flat and almost
/// black, so a black grab with a cursor or a taskbar sliver in it passed and the
/// user got a black screenshot. These rules look at the distribution instead.
pub fn looks_broken(stats: &FrameStats) -> bool {
    // Nothing anywhere in the frame is lit: the backend handed us an empty image.
    if stats.max <= 6.0 {
        return true;
    }
    // Almost everything is black; the only lit pixels are noise (a cursor, a
    // taskbar sliver). This is BitBlt's classic HDR / hardware-overlay failure.
    if stats.mean < 2.0 && stats.p99 <= 8.0 {
        return true;
    }
    // Perfectly flat *and* dark: no window, no wallpaper, nothing was composited.
    stats.max - stats.min < 2.0 && stats.mean < 8.0
}

/// `(mean, min, max)` luminance over a coarse sample grid — kept for the log and
/// the diagnostics that want the raw triple.
pub fn luminance(image: &RgbaImage) -> (f32, f32, f32) {
    let stats = frame_stats(image);
    (stats.mean, stats.min, stats.max)
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

/// Why WGC refused to hand over a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WgcFailure {
    /// The OS refused *this* process/session right now — may well work later.
    Denied,
    /// WGC does not exist on this system at all.
    Unsupported,
}

/// Classify a WGC error message. `xcap` forwards the raw Win32/WinRT message text
/// and HRESULT, so both the text and the codes are checked (the text is localised
/// on non-English Windows).
fn classify_wgc_error(text: &str) -> Option<WgcFailure> {
    let lower = text.to_lowercase();
    if lower.contains("0x80070005")
        || lower.contains("permission")
        || lower.contains("拒绝访问")
        || lower.contains("access is denied")
    {
        return Some(WgcFailure::Denied);
    }
    for code in [
        "0x80004001", // E_NOTIMPL
        "0x80040154", // REGDB_E_CLASSNOTREG
        "0x80040111", // CLASS_E_CLASSNOTAVAILABLE
        "0x80070032", // ERROR_NOT_SUPPORTED
        "0x80070490", // ERROR_NOT_FOUND
    ] {
        if lower.contains(code) {
            return Some(WgcFailure::Unsupported);
        }
    }
    if lower.contains("not supported") || lower.contains("不支持") || lower.contains("找不到") {
        return Some(WgcFailure::Unsupported);
    }
    None
}

fn sleep_ms(ms: u64) {
    std::thread::sleep(Duration::from_millis(ms));
}

/// Grab a monitor: Windows Graphics Capture first (HDR-correct), GDI as fallback.
fn grab(monitor: &xcap::Monitor, info: &MonitorInfo) -> Result<Capture, String> {
    // Let DWM finish the frame we are about to read; grabbing while the
    // compositor is still catching up (right after our own window was hidden)
    // is one of the ways a capture turns out black.
    crate::win::dwm_flush();

    let mut notes: Vec<String> = Vec::new();

    // ---- Windows Graphics Capture ----------------------------------------
    let cooldown = wgc_cooldown_left();
    if cooldown.is_zero() {
        // WGC occasionally yields one blank frame while the capture item warms
        // up, so a blank result is retried before giving up on the backend.
        for attempt in 1..=2u32 {
            match monitor.capture_image() {
                Ok(image) => {
                    let stats = frame_stats(&image);
                    if !looks_broken(&stats) {
                        crate::applog::log(format!(
                            "抓屏：Windows Graphics Capture 成功（{}x{}，{}）",
                            image.width(),
                            image.height(),
                            stats.describe()
                        ));
                        return Ok(capture_from(image, info, "wgc"));
                    }
                    crate::applog::log(format!(
                        "抓屏：WGC 第 {attempt} 次结果不可用（{}）",
                        stats.describe()
                    ));
                    notes.push(format!(
                        "WGC 第 {attempt} 次返回黑屏（{}）",
                        stats.describe()
                    ));
                }
                Err(err) => {
                    let text = err.to_string();
                    notes.push(format!("WGC 第 {attempt} 次失败：{text}"));
                    match classify_wgc_error(&text) {
                        Some(WgcFailure::Denied) => {
                            wgc_cooldown_start(WGC_DENIED_COOLDOWN);
                            crate::applog::log(format!(
                                "抓屏：WGC 第 {attempt} 次被系统拒绝（{text}）。本次改用 GDI；\
                                 {} 秒后自动重试 WGC（受限进程/安全桌面期间会被拒）",
                                WGC_DENIED_COOLDOWN.as_secs()
                            ));
                            break;
                        }
                        Some(WgcFailure::Unsupported) => {
                            wgc_cooldown_start(WGC_UNSUPPORTED_COOLDOWN);
                            crate::applog::log(format!(
                                "抓屏：本系统的 Windows Graphics Capture 不可用（{text}），\
                                 后续直接使用 GDI"
                            ));
                            break;
                        }
                        None => {
                            crate::applog::log(format!(
                                "抓屏：WGC 第 {attempt} 次失败：{text}"
                            ));
                        }
                    }
                }
            }
            sleep_ms(90);
        }
    } else {
        crate::applog::log(format!(
            "抓屏：WGC 冷却中（还剩 {:.0}s），本次直接使用 GDI",
            cooldown.as_secs_f32()
        ));
        notes.push(format!(
            "WGC 冷却中（还剩 {:.0}s，上次被系统拒绝）",
            cooldown.as_secs_f32()
        ));
    }

    // ---- GDI fallback ----------------------------------------------------
    // CAPTUREBLT first (includes layered windows and the cursor). Some display
    // drivers and HDR / hardware-overlay desktops answer that grab with an
    // all-black bitmap, so a plain SRCCOPY grab is tried afterwards.
    for (variant, layered) in [("CAPTUREBLT", true), ("SRCCOPY", false)] {
        match crate::win::capture_screen_gdi_opts(
            info.x,
            info.y,
            info.w as i32,
            info.h as i32,
            layered,
        ) {
            Ok(bitmap) => {
                let Some(image) = RgbaImage::from_raw(bitmap.width, bitmap.height, bitmap.rgba)
                else {
                    notes.push(format!("GDI（{variant}）位图转换失败"));
                    continue;
                };
                let stats = frame_stats(&image);
                if !looks_broken(&stats) {
                    crate::applog::log(format!(
                        "抓屏：改用 GDI 备用方式成功（{variant}，{}x{}，{}）",
                        image.width(),
                        image.height(),
                        stats.describe()
                    ));
                    return Ok(capture_from(image, info, "gdi"));
                }
                crate::applog::log(format!(
                    "抓屏：GDI（{variant}）同样全黑（{}）",
                    stats.describe()
                ));
                notes.push(format!("GDI（{variant}）返回黑屏（{}）", stats.describe()));
            }
            Err(err) => {
                crate::applog::log(format!("抓屏：GDI（{variant}）失败：{err}"));
                notes.push(format!("GDI（{variant}）失败：{err}"));
            }
        }
    }

    // ---- Everything failed ----------------------------------------------
    let access = crate::win::process_access();
    let advice = if access.capture_likely_restricted() {
        "当前进程以受限（低完整性）身份运行，Windows 会拒绝图形捕获接口，GDI 也拿不到画面。\
         请从桌面正常启动本程序（不要在受限/沙箱终端里运行），或让代理以普通权限直接启动它。"
    } else {
        "这通常发生在开启 HDR 的显示器、独占全屏内容或受保护的窗口上；\
         可以临时关闭 HDR（设置 → 系统 → 显示 → HDR）后再试。"
    };
    Err(format!(
        "抓屏得到全黑画面（已尝试 Windows Graphics Capture 与 GDI 两种方式）。{advice}\n尝试记录：\n  · {}",
        notes.join("\n  · ")
    ))
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

    match grab(monitor, info) {
        Ok(capture) => Ok(capture),
        Err(first) => {
            // A single dead grab can be a one-frame race with the compositor (our
            // own window disappearing, a display state change). Give it one more
            // round before telling the user capture is impossible.
            crate::applog::log(format!("抓屏：第一次抓屏失败，稍后重试一次。原因：{first}"));
            sleep_ms(160);
            grab(monitor, info).map_err(|second| format!("{second}\n（已自动重试一次）"))
        }
    }
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

/// `--diagnose`: exercise every backend explicitly and report what each returns.
///
/// This is what answers "the screenshot does not work when it is not run as
/// administrator": it shows the process access context (a low-integrity token is
/// refused by WGC), what WGC says, and whether either GDI variant still produces
/// a usable frame.
pub fn diagnose_lines() -> Vec<String> {
    let mut out = Vec::new();
    out.push(format!("进程权限：{}", crate::win::describe_process_access()));
    out.push(format!("显示状态：{}", crate::win::describe_display_color()));
    out.push(format!(
        "WGC 冷却：{}",
        match wgc_cooldown_left() {
            d if d.is_zero() => "无（下次抓屏会先试 WGC）".to_string(),
            d => format!("还剩 {:.0}s（上次被拒绝）", d.as_secs_f32()),
        }
    ));

    let monitors = match xcap::Monitor::all() {
        Ok(m) => m,
        Err(e) => {
            out.push(format!("显示器枚举：失败（{e}）"));
            return out;
        }
    };
    let (cx, cy) = crate::win::cursor_position();
    let mut entries = Vec::new();
    for m in &monitors {
        if let Ok(info) = MonitorInfo::read(m) {
            entries.push((m, info));
        }
    }
    out.push(format!(
        "显示器：{} 台，鼠标位于 ({cx}, {cy})",
        entries.len()
    ));
    for (_, info) in &entries {
        out.push(format!(
            "  · {} {}x{} @{:.0}% 位置 ({}, {}){}",
            info.name,
            info.w,
            info.h,
            info.scale * 100.0,
            info.x,
            info.y,
            if info.primary { " [主屏]" } else { "" }
        ));
    }
    let index = entries
        .iter()
        .position(|(_, info)| info.contains(cx, cy))
        .or_else(|| entries.iter().position(|(_, info)| info.primary))
        .unwrap_or(0);
    let Some((monitor, info)) = entries.get(index) else {
        out.push("显示器细节：不可读".to_string());
        return out;
    };
    out.push(format!(
        "目标显示器：{}（抓屏用这一块）",
        info.name
    ));

    crate::win::dwm_flush();

    match monitor.capture_image() {
        Ok(image) => {
            let stats = frame_stats(&image);
            out.push(format!(
                "Windows Graphics Capture：成功 {}x{}，{}{}",
                image.width(),
                image.height(),
                stats.describe(),
                if looks_broken(&stats) { " → 判定为黑屏" } else { "" }
            ));
        }
        Err(err) => {
            let text = err.to_string();
            out.push(format!(
                "Windows Graphics Capture：失败（{text}）{}",
                match classify_wgc_error(&text) {
                    Some(WgcFailure::Denied) => " → 当前进程/会话被系统拒绝",
                    Some(WgcFailure::Unsupported) => " → 本系统不支持",
                    None => "",
                }
            ));
        }
    }

    for (variant, layered) in [("GDI+CAPTUREBLT", true), ("GDI（纯 SRCCOPY）", false)] {
        match crate::win::capture_screen_gdi_opts(
            info.x,
            info.y,
            info.w as i32,
            info.h as i32,
            layered,
        ) {
            Ok(bitmap) => {
                let Some(image) = RgbaImage::from_raw(bitmap.width, bitmap.height, bitmap.rgba)
                else {
                    out.push(format!("{variant}：位图转换失败"));
                    continue;
                };
                let stats = frame_stats(&image);
                out.push(format!(
                    "{variant}：{}x{}，{}{}",
                    image.width(),
                    image.height(),
                    stats.describe(),
                    if looks_broken(&stats) { " → 判定为黑屏" } else { "" }
                ));
            }
            Err(err) => out.push(format!("{variant}：失败（{err}）")),
        }
    }

    match capture_monitor_at(cx, cy) {
        Ok(cap) => out.push(format!(
            "结论：可以截图——使用后端 {}，{}x{}",
            cap.backend, cap.w, cap.h
        )),
        Err(err) => out.push(format!("结论：截图失败——{err}")),
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    fn solid(w: u32, h: u32, rgb: [u8; 3]) -> RgbaImage {
        RgbaImage::from_pixel(w, h, Rgba([rgb[0], rgb[1], rgb[2], 255]))
    }

    #[test]
    fn all_black_frame_is_rejected() {
        let stats = frame_stats(&solid(320, 200, [0, 0, 0]));
        assert!(looks_broken(&stats), "{stats:?}");
    }

    #[test]
    fn black_frame_with_a_cursor_sliver_is_rejected() {
        // The old check (flat + dark) accepted this: a black grab with the cursor
        // painted in it reached the overlay as a black screenshot.
        let mut image = solid(320, 200, [0, 0, 0]);
        for y in 10..18 {
            for x in 10..18 {
                image.put_pixel(x, y, Rgba([255, 255, 255, 255]));
            }
        }
        let stats = frame_stats(&image);
        assert!(looks_broken(&stats), "{stats:?}");
    }

    #[test]
    fn normal_desktop_frame_is_accepted() {
        let mut image = RgbaImage::new(320, 200);
        for y in 0..200u32 {
            for x in 0..320u32 {
                let l = ((x + y) % 200) as u8 + 20;
                image.put_pixel(x, y, Rgba([l, l / 2, 255 - l, 255]));
            }
        }
        let stats = frame_stats(&image);
        assert!(!looks_broken(&stats), "{stats:?}");
    }

    #[test]
    fn dark_but_real_desktop_is_accepted() {
        // A dark-theme desktop: low mean, but text and icons are bright.
        let mut image = solid(320, 200, [16, 18, 22]);
        for y in (10..190).step_by(20) {
            for x in (10..300).step_by(2) {
                image.put_pixel(x, y, Rgba([230, 232, 238, 255]));
            }
        }
        let stats = frame_stats(&image);
        assert!(!looks_broken(&stats), "{stats:?}");
    }

    #[test]
    fn flat_mid_grey_is_accepted() {
        let stats = frame_stats(&solid(320, 200, [128, 128, 128]));
        assert!(!looks_broken(&stats), "{stats:?}");
    }

    #[test]
    fn error_classification_covers_localised_and_code_forms() {
        assert_eq!(
            classify_wgc_error("You do not have permission to capture the given monitor. (0x80070005)"),
            Some(WgcFailure::Denied)
        );
        assert_eq!(
            classify_wgc_error("拒绝访问。 (0x80070005)"),
            Some(WgcFailure::Denied)
        );
        assert_eq!(
            classify_wgc_error("Class not registered (0x80040154)"),
            Some(WgcFailure::Unsupported)
        );
        assert_eq!(classify_wgc_error("something else went wrong"), None);
    }
}
