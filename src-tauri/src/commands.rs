//! 前端 invoke 命令与内部共享逻辑。

use tauri::{AppHandle, LogicalSize, Manager, PhysicalPosition, WebviewUrl, WebviewWindowBuilder};

use crate::state::{self, new_account_id, now_secs, Account, Snapshot};
use crate::store;

/// 获取状态锁（全局 'static，守卫可跨语句持有）
fn lock_inner() -> std::sync::MutexGuard<'static, state::Inner> {
    state::shared().lock().expect("state poisoned")
}

/// 保存当前配置（账号 / 选中 / 间隔 / 自启）到 config.json
fn persist_config(app: &AppHandle) -> Result<(), String> {
    let (accounts, selected, interval, launch) = {
        let g = lock_inner();
        (
            g.accounts.clone(),
            g.selected_account_id.clone(),
            g.refresh_interval_secs,
            g.launch_at_startup,
        )
    };
    store::save_config(app, &accounts, selected.as_deref(), interval, launch)
}

fn sync_ui(app: &AppHandle, rebuild_menu: bool) {
    if rebuild_menu {
        crate::tray::rebuild_menu(app);
    }
    crate::tray::update(app);
    state::emit_state(app);
}

// ---------- 查询 ----------

#[tauri::command]
pub fn get_state(app: AppHandle) -> Snapshot {
    state::snapshot(&app)
}

// ---------- 刷新 ----------

#[tauri::command]
pub async fn refresh_all(app: AppHandle) -> Result<Snapshot, String> {
    refresh_all_impl(&app).await?;
    Ok(state::snapshot(&app))
}

/// 并发刷新所有账号；单账号失败保留旧缓存值并记录错误，不中断整体流程
pub async fn refresh_all_impl(app: &AppHandle) -> Result<(), String> {
    let accounts: Vec<Account> = {
        let g = lock_inner();
        g.accounts.clone()
    };

    {
        use tauri::Emitter;
        let _ = app.emit("refresh-started", ());
    }

    if accounts.is_empty() {
        sync_ui(app, false);
        return Ok(());
    }

    // 并发拉取（钥匙串读取是阻塞调用，放到阻塞线程池）
    let mut jobs = Vec::with_capacity(accounts.len());
    for acc in accounts {
        jobs.push(tauri::async_runtime::spawn(async move {
            // 钥匙串读取是阻塞调用，放到阻塞线程池
            let id_for_key = acc.id.clone();
            let key_result =
                match tauri::async_runtime::spawn_blocking(move || store::get_api_key(&id_for_key))
                    .await
                {
                    Ok(r) => r,
                    Err(e) => Err(format!("读取钥匙串任务失败: {e}")),
                };
            let result = match key_result {
                Ok(k) if !k.trim().is_empty() => {
                    crate::api::fetch_usage(&k, &acc.org, &acc.project, acc.account_type).await
                }
                Ok(_) => Err("未配置 API Key".to_string()),
                Err(e) => Err(e),
            };
            (acc.id, result)
        }));
    }

    let mut results = Vec::with_capacity(jobs.len());
    for j in jobs {
        match j.await {
            Ok(pair) => results.push(pair),
            Err(e) => results.push((String::new(), Err(format!("刷新任务异常: {e}")))),
        }
    }

    let now = now_secs();
    {
        let mut g = lock_inner();
        for (id, result) in results {
            if id.is_empty() {
                continue;
            }
            match result {
                Ok(mut u) => {
                    u.updated_at = Some(now);
                    g.usage.insert(id, u);
                }
                Err(e) => {
                    // 失败时保留上次缓存数值，仅标记错误
                    let entry = g.usage.entry(id).or_default();
                    entry.error = Some(e);
                }
            }
        }
        g.last_refresh_at = Some(now);
    }

    if let Err(e) = store::save_cache(app, &lock_inner().usage.clone()) {
        eprintln!("[refresh] 保存缓存失败: {e}");
    }
    sync_ui(app, true); // 菜单里的账号百分比也随刷新更新
    Ok(())
}

// ---------- 账号管理 ----------

#[tauri::command]
pub fn select_account(app: AppHandle, id: String) -> Result<(), String> {
    select_account_impl(&app, id)
}

pub fn select_account_impl(app: &AppHandle, id: String) -> Result<(), String> {
    {
        let mut g = lock_inner();
        if !g.accounts.iter().any(|a| a.id == id) {
            return Err("账号不存在".into());
        }
        g.selected_account_id = Some(id);
    }
    persist_config(app)?;
    sync_ui(app, true);
    Ok(())
}

/// 新增或更新账号。id 为 None 表示新增；apiKey 为 None 表示编辑时保持不变。
#[tauri::command]
pub fn save_account(
    app: AppHandle,
    id: Option<String>,
    name: String,
    api_key: Option<String>,
    org: String,
    project: String,
    account_type: u8,
) -> Result<Account, String> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("账号名称不能为空".into());
    }
    if account_type != 1 && account_type != 2 {
        return Err("套餐类型必须是 1（个人）或 2（团队）".into());
    }

    let acc = match id {
        Some(id) => {
            let key = api_key
                .map(|k| k.trim().to_string())
                .filter(|k| !k.is_empty());
            if let Some(k) = &key {
                store::set_api_key(&id, k)?;
            }
            let mut g = lock_inner();
            let acc = g
                .accounts
                .iter_mut()
                .find(|a| a.id == id)
                .ok_or_else(|| "账号不存在".to_string())?;
            acc.name = name;
            acc.org = org.trim().to_string();
            acc.project = project.trim().to_string();
            acc.account_type = account_type;
            acc.clone()
        }
        None => {
            let key = api_key
                .map(|k| k.trim().to_string())
                .filter(|k| !k.is_empty())
                .ok_or_else(|| "新增账号必须填写 API Key".to_string())?;
            let acc = Account {
                id: new_account_id(),
                name,
                org: org.trim().to_string(),
                project: project.trim().to_string(),
                account_type,
            };
            store::set_api_key(&acc.id, &key)?;
            let mut g = lock_inner();
            if g.selected_account_id.is_none() {
                g.selected_account_id = Some(acc.id.clone());
            }
            g.accounts.push(acc.clone());
            acc
        }
    };

    persist_config(&app)?;
    sync_ui(&app, true);
    // 新增/修改凭证后立即刷新一次，尽快给出反馈
    let h = app.clone();
    tauri::async_runtime::spawn(async move {
        let _ = refresh_all_impl(&h).await;
    });
    Ok(acc)
}

#[tauri::command]
pub fn delete_account(app: AppHandle, id: String) -> Result<(), String> {
    {
        let mut g = lock_inner();
        g.accounts.retain(|a| a.id != id);
        g.usage.remove(&id);
        if g.selected_account_id.as_deref() == Some(id.as_str()) {
            g.selected_account_id = g.accounts.first().map(|a| a.id.clone());
        }
    }
    if let Err(e) = store::delete_api_key(&id) {
        eprintln!("[delete] 删除钥匙串条目失败: {e}");
    }
    persist_config(&app)?;
    store::save_cache(&app, &lock_inner().usage.clone())?;
    sync_ui(&app, true);
    Ok(())
}

// ---------- 设置 ----------

#[tauri::command]
pub fn set_refresh_interval(app: AppHandle, minutes: u32) -> Result<(), String> {
    if !(1..=60).contains(&minutes) {
        return Err("刷新间隔必须在 1~60 分钟之间".into());
    }
    lock_inner().refresh_interval_secs = (minutes as u64) * 60;
    persist_config(&app)?;
    state::emit_state(&app);
    Ok(())
}

#[tauri::command]
pub fn set_launch_at_startup(app: AppHandle, enabled: bool) -> Result<(), String> {
    {
        use tauri_plugin_autostart::ManagerExt;
        let auto = app.autolaunch();
        if enabled {
            auto.enable()
                .map_err(|e| format!("开启开机自启失败: {e}"))?;
        } else {
            auto.disable()
                .map_err(|e| format!("关闭开机自启失败: {e}"))?;
        }
    }
    lock_inner().launch_at_startup = enabled;
    persist_config(&app)?;
    state::emit_state(&app);
    Ok(())
}

// ---------- 窗口 ----------

#[tauri::command]
pub fn open_settings(app: AppHandle) -> Result<(), String> {
    open_settings_impl(&app)
}

pub fn open_settings_impl(app: &AppHandle) -> Result<(), String> {
    if let Some(w) = app.get_webview_window("settings") {
        let _ = w.show();
        let _ = w.set_focus();
        return Ok(());
    }
    WebviewWindowBuilder::new(app, "settings", WebviewUrl::App("settings.html".into()))
        .title("GLM Monitor · 设置")
        .inner_size(460.0, 660.0)
        .min_inner_size(420.0, 560.0)
        .resizable(true)
        .build()
        .map(|_| ())
        .map_err(|e| format!("打开设置窗口失败: {e}"))
}

/// 前端按内容实际高度调整面板窗口尺寸。
/// macOS 的 setContentSize 固定左下角（窗口向上生长），
/// resize 后必须把左上角设回原位，否则面板顶边会滑离菜单栏。
#[tauri::command]
pub fn resize_panel(app: AppHandle, height: f64) -> Result<(), String> {
    let panel = app.get_webview_window("panel").ok_or("面板窗口不存在")?;
    let h = height.clamp(120.0, 1200.0);
    let top_left = panel.outer_position().map_err(|e| e.to_string())?;
    panel
        .set_size(LogicalSize::new(344.0, h))
        .map_err(|e| e.to_string())?;
    panel
        .set_position(PhysicalPosition::new(top_left.x, top_left.y))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn hide_panel(app: AppHandle) -> Result<(), String> {
    let panel = app.get_webview_window("panel").ok_or("面板窗口不存在")?;
    panel.hide().map_err(|e| e.to_string())
}
