use eframe::egui::IconData;

const ICON_SIZE: usize = 64;

/// Signed distance to a centered rounded rectangle (negative = inside).
fn sd_rounded_box(px: f32, py: f32, cx: f32, cy: f32, half_w: f32, half_h: f32, r: f32) -> f32 {
    let qx = (px - cx).abs() - (half_w - r);
    let qy = (py - cy).abs() - (half_h - r);
    let outside = (qx.max(0.0).hypot(qy.max(0.0))).max(0.0);
    let inside = qx.max(qy).min(0.0);
    outside + inside - r
}

fn in_rect(x: f32, y: f32, rx: f32, ry: f32, rw: f32, rh: f32) -> bool {
    x >= rx && x <= rx + rw && y >= ry && y <= ry + rh
}

fn in_rounded_rect(x: f32, y: f32, rx: f32, ry: f32, rw: f32, rh: f32, r: f32) -> bool {
    let cx = rx + rw * 0.5;
    let cy = ry + rh * 0.5;
    sd_rounded_box(x, y, cx, cy, rw * 0.5, rh * 0.5, r) <= 0.0
}

fn point_in_polygon(x: f32, y: f32, verts: &[(f32, f32)]) -> bool {
    let mut inside = false;
    let n = verts.len();
    let mut j = n - 1;
    for i in 0..n {
        let (xi, yi) = verts[i];
        let (xj, yj) = verts[j];
        let intersects = ((yi > y) != (yj > y))
            && (x < (xj - xi) * (y - yi) / ((yj - yi) + f32::EPSILON) + xi);
        if intersects {
            inside = !inside;
        }
        j = i;
    }
    inside
}

fn lerp_u8(a: u8, b: u8, t: f32) -> u8 {
    let t = t.clamp(0.0, 1.0);
    (a as f32 + (b as f32 - a as f32) * t).round() as u8
}

fn blend_rgba(dst: [u8; 4], src: [u8; 4]) -> [u8; 4] {
    let sa = src[3] as f32 / 255.0;
    let inv = 1.0 - sa;
    [
        (src[0] as f32 * sa + dst[0] as f32 * inv).round() as u8,
        (src[1] as f32 * sa + dst[1] as f32 * inv).round() as u8,
        (src[2] as f32 * sa + dst[2] as f32 * inv).round() as u8,
        255,
    ]
}

/// Sample the 128x128 SVG design (`assets/lz-editor.svg`) at floating-point coordinate `(x, y)`.
fn sample_svg(x: f32, y: f32) -> [u8; 4] {
    // Window frame: rect x=6 y=6 w=116 h=116 rx=16 stroke=#40D2FF stroke-width=4
    let frame_sd = sd_rounded_box(x, y, 64.0, 64.0, 58.0, 58.0, 16.0);
    if frame_sd > 2.0 {
        return [0, 0, 0, 0];
    }

    // Cyan border ring (#40D2FF)
    if frame_sd >= -2.0 {
        return [0x40, 0xD2, 0xFF, 255];
    }

    // Interior deep navy gradient: #0E2A5C (top) -> #081838 (bottom)
    let bg_t = ((y - 6.0) / 116.0).clamp(0.0, 1.0);
    let mut color = [
        lerp_u8(0x0E, 0x08, bg_t),
        lerp_u8(0x2A, 0x18, bg_t),
        lerp_u8(0x5C, 0x38, bg_t),
        255,
    ];

    // Top VZ status bar (y <= 29)
    if y <= 29.0 {
        color = [0x16, 0x3E, 0x82, 255];
    }
    if (28.0..=30.0).contains(&y) {
        color = [0x40, 0xD2, 0xFF, 255];
    }
    // Status bar badges
    if in_rounded_rect(x, y, 18.0, 15.0, 22.0, 9.0, 2.0) {
        color = [0x40, 0xD2, 0xFF, 255];
    }
    if in_rounded_rect(x, y, 82.0, 17.0, 28.0, 5.0, 2.0) {
        color = blend_rgba(color, [0x70, 0xE8, 0xFF, 217]);
    }

    // Vector letter L: M22 44 H32 V74 H48 V83 H22 Z
    let in_l = in_rect(x, y, 22.0, 44.0, 10.0, 39.0) || in_rect(x, y, 22.0, 74.0, 26.0, 9.0);
    if in_l {
        color = [0xEA, 0xF4, 0xFF, 255];
    }

    // Vector letter Z: M54 44 H84 V52 L65 74 H84 V83 H54 V75 L73 53 H54 Z
    const Z_VERTS: [(f32, f32); 10] = [
        (54.0, 44.0),
        (84.0, 44.0),
        (84.0, 52.0),
        (65.0, 74.0),
        (84.0, 74.0),
        (84.0, 83.0),
        (54.0, 83.0),
        (54.0, 75.0),
        (73.0, 53.0),
        (54.0, 53.0),
    ];
    if point_in_polygon(x, y, &Z_VERTS) {
        color = [0xEA, 0xF4, 0xFF, 255];
    }

    // Glowing PC-98 cursor block: rect x=92 y=44 w=16 h=39 rx=2 (#70E8FF -> #2E8CD0)
    if in_rounded_rect(x, y, 92.0, 44.0, 16.0, 39.0, 2.0) {
        let tx = (x - 92.0) / 16.0;
        let ty = (y - 44.0) / 39.0;
        let t = ((tx + ty) * 0.5).clamp(0.0, 1.0);
        color = [
            lerp_u8(0x70, 0x2E, t),
            lerp_u8(0xE8, 0x8C, t),
            lerp_u8(0xFF, 0xD0, t),
            255,
        ];
    }

    // Bottom F-key guide strip
    if (97.0..=99.0).contains(&y) {
        color = [0x2A, 0x62, 0xA8, 255];
    }
    for &kx in &[16.0_f32, 36.0, 56.0, 76.0, 96.0] {
        if in_rounded_rect(x, y, kx, 104.0, 16.0, 10.0, 2.0) {
            color = [0x2E, 0x8C, 0xD0, 255];
        }
    }

    color
}

/// Build the 64x64 RGBA window icon matching `assets/lz-editor.svg` with 2x2 supersampling.
pub fn build_app_icon() -> IconData {
    let mut rgba = vec![0u8; ICON_SIZE * ICON_SIZE * 4];

    for py in 0..ICON_SIZE {
        for px in 0..ICON_SIZE {
            let mut acc_r = 0.0_f32;
            let mut acc_g = 0.0_f32;
            let mut acc_b = 0.0_f32;
            let mut acc_a = 0.0_f32;

            // 2x2 supersampling mapped directly to the 128x128 SVG coordinate grid
            for sy in 0..2 {
                for sx in 0..2 {
                    let x = (px * 2 + sx) as f32 + 0.5;
                    let y = (py * 2 + sy) as f32 + 0.5;
                    let s = sample_svg(x, y);
                    let a = s[3] as f32 / 255.0;
                    acc_r += s[0] as f32 * a;
                    acc_g += s[1] as f32 * a;
                    acc_b += s[2] as f32 * a;
                    acc_a += a;
                }
            }

            let idx = (py * ICON_SIZE + px) * 4;
            if acc_a > 0.001 {
                rgba[idx] = (acc_r / acc_a).round().clamp(0.0, 255.0) as u8;
                rgba[idx + 1] = (acc_g / acc_a).round().clamp(0.0, 255.0) as u8;
                rgba[idx + 2] = (acc_b / acc_a).round().clamp(0.0, 255.0) as u8;
                rgba[idx + 3] = ((acc_a / 4.0) * 255.0).round().clamp(0.0, 255.0) as u8;
            }
        }
    }

    IconData {
        rgba,
        width: ICON_SIZE as u32,
        height: ICON_SIZE as u32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_app_icon_geometry_and_colors() {
        let icon = build_app_icon();
        assert_eq!(icon.width, 64);
        assert_eq!(icon.height, 64);
        assert_eq!(icon.rgba.len(), 64 * 64 * 4);

        let pixel = |x: usize, y: usize| -> [u8; 4] {
            let i = (y * 64 + x) * 4;
            [icon.rgba[i], icon.rgba[i + 1], icon.rgba[i + 2], icon.rgba[i + 3]]
        };

        // Outside rounded corner must be transparent
        assert_eq!(pixel(0, 0)[3], 0);
        assert_eq!(pixel(63, 63)[3], 0);

        // Top border center (svg x=64, y=6 -> pixel 32, 3) must be cyan (#40D2FF)
        let border = pixel(32, 3);
        assert_eq!(border[3], 255);
        assert!(border[2] > 230 && border[1] > 180);

        // White 'L' stem (svg x=26, y=60 -> pixel 13, 30)
        let letter_l = pixel(13, 30);
        assert_eq!(letter_l, [0xEA, 0xF4, 0xFF, 255]);

        // White 'Z' top bar (svg x=70, y=48 -> pixel 35, 24)
        let letter_z = pixel(35, 24);
        assert_eq!(letter_z, [0xEA, 0xF4, 0xFF, 255]);

        // Glowing cyan cursor block (svg x=100, y=64 -> pixel 50, 32)
        let cursor = pixel(50, 32);
        assert_eq!(cursor[3], 255);
        assert!(cursor[2] > 210 && cursor[1] > 160);
    }
}
