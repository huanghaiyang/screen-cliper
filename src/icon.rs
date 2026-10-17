//! Procedurally generated application/tray icon.
//!
//! Generating the icon in code keeps the repository free of binary assets and
//! guarantees the tray icon, the window icon and the settings preview always
//! match. The mark is a "crop frame" over a blue rounded square.

/// Returns (rgba bytes, width, height) for a square icon.
pub fn icon_rgba(size: u32) -> (Vec<u8>, u32, u32) {
    let n = size.max(8) as f32;
    let mut buf = vec![0u8; (size * size * 4) as usize];

    let inset = n * 0.20;
    let arm = n * 0.27;
    let thickness = n * 0.085;
    let radius = n * 0.24;

    for y in 0..size {
        for x in 0..size {
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;

            // Background: rounded square with a vertical blue gradient.
            let bg_d = sd_round_rect(px, py, n * 0.5, n * 0.5, n * 0.5 - 0.5, n * 0.5 - 0.5, radius);
            let bg_cov = (0.5 - bg_d).clamp(0.0, 1.0);
            if bg_cov <= 0.0 {
                continue;
            }
            let t = (py / n).clamp(0.0, 1.0);
            let top = [64.0, 140.0, 255.0];
            let bottom = [24.0, 78.0, 214.0];
            let mut rgb = [
                top[0] + (bottom[0] - top[0]) * t,
                top[1] + (bottom[1] - top[1]) * t,
                top[2] + (bottom[2] - top[2]) * t,
            ];

            // Crop-frame brackets in white.
            let mut d = f32::MAX;
            for &(sx, sy) in &[(-1.0f32, -1.0f32), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                let cx = if sx < 0.0 { inset } else { n - inset };
                let cy = if sy < 0.0 { inset } else { n - inset };
                let hx = cx + sx * arm * 0.5;
                let hy = cy + sy * arm * 0.5;
                // Horizontal arm.
                d = d.min(sd_round_rect(px, py, hx, cy, arm * 0.5, thickness * 0.5, thickness * 0.35));
                // Vertical arm.
                d = d.min(sd_round_rect(px, py, cx, hy, thickness * 0.5, arm * 0.5, thickness * 0.35));
            }
            let fg = (0.5 - d).clamp(0.0, 1.0);
            if fg > 0.0 {
                for i in 0..3 {
                    rgb[i] = rgb[i] + (255.0 - rgb[i]) * fg;
                }
            }

            let idx = ((y * size + x) * 4) as usize;
            buf[idx] = rgb[0].clamp(0.0, 255.0) as u8;
            buf[idx + 1] = rgb[1].clamp(0.0, 255.0) as u8;
            buf[idx + 2] = rgb[2].clamp(0.0, 255.0) as u8;
            buf[idx + 3] = (bg_cov * 255.0) as u8;
        }
    }
    (buf, size, size)
}

fn sd_round_rect(px: f32, py: f32, cx: f32, cy: f32, hw: f32, hh: f32, r: f32) -> f32 {
    let r = r.min(hw).min(hh).max(0.0);
    let qx = (px - cx).abs() - (hw - r);
    let qy = (py - cy).abs() - (hh - r);
    let ax = qx.max(0.0);
    let ay = qy.max(0.0);
    (ax * ax + ay * ay).sqrt() + qx.max(qy).min(0.0) - r
}
