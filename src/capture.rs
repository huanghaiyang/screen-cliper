//! Screen capture. Captures the monitor under the mouse cursor (the monitor the
//! user is looking at), together with its geometry so the overlay can map mouse
//! positions back to captured pixels exactly.

use image::RgbaImage;

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

fn grab(m: &xcap::Monitor, info: &MonitorInfo) -> Result<Capture, String> {
    let image = m
        .capture_image()
        .map_err(|e| format!("抓屏失败（显示器 {}）：{e}", info.name))?;
    Ok(Capture {
        x: info.x,
        y: info.y,
        w: image.width(),
        h: image.height(),
        image,
        scale: if info.scale > 0.1 { info.scale } else { 1.0 },
        name: info.name.clone(),
    })
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
