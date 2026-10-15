//! 菜单栏托盘：状态图标 + title/tooltip 更新、右键菜单、面板开合与定位。

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use tauri::menu::{CheckMenuItemBuilder, Menu, MenuBuilder, MenuItem, SubmenuBuilder};
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, PhysicalPosition, Rect};

use crate::state::{now_secs, Usage};
use crate::ui;

pub const TRAY_ID: &str = "glm-main";
/// 面板窗口总宽（320 内容 + 两侧 12px 阴影留白）
const PANEL_WINDOW_W: f64 = 344.0;
/// 失焦自动隐藏后 300ms 内的托盘点击视为同一次交互，避免面板闪烁
const BLUR_GUARD_MS: u64 = 300;

static LAST_HIDE_MS: AtomicU64 = AtomicU64::new(0);

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub fn init(app: &tauri::App) -> tauri::Result<()> {
    let handle = app.handle();
    let menu = build_menu(handle)?;
    let _tray = TrayIconBuilder::with_id(TRAY_ID)
        .icon(ui::tray_icon(None))
        .icon_as_template(false)
        .show_menu_on_left_click(false)
        .menu(&menu)
        .on_menu_event(on_menu_event)
        .on_tray_icon_event(on_tray_event)
        .build(app)?;
    update(handle);
    Ok(())
}

fn on_menu_event(app: &AppHandle, event: tauri::menu::MenuEvent) {
    let id = event.id().as_ref().to_string();
    match id.as_str() {
        "quit" => app.exit(0),
        "refresh" => {
            let h = app.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = crate::commands::refresh_all_impl(&h).await {
                    eprintln!("[tray] 手动刷新失败: {e}");
                }
            });
        }
        "settings" => {
            if let Err(e) = crate::commands::open_settings_impl(app) {
                eprintln!("[tray] 打开设置失败: {e}");
            }
        }
        other => {
            if let Some(acc_id) = other.strip_prefix("switch:") {
                if let Err(e) = crate::commands::select_account_impl(app, acc_id.to_string()) {
                    eprintln!("[tray] 切换账号失败: {e}");
                }
            }
        }
    }
}

fn on_tray_event(tray: &TrayIcon<tauri::Wry>, event: TrayIconEvent) {
    if let TrayIconEvent::Click {
        button: MouseButton::Left,
        button_state: MouseButtonState::Up,
        position,
        rect,
        ..
    } = event
    {
        toggle_panel(tray.app_handle(), position, rect);
    }
}

// ---------- 面板开合 ----------

fn toggle_panel(app: &AppHandle, click: PhysicalPosition<f64>, rect: Rect) {
    let Some(panel) = app.get_webview_window("panel") else {
        return;
    };
    let now = now_ms();
    if panel.is_visible().unwrap_or(false) {
        let _ = panel.hide();
        LAST_HIDE_MS.store(now, Ordering::SeqCst);
    } else if now.saturating_sub(LAST_HIDE_MS.load(Ordering::SeqCst)) > BLUR_GUARD_MS {
        position_panel(app, &panel, click, Some(rect));
        let _ = panel.show();
        let _ = panel.set_focus();
        // 面板打开时自动刷新一次
        let h = app.clone();
        tauri::async_runtime::spawn(async move {
            let _ = crate::commands::refresh_all_impl(&h).await;
        });
    }
}

/// 面板失焦时自动隐藏（点击面板外部关闭）
pub fn on_panel_blur(app: &AppHandle) {
    if let Some(panel) = app.get_webview_window("panel") {
        if panel.is_visible().unwrap_or(false) {
            let _ = panel.hide();
            LAST_HIDE_MS.store(now_ms(), Ordering::SeqCst);
        }
    }
}

/// tao Monitor 的点坐标包围盒 (x, y, w, h)。
/// 实测（诊断日志）：tao 的 position 是「origin×各屏自己的缩放」、
/// size 是「原始像素」，分别除以各自缩放后才是 macOS 全局点空间。
#[cfg(target_os = "macos")]
fn monitor_point_bounds(m: &tauri::Monitor) -> (f64, f64, f64, f64) {
    let s = m.scale_factor();
    (
        m.position().x as f64 / s,
        m.position().y as f64 / s,
        m.size().width as f64 / s,
        m.size().height as f64 / s,
    )
}

/// 找出托盘图标所在的显示器。
///
/// 托盘事件坐标按「图标所在屏的缩放」换算，而找屏又需要先知道缩放，
/// 互为因果，且不同假设可能都"自洽"（实测副屏 1x 与主屏 2x 的解读可同时成立）。
/// 权威解：直接读全局光标位置——tao 的 cursor_position 按主屏缩放换算，
/// 除以主屏缩放即真实全局点坐标，点击瞬间光标就在图标上。
#[cfg(target_os = "macos")]
fn find_icon_monitor(app: &AppHandle, click_x: f64, click_y: f64) -> Option<tauri::Monitor> {
    let monitors = app.available_monitors().ok()?;
    let contains = |m: &tauri::Monitor, x: f64, y: f64| -> bool {
        let (mx, my, w, h) = monitor_point_bounds(m);
        x >= mx && x <= mx + w && y >= my && y <= my + h
    };
    // 权威解：全局光标的真实点坐标
    if let Ok(cur) = app.cursor_position() {
        if let Ok(Some(primary)) = app.primary_monitor() {
            let s = primary.scale_factor();
            let (x, y) = (cur.x / s, cur.y / s);
            if let Some(m) = monitors.iter().find(|m| contains(m, x, y)) {
                return Some(m.clone());
            }
        }
    }
    // 兜底：按点击原始坐标找最近屏
    monitors.into_iter().min_by(|a, b| {
        fn dist(m: &tauri::Monitor, x: f64, y: f64) -> f64 {
            let (mx, my, w, h) = monitor_point_bounds(m);
            let dx = if x < mx {
                mx - x
            } else if x > mx + w {
                x - (mx + w)
            } else {
                0.0
            };
            let dy = if y < my {
                my - y
            } else if y > my + h {
                y - (my + h)
            } else {
                0.0
            };
            dx * dx + dy * dy
        }
        dist(a, click_x, click_y)
            .partial_cmp(&dist(b, click_x, click_y))
            .unwrap_or(std::cmp::Ordering::Equal)
    })
}

#[cfg(target_os = "macos")]
fn position_panel(
    app: &AppHandle,
    panel: &tauri::WebviewWindow,
    click: PhysicalPosition<f64>,
    rect: Option<Rect>,
) {
    // ---- 第 1 步：把托盘事件坐标还原为真实全局点坐标 ----
    //
    // 实测（NSScreen/CG API 交叉验证）：macOS 全局坐标是「点」空间，
    // tray-icon 的事件坐标 = 点坐标 × 图标所在屏缩放，且 Y 已翻转为上缘原点：
    //   X: pt_x × s_icon
    //   Y: pt_y(从各屏顶部起算的全局点) × s_icon
    let Some(m) = find_icon_monitor(app, click.x, click.y) else {
        return;
    };
    let s_icon = m.scale_factor();

    let (icon_right_pt, icon_top_pt, icon_bottom_pt) = match rect {
        Some(r) => {
            let p: tauri::PhysicalPosition<i32> = r.position.to_physical(s_icon);
            let s: tauri::PhysicalSize<u32> = r.size.to_physical(s_icon);
            let bottom_pt = p.y as f64 / s_icon;
            let top_pt = bottom_pt - s.height as f64 / s_icon;
            ((p.x as f64 + s.width as f64) / s_icon, top_pt, bottom_pt)
        }
        None => {
            let x_pt = click.x / s_icon;
            let y_pt = click.y / s_icon;
            (x_pt, y_pt - 12.0, y_pt)
        }
    };

    // ---- 第 2 步：在点坐标空间内计算面板目标位置 ----
    let (m_x, m_y, m_w, m_h) = monitor_point_bounds(&m);
    let win_scale = panel.scale_factor().unwrap_or(1.0);
    let panel_h = panel
        .outer_size()
        .map(|s| s.height as f64 / win_scale)
        .unwrap_or(420.0);

    // 面板视觉右缘（去掉 12px 留白）对齐托盘图标右缘左侧 8px
    let mut x_pt = icon_right_pt - 8.0 - (PANEL_WINDOW_W - 12.0);
    x_pt = x_pt.clamp(m_x, (m_x + m_w - PANEL_WINDOW_W).max(m_x));

    // 顶栏（菜单栏在上）面板贴图标下方；底栏贴图标上方。
    // 窗口四周有 12px CSS 阴影留白：顶栏时"窗口顶边 + 12"= 图标底缘
    let below = icon_bottom_pt < m_y + m_h / 2.0;
    let y_pt = if below {
        (icon_bottom_pt - 12.0).min(m_y + m_h - panel_h)
    } else {
        (icon_top_pt + 12.0 - panel_h).max(m_y)
    };

    // ---- 第 3 步：按 tao 的换算规则预乘，落到正确屏幕 ----
    //
    // tao set_position: logical = phys / 窗口所在屏缩放(win_scale)，
    // 然后 cocoa_x = logical.x，cocoa_y = mainPtH − logical.y。
    // 要让窗口左上角落在点坐标 (x_pt, y_pt)，需传：
    //   phys.x = x_pt × win_scale
    //   phys.y = y_pt × win_scale
    let (phys_x, phys_y) = (x_pt * win_scale, y_pt * win_scale);
    let cur = app.cursor_position().ok();
    let cur_pt = cur.and_then(|c| {
        app.primary_monitor()
            .ok()
            .flatten()
            .map(|p| format!("({},{})", c.x / p.scale_factor(), c.y / p.scale_factor()))
    });
    eprintln!(
        "[tray] click=({},{}) s_icon={m_s:.1} monitor=({m_x},{m_y},{m_w}x{m_h}) cursor_pt={cur:?} s_win={win_scale:.1} -> pt=({x_pt},{y_pt}) phys=({phys_x},{phys_y})",
        click.x, click.y, m_s = s_icon
    );
    let _ = panel.set_position(PhysicalPosition::new(phys_x as i32, phys_y as i32));
    let _ = cur_pt;
}

/// 非 macOS（Windows）：全局物理像素空间是统一坐标系，直接按物理坐标计算。
#[cfg(not(target_os = "macos"))]
fn position_panel(
    app: &AppHandle,
    panel: &tauri::WebviewWindow,
    click: PhysicalPosition<f64>,
    rect: Option<Rect>,
) {
    let Some(m) = monitor_containing(app, click.x, click.y) else {
        return;
    };
    let scale = m.scale_factor();
    let win_w = PANEL_WINDOW_W * scale;
    let (anchor_right, anchor_top, anchor_bottom) = match rect {
        Some(r) => {
            let p: tauri::PhysicalPosition<i32> = r.position.to_physical(scale);
            let s: tauri::PhysicalSize<u32> = r.size.to_physical(scale);
            (
                p.x as f64 + s.width as f64,
                p.y as f64,
                p.y as f64 + s.height as f64,
            )
        }
        None => (click.x, click.y - 4.0, click.y),
    };

    let mut x = anchor_right - 8.0 * scale - (PANEL_WINDOW_W - 12.0) * scale;
    let mp = m.position();
    let ms = m.size();
    let min_x = mp.x as f64;
    let max_x = mp.x as f64 + ms.width as f64 - win_w;
    if max_x > min_x {
        x = x.clamp(min_x, max_x);
    }

    let monitor_mid_y = mp.y as f64 + ms.height as f64 / 2.0;
    let panel_h = panel
        .outer_size()
        .map(|s| s.height as f64)
        .unwrap_or(420.0 * scale);
    let pad = 12.0 * scale;
    let max_y = mp.y as f64 + ms.height as f64;
    let y = if anchor_bottom > monitor_mid_y {
        (anchor_top - panel_h + pad).max(mp.y as f64)
    } else {
        (anchor_bottom - pad).min(max_y)
    };

    let _ = panel.set_position(PhysicalPosition::new(x as i32, y as i32));
}

/// 物理坐标落在哪个显示器上（Windows 路径用）；都不含时取距离最近的
#[cfg(not(target_os = "macos"))]
fn monitor_containing(app: &AppHandle, x: f64, y: f64) -> Option<tauri::Monitor> {
    let monitors = app.available_monitors().ok()?;
    monitors.into_iter().min_by(|a, b| {
        fn dist(m: &tauri::Monitor, x: f64, y: f64) -> f64 {
            let (mp, ms) = (m.position(), m.size());
            let (mx, my) = (mp.x as f64, mp.y as f64);
            let dx = if x < mx {
                mx - x
            } else if x > mx + ms.width as f64 {
                x - (mx + ms.width as f64)
            } else {
                0.0
            };
            let dy = if y < my {
                my - y
            } else if y > my + ms.height as f64 {
                y - (my + ms.height as f64)
            } else {
                0.0
            };
            dx * dx + dy * dy
        }
        dist(a, x, y)
            .partial_cmp(&dist(b, x, y))
            .unwrap_or(std::cmp::Ordering::Equal)
    })
}

// ---------- 菜单 ----------

pub fn build_menu(app: &AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    let (accounts, selected, usage) = {
        let g = crate::state::shared().lock().expect("state poisoned");
        (
            g.accounts.clone(),
            g.selected_account_id.clone(),
            g.usage.clone(),
        )
    };

    let mut sub = SubmenuBuilder::new(app, "切换账号");
    if accounts.is_empty() {
        sub = sub.item(&MenuItem::with_id(
            app,
            "no-acc",
            "（未配置账号）",
            false,
            None::<&str>,
        )?);
    }
    for a in &accounts {
        let label = match usage.get(&a.id).and_then(|u| u.five_hour_used) {
            Some(p) => format!("{} · {}%", a.name, p.round() as i64),
            None => a.name.clone(),
        };
        let checked = Some(&a.id) == selected.as_ref();
        let item = CheckMenuItemBuilder::with_id(format!("switch:{}", a.id), label)
            .checked(checked)
            .build(app)?;
        sub = sub.item(&item);
    }
    let switch = sub.build()?;

    MenuBuilder::new(app)
        .item(&switch)
        .separator()
        .item(&MenuItem::with_id(
            app,
            "refresh",
            "刷新数据",
            true,
            None::<&str>,
        )?)
        .item(&MenuItem::with_id(
            app,
            "settings",
            "设置…",
            true,
            None::<&str>,
        )?)
        .separator()
        .item(&MenuItem::with_id(
            app,
            "quit",
            "退出 GLM Monitor",
            true,
            None::<&str>,
        )?)
        .build()
}

/// 账号列表或选中状态变化后重建右键菜单
pub fn rebuild_menu(app: &AppHandle) {
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        if let Ok(menu) = build_menu(app) {
            let _ = tray.set_menu(Some(menu));
        }
    }
}

// ---------- 托盘外观 ----------

fn human_duration(secs: i64) -> String {
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    if h > 0 {
        format!("{h}h {m}m")
    } else {
        format!("{m}m")
    }
}

fn tooltip_text(name: Option<&str>, usage: Option<&Usage>) -> String {
    let Some(name) = name else {
        return "GLM 额度监控\n未配置账号".to_string();
    };
    let Some(u) = usage else {
        return format!("{name} · 暂无数据");
    };
    let Some(p) = u.five_hour_used else {
        return format!(
            "{name} · 获取失败\n{}",
            u.error.as_deref().unwrap_or("请稍后重试")
        );
    };
    let weekly = match u.weekly_used {
        Some(w) => format!("周额度 已用 {}%", w.round() as i64),
        None => "周额度 --".to_string(),
    };
    let reset = match u.five_hour_reset_at {
        Some(t) => {
            let remain = (t - now_secs()).max(0);
            format!("{} 后重置", human_duration(remain))
        }
        None => "重置时间未知".to_string(),
    };
    format!("{name} · 5h 已用 {}%\n{weekly}\n{reset}", p.round() as i64)
}

/// 状态变化后更新托盘图标 / 标题 / 悬停提示
pub fn update(app: &AppHandle) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return;
    };
    let (sel_name, usage) = {
        let g = crate::state::shared().lock().expect("state poisoned");
        match g.selected_account() {
            Some(acc) => (
                Some(acc.name.clone()),
                Some(g.usage.get(&acc.id).cloned().unwrap_or_default()),
            ),
            None => (None, None),
        }
    };

    let pct = usage.as_ref().and_then(|u| u.five_hour_used);
    let _ = tray.set_icon(Some(ui::tray_icon(pct)));

    // Windows 托盘不支持 title 文字，靠图标+tooltip 表达
    #[cfg(target_os = "macos")]
    {
        let title = match pct {
            Some(p) => format!("G·{}%", p.round() as i64),
            None => "G·--".to_string(),
        };
        let _ = tray.set_title(Some(title));
    }

    let _ = tray.set_tooltip(Some(tooltip_text(sel_name.as_deref(), usage.as_ref())));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn human_duration_format() {
        assert_eq!(human_duration(8040), "2h 14m"); // 2h14m
        assert_eq!(human_duration(45), "0m");
        assert_eq!(human_duration(3600), "1h 0m");
        assert_eq!(human_duration(0), "0m");
    }

    #[test]
    fn tooltip_without_account() {
        assert!(tooltip_text(None, None).contains("未配置账号"));
    }
}
