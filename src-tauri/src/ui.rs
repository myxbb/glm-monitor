//! 纯代码生成托盘图标（RGBA 位图，无需图片资源）。
//!
//! - macOS：彩色实心圆点（菜单栏文字由 tray title 提供）
//! - Windows：进度环形图（托盘区不支持 title，用图形表达已用百分比）

use tauri::image::Image;

pub const GREEN: [u8; 3] = [52, 199, 89]; // #34C759
pub const YELLOW: [u8; 3] = [255, 204, 0]; // #FFCC00
pub const RED: [u8; 3] = [255, 59, 48]; // #FF3B30
pub const GRAY: [u8; 3] = [142, 142, 147]; // #8E8E93

/// 5h 已用百分比 → 状态色（≤50 绿，≤80 黄，>80 红，未知 灰）
pub fn status_rgb(pct: Option<f64>) -> [u8; 3] {
    match pct {
        Some(p) if p <= 50.0 => GREEN,
        Some(p) if p <= 80.0 => YELLOW,
        Some(_) => RED,
        None => GRAY,
    }
}

fn clamp01(x: f64) -> f64 {
    x.clamp(0.0, 1.0)
}

/// 托盘状态图标
pub fn tray_icon(pct: Option<f64>) -> Image<'static> {
    let rgb = status_rgb(pct);
    #[cfg(target_os = "windows")]
    {
        pie_icon(pct, rgb, 32)
    }
    #[cfg(not(target_os = "windows"))]
    {
        dot_icon(rgb, 40)
    }
}

/// 实心彩色圆点（macOS 菜单栏）
#[cfg_attr(target_os = "windows", allow(dead_code))]
pub fn dot_icon(rgb: [u8; 3], size: u32) -> Image<'static> {
    let mut buf = Vec::with_capacity((size * size * 4) as usize);
    let c = (size as f64 - 1.0) / 2.0;
    let r = size as f64 * 0.36;
    for y in 0..size {
        for x in 0..size {
            let d = (((x as f64) - c).powi(2) + ((y as f64) - c).powi(2)).sqrt();
            // 1px 软边抗锯齿
            let a = clamp01((r - d + 0.75) / 1.5);
            buf.extend_from_slice(&[rgb[0], rgb[1], rgb[2], (a * 255.0) as u8]);
        }
    }
    Image::new_owned(buf, size, size)
}

/// 进度环形图（Windows 托盘）：灰色底环 + 状态色弧线，弧长 = 已用百分比
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub fn pie_icon(pct: Option<f64>, rgb: [u8; 3], size: u32) -> Image<'static> {
    let pct = pct.unwrap_or(0.0).clamp(0.0, 100.0);
    let mut buf = Vec::with_capacity((size * size * 4) as usize);
    let c = (size as f64 - 1.0) / 2.0;
    let outer = size as f64 * 0.47;
    let inner = size as f64 * 0.28;
    for y in 0..size {
        for x in 0..size {
            let dx = (x as f64) - c;
            let dy = (y as f64) - c;
            let d = (dx * dx + dy * dy).sqrt();
            let edge = clamp01((outer - d + 0.75) / 1.5) * clamp01((d - inner + 0.75) / 1.5);
            if edge <= 0.0 {
                buf.extend_from_slice(&[0, 0, 0, 0]);
                continue;
            }
            // 角度：正上方为 0°，顺时针增长
            let ang = (dx.atan2(-dy).to_degrees() + 360.0) % 360.0;
            let filled = ang < pct / 100.0 * 360.0;
            let col = if filled { rgb } else { GRAY };
            buf.extend_from_slice(&[col[0], col[1], col[2], (edge * 255.0) as u8]);
        }
    }
    Image::new_owned(buf, size, size)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_color_mapping() {
        assert_eq!(status_rgb(Some(0.0)), GREEN);
        assert_eq!(status_rgb(Some(50.0)), GREEN);
        assert_eq!(status_rgb(Some(51.0)), YELLOW);
        assert_eq!(status_rgb(Some(80.0)), YELLOW);
        assert_eq!(status_rgb(Some(80.1)), RED);
        assert_eq!(status_rgb(Some(100.0)), RED);
        assert_eq!(status_rgb(None), GRAY);
    }

    #[test]
    fn icon_buffer_sizes() {
        let dot = dot_icon(GREEN, 40);
        assert_eq!(dot.rgba().len(), 40 * 40 * 4);

        let pie = pie_icon(Some(69.0), YELLOW, 32);
        assert_eq!(pie.rgba().len(), 32 * 32 * 4);
    }

    #[test]
    fn pie_arc_coverage() {
        // 100% 时右侧（90°）应为状态色，0% 时应为灰色
        let full = pie_icon(Some(100.0), GREEN, 32);
        let px = |img: &Image, x: usize, y: usize| {
            let i = (y * 32 + x) * 4;
            (img.rgba()[i], img.rgba()[i + 1], img.rgba()[i + 2])
        };
        let center = 16;
        let right = (center + 12, center);
        assert_eq!(px(&full, right.0, right.1), (GREEN[0], GREEN[1], GREEN[2]));

        let empty = pie_icon(Some(0.0), GREEN, 32);
        assert_eq!(px(&empty, right.0, right.1), (GRAY[0], GRAY[1], GRAY[2]));
    }
}
