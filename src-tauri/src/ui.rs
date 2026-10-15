//! 纯代码生成托盘图标（RGBA 位图，无需新增图片解码依赖）。
//!
//! - macOS：彩色实心圆点（菜单栏文字由 tray title 提供）
//! - Windows：应用 Logo + 状态色角标（托盘区不支持 title，
//!   角标颜色表达账号状态；具体百分比在 tooltip 与面板中展示）

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
        logo_badge_icon(rgb, 32)
    }
    #[cfg(not(target_os = "windows"))]
    {
        dot_icon(rgb, 40)
    }
}

/// 实心彩色圆点（macOS 菜单栏；Windows 上作为 Logo 解码失败的兜底）
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

/// Windows 托盘：应用 Logo + 右下角状态色圆点角标（白描边，深浅色任务栏上都可见）。
/// 嵌入 icons/32x32.png（32×32 RGBA，tauri 的 image-png 特性解码），解码失败时退回纯色圆点。
#[cfg(target_os = "windows")]
pub fn logo_badge_icon(badge_rgb: [u8; 3], size: u32) -> Image<'static> {
    let img = match Image::from_bytes(include_bytes!("../icons/32x32.png")) {
        Ok(img) => img,
        Err(_) => return dot_icon(badge_rgb, size),
    };
    let mut buf = img.rgba().to_vec();
    if buf.len() != (size * size * 4) as usize {
        return dot_icon(badge_rgb, size);
    }

    // source-over 合成：先白描边圈，再状态色实心
    let c = size as f64 * 0.73;
    let r_fill = size as f64 * 0.20;
    let r_ring = r_fill + 1.6;
    for y in 0..size {
        for x in 0..size {
            let d = (((x as f64) - c).powi(2) + ((y as f64) - c).powi(2)).sqrt();
            let a_fill = clamp01((r_fill - d + 0.75) / 1.5);
            let a_ring = clamp01((r_ring - d + 0.75) / 1.5) * clamp01((d - r_fill + 0.75) / 1.5);
            if a_fill <= 0.0 && a_ring <= 0.0 {
                continue;
            }
            let i = ((y * size + x) * 4) as usize;
            for (a, col) in [(a_ring, [255u8, 255, 255]), (a_fill, badge_rgb)] {
                if a <= 0.0 {
                    continue;
                }
                let sa = a as f32;
                let da = buf[i + 3] as f32 / 255.0;
                let oa = sa + da * (1.0 - sa);
                for ch in 0..3 {
                    let sc = col[ch] as f32;
                    let dc = buf[i + ch] as f32;
                    buf[i + ch] = ((sc * sa + dc * da * (1.0 - sa)) / oa) as u8;
                }
                buf[i + 3] = (oa * 255.0) as u8;
            }
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
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn logo_badge_composites_badge_color() {
        let img = logo_badge_icon(GREEN, 32);
        assert_eq!(img.rgba().len(), 32 * 32 * 4);
        // 角标圆心 (23,23) 处应为纯状态色
        let i = (23 * 32 + 23) * 4;
        assert_eq!(&img.rgba()[i..i + 3], &GREEN);
        assert_eq!(img.rgba()[i + 3], 255);
    }
}
