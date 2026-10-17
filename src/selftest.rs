//! Offscreen self-test: renders every annotation primitive through the real
//! rasterizer and writes PNGs, so the rendering pipeline can be verified without
//! a display (used by `screen-cliper --selftest <dir>`).

use crate::raster::{self, Region};
use crate::shapes::{RectF, Shape, StickerKind};
use image::{Rgba, RgbaImage};
use std::path::Path;

pub fn run(out_dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(out_dir).map_err(|e| format!("创建输出目录失败：{e}"))?;

    let (w, h) = (1280u32, 800u32);
    let base = make_base(w, h);
    let base_path = out_dir.join("01-base.png");
    base.save_with_format(&base_path, image::ImageFormat::Png)
        .map_err(|e| format!("保存底图失败：{e}"))?;

    let shapes = sample_shapes();
    let annotated = raster::render_region(&base, Region::new(0.0, 0.0, w as f32, h as f32), &shapes, None);
    let ann_path = out_dir.join("02-annotated.png");
    annotated
        .save_with_format(&ann_path, image::ImageFormat::Png)
        .map_err(|e| format!("保存标注图失败：{e}"))?;

    // Sticker close-up sheet (checks anti-aliasing at large sizes).
    let mut sheet = RgbaImage::from_pixel(1200, 520, Rgba([24, 26, 32, 255]));
    for (i, kind) in StickerKind::ALL.iter().enumerate() {
        let cx = 110.0 + (i as f32) * 135.0;
        let shapes = vec![Shape::Sticker {
            center: [cx, 170.0],
            size: 150.0,
            kind: *kind,
            color: [255, 214, 10, 255],
        }];
        let layer = raster::render_region(
            &sheet,
            Region::new(0.0, 0.0, 1200.0, 520.0),
            &shapes,
            None,
        );
        sheet = layer;
    }
    let mut labels = Vec::new();
    for (i, kind) in StickerKind::ALL.iter().enumerate() {
        labels.push(Shape::Text {
            pos: [60.0 + (i as f32) * 135.0, 300.0],
            text: kind.label().to_string(),
            color: [230, 230, 235, 255],
            size: 22.0,
        });
    }
    let sheet = raster::render_region(
        &sheet,
        Region::new(0.0, 0.0, 1200.0, 520.0),
        &labels,
        None,
    );
    let sheet_path = out_dir.join("03-stickers.png");
    sheet
        .save_with_format(&sheet_path, image::ImageFormat::Png)
        .map_err(|e| format!("保存贴花图失败：{e}"))?;

    // Crop of the annotated image, exercising the selection-region render path.
    let crop = raster::render_region(
        &base,
        Region::new(560.0, 340.0, 640.0, 240.0),
        &shapes,
        None,
    );
    let crop_path = out_dir.join("04-region-crop.png");
    crop.save_with_format(&crop_path, image::ImageFormat::Png)
        .map_err(|e| format!("保存裁剪图失败：{e}"))?;

    println!("自检完成：");
    println!("  字体：{}", crate::fonts::describe());
    println!("  文本测量：{}", {
        let (tw, th) = raster::measure_text("截图标注 ScreenCliper 123", 28.0);
        format!("{tw:.1} x {th:.1} px")
    });
    println!("  {}", base_path.display());
    println!("  {}", ann_path.display());
    println!("  {}", sheet_path.display());
    println!("  {}", crop_path.display());
    Ok(())
}

/// Draw a neutral fake desktop, used to produce clean documentation
/// screenshots without embedding the real (private) desktop of whoever
/// happened to run the capture.
pub fn synth_desktop(w: u32, h: u32) -> RgbaImage {
    let mut img = RgbaImage::new(w, h);
    // Wallpaper: diagonal gradient with a soft bluish glow.
    for y in 0..h {
        for x in 0..w {
            let dx = x as f32 / w as f32;
            let dy = y as f32 / h as f32;
            let t = (dx * 0.6 + dy * 0.4).clamp(0.0, 1.0);
            let glow = {
                let gx = dx - 0.72;
                let gy = dy - 0.28;
                (1.0 - (gx * gx + gy * gy).sqrt() * 2.1).clamp(0.0, 1.0)
            };
            let r = (18.0 + 26.0 * t + 40.0 * glow) as u8;
            let g = (22.0 + 34.0 * t + 52.0 * glow) as u8;
            let b = (32.0 + 52.0 * t + 78.0 * glow) as u8;
            img.put_pixel(x, y, Rgba([r, g, b, 255]));
        }
    }

    // A large application window.
    let wx = (w as f32 * 0.08) as u32;
    let wy = (h as f32 * 0.12) as u32;
    let ww = (w as f32 * 0.62) as u32;
    let wh = (h as f32 * 0.66) as u32;
    fill_rect(&mut img, wx, wy, ww, wh, [30, 33, 40, 255]);
    fill_rect(&mut img, wx, wy, ww, 64, [42, 46, 56, 255]);
    // Traffic lights.
    for (i, c) in [[255, 95, 86], [255, 189, 46], [39, 201, 63]].iter().enumerate() {
        fill_rect(&mut img, wx + 26 + i as u32 * 34, wy + 24, 18, 18, [c[0], c[1], c[2], 255]);
    }
    // Sidebar.
    fill_rect(&mut img, wx, wy + 64, (ww as f32 * 0.22) as u32, wh - 64, [36, 39, 47, 255]);
    // Content lines.
    for i in 0..14u32 {
        let y = wy + 110 + i * 44;
        if y + 16 > wy + wh {
            break;
        }
        let indent = (ww as f32 * 0.26) as u32 + if i % 3 == 0 { 24 } else { 0 };
        let len = (ww as f32 * (if i % 4 == 0 { 0.52 } else { 0.40 })) as u32;
        fill_rect(&mut img, wx + indent, y, len, 14, [64, 69, 82, 255]);
    }

    // A second, "photo" window (gives the mosaic/blur tools something to act on).
    let px = (w as f32 * 0.74) as u32;
    let py = (h as f32 * 0.18) as u32;
    let pw = (w as f32 * 0.20) as u32;
    let ph = (h as f32 * 0.50) as u32;
    fill_rect(&mut img, px, py, pw, ph, [238, 240, 245, 255]);
    for y in 0..ph - 40 {
        for x in 0..pw {
            let fx = x as f32 / pw as f32;
            let fy = y as f32 / ph as f32;
            let band = if ((x / 26) + (y / 26)) % 2 == 0 { 1.0 } else { 0.0 };
            let r = (80.0 + 150.0 * fx + 20.0 * band) as u8;
            let g = (60.0 + 120.0 * fy + 18.0 * band) as u8;
            let b = (140.0 + 90.0 * (1.0 - fx) + 16.0 * band) as u8;
            img.put_pixel(px + x, py + 40 + y, Rgba([r, g, b, 255]));
        }
    }
    fill_rect(&mut img, px, py, pw, 40, [222, 225, 232, 255]);

    // Taskbar.
    fill_rect(&mut img, 0, h - 72, w, 72, [24, 26, 32, 255]);
    for i in 0..9u32 {
        fill_rect(&mut img, 24 + i * 62, h - 62, 44, 44, [58, 63, 74, 255]);
    }

    // Labels rendered through the real rasterizer (also proves CJK glyphs work).
    let labels = vec![
        Shape::Text {
            pos: [(wx as f32) + 140.0, (wy as f32) + 18.0],
            text: "演示工程 — 设计稿评审".to_string(),
            color: [226, 230, 238, 255],
            size: 26.0,
        },
        Shape::Text {
            pos: [(wx as f32) + (ww as f32) * 0.26, (wy as f32) + 78.0],
            text: "ScreenCliper 截图标注演示".to_string(),
            color: [180, 188, 200, 255],
            size: 22.0,
        },
        Shape::Text {
            pos: [(px as f32) + 18.0, (py as f32) + 8.0],
            text: "预览图".to_string(),
            color: [60, 64, 72, 255],
            size: 20.0,
        },
    ];
    let (w2, h2) = (img.width() as f32, img.height() as f32);
    raster::render_region(&img, Region::new(0.0, 0.0, w2, h2), &labels, None)
}
fn make_base(w: u32, h: u32) -> RgbaImage {
    let mut img = RgbaImage::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let t = (x as f32 / w as f32) * 0.55 + (y as f32 / h as f32) * 0.45;
            let r = (26.0 + 44.0 * t) as u8;
            let g = (34.0 + 58.0 * t) as u8;
            let b = (48.0 + 74.0 * t) as u8;
            img.put_pixel(x, y, Rgba([r, g, b, 255]));
        }
    }
    // Grid.
    for x in (0..w).step_by(80) {
        for y in 0..h {
            let p = img.get_pixel_mut(x, y);
            p.0[0] = p.0[0].saturating_add(26);
            p.0[1] = p.0[1].saturating_add(26);
            p.0[2] = p.0[2].saturating_add(26);
        }
    }
    for y in (0..h).step_by(80) {
        for x in 0..w {
            let p = img.get_pixel_mut(x, y);
            p.0[0] = p.0[0].saturating_add(26);
            p.0[1] = p.0[1].saturating_add(26);
            p.0[2] = p.0[2].saturating_add(26);
        }
    }
    // A couple of colourful blocks and a "photo-ish" swatch area.
    fill_rect(&mut img, 600, 360, 260, 170, [70, 120, 200, 255]);
    fill_rect(&mut img, 880, 360, 260, 170, [200, 110, 70, 255]);
    for y in 360..530 {
        for x in 600..1140 {
            if (x / 12 + y / 12) % 2 == 0 {
                let p = img.get_pixel_mut(x, y);
                p.0[0] = p.0[0].saturating_add(40);
                p.0[1] = p.0[1].saturating_add(20);
            }
        }
    }
    img
}

fn fill_rect(img: &mut RgbaImage, x0: u32, y0: u32, w: u32, h: u32, c: [u8; 4]) {
    for y in y0..(y0 + h).min(img.height()) {
        for x in x0..(x0 + w).min(img.width()) {
            img.put_pixel(x, y, Rgba(c));
        }
    }
}

fn sample_shapes() -> Vec<Shape> {
    let mut shapes = Vec::new();

    shapes.push(Shape::Rect {
        a: [80.0, 90.0],
        b: [400.0, 290.0],
        color: [235, 59, 48, 255],
        width: 5.0,
    });
    shapes.push(Shape::Ellipse {
        a: [440.0, 90.0],
        b: [760.0, 290.0],
        color: [52, 199, 89, 255],
        width: 5.0,
    });
    shapes.push(Shape::Arrow {
        a: [820.0, 285.0],
        b: [1140.0, 95.0],
        color: [255, 149, 0, 255],
        width: 6.0,
    });
    shapes.push(Shape::Stroke {
        points: vec![
            [80.0, 420.0],
            [160.0, 350.0],
            [240.0, 450.0],
            [320.0, 355.0],
            [400.0, 465.0],
        ],
        color: [0, 199, 255, 255],
        width: 5.0,
        alpha: 1.0,
    });
    shapes.push(Shape::Stroke {
        points: vec![[70.0, 620.0], [560.0, 620.0]],
        color: [255, 214, 10, 255],
        width: 26.0,
        alpha: 0.32,
    });
    shapes.push(Shape::Text {
        pos: [80.0, 650.0],
        text: "截图标注 ScreenCliper 中文测试 123".to_string(),
        color: [255, 255, 255, 255],
        size: 30.0,
    });
    shapes.push(Shape::Mosaic {
        rect: RectF::new(620.0, 380.0, 230.0, 130.0).as_xywh(),
        block: 14,
    });
    shapes.push(Shape::Blur {
        rect: RectF::new(900.0, 380.0, 230.0, 130.0).as_xywh(),
        radius: 8.0,
    });

    // Stickers row.
    for (i, kind) in StickerKind::ALL.iter().enumerate() {
        shapes.push(Shape::Sticker {
            center: [100.0 + (i as f32) * 78.0, 740.0],
            size: 62.0,
            kind: *kind,
            color: [255, 214, 10, 255],
        });
    }
    // Numbered badges.
    for i in 0..5u32 {
        shapes.push(Shape::Number {
            center: [760.0 + (i as f32) * 70.0, 740.0],
            size: 52.0,
            n: i + 1,
            color: [255, 45, 85, 255],
        });
    }
    // Pasted bitmap (checkerboard with alpha).
    let mut patch = RgbaImage::new(120, 120);
    for y in 0..120 {
        for x in 0..120 {
            let inside = x > 6 && x < 114 && y > 6 && y < 114;
            let c = if !inside {
                [0, 0, 0, 0]
            } else if (x / 15 + y / 15) % 2 == 0 {
                [250, 250, 252, 235]
            } else {
                [40, 44, 52, 235]
            };
            patch.put_pixel(x, y, Rgba(c));
        }
    }
    shapes.push(Shape::Bitmap {
        rect: [1130.0, 640.0, 120.0, 120.0],
        image: std::sync::Arc::new(patch),
    });

    shapes
}
