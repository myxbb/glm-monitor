//! GLM Coding Plan 额度接口调用与解析。
//!
//! 接口（非官方，可能变更，必须容错）：
//! - 首选：`GET https://open.bigmodel.cn/api/monitor/usage/quota/limit`
//!   请求头 `Authorization: Bearer {apiKey}`（可选附带 org/project 头）
//! - 兜底：配置了 org/project 时改用 `POST`，`Authorization: {apiKey}`（raw），
//!   请求体 `{"type": 1|2}`（浏览器 DevTools 抓到的站点自身调用方式）
//!
//! 响应两种形态：
//! - v1/v2 套餐：`{"code":200,"data":{"limits":[...],"level":"..."}}`
//! - v3 积分套餐：顶层数组 `[...]`
//!
//! limits 单项结构：
//! ```json
//! {
//!   "type": "TOKENS_LIMIT" | "CREDIT_LIMIT" | "TIME_LIMIT",
//!   "unit": 1,           // 重置周期单位：1=天 3=小时 5=月 6=周
//!   "number": 5,         // 周期数值
//!   "usage": 120,        // 总额度（积分制）
//!   "currentValue": 30,  // 已用（积分制）
//!   "percentage": 25,    // 已用百分比 0-100
//!   "nextResetTime": 1730000000000  // 毫秒时间戳
//! }
//! ```
//! 5h 窗口 = TOKENS/CREDIT + unit=3；周窗口 = (unit=1,number=7) 或 (unit=6,number=1)。
//! MCP 额度 = TIME_LIMIT 条目（有 percentage 的首个）。

use std::sync::OnceLock;
use std::time::Duration;

use serde_json::Value;

use crate::state::Usage;

pub const ENDPOINT: &str = "https://open.bigmodel.cn/api/monitor/usage/quota/limit";

static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

fn client() -> &'static reqwest::Client {
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            // 接口对非常规 User-Agent 可能有限制，伪装成浏览器 UA
            .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.4 Safari/605.1.15")
            .build()
            .expect("failed to build reqwest client")
    })
}

pub async fn fetch_usage(
    api_key: &str,
    org: &str,
    project: &str,
    account_type: u8,
) -> Result<Usage, String> {
    let key = api_key.trim();
    if key.is_empty() {
        return Err("未配置 API Key".into());
    }

    // 首选 GET + Bearer（仅凭 API Key 即可查询）
    let mut resp = send_get(key, org, project).await;
    // 失败且配置了 org/project 时，尝试 POST 抓包形式
    if resp.is_err() && !org.trim().is_empty() && !project.trim().is_empty() {
        resp = send_post(key, org, project, account_type).await;
    }
    let body = resp?;

    let v: Value = serde_json::from_str(&body).map_err(|e| format!("响应不是合法 JSON: {e}"))?;
    match parse_usage(&v) {
        Ok(u) => Ok(u),
        Err(e) => {
            eprintln!("[api] 解析失败: {e}；原始响应: {}", truncate(&body, 600));
            Err(e)
        }
    }
}

async fn send_get(key: &str, org: &str, project: &str) -> Result<String, String> {
    let mut req = client()
        .get(ENDPOINT)
        .header("Authorization", format!("Bearer {key}"));
    if !org.trim().is_empty() {
        req = req.header("bigmodel-organization", org.trim());
    }
    if !project.trim().is_empty() {
        req = req.header("bigmodel-project", project.trim());
    }
    let resp = req.send().await.map_err(|e| format!("请求失败: {e}"))?;
    let status = resp.status();
    let body = resp
        .text()
        .await
        .map_err(|e| format!("读取响应失败: {e}"))?;
    check_status(status, &body)?;
    Ok(body)
}

async fn send_post(
    key: &str,
    org: &str,
    project: &str,
    account_type: u8,
) -> Result<String, String> {
    let resp = client()
        .post(ENDPOINT)
        // 注意：raw 形式，不加 "Bearer " 前缀
        .header("Authorization", key)
        .header("bigmodel-organization", org.trim())
        .header("bigmodel-project", project.trim())
        .json(&serde_json::json!({ "type": account_type as i32 }))
        .send()
        .await
        .map_err(|e| format!("请求失败: {e}"))?;
    let status = resp.status();
    let body = resp
        .text()
        .await
        .map_err(|e| format!("读取响应失败: {e}"))?;
    check_status(status, &body)?;
    Ok(body)
}

fn check_status(status: reqwest::StatusCode, body: &str) -> Result<(), String> {
    if !status.is_success() {
        let reason = match status.as_u16() {
            401 | 403 => "API Key 无效或已过期",
            429 => "请求过于频繁（被限流）",
            _ => "服务暂时不可用",
        };
        return Err(format!(
            "HTTP {}（{}）: {}",
            status.as_u16(),
            reason,
            truncate(body, 120)
        ));
    }
    Ok(())
}

fn truncate(s: &str, max: usize) -> String {
    let t: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if t.chars().count() > max {
        format!("{}…", t.chars().take(max).collect::<String>())
    } else {
        t
    }
}

// ---------- 解析 ----------

fn item_str<'a>(item: &'a Value, key: &str) -> Option<&'a str> {
    item.get(key).and_then(Value::as_str)
}

fn item_i64(item: &Value, key: &str) -> Option<i64> {
    item.get(key).and_then(Value::as_i64)
}

fn item_f64(item: &Value, key: &str) -> Option<f64> {
    item.get(key).and_then(Value::as_f64)
}

/// 是否为额度类条目（排除 MCP 的 TIME_LIMIT）
fn is_quota_limit(item: &Value) -> bool {
    matches!(
        item_str(item, "type"),
        Some("TOKENS_LIMIT") | Some("CREDIT_LIMIT")
    )
}

/// MCP 额度条目
fn is_mcp_limit(item: &Value) -> bool {
    item_str(item, "type") == Some("TIME_LIMIT")
}

/// 5 小时窗口：unit=3（小时）
fn is_five_hour(item: &Value) -> bool {
    is_quota_limit(item) && item_i64(item, "unit") == Some(3)
}

/// 周窗口：v1/v2 编码为 unit=1(天)×number=7；v3 积分制为 unit=6(周)×number=1
fn is_weekly(item: &Value) -> bool {
    matches!(
        (item_i64(item, "unit"), item_i64(item, "number")),
        (Some(1), Some(7)) | (Some(6), Some(1))
    )
}

/// 归一化 limits 数组（兼容 v1/v2 包裹结构与 v3 顶层数组）
fn limits_of(v: &Value) -> Result<Vec<&Value>, String> {
    if let Some(arr) = v.as_array() {
        if arr.is_empty() {
            return Err("账户暂无 Coding Plan 额度（可能未订阅）".into());
        }
        return Ok(arr.iter().collect());
    }
    if let Some(code) = v.get("code").and_then(Value::as_i64) {
        if code != 200 {
            let msg = v.get("msg").and_then(Value::as_str).unwrap_or("未知错误");
            if code == 401 || code == 403 {
                return Err(format!("API Key 无效或已过期（{msg}）"));
            }
            return Err(format!("额度接口错误：{msg}"));
        }
    }
    match v.pointer("/data/limits").and_then(Value::as_array) {
        Some(arr) if !arr.is_empty() => Ok(arr.iter().collect()),
        _ => Err("响应中没有 limits 数据（账户可能未订阅 Coding Plan）".into()),
    }
}

pub fn parse_usage(v: &Value) -> Result<Usage, String> {
    let limits = limits_of(v)?;

    // 主窗口（5h）：优先 unit=3 的 TOKENS/CREDIT 项，兜底取首个 TOKENS/CREDIT 项
    let five = limits
        .iter()
        .find(|i| is_five_hour(i))
        .or_else(|| limits.iter().find(|i| is_quota_limit(i)));
    // 周窗口：优先同属额度类的周窗口项；兜底时排除 MCP 的 TIME_LIMIT
    let week = limits
        .iter()
        .find(|i| is_weekly(i) && is_quota_limit(i))
        .or_else(|| {
            limits
                .iter()
                .find(|i| is_weekly(i) && item_str(i, "type") != Some("TIME_LIMIT"))
        });

    let five_used = five
        .and_then(|i| item_f64(i, "percentage"))
        .map(|p| p.clamp(0.0, 100.0));
    let week_used = week
        .and_then(|i| item_f64(i, "percentage"))
        .map(|p| p.clamp(0.0, 100.0));
    // nextResetTime 为毫秒时间戳
    let reset_at = five
        .and_then(|i| item_f64(i, "nextResetTime"))
        .map(|ms| (ms / 1000.0).round() as i64);
    // MCP 额度：首个带 percentage 的 TIME_LIMIT 条目
    let mcp_used = limits
        .iter()
        .find(|i| is_mcp_limit(i) && item_f64(i, "percentage").is_some())
        .and_then(|i| item_f64(i, "percentage"))
        .map(|p| p.clamp(0.0, 100.0));

    if five_used.is_none() && week_used.is_none() && mcp_used.is_none() {
        return Err("响应中未找到可识别的额度窗口".into());
    }

    Ok(Usage {
        five_hour_used: five_used,
        five_hour_reset_at: reset_at,
        weekly_used: week_used,
        mcp_used,
        updated_at: None,
        error: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_v1_wrapped() {
        // v1/v2：{code, data:{limits, level}} 包裹结构，次数额度
        let body = json!({
            "code": 200,
            "data": {
                "level": "pro",
                "limits": [
                    { "type": "TOKENS_LIMIT", "unit": 3, "number": 5,
                      "percentage": 42.5, "nextResetTime": 1730000000000i64 },
                    { "type": "TOKENS_LIMIT", "unit": 1, "number": 7,
                      "percentage": 28.0, "nextResetTime": 1730100000000i64 },
                    { "type": "TIME_LIMIT", "unit": 6, "number": 1, "percentage": 10.0 }
                ]
            }
        });
        let u = parse_usage(&body).unwrap();
        assert_eq!(u.five_hour_used, Some(42.5));
        assert_eq!(u.five_hour_reset_at, Some(1730000000));
        assert_eq!(u.weekly_used, Some(28.0));
        assert_eq!(u.mcp_used, Some(10.0));
    }

    #[test]
    fn parse_v3_top_level_array() {
        // v3 积分制：顶层数组，周窗口编码 unit=6 number=1
        let body = json!([
            { "type": "CREDIT_LIMIT", "unit": 3, "number": 5,
              "usage": 120, "currentValue": 30,
              "percentage": 25, "nextResetTime": 1730000000000i64 },
            { "type": "CREDIT_LIMIT", "unit": 6, "number": 1,
              "percentage": 9.4, "nextResetTime": 1730500000000i64 }
        ]);
        let u = parse_usage(&body).unwrap();
        assert_eq!(u.five_hour_used, Some(25.0));
        assert_eq!(u.weekly_used, Some(9.4));
        assert_eq!(u.five_hour_reset_at, Some(1730000000));
        assert_eq!(u.mcp_used, None);
    }

    #[test]
    fn parse_mcp_only() {
        // 只有 MCP（TIME_LIMIT）额度时也算有效响应
        let body = json!([
            { "type": "TIME_LIMIT", "unit": 6, "number": 1, "percentage": 66.0 }
        ]);
        let u = parse_usage(&body).unwrap();
        assert_eq!(u.five_hour_used, None);
        assert_eq!(u.weekly_used, None);
        assert_eq!(u.mcp_used, Some(66.0));
    }

    #[test]
    fn parse_weekly_as_day7() {
        // v1/v2 周额度：unit=1(天) number=7
        let body = json!([
            { "type": "TOKENS_LIMIT", "unit": 3, "number": 5, "percentage": 0.0 },
            { "type": "TOKENS_LIMIT", "unit": 1, "number": 7, "percentage": 55.0 }
        ]);
        let u = parse_usage(&body).unwrap();
        assert_eq!(u.five_hour_used, Some(0.0));
        assert_eq!(u.weekly_used, Some(55.0));
    }

    #[test]
    fn business_error_auth() {
        let body = json!({ "code": 401, "msg": "unauthorized" });
        let err = parse_usage(&body).unwrap_err();
        assert!(err.contains("API Key"), "unexpected: {err}");
    }

    #[test]
    fn empty_array_means_no_subscription() {
        let body = json!([]);
        let err = parse_usage(&body).unwrap_err();
        assert!(err.contains("未订阅"));
    }

    #[test]
    fn no_quota_window_found() {
        // 未知类型且不带任何窗口编码（unit=5=月，非 5h/周/MCP）
        let body =
            json!([{ "type": "SOMETHING_ELSE", "unit": 5, "number": 1, "percentage": 10.0 }]);
        assert!(parse_usage(&body).is_err());
    }
}
