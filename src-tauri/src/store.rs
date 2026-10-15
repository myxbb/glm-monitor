//! 持久化：配置与额度缓存（tauri-plugin-store），API Key（系统钥匙串）。
//!
//! - config.json：账号元信息、选中账号、刷新间隔、开机自启
//! - cache.json：各账号额度缓存（独立存储，避免频繁写配置）
//! - apiKey：macOS Keychain / Windows 凭据管理器（keyring），绝不落盘

use std::collections::HashMap;
use std::sync::Arc;

use tauri::AppHandle;
use tauri_plugin_store::{Store, StoreExt};

use crate::state::{Account, Usage};

const CONFIG_FILE: &str = "config.json";
const CACHE_FILE: &str = "cache.json";
const KEYRING_SERVICE: &str = "glm-monitor";

pub struct Config {
    pub accounts: Vec<Account>,
    pub selected: Option<String>,
    pub refresh_interval_secs: u64,
    pub launch_at_startup: bool,
}

fn open(app: &AppHandle, file: &str) -> Result<Arc<Store<tauri::Wry>>, String> {
    app.store(file)
        .map_err(|e| format!("打开本地存储 {file} 失败: {e}"))
}

// ---------- 配置 ----------

pub fn load_config(app: &AppHandle) -> Config {
    let defaults = || Config {
        accounts: Vec::new(),
        selected: None,
        refresh_interval_secs: 300,
        launch_at_startup: false,
    };
    let Ok(store) = open(app, CONFIG_FILE) else {
        return defaults();
    };
    let accounts = store
        .get("accounts")
        .and_then(|v| serde_json::from_value::<Vec<Account>>(v).ok())
        .unwrap_or_default();
    let selected = store
        .get("selectedAccountId")
        .and_then(|v| v.as_str().map(|s| s.to_string()));
    let refresh_interval_secs = store
        .get("refreshInterval")
        .and_then(|v| v.as_u64())
        .unwrap_or(300)
        .clamp(60, 3600);
    let launch_at_startup = store
        .get("launchAtStartup")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    Config {
        accounts,
        selected,
        refresh_interval_secs,
        launch_at_startup,
    }
}

pub fn save_config(
    app: &AppHandle,
    accounts: &[Account],
    selected: Option<&str>,
    refresh_interval_secs: u64,
    launch_at_startup: bool,
) -> Result<(), String> {
    let store = open(app, CONFIG_FILE)?;
    let accounts_value =
        serde_json::to_value(accounts).map_err(|e| format!("序列化账号失败: {e}"))?;
    let selected_value = match selected {
        Some(s) => serde_json::Value::String(s.to_string()),
        None => serde_json::Value::Null,
    };
    store.set("accounts", accounts_value);
    store.set("selectedAccountId", selected_value);
    store.set("refreshInterval", serde_json::json!(refresh_interval_secs));
    store.set("launchAtStartup", serde_json::json!(launch_at_startup));
    store.save().map_err(|e| format!("保存配置失败: {e}"))
}

// ---------- 额度缓存 ----------

pub fn load_cache(app: &AppHandle) -> HashMap<String, Usage> {
    let mut map = HashMap::new();
    let Ok(store) = open(app, CACHE_FILE) else {
        return map;
    };
    for (k, v) in store.entries() {
        if let Ok(u) = serde_json::from_value::<Usage>(v) {
            map.insert(k, u);
        }
    }
    map
}

pub fn save_cache(app: &AppHandle, usage: &HashMap<String, Usage>) -> Result<(), String> {
    let store = open(app, CACHE_FILE)?;
    // 先删除已不存在的账号条目，再全量写入
    for (k, _) in store.entries() {
        if !usage.contains_key(&k) {
            let _ = store.delete(k);
        }
    }
    for (id, u) in usage {
        let v = serde_json::to_value(u).map_err(|e| format!("序列化缓存失败: {e}"))?;
        store.set(id, v);
    }
    store.save().map_err(|e| format!("保存缓存失败: {e}"))
}

// ---------- API Key（系统钥匙串） ----------

pub fn set_api_key(account_id: &str, key: &str) -> Result<(), String> {
    keyring::Entry::new(KEYRING_SERVICE, account_id)
        .and_then(|e| e.set_password(key))
        .map_err(|e| format!("写入系统钥匙串失败: {e}"))
}

pub fn get_api_key(account_id: &str) -> Result<String, String> {
    keyring::Entry::new(KEYRING_SERVICE, account_id)
        .and_then(|e| e.get_password())
        .map_err(|e| format!("读取系统钥匙串失败（账号可能未配置 API Key）: {e}"))
}

pub fn delete_api_key(account_id: &str) -> Result<(), String> {
    match keyring::Entry::new(KEYRING_SERVICE, account_id) {
        Ok(entry) => match entry.delete_credential() {
            Ok(()) => Ok(()),
            // 条目本就不存在时视为成功
            Err(e) if format!("{e:?}").contains("NoEntry") => Ok(()),
            Err(e) => Err(format!("删除钥匙串条目失败: {e}")),
        },
        Err(e) => Err(format!("访问系统钥匙串失败: {e}")),
    }
}
