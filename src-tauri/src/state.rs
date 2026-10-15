//! 全局应用状态：账号配置、额度缓存、选中账号与设置项。
//!
//! State 以 `Arc<Mutex<Inner>>` 形式托管在 Tauri 中，
//! 锁只在短临界区内持有（网络请求等耗时操作在锁外执行）。

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tauri::{App, AppHandle};

/// 单个 GLM 账号配置（apiKey 只存系统钥匙串，不落盘）
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Account {
    pub id: String,
    pub name: String,
    /// 组织 ID，可留空（留空时仅用 API Key 查询）
    #[serde(default)]
    pub org: String,
    /// 项目 ID，可留空
    #[serde(default)]
    pub project: String,
    /// 套餐类型：1 = 个人套餐，2 = 团队套餐
    #[serde(rename = "type", default = "default_account_type")]
    pub account_type: u8,
}

fn default_account_type() -> u8 {
    1
}

/// 单账号额度快照。全部字段 Option，接口失败时保留上次缓存值。
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Usage {
    /// 5h 额度已用百分比（0-100）
    #[serde(
        rename = "fiveHourUsed",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub five_hour_used: Option<f64>,
    /// 5h 额度重置时间（Unix 秒）
    #[serde(
        rename = "fiveHourResetAt",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub five_hour_reset_at: Option<i64>,
    /// 周额度已用百分比（0-100）
    #[serde(
        rename = "weeklyUsed",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub weekly_used: Option<f64>,
    /// 上次成功获取时间（Unix 秒）
    #[serde(rename = "updatedAt", default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<i64>,
    /// 最近一次请求的错误信息（成功时为 None；有值时保留旧数值）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// 可变内核
pub struct Inner {
    pub accounts: Vec<Account>,
    pub selected_account_id: Option<String>,
    /// 轮询间隔（秒）
    pub refresh_interval_secs: u64,
    pub launch_at_startup: bool,
    pub usage: HashMap<String, Usage>,
    pub last_refresh_at: Option<i64>,
}

impl Inner {
    pub fn selected_account(&self) -> Option<&Account> {
        let id = self.selected_account_id.as_ref()?;
        self.accounts.iter().find(|a| &a.id == id)
    }
}

/// 进程级全局状态（setup 时初始化一次）。
/// 相比 Tauri 的 `app.state()`，全局 'static 引用让锁守卫可以自由跨越语句，
/// 避免在命令/异步任务间传递生命周期。
pub static STATE: OnceLock<Arc<Mutex<Inner>>> = OnceLock::new();

/// 获取状态句柄（setup 之前调用会 panic）
pub fn shared() -> &'static Arc<Mutex<Inner>> {
    STATE.get().expect("app state not initialized")
}

/// 推送给前端的完整快照（不含 apiKey）
#[derive(Serialize, Clone, Debug)]
pub struct Snapshot {
    pub accounts: Vec<Account>,
    pub usage: HashMap<String, Usage>,
    #[serde(rename = "selectedAccountId")]
    pub selected_account_id: Option<String>,
    #[serde(rename = "refreshIntervalMin")]
    pub refresh_interval_min: u32,
    #[serde(rename = "launchAtStartup")]
    pub launch_at_startup: bool,
    #[serde(rename = "lastRefreshAt")]
    pub last_refresh_at: Option<i64>,
}

pub fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 生成账号 ID
pub fn new_account_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    format!("acc_{}_{}", now_ms(), n)
}

pub fn snapshot(_app: &AppHandle) -> Snapshot {
    let g = shared().lock().expect("state poisoned");
    Snapshot {
        accounts: g.accounts.clone(),
        usage: g.usage.clone(),
        selected_account_id: g.selected_account_id.clone(),
        refresh_interval_min: (g.refresh_interval_secs / 60).max(1) as u32,
        launch_at_startup: g.launch_at_startup,
        last_refresh_at: g.last_refresh_at,
    }
}

/// 状态变化后广播给所有窗口
pub fn emit_state(app: &AppHandle) {
    use tauri::Emitter;
    let _ = app.emit("state-changed", snapshot(app));
}

/// 应用启动时初始化状态：读配置与缓存、校正选中账号、同步开机自启
pub fn init(app: &mut App) -> tauri::Result<()> {
    let handle = app.handle();

    let cfg = crate::store::load_config(handle);
    let cache = crate::store::load_cache(handle);

    let mut inner = Inner {
        accounts: cfg.accounts,
        selected_account_id: cfg.selected,
        refresh_interval_secs: cfg.refresh_interval_secs,
        launch_at_startup: cfg.launch_at_startup,
        usage: cache,
        last_refresh_at: None,
    };

    // 选中账号失效（被删除或首次启动）时回退到第一个账号
    let selected_valid = inner
        .selected_account_id
        .as_deref()
        .is_some_and(|id| inner.accounts.iter().any(|a| a.id == id));
    if !selected_valid {
        inner.selected_account_id = inner.accounts.first().map(|a| a.id.clone());
    }

    // 让系统的开机自启状态与配置保持一致
    {
        use tauri_plugin_autostart::ManagerExt;
        let auto = handle.autolaunch();
        if inner.launch_at_startup {
            let _ = auto.enable();
        } else {
            let _ = auto.disable();
        }
    }

    let _ = STATE.set(Arc::new(Mutex::new(inner)));
    Ok(())
}
