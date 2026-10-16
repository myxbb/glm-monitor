# GLM Monitor — GLM Coding Plan 多账号额度菜单栏监控

跨平台（macOS + Windows）菜单栏/托盘小工具，监控多个 GLM 账号的 Coding Plan
额度消耗。Tauri 2 + Rust + 原生前端，安装包小（目标 < 15MB）。

## 功能

- **菜单栏常驻**（macOS）：`● 69%` —— 圆点图标颜色随 5h 已用额度变化
  （≤50% 绿 / ≤80% 黄 / >80% 红 / 无数据灰），文字为当前选中账号的 5h 已用百分比
- **Windows 托盘**：应用 Logo + 状态色角标（托盘区不支持文字），面板内显示
  完整 `GLM·69%`
- **点击图标展开面板**：所有账号的 5h 已用百分比、进度条，副行为
  周额度 / MCP 额度 / 重置倒计时
- **单击账号行**切换菜单栏展示的账号（面板不关闭）；右键托盘图标也可快速切换
- **手动刷新**（⟳，带旋转动画）；面板打开时自动刷新一次
- **后台自动刷新**：默认 5 分钟（可在设置中调整 1~60 分钟），面板关闭时照常轮询
- **深浅色模式**自动跟随系统；接口失败时优雅降级（保留缓存值并标注，百分比显示 `--`）
- **开机自启**可配置
- API Key 存放在**系统钥匙串**（macOS Keychain / Windows 凭据管理器），不落盘

## 开发

需要 Rust（`rustup`）与 macOS 的 Xcode Command Line Tools / Windows 的 MSVC 工具链。

```bash
# 方式一：直接用 cargo（无需 npm）
cd src-tauri && cargo run

# 方式二：用 tauri CLI
npm install
npm run dev
```

单元测试（接口解析、状态色映射、图标生成）：

```bash
cd src-tauri && cargo test
```

## 构建

```bash
npm run build            # 或: cargo install tauri-cli --version "^2" && cargo tauri build
```

产物在 `src-tauri/target/release/bundle/`（macOS: `.app`/`.dmg`；Windows: `.msi`/`.exe`）。

## 使用与凭证获取

1. 首次运行后点击 ⚙️ 打开设置，添加账号（名称 + API Key）。
2. **只需 API Key**（智谱开放平台的 Key，即 Claude Code/GLM Coding 配置的那个）。

## 常见问题

### macOS 提示「"GLM Monitor"已损坏，无法打开。你应该将它移到废纸篓」

Release 中的安装包未经开发者签名与公证，macOS 会给下载安装的应用附加隔离（quarantine）属性，导致 Gatekeeper 误报"已损坏"。应用本身没有问题，在终端执行以下命令清除隔离属性后即可正常打开：

```bash
xattr -cr "/Applications/GLM Monitor.app"
```

> 需要 macOS 15+（Sequoia）的旧写法 `sudo xattr -rd com.apple.quarantine ...` 在新系统中可能无效，建议使用上面的完整清除写法。

若仍无法打开，可在「系统设置 → 隐私与安全性」页面底部点击「仍要打开」。

## 数据与存储

| 数据 | 位置 |
|------|------|
| 账号配置（不含 Key）、选中账号、刷新间隔、自启开关 | `config.json`（tauri-plugin-store） |
| 各账号额度缓存 | `cache.json`（独立存储，减少写盘） |
| API Key | 系统钥匙串（keyring crate） |

## 接口说明（非官方）

- 首选 `GET https://open.bigmodel.cn/api/monitor/usage/quota/limit`，
  请求头 `Authorization: Bearer {apiKey}`
- 兜底（配置了 org/project 时）`POST` 同地址，`Authorization: {apiKey}`（raw），
  请求体 `{"type": 1|2}`
- 响应中 `limits[].percentage` 即已用百分比（0-100），`nextResetTime` 为毫秒时间戳；
  5h 窗口 = `unit:3`，周窗口 = `unit:1,number:7` 或 `unit:6,number:1`，
  MCP 额度 = `type:TIME_LIMIT` 条目

该接口非官方公开接口，可能变更；解析失败时错误会写入 stderr（`cargo run` 可见），
面板与菜单栏均按"无数据"降级，不会崩溃。

## 目录结构

```
glm-monitor/
├── src/                      # 前端（原生 HTML/CSS/JS，无框架）
│   ├── index.html            # 菜单栏面板
│   ├── main.js
│   ├── settings.html         # 设置窗口
│   ├── settings.js
│   └── style.css
├── src-tauri/
│   ├── src/
│   │   ├── main.rs           # 入口：插件、托盘、轮询初始化
│   │   ├── state.rs          # 全局状态与快照
│   │   ├── store.rs          # 配置/缓存持久化 + 钥匙串
│   │   ├── api.rs            # GLM 接口调用与解析（含单元测试）
│   │   ├── tray.rs           # 托盘图标/菜单/面板开合定位
│   │   ├── ui.rs             # 纯代码生成状态图标（圆点/进度环）
│   │   ├── poller.rs         # 后台定时轮询
│   │   └── commands.rs       # 前端 invoke 命令
│   ├── capabilities/
│   └── tauri.conf.json
├── scripts/gen_icons.py      # 应用图标生成（macOS）
└── package.json              # 仅提供 tauri CLI 便捷脚本
```
