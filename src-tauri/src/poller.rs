//! 后台定时轮询：按用户配置的间隔刷新全部账号（面板关闭时依然刷新）。

use std::time::Duration;

use tauri::AppHandle;

use crate::state;

pub fn start(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            // 每轮读取最新间隔；改设置后最迟在旧周期结束后生效
            let secs = state::shared()
                .lock()
                .expect("state poisoned")
                .refresh_interval_secs;
            tokio::time::sleep(Duration::from_secs(secs)).await;
            if let Err(e) = crate::commands::refresh_all_impl(&app).await {
                eprintln!("[poller] 刷新失败: {e}");
            }
        }
    });
}
