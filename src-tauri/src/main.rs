//! GLM Monitor —— GLM Coding Plan 多账号额度菜单栏监控（Tauri 2）

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod api;
mod commands;
mod poller;
mod state;
mod store;
mod tray;
mod ui;

use tauri::{Manager, RunEvent, WindowEvent};

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_store::Builder::new().build())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .setup(|app| {
            // 菜单栏工具：不显示 Dock 图标（macOS）
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            state::init(app)?;
            tray::init(app)?;
            poller::start(app.handle().clone());

            // 启动后立即拉取一次，托盘尽快出数
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = commands::refresh_all_impl(&handle).await {
                    eprintln!("[startup] 首次刷新失败: {e}");
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_state,
            commands::refresh_all,
            commands::select_account,
            commands::save_account,
            commands::delete_account,
            commands::set_refresh_interval,
            commands::set_launch_at_startup,
            commands::open_settings,
            commands::resize_panel,
            commands::hide_panel,
        ])
        .on_window_event(|window, event| {
            // 面板失焦自动隐藏（点击外部关闭）
            if window.label() == "panel" {
                if let WindowEvent::Focused(false) = event {
                    tray::on_panel_blur(window.app_handle());
                }
            }
        })
        .build(tauri::generate_context!())
        .expect("failed to build tauri application")
        .run(|_app, event| {
            // 所有窗口关闭后仍保留托盘常驻（退出走托盘菜单）
            if let RunEvent::ExitRequested {
                code: None, api, ..
            } = event
            {
                api.prevent_exit();
            }
        });
}
