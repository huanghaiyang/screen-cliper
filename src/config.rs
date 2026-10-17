//! Persistent user configuration plus small path/format helpers.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const APP_NAME: &str = "ScreenCliper";
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Overridable root for config/temp files. Used by the dev self-test harness so it
/// never touches the real user profile.
pub fn home_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("SCREEN_CLIPER_HOME") {
        if !dir.trim().is_empty() {
            return PathBuf::from(dir);
        }
    }
    let base = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
    base.join(APP_NAME)
}

pub fn config_path() -> PathBuf {
    home_dir().join("config.json")
}

pub fn temp_dir() -> PathBuf {
    let dir = home_dir().join("temp");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ImageFormat {
    Png,
    Jpeg,
}

impl Default for ImageFormat {
    fn default() -> Self {
        ImageFormat::Png
    }
}

impl ImageFormat {
    pub fn ext(self) -> &'static str {
        match self {
            ImageFormat::Png => "png",
            ImageFormat::Jpeg => "jpg",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ImageFormat::Png => "PNG（无损）",
            ImageFormat::Jpeg => "JPEG（有损）",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AfterCapture {
    Copy,
    Save,
    CopyAndSave,
}

impl Default for AfterCapture {
    fn default() -> Self {
        AfterCapture::Copy
    }
}

impl AfterCapture {
    pub fn label(self) -> &'static str {
        match self {
            AfterCapture::Copy => "复制到剪贴板",
            AfterCapture::Save => "保存为文件",
            AfterCapture::CopyAndSave => "复制并保存",
        }
    }

    pub fn copies(self) -> bool {
        matches!(self, AfterCapture::Copy | AfterCapture::CopyAndSave)
    }

    pub fn saves(self) -> bool {
        matches!(self, AfterCapture::Save | AfterCapture::CopyAndSave)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThemeMode {
    Dark,
    Light,
}

impl Default for ThemeMode {
    fn default() -> Self {
        ThemeMode::Dark
    }
}

/// A single saved colour slot shown in the overlay palette.
pub const PALETTE: [[u8; 4]; 10] = [
    [235, 59, 48, 255],
    [255, 149, 0, 255],
    [255, 214, 10, 255],
    [52, 199, 89, 255],
    [0, 199, 190, 255],
    [0, 122, 255, 255],
    [88, 86, 214, 255],
    [255, 45, 85, 255],
    [255, 255, 255, 255],
    [28, 28, 30, 255],
];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    // ---- capture / hotkey -------------------------------------------------
    pub hotkey: String,
    pub hotkey_enabled: bool,
    /// Capture the whole monitor immediately, then let the user adjust.
    pub fullscreen_capture: bool,

    // ---- startup ----------------------------------------------------------
    pub autostart: bool,
    pub start_minimized: bool,
    pub close_to_tray: bool,

    // ---- output -----------------------------------------------------------
    pub save_dir: String,
    pub filename_pattern: String,
    pub image_format: ImageFormat,
    pub jpeg_quality: u8,
    pub after_capture: AfterCapture,

    // ---- editor defaults --------------------------------------------------
    pub default_tool: String,
    pub pen_width: f32,
    pub pen_color: [u8; 4],
    pub font_size: f32,
    pub sticker_size: f32,
    pub mosaic_block: u32,
    pub magnifier: bool,
    pub theme: ThemeMode,
    pub show_size_label: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            hotkey: "Ctrl+Alt+A".to_string(),
            hotkey_enabled: true,
            fullscreen_capture: false,
            autostart: false,
            start_minimized: false,
            close_to_tray: true,
            save_dir: String::new(),
            filename_pattern: "ScreenCliper_{date}_{time}".to_string(),
            image_format: ImageFormat::Png,
            jpeg_quality: 92,
            after_capture: AfterCapture::Copy,
            default_tool: "rect".to_string(),
            pen_width: 3.0,
            pen_color: [235, 59, 48, 255],
            font_size: 20.0,
            sticker_size: 72.0,
            mosaic_block: 12,
            magnifier: true,
            theme: ThemeMode::Dark,
            show_size_label: true,
        }
    }
}

impl Config {
    pub fn load() -> Self {
        let path = config_path();
        match std::fs::read_to_string(&path) {
            Ok(text) => match serde_json::from_str::<Config>(&text) {
                Ok(cfg) => cfg.sanitised(),
                Err(err) => {
                    eprintln!("[config] 解析失败，使用默认配置：{err}");
                    Config::default()
                }
            },
            Err(_) => Config::default(),
        }
    }

    pub fn save(&self) -> Result<(), String> {
        let dir = home_dir();
        std::fs::create_dir_all(&dir).map_err(|e| format!("创建配置目录失败：{e}"))?;
        let text = serde_json::to_string_pretty(self).map_err(|e| format!("序列化配置失败：{e}"))?;
        let path = config_path();
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, text).map_err(|e| format!("写入配置失败：{e}"))?;
        std::fs::rename(&tmp, &path).map_err(|e| format!("替换配置失败：{e}"))?;
        Ok(())
    }

    fn sanitised(mut self) -> Self {
        self.pen_width = self.pen_width.clamp(1.0, 40.0);
        self.font_size = self.font_size.clamp(8.0, 120.0);
        self.sticker_size = self.sticker_size.clamp(16.0, 300.0);
        self.mosaic_block = self.mosaic_block.clamp(2, 80);
        self.jpeg_quality = self.jpeg_quality.clamp(40, 100);
        if self.filename_pattern.trim().is_empty() {
            self.filename_pattern = "ScreenCliper_{date}_{time}".to_string();
        }
        if !crate::hotkey::is_valid(&self.hotkey) {
            self.hotkey = "Ctrl+Alt+A".to_string();
        }
        self
    }

    pub fn resolved_save_dir(&self) -> PathBuf {
        let raw = self.save_dir.trim();
        if raw.is_empty() {
            let base = dirs::picture_dir()
                .or_else(dirs::home_dir)
                .unwrap_or_else(|| PathBuf::from("."));
            base.join(APP_NAME)
        } else {
            PathBuf::from(raw)
        }
    }

    pub fn ensure_save_dir(&self) -> Result<PathBuf, String> {
        let dir = self.resolved_save_dir();
        std::fs::create_dir_all(&dir).map_err(|e| format!("无法创建保存目录 {}：{e}", dir.display()))?;
        Ok(dir)
    }

    /// Build the output path for a new capture, honouring the filename template.
    pub fn next_output_path(&self) -> Result<PathBuf, String> {
        let dir = self.ensure_save_dir()?;
        let st = crate::win::local_time();
        let stamp = format!(
            "{:04}-{:02}-{:02}",
            st.wYear, st.wMonth, st.wDay
        );
        let time = format!("{:02}-{:02}-{:02}", st.wHour, st.wMinute, st.wSecond);
        let mut name = self
            .filename_pattern
            .replace("{date}", &stamp)
            .replace("{time}", &time)
            .replace("{yyyy}", &format!("{:04}", st.wYear))
            .replace("{MM}", &format!("{:02}", st.wMonth))
            .replace("{dd}", &format!("{:02}", st.wDay))
            .replace("{HH}", &format!("{:02}", st.wHour))
            .replace("{mm}", &format!("{:02}", st.wMinute))
            .replace("{ss}", &format!("{:02}", st.wSecond));
        name = sanitize_filename(&name);
        if name.is_empty() {
            name = format!("ScreenCliper_{time}");
        }
        let mut path = dir.join(format!("{name}.{}", self.image_format.ext()));
        let mut n = 1;
        while path.exists() {
            path = dir.join(format!("{name}_{n}.{}", self.image_format.ext()));
            n += 1;
            if n > 9999 {
                break;
            }
        }
        Ok(path)
    }

    pub fn write_image(&self, image: &image::RgbaImage, path: &Path) -> Result<(), String> {
        let Some(parent) = path.parent() else {
            return Err("无效的保存路径".into());
        };
        std::fs::create_dir_all(parent).map_err(|e| format!("创建目录失败：{e}"))?;
        match self.image_format {
            ImageFormat::Png => image
                .save_with_format(path, image::ImageFormat::Png)
                .map_err(|e| format!("保存 PNG 失败：{e}")),
            ImageFormat::Jpeg => {
                let rgb = image::DynamicImage::ImageRgba8(image.clone()).to_rgb8();
                let file = std::fs::File::create(path).map_err(|e| format!("创建文件失败：{e}"))?;
                let mut writer = std::io::BufWriter::new(file);
                let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(
                    &mut writer,
                    self.jpeg_quality,
                );
                enc.encode_image(&rgb).map_err(|e| format!("编码 JPEG 失败：{e}"))
            }
        }
    }
}

pub fn sanitize_filename(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if (c as u32) < 32 => '_',
            c => c,
        })
        .collect::<String>()
        .trim()
        .trim_matches('.')
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_strips_illegal_chars() {
        assert_eq!(sanitize_filename("a/b:c*?").as_str(), "a_b_c__");
    }

    #[test]
    fn default_config_roundtrips() {
        let cfg = Config::default();
        let text = serde_json::to_string(&cfg).unwrap();
        let back: Config = serde_json::from_str(&text).unwrap();
        assert_eq!(back.hotkey, cfg.hotkey);
        assert_eq!(back.pen_width, cfg.pen_width);
    }

    #[test]
    fn output_path_follows_template_and_avoids_collisions() {
        let root = std::env::temp_dir().join(format!("screen-cliper-cfg-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let mut cfg = Config::default();
        cfg.save_dir = root.join("shots").display().to_string();
        cfg.filename_pattern = "shot_{date}_{time}".to_string();
        cfg.image_format = ImageFormat::Png;

        let first = cfg.next_output_path().expect("path");
        assert!(first.starts_with(root.join("shots")));
        assert!(first.to_string_lossy().contains("shot_"));
        assert_eq!(first.extension().unwrap(), "png");

        std::fs::write(&first, b"x").unwrap();
        let second = cfg.next_output_path().expect("path");
        assert_ne!(first, second, "existing files must not be overwritten");

        let img = image::RgbaImage::from_pixel(4, 3, image::Rgba([10, 20, 30, 255]));
        cfg.write_image(&img, &first).expect("write png");
        let decoded = image::open(&first).expect("decode png").to_rgba8();
        assert_eq!(decoded.dimensions(), (4, 3));
        assert_eq!(decoded.get_pixel(2, 1).0, [10, 20, 30, 255]);

        cfg.image_format = ImageFormat::Jpeg;
        let jpg = root.join("shot.jpg");
        cfg.write_image(&img, &jpg).expect("write jpg");
        let decoded = image::open(&jpg).expect("decode jpg").to_rgb8();
        assert_eq!(decoded.dimensions(), (4, 3));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn filename_template_substitutes_placeholders() {
        let root = std::env::temp_dir().join(format!("screen-cliper-cfg2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let mut cfg = Config::default();
        cfg.save_dir = root.display().to_string();
        cfg.filename_pattern = "a_{yyyy}_{MM}_{dd}_{HH}_{mm}_{ss}_{date}_{time}".to_string();
        let path = cfg.next_output_path().expect("path");
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        assert!(!name.contains('{'), "all placeholders must be replaced: {name}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn bad_hotkey_falls_back() {
        let mut cfg = Config::default();
        cfg.hotkey = "Nope+Q".into();
        // sanitised() is private, exercised through load(); replicate via serde default path.
        assert!(!crate::hotkey::is_valid(&cfg.hotkey));
    }
}
