# AI Proxy 供应商登录探测与 UI 重构 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 桌面端通过插件用 Cookie 探测各供应商登录状态，只展示已登录供应商供用户选择；把「编程工具」重构为「应用」页并提供复制命令 / 写入配置两个动作；UI 组件全面换用 shadcn/ui。

**Architecture:** 插件 background 用 `chrome.cookies` 探测并在 WS 连接、cookie 变化、收到 `refresh_providers` 时向桌面端推 `providers_status`；桌面端缓存状态，经 Tauri 命令暴露给 React 界面。代理接口与供应商队列完全不变。

**Tech Stack:** 既有 Tauri 2 + Rust + React 18 + Vite；新增 Tailwind CSS、shadcn/ui、lucide-react。

## Global Constraints

- 代码根：桌面端 `ai-proxy/desktop/`，插件 `ai-proxy/extension/`。
- 本次只做 DeepSeek 一家供应商；数据结构需支持后续扩展。
- 代理 HTTP 接口、供应商队列、限流、会话复用一律不改。
- 配置接管底层逻辑（备份/写入/还原）不改，只改 UI 入口。
- 每条 WS 状态消息覆盖旧状态，无请求/响应对应。
- `cargo test` 与 `npm run build` 必须全绿；每个任务单独提交。
- 使用 shadcn/ui 的具体安装命令以执行时 Context7 查询到的最新文档为准。

---

### Task 1: 桌面端接收 providers_status 并缓存

**Files:**
- Modify: `ai-proxy/desktop/src-tauri/src/state.rs`
- Modify: `ai-proxy/desktop/src-tauri/src/http/mod.rs`
- Modify: `ai-proxy/desktop/src-tauri/src/bridge.rs`

**Interfaces:**
- Produces: `AppState.provider_status: RwLock<HashMap<String, bool>>`。
- Produces: `AppState::update_provider_status(&self, v: &serde_json::Value)`（解析 `{"providers":[{"id":"deepseek","logged_in":true}]}`）。
- Produces: `BridgeHub::notify(&self, msg: serde_json::Value) -> bool`（向插件发送任意消息）。

- [ ] **Step 1: 写失败测试**（state.rs 底部）：

```rust
#[test]
fn parses_providers_status_message() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new(dir.path().to_path_buf());
    let msg = serde_json::json!({
        "type": "providers_status",
        "providers": [{ "id": "deepseek", "logged_in": true }]
    });
    state.update_provider_status(&msg);
    assert_eq!(*state.provider_status.read().unwrap().get("deepseek").unwrap(), true);
}
```

Run: `cd ai-proxy/desktop/src-tauri && cargo test state`
Expected: 编译失败，字段与函数不存在。

- [ ] **Step 2: 实现状态字段与更新函数**

`AppState` 增加 `pub provider_status: Arc<RwLock<HashMap<String, bool>>>`，构造时初始化空表。`update_provider_status`：遍历 `providers` 数组，把 `id` 与 `logged_in` 写入；若当前 `active_provider` 对应的登录态变为 false，把 `settings.active_provider` 置空并记录日志「供应商已登出」。

`bridge.rs` 增加：

```rust
pub fn notify(&self, msg: Value) -> bool {
    self.sink.read().unwrap().as_ref().map(|tx| tx.send(msg.to_string()).is_ok()).unwrap_or(false)
}
```

- [ ] **Step 3: WS 路由接入**

`http/mod.rs` 的 `ws_loop` 收到 `Message::Text` 后，若 JSON 的 `type` 为 `"providers_status"`，先调 `state.update_provider_status(&v)` 再交给 `bridge.handle_client(v)`。

- [ ] **Step 4: 跑测试并提交**

Run: `cd ai-proxy/desktop/src-tauri && cargo test`
Expected: 全部 PASS。

```bash
git add ai-proxy/desktop/src-tauri
git commit -m "feat(desktop): 接收并缓存供应商登录状态"
```

---

### Task 2: 供应商命令 get_providers_status / select_provider / refresh_providers

**Files:**
- Modify: `ai-proxy/desktop/src-tauri/src/commands.rs`
- Modify: `ai-proxy/desktop/src-tauri/src/settings.rs`
- Modify: `ai-proxy/desktop/src-tauri/src/lib.rs`

**Interfaces:**
- Produces: `get_providers_status() -> Vec<Value>`（每项 `{id, logged_in, enabled, min_interval_ms, active}`）。
- Produces: `select_provider(id: String) -> Result<AppSettings, String>`（仅已登录供应商可选中）。
- Produces: `refresh_providers() -> bool`（向插件发 `{"type":"refresh_providers"}`，返回是否发出）。

- [ ] **Step 1: 抽纯逻辑并写失败测试**（settings.rs）：

```rust
pub fn apply_selection(
    settings: &mut AppSettings,
    status: &HashMap<String, bool>,
    id: &str,
) -> Result<(), String> {
    if !settings.providers.contains_key(id) {
        return Err("未知供应商".into());
    }
    if !status.get(id).copied().unwrap_or(false) {
        return Err("该供应商未登录，无法选择".into());
    }
    settings.active_provider = id.to_string();
    Ok(())
}
```

测试：未知 id 报错；未登录报错；已登录成功且 `active_provider` 更新。

- [ ] **Step 2: 实现三个命令**

`get_providers_status`：合并 `settings.providers` 与 `state.provider_status`，`active` 取自 `settings.active_provider`。
`select_provider`：锁 settings → `apply_selection` → 持久化 save → 返回快照；失败返回中文错误。
`refresh_providers`：`state.bridge.notify(json!({"type":"refresh_providers"}))`。
`lib.rs` 注册三个命令。

- [ ] **Step 3: 跑测试并提交**

Run: `cd ai-proxy/desktop/src-tauri && cargo test`
Expected: 全部 PASS。

```bash
git add ai-proxy/desktop/src-tauri
git commit -m "feat(desktop): 供应商状态与选择命令"
```

---

### Task 3: Tailwind 与 shadcn/ui 脚手架

**Files:**
- Modify: `ai-proxy/desktop/package.json`、`vite.config.js`、`src/main.jsx`、`src/styles.css`（改为 Tailwind 入口）
- Create: `components.json`、`src/lib/utils.js`、`src/components/ui/*`

- [ ] **Step 1: 查文档**

按用户全局规则，用 Context7 `resolve-library-id` + `query-docs` 查询 shadcn/ui 当前版本在 Vite + React 项目的安装方式（Tailwind 版本、`components.json`、`@` 别名配置），以查到的命令为准执行。

- [ ] **Step 2: 安装与初始化**

安装 tailwindcss 与所需 Radix 包、lucide-react；配置 `@` 别名指向 `src`；执行 shadcn 初始化并添加组件：button、card、badge、input、label、switch、scroll-area、tabs。

- [ ] **Step 3: 验证**

Run: `cd ai-proxy/desktop && npm run build`
Expected: 构建成功。

```bash
git add ai-proxy/desktop
git commit -m "feat(desktop): 接入 Tailwind 与 shadcn/ui"
```

---

### Task 4: 两步流界面重构

**Files:**
- Modify: `ai-proxy/desktop/src/App.jsx`（重写）、`src/styles.css`

**Interfaces:**
- Consumes: Task 2 的命令 + 既有 `get_settings/get_logs/get_manual_config/detect_tools/takeover_tool/restore_tool/install_codex_env/update_provider`。

- [ ] **Step 1: 结构**

顶部状态条（logo、插件连接状态点、当前供应商徽标）；主体两块：「供应商」与「应用」，底部侧栏入口「日志」「手动配置」。

- [ ] **Step 2: 供应商页**

只渲染 `logged_in === true` 的卡片；点击卡片调 `select_provider`；卡片内 `update_provider` 改间隔（1–30 秒）与开关；全部未登录时显示空态卡片 + 「去登录」按钮（`window.open("https://chat.deepseek.com/")`）。

- [ ] **Step 3: 应用页**

「编程工具」文案全部改为「应用」。每款应用卡片：应用名 + 检测路径（`detect_tools`，未检测到显示「未检测到」）；「复制启动命令」用 `get_manual_config` 的 `claude_cmd` / `codex_cmd`；「写入配置」调 `takeover_tool`，成功后刷新并显示「已接管」徽标与「还原」按钮（`restore_tool`）。Codex 环境变量操作保留在 Codex 卡片内。

- [ ] **Step 4: 侧栏**

日志页沿用 `get_logs`；手动配置页展示地址/token 与两条复制命令（复用现有文案与 `navigator.clipboard`）。

- [ ] **Step 5: 验证与提交**

Run: `cd ai-proxy/desktop && npm run build`
Expected: 构建成功；`npm run tauri dev` 手工走查五个场景（登录显示/登出消失/选择生效/复制命令/接管还原）。

```bash
git add ai-proxy/desktop
git commit -m "feat(desktop): shadcn 两步流界面与供应商选择"
```

---

### Task 5: 插件 Cookie 登录探测

**Files:**
- Modify: `ai-proxy/extension/manifest.json`、`background.js`

- [ ] **Step 1: 确定 DeepSeek 会话 cookie 键名**

临时在 background 加 `chrome.cookies.getAll({ domain: "chat.deepseek.com" })` 并把全部 `name` 打印到 service worker 控制台；在 chrome://extensions 打开该扩展的 service worker 视图确认键名，写入常量 `DEEPSEEK_COOKIE_KEYS`，删除临时代码。

- [ ] **Step 2: 实现探测与推送**

`manifest.json` permissions 增加 `"cookies"`。background 实现：

```js
async function detectProviders() {
  const cookies = await chrome.cookies.getAll({ domain: "chat.deepseek.com" });
  const loggedIn = DEEPSEEK_COOKIE_KEYS.some((k) => cookies.some((c) => c.name === k));
  return { type: "providers_status", providers: [{ id: "deepseek", logged_in: loggedIn }] };
}
```

WS `onopen` 时发送探测结果；`chrome.cookies.onChanged.addListener(({ cookie }) => { if (cookie.domain.includes("chat.deepseek.com")) detectProviders().then(send); })`；`onmessage` 收到 `refresh_providers` 时 `detectProviders().then(send)`。

- [ ] **Step 3: 语法检查并提交**

Run: `cd ai-proxy/extension && node --check background.js && node -e "JSON.parse(require('fs').readFileSync('manifest.json','utf8')); console.log('manifest ok')"`
Expected: 无报错。

```bash
git add ai-proxy/extension
git commit -m "feat(extension): Cookie 登录状态探测"
```

---

### Task 6: 联调验收与文档

**Files:**
- Modify: `ai-proxy/extension/README.md`

- [ ] 插件加载后桌面端供应商页只显示已登录的 DeepSeek；`curl http://127.0.0.1:8787/logs` 可见探测状态日志。
- [ ] 浏览器退出登录再登录，桌面端状态实时更新；选中状态下登出会清空选中并提示。
- [ ] 复制启动命令与写入配置（含还原）在真实 Claude Code 上各验证一次。
- [ ] README 补充「登录探测」说明与排查提示（插件未连接 / cookie 过期）。

```bash
git add ai-proxy/extension/README.md
git commit -m "docs(extension): 登录探测与验收说明"
```

## 验收

- 未登录供应商不出现在列表；登录/登出状态实时同步；选择后请求走所选供应商。
- 「应用」页复制命令与写入配置（备份/还原）可用。
- 界面全部使用 shadcn 组件，无手写 CSS。
- `cargo test`、`npm run build` 全绿；插件语法与 manifest 校验通过。
