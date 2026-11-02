# AI Proxy 桌面端 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在 `ai-proxy/desktop/` 实现 Tauri 桌面应用：内置 Anthropic + OpenAI 双协议本地代理，经单一插件把请求转发到 DeepSeek 网页，并提供供应商开关、请求间隔设置与编程工具配置接管。

**Architecture:** Rust（tokio + axum）承载代理、排队、会话复用、SSE 与 XML 解析、配置接管；React 前端只经 Tauri command 读写状态。插件经 WebSocket 回传网页原始流。

**Tech Stack:** Tauri 2、Rust 2021、tokio、axum 0.8、serde、regex、toml_edit、React 18、Vite 5。

## Global Constraints

- 代码根：`ai-proxy/desktop/`；插件代码在 `ai-proxy/extension/`，本计划不修改。
- 代理只绑定 `127.0.0.1`，默认端口 `8787`。
- 工具调用只用 XML（`<tool_call>` / `<param>` / `<tool_result>`），不保留 DSML。
- 请求间隔按供应商独立保存，默认 `5000ms`，界面范围 `1..=30` 秒，修改即时生效。
- 第一版供应商仅 `deepseek`，数据结构需支持后续扩展。
- 错误与日志使用中文；日志保留最近 500 条；`cargo test` 必须全绿。

## 文件结构

```text
ai-proxy/desktop/
├── package.json / vite.config.js / index.html
├── src/                      # React 界面
│   ├── main.jsx
│   ├── App.jsx
│   ├── api.js                # invoke 封装
│   └── styles.css
└── src-tauri/
    ├── Cargo.toml / build.rs / tauri.conf.json
    ├── src/
    │   ├── main.rs / lib.rs
    │   ├── settings.rs       # 配置结构与 JSON 持久化
    │   ├── state.rs          # AppState
    │   ├── auth.rs           # token 校验
    │   ├── logs.rs           # 环形日志
    │   ├── bridge.rs         # WS 桥接中心
    │   ├── types.rs          # Message / Block / ToolSpec
    │   ├── commands.rs       # Tauri commands
    │   ├── parser/
    │   │   ├── xml_tool.rs   # 流式 XML 工具解析
    │   │   └── deepseek_sse.rs
    │   ├── provider/
    │   │   ├── controller.rs # 队列、间隔、退避
    │   │   ├── prompt.rs     # XML 提示词构建
    │   │   └── session.rs    # 会话复用
    │   ├── http/
    │   │   ├── mod.rs        # 路由
    │   │   ├── anthropic.rs
    │   │   └── openai.rs
    │   └── config_manager/
    │       ├── detect.rs
    │       ├── claude.rs
    │       └── codex.rs
    └── tests/                # bridge / 双协议 / 配置接管集成测试
```

---

### Task 1: Tauri 脚手架与健康检查

**Files:** `package.json`、`vite.config.js`、`index.html`、`src/main.jsx`、`src/App.jsx`、`src/styles.css`、`src-tauri/Cargo.toml`、`build.rs`、`tauri.conf.json`、`src/main.rs`、`src/lib.rs`

**Interfaces:** 产出 `ai_proxy_desktop_lib::run()`；启动后 `GET /status` 返回 `{"bridge":false,"pending":0}`。

- [ ] 按 Tauri 2 + React + Vite 标准结构创建上述文件；Cargo 依赖含 `tauri`、`tokio`、`axum`、`serde`、`serde_json`、`uuid`、`chrono`、`regex`、`toml_edit`；dev 依赖含 `tokio-tungstenite`、`reqwest`、`tempfile`。
- [ ] `lib.rs` 的 `setup` 中用 `tokio::net::TcpListener` 绑定 `127.0.0.1:8787`，`axum::serve` 提供临时 `/status`。
- [ ] 验证：`npm install && npm run build`；`cargo check`；`npm run tauri dev` 后 `curl http://127.0.0.1:8787/status` 返回上述 JSON。
- [ ] 提交：`git commit -m "feat(desktop): Tauri 桌面应用骨架"`。

---

### Task 2: XML 工具调用流式解析器

**Files:** `src-tauri/src/parser/mod.rs`、`parser/xml_tool.rs`、`lib.rs`

**Interfaces:** `ParseEvent { Text(String), ToolUse { name: String, input: serde_json::Value } }`；`XmlToolParser::{new,push,finish}`。

- [ ] 先写失败测试：纯文本透传；完整 `<tool_call>` 解析；同一流中多个调用；逐字符分片；参数 `string="false"` 按 JSON 解析；代码围栏内 JSON 体；DSML 文本按普通文字；出现 `<tool_result>` 后丢弃剩余内容。
- [ ] 实现标记式状态机：`buf` + `call` 两个状态；开始标记 `<tool_call name="X">`；结束于 `</tool_call>`、下一个开始标记或流结束；参数用 `<param name="K" string="true|false">V</param>`；坏结构降级为原文输出，不中断。
- [ ] 验证：`cargo test xml_tool` 全绿。
- [ ] 提交：`git commit -m "feat(desktop): XML 工具调用流式解析器"`。

---

### Task 3: DeepSeek SSE 解析器

**Files:** `parser/deepseek_sse.rs`、`parser/mod.rs`

**Interfaces:** `DeepSseEvent { Text(String), Done }`；`DeepSeekSseParser::{new,push}`。

- [ ] 测试样例：初始快照 `{"v":{"response":{"fragments":[...]}}}`；带路径 APPEND；省略 `p/o` 的续行；`response/fragments` 新增片段；`THINK` 片段不输出文本；`response/status=FINISHED` 产出 `Done`；非法 JSON 忽略；按行缓冲跨 chunk 拼接。
- [ ] 移植 v1 `deepseek-sse.js` 的路径匹配逻辑；只输出 `RESPONSE` 片段。
- [ ] 验证：`cargo test deepseek_sse` 全绿。
- [ ] 提交：`git commit -m "feat(desktop): DeepSeek SSE 流解析器"`。

---

### Task 4: 核心类型、XML 提示词与会话复用

**Files:** `types.rs`、`provider/mod.rs`、`provider/prompt.rs`、`provider/session.rs`

**Interfaces:** `Block`/`Message`/`ToolSpec`；`build_prompt(system, tools, messages) -> String`；`SessionStore::{key,find,save}`。

- [ ] 测试：system/tools/messages 均按 XML 拼接；历史 `tool_use` 还原成 `<tool_call>`；`tool_result` 带 `error="true"`；提示词中不出现 DSML。
- [ ] 会话测试：同 key 且消息变长时只返回新增部分；历史变短返回 `None`；空回复重试返回最后一条；超过 20 条淘汰最旧。
- [ ] 会话 key 规则：`normalize(system)[0..2000] + "\n" + normalize(首条 user)[0..2000]`。
- [ ] 验证：`cargo test` 全绿。
- [ ] 提交：`git commit -m "feat(desktop): XML 提示词与会话复用"`。

---

### Task 5: 供应商排队、间隔与限流退避

**Files:** `provider/controller.rs`、`settings.rs`（最小版 `ProviderSettings`/`AppSettings`）

**Interfaces:** `ProviderController::new(Arc<RwLock<AppSettings>>, provider)`；`async enqueue<F, Fut, T>(&self, f: F) -> Result<T, String>`。

- [ ] 测试（`tokio::test(start_paused = true)`）：两次发送间隔不少于配置值；连续两次「频繁」错误后第 3 次成功；非限流错误立即返回。
- [ ] 实现：`tokio::sync::Mutex<()>` 串行化；`last_send: Mutex<Option<Instant>>`；每次发送前读取最新 `min_interval_ms`；退避固定 `[15s,30s,60s]`；正则 `频繁|稍后重试|rate.?limit|HTTP 429`。
- [ ] 验证：`cargo test controller` 全绿。
- [ ] 提交：`git commit -m "feat(desktop): 供应商排队与限流退避"`。

---

### Task 6: 设置持久化与日志缓冲

**Files:** `settings.rs`、`logs.rs`

**Interfaces:** `load(dir)->AppSettings`、`save(dir,&AppSettings)->io::Result<()>`；`LogBuffer::{new,push,snapshot}`。

- [ ] 测试：默认值（端口 8787、间隔 5000、token 非空）；保存后重新加载一致；目录不存在时自动创建。
- [ ] 实现 JSON 文件 `settings.json`；`AppSettings` 含 `port/token/active_provider/providers/managed_tools`；日志带 `HH:MM:SS` 前缀，超过 500 条淘汰最旧。
- [ ] 验证：`cargo test settings` 全绿。
- [ ] 提交：`git commit -m "feat(desktop): 设置持久化与日志缓冲"`。

---

### Task 7: WebSocket 桥接、应用状态与调试端点

**Files:** `bridge.rs`、`state.rs`、`http/mod.rs`、`tests/bridge_e2e.rs`

**Interfaces:** `BridgeHub::{attach,detach,connected,pending_count,ask,handle_client}`；`AppState`；`http::router(AppState)`。

- [ ] 协议：服务端发 `{type:"send_prompt",id,prompt,new_chat,chat_url}`；插件回 `stream_chunk` / `stream_end`（可带 url）/ `stream_error`；断线时所有 pending 收到「插件已断开」。
- [ ] `/bridge` 仅接受空 Origin（测试）或 `chrome-extension://` 来源；写入用独立发送任务，读取循环解析 JSON。
- [ ] `/status` 返回连接与 pending；`/test` 原样输出网页流文本，便于调试。
- [ ] 集成测试：真实 WS 客户端连接 → 收到 send_prompt → 回 hello + stream_end → POST /test 返回 `hello`。
- [ ] 验证：`cargo test bridge` 全绿。
- [ ] 提交：`git commit -m "feat(desktop): WebSocket 桥接与应用状态"`。

---

### Task 8: Anthropic Messages 端点

**Files:** `http/anthropic.rs`、`http/mod.rs`

**Interfaces:** `POST /v1/messages`（流式/非流式）、`POST /v1/messages/count_tokens`。

- [ ] 请求解析：`system`（字符串或块）、`messages`（text/tool_use/tool_result）、`tools`、`stream`；标题请求（无 tools 且 system 含 title）本地返回 `{"title":…}`。
- [ ] 处理链：会话查找 → `build_prompt`/续接提示 → `ProviderController.enqueue(bridge.ask)` → `DeepSeekSseParser` → `XmlToolParser` → content blocks。
- [ ] 流式事件顺序：`message_start`、`content_block_start/delta/stop`、`message_delta`、`message_stop`，每 10 秒 `ping`；工具块使用 `input_json_delta`。
- [ ] 非流式返回完整 message；错误 503；流中错误追加 `[代理错误]` 文本。
- [ ] 保存会话：成功 `retryable=false`；失败或空回复 `retryable=true`。
- [ ] 集成测试：假插件返回文本流与工具调用流，断言内容与 `stop_reason`。
- [ ] 验证：`cargo test anthropic` 全绿；提交 `git commit -m "feat(desktop): Anthropic 兼容端点"`。

---

### Task 9: OpenAI 兼容端点

**Files:** `http/openai.rs`、`http/mod.rs`

**Interfaces:** `POST /v1/chat/completions`（流式/非流式）。

- [ ] 请求映射：system/user/assistant.content → Text；assistant.tool_calls → ToolUse；tool 角色 → ToolResult；`tools[].function` → ToolSpec。
- [ ] 非流式响应：`chat.completion` + `message.content` 或 `tool_calls`，`finish_reason=stop/tool_calls`。
- [ ] 流式响应：`chat.completion.chunk`，文本 delta 或 `tool_calls` delta，最后 `[DONE]`。
- [ ] 集成测试：文本与工具调用各一条，断言 JSON 形状。
- [ ] 验证：`cargo test openai` 全绿；提交 `git commit -m "feat(desktop): OpenAI 兼容端点"`。

---

### Task 10: Token 校验与状态详情

**Files:** `auth.rs`、两个端点、`http/mod.rs`

- [ ] `authorized(headers, settings)`：接受 `x-api-key`、`Authorization: Bearer <token>`；`/status` 不校验。
- [ ] 无 token 返回 401；插件未连接返回 503；`/status` 增加 `provider/interval_ms`。
- [ ] 测试：无 token 401、正确 token 503、错 token 401。
- [ ] 验证：`cargo test auth` 全绿；提交 `git commit -m "feat(desktop): token 校验与状态接口"`。

---

### Task 11: 编程工具配置接管

**Files:** `config_manager/mod.rs`、`detect.rs`、`claude.rs`、`codex.rs`、`tests/config_manager.rs`

**Interfaces:** `DetectedTool { tool, path, config_path }`；`detect_all()`；两工具 `takeover(home, base_url, token)` / `restore(home, backup)`。

- [ ] 探测：环境 PATH + 登录 shell PATH + `/usr/local/bin`、`/opt/homebrew/bin`、`~/.local/bin`、`~/.npm-global/bin`；找不到返回空列表（手动模式兜底）。
- [ ] Claude：读写 `~/.claude/settings.json`，合并 `env.ANTHROPIC_BASE_URL` 与 `env.ANTHROPIC_AUTH_TOKEN`，保留所有其他键。
- [ ] Codex：用 `toml_edit` 保留格式，写入 `model_providers.bridge`（base_url、env_key=`DEEPSEEK_BRIDGE_TOKEN`）并把 `model` 设为 `bridge`。
- [ ] 备份：`<原文件>.bak-YYYYMMDD-HHMMSS`；幂等：已接管时只更新 token；还原复制备份回原路径。
- [ ] 测试（tempdir）：preserve/backup、二次接管不新增备份、还原后内容与原始一致、坏文件中止且不改原文件。
- [ ] 验证：`cargo test config_manager` 全绿；提交 `git commit -m "feat(desktop): Claude Code 与 Codex 配置接管"`。

---

### Task 12: Tauri 命令、界面与手工验收

**Files:** `commands.rs`、`lib.rs`、`src/App.jsx`、`src/api.js`、`src/styles.css`

**Interfaces:** `get_settings`、`update_provider`、`get_status`、`get_logs`、`detect_tools`、`takeover_tool`、`restore_tool`、`get_manual_config`、`install_codex_env`。

- [ ] `update_provider` 校验 `1000..=30000` 并立即持久化；`get_manual_config` 返回 Anthropic 地址、OpenAI 地址、token 与两工具配置要点。
- [ ] `install_codex_env(append: bool)`：`append=false` 只返回 `export DEEPSEEK_BRIDGE_TOKEN=<token>` 文本；`append=true` 先备份 `~/.zshrc` 再追加该行，重复调用不重复追加。界面在 Codex 卡片上以勾选项明示该操作。
- [ ] 界面四个 tab：状态、供应商、编程工具、日志；供应商页除 DeepSeek 外显示 ChatGPT、Gemini 灰色「即将支持」卡片；状态每 3 秒刷新；间隔输入框默认 5 秒并带限流提示；工具页含检测/接管/还原与手动复制。
- [ ] 验证：`npm run build`、`cargo test` 全绿；`npm run tauri dev` 手工走查：修改间隔重启仍保留、接管/还原文件内容正确、复制按钮可用、日志实时出现。
- [ ] 提交：`git commit -m "feat(desktop): 配置命令与桌面界面"`。

## 验收

- 假插件下两个协议端点均通过集成测试，文本与工具调用两种回复正确。
- token、错误、空回复、限流退避路径有测试覆盖。
- 间隔默认 5 秒、可调 1–30 秒、按供应商持久化，修改即时生效。
- Claude Code / Codex 接管、备份、还原、幂等与坏文件保护通过测试。
