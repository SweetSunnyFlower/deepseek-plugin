# DeepSeek Bridge v2 设计文档

- 日期：2026-11-02
- 状态：已与用户确认，待评审
- 范围：在现有 `deepseek-plugin` 仓库上做一次重构与产品化升级

## 1. 背景与目标

现状（v1）：本地 Node 代理 + 一个 DeepSeek 专用 Chrome 插件，把 Claude Code 的请求转成 DSML 提示词注入 chat.deepseek.com 网页会话，再把流式回复转回 Anthropic 格式。定位是开发者自用工具，命令行启动，无界面。

v2 目标：

- 做给非技术用户：桌面图形应用，开箱即用，不需要命令行。
- 多供应商架构：第一版只接入 DeepSeek，但插件和代理都按「每供应商一个适配器」设计，ChatGPT、Gemini 后续以加表项方式接入。
- 多编程工具支持：自动探测并接管 Claude Code、Codex 等工具的配置；探测不到时提供手动配置所需的代理地址和 token。
- 统一 XML 工具调用协议，移除 DSML。

## 2. 范围与非目标

### 2.1 第一版范围

- Tauri 桌面应用（Rust 后端 + Web 前端），内置代理服务。
- 代理双协议：Anthropic Messages API 与 OpenAI 兼容 Chat Completions API，均支持流式 SSE 与非流式。
- 单个 Chrome 插件（MV3），按 hostname 自动识别供应商，v1 仅注册 DeepSeek 适配器。
- 工具调用与提示词统一 XML，DSML 代码全部移除。
- 配置接管：Claude Code（`~/.claude/settings.json`）与 Codex（`~/.codex/config.toml`），含备份、写入、一键还原。
- 手动模式：界面始终展示代理地址与 token，供用户自行配置到任意 AI 编程工具。
- 每供应商独立可设置的请求频率，默认 5 秒一次。

### 2.2 非目标（第一版不做）

- ChatGPT、Gemini 供应商适配器。
- pi、opencode 等其余编程工具的自动接管（手动模式可覆盖）。
- 官方 API key 直连（网页会话是唯一通道）。
- 无浏览器纯 cookie 的 HTTP 方案。
- 自动更新、代码签名与正式分发渠道（先手动分发安装包）。

## 3. 产品形态

用户流程：

1. 打开桌面应用，看到支持的供应商列表（DeepSeek 可用，其余灰显「即将支持」）。
2. 点击开启 DeepSeek，应用生成并显示本机代理地址 `http://127.0.0.1:8787` 与随机 token。
3. 应用探测已安装的编程工具，用户勾选要接管的工具（如 Claude Code、Codex），应用自动备份原配置并写入新配置。
4. 探测不到工具时，界面提供每个工具的配置说明和复制按钮，用户手动完成。
5. 用户在浏览器登录 chat.deepseek.com 并安装插件，应用显示插件连接状态。
6. 用户正常使用自己的编程工具，流量经本地代理转发到 DeepSeek 网页会话。

界面四个页面：状态（插件连接、当前供应商）、供应商（开关 + 请求间隔设置）、工具（探测 / 接管 / 还原 / 手动配置说明）、日志（请求与错误记录，便于排查）。

## 4. 总体架构

### 4.1 仓库布局

新代码全部放在 `ai-proxy/` 目录，分两个子目录：

- `ai-proxy/desktop/`：Tauri 桌面应用（Rust 后端 + React 前端）。
- `ai-proxy/extension/`：Chrome 插件（MV3）。

现有根目录的 `proxy/`（Node 实现）与 `extension/`（v1 插件）保留至 v2 功能对等验收通过后再删除；设计文档与实现计划分别落在 `docs/superpowers/specs/` 与 `docs/superpowers/plans/`。

### 4.2 总体架构

```text
┌──────────────┐  Anthropic/OpenAI   ┌────────────────────────────┐
│ Claude Code   │ ─────────────────▶ │                            │
│ Codex / 其他  │ ◀───────────────── │  桌面应用（Tauri + Rust）   │
└──────────────┘  SSE 流式回复        │  ┌──────────────────────┐  │
                                     │  │ 代理服务 (127.0.0.1)  │  │
                                     │  │ /v1/messages         │  │
                                     │  │ /v1/chat/completions │  │
                                     │  │ /bridge (WebSocket)  │  │
                                     │  └──────────┬───────────┘  │
                                     │             │ WebSocket    │
                                     │  ┌──────────▼───────────┐  │
                                     │  │ 供应商控制器          │  │
                                     │  │ 队列/限流/会话复用    │  │
                                     │  └──────────┬───────────┘  │
                                     │             │              │
                                     │  ┌──────────▼───────────┐  │
                                     │  │ 前端 (WebView)        │  │
                                     │  │ 设置/状态/日志        │  │
                                     │  └─────────────────────┘  │
                                     └────────────┬───────────────┘
                                                  │ WebSocket
                                     ┌────────────▼───────────────┐
                                     │ Chrome 插件                 │
                                     │ background: WS 客户端/路由  │
                                     │ content: 每站适配器(查表)   │
                                     │ inject: 每站流拦截(查表)    │
                                     └────────────┬───────────────┘
                                                  │ 真实网页会话
                                     ┌────────────▼───────────────┐
                                     │ chat.deepseek.com 等        │
                                     └────────────────────────────┘
```

编程工具与网页之间没有直连，所有报文经代理中转。代理面向编程工具说两种协议，面向插件走私有 WebSocket 协议。

## 5. 组件设计

### 5.1 代理服务（Rust，tokio + axum）

HTTP 端点：

- `POST /v1/messages`：Anthropic Messages API，支持 `stream: true/false`。
- `POST /v1/chat/completions`：OpenAI 兼容，支持 `stream: true/false`。
- 两协议各自的 token 统计（Anthropic 的 `count_tokens`、OpenAI 响应的 `usage` 字段）：长度估算即可，与 v1 口径一致（字符数 / 3）。
- `GET /status`：插件连接、队列、当前供应商等状态，仅绑定 127.0.0.1，不校验 token。
- 其余端点校验请求头里的 token（Anthropic 的 `x-api-key` / `Authorization`，OpenAI 的 `Authorization: Bearer`），与界面展示的 token 比对，防止本机其他程序误用。

`/bridge` WebSocket：只接受 `chrome-extension://` 来源连接；同一时刻一个连接，断开时挂起的请求全部失败并通知。

内部消息表示：统一 `Message { role, content: Vec<Block> }`，`Block` 为 `Text | ToolUse | ToolResult` 三态。Anthropic 前端是原生映射；OpenAI 前端把 `ToolUse` 映射为 assistant 消息的 `tool_calls`、`ToolResult` 映射为 `tool` 角色消息。

### 5.2 供应商控制器

- 每供应商一份配置：`{ enabled, min_interval_ms, backoff: [u64;3], rate_limit_patterns }`，存共享状态（`Arc<RwLock<..>>`），界面改动即时生效。
- 每供应商一个 FIFO 队列；同一供应商的所有编程工具共用队列，间隔全局强制：两次发送之间不少于 `min_interval_ms`（默认 5000ms）。
- 限流检测用每供应商自己的提示语正则；命中后按 15s / 30s / 60s 退避重试，退避时长 v1 不做界面配置。
- 会话复用沿用 v1 思路并加供应商维度：key = `provider + norm(system)[0..2000] + norm(首条 user)[0..2000]`；记录网页对话 URL 与已发送消息数，后续轮次只发新增消息；历史变短（压缩/回退）时开新对话；上一轮空回复/出错允许重发最后一条。
- 标题类请求（无 tools 且 system 含 title）本地直接回答，不占用网页会话。

### 5.3 XML 提示词与解析

提示词结构（每供应商可覆盖的格式函数，v1 只有默认 XML 实现）：

```xml
<system>…</system>
<tools>
…每个工具的名称、描述、参数 JSON Schema…
调用规则：<tool_call name="工具名"> 包裹 <param>；一次可多个互不依赖调用；
调用完立即停止；Edit 的 old_string 需逐字一致；结果由 <tool_result> 给出，
不要自行编写；不需要工具时直接文字回答。
</tools>
<user>…</user>
<assistant>…</assistant>
…
请以 assistant 的身份回复最后一条 user 消息，需要时按约定格式调用工具。
```

工具调用标记（历史消息上屏与模型输出使用同一格式）：

```xml
<tool_call name="Read">
<param name="file_path" string="true">/abs/a.js</param>
<param name="offset" string="false">1</param>
</tool_call>
```

流式解析器（移植 v1 `tool-parser.js` 的标记式思路，XML-only）：

- 开始标记 `<tool_call name="X">`；结束于 `</tool_call>`、下一个开始标记或流结束。
- 参数按 `<param name="K" string="true|false">V</param>` 解析；非字符串值 JSON 解析，失败按原文。
- 调用体外的文本按普通文本输出；容忍包裹的 ```` ``` ```` 围栏；结构损坏时整段降级为普通文本，不中断回复。
- 检测到模型自行编造的 `<tool_result>` 起的内容全部丢弃（防幻觉注入）。
- 不保留任何 DSML 解析逻辑。

### 5.4 插件（单插件，每站适配器）

保持 v1 的双 world 结构：

- `inject.js`（MAIN world）：按 hostname 查表选择流拦截器。v1 仅注册 `chat.deepseek.com`（拦截 `/api/v0/chat/*` 的 fetch/XHR，tee 响应流，postMessage 给 content）。
- `content.js`（isolated world）：按 hostname 查表选择页面操作适配器（输入框选择器、赋值与回车事件注入、看门狗）。v1 仅注册 DeepSeek 适配器（`textarea` + React setter）。
- `background.js`：持有到代理 `/bridge` 的 WebSocket，转发 `send_prompt` 并按 hostname 路由到对应标签页；断线 2 秒重连；20 秒心跳。

manifest 的 `matches` 预置所有目标域名；新增供应商 = 新增两个适配器表项，不动 background。

### 5.5 配置管理器（工具接管）

探测：读登录 shell 的 PATH + 常见安装目录（nvm/npm 全局路径、`~/.local/bin`、`/usr/local/bin` 等）找 `claude`、`codex` 可执行文件；全部失败时提供文件选择器手动指定。

接管流程（每工具）：解析校验现有配置 → 时间戳备份（`<原文件>.bak-YYYYMMDD-HHMMSS`，备份路径记入应用状态）→ 写入最小改动 → 界面提示成功。幂等：已接管（base_url 指向本代理）时只更新 token，不重复备份、不重复包裹。

- Claude Code：读写 `~/.claude/settings.json`，合并 `env` 字段：`ANTHROPIC_BASE_URL=http://127.0.0.1:8787`、`ANTHROPIC_AUTH_TOKEN=<token>`，保留其余键。
- Codex：读写 `~/.codex/config.toml`，追加 `model_providers.bridge`（`base_url=http://127.0.0.1:8787/v1`、`env_key=DEEPSEEK_BRIDGE_TOKEN`）并切换 `model`。Codex 要求 key 来自环境变量：界面提供「追加一行 export 到 shell 配置」的可选操作（明示修改的文件，写入前也备份），用户不同意时显示复制内容。

还原：从应用状态记录的备份文件恢复，恢复后清除接管标记。

手动模式：无论探测结果如何，「代理地址 + token + 每工具配置要点」始终在界面可见可复制。

### 5.6 界面与持久化

前端：React + TypeScript + Vite（Tauri 常规组合），与 Rust 后端全部经 Tauri command 通信，不走 HTTP。

应用配置持久化于 Tauri `app_config_dir` 的 JSON：`{ port, token, active_provider, providers: { deepseek: { enabled, min_interval_ms } }, managed_tools: [...] }`。token 首次启动随机生成。

## 6. 错误处理

- 插件未连接 / 无目标标签页 / 标签页无响应：请求以结构化错误返回（流式响应内附错误文本块），状态页同步可见。
- 网页改版找不到输入框：适配器抛「找不到输入框」，20 秒看门狗兜底「发送后无回复」。
- 限流：按 5.2 的退避策略重试，耗尽后向用户报错。
- 空回复：记录模型原文，向调用方返回错误文本。
- 配置接管：任何解析/写入失败立即中止，保留原文件不动，提示用备份还原。
- 频率设置：界面限制 1–30 秒整数，附「间隔太短容易触发网页限流」提示。

## 7. 测试

- 单元：XML 流式解析器（真实 + 对抗样本：截断、围栏、坏结构、编造 tool_result）；提示词构建器（含 OpenAI/Anthropic 两种映射）；会话 key 与续接逻辑。
- 配置接管：对两工具的配置模板做夹具测试（临时目录）：首次接管、重复接管幂等、备份内容、还原、坏文件中止。
- 集成：代理 + 模拟插件 WebSocket 客户端，验证两种协议端点的 SSE 事件形状、非流式响应、token 校验、队列间隔与退避。
- 手工验收：Claude Code、Codex 各完整跑一轮含多工具调用的任务；界面全流程走查（开启 → 接管 → 日志 → 还原）。

## 8. 移植与淘汰

v1 的 Node 实现整体退役，但作为移植参考：排队/退避/会话复用/标题本地回复逻辑原样搬入 Rust；`deepseek-sse.js` 的 JSON-patch SSE 解析与 `tool-parser.js` 的标记式解析逐条对照移植；插件三个文件重构为「background + 适配器注册表」结构。旧目录在功能对等验证通过后删除。

## 9. 已知风险与后续决策点

- DeepSeek 对 DSML 有原生倾向，XML 模式下可能偶发漏出 DSML 标记。按既定决定 v1 不写兼容；实测若出现，最便宜的修法是解析器加「DSML 视为 XML 别名」的兜底规则（届时单独评审）。
- GUI 启动拿不到 shell 登录 PATH，探测可能漏报；靠常见目录扫描与手动选择器兜底。
- 网页 DOM 改版风险由「每站适配器 + 看门狗 + 日志页」共同兜底，改版只改适配器。
- Codex 的环境变量摩擦点靠「授权追加 export + 复制说明」两条路径缓解。
