//! Access to a Windows CJK-capable font, shared by egui (UI text) and the
//! software rasterizer (text baked into the exported image).

use std::path::PathBuf;
use std::sync::OnceLock;

pub struct FontFile {
    pub bytes: Vec<u8>,
    pub path: String,
    pub label: &'static str,
}

static FONT_FILE: OnceLock<Option<FontFile>> = OnceLock::new();
static GLYPH_FONT: OnceLock<Option<ab_glyph::FontVec>> = OnceLock::new();

/// Ordered by preference: Microsoft YaHei first (best CJK + good Latin hinting).
const CANDIDATES: &[(&str, &str)] = &[
    ("msyh.ttc", "微软雅黑"),
    ("msyh.ttf", "微软雅黑"),
    ("Deng.ttf", "等线"),
    ("simhei.ttf", "黑体"),
    ("msjh.ttc", "微軟正黑體"),
    ("simsun.ttc", "宋体"),
    ("segoeui.ttf", "Segoe UI"),
    ("arial.ttf", "Arial"),
];

pub fn windows_fonts_dir() -> PathBuf {
    let windir = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".to_string());
    PathBuf::from(windir).join("Fonts")
}

pub fn font_file() -> Option<&'static FontFile> {
    FONT_FILE
        .get_or_init(|| {
            for (file, label) in CANDIDATES {
                let path = windows_fonts_dir().join(file);
                match std::fs::read(&path) {
                    Ok(bytes) if !bytes.is_empty() => {
                        return Some(FontFile {
                            bytes,
                            path: path.display().to_string(),
                            label,
                        });
                    }
                    _ => continue,
                }
            }
            None
        })
        .as_ref()
}

/// Parsed font used by the rasterizer. Returns `None` when no usable font exists;
/// text rendering then degrades to a no-op instead of panicking.
pub fn glyph_font() -> Option<&'static ab_glyph::FontVec> {
    GLYPH_FONT
        .get_or_init(|| {
            let file = font_file()?;
            ab_glyph::FontVec::try_from_vec(file.bytes.clone()).ok()
        })
        .as_ref()
}

/// Install the CJK font into egui so Chinese labels render instead of tofu.
pub fn install_egui_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    if let Some(file) = font_file() {
        fonts.font_data.insert(
            "cjk".to_owned(),
            egui::FontData::from_owned(file.bytes.clone()),
        );
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            fonts
                .families
                .entry(family)
                .or_default()
                .insert(0, "cjk".to_owned());
        }
    }
    ctx.set_fonts(fonts);
}

pub fn describe() -> String {
    match font_file() {
        Some(f) => format!("{}（{}）", f.label, f.path),
        None => "未找到中文字体（将从 C:\\Windows\\Fonts 读取）".to_string(),
    }
}
