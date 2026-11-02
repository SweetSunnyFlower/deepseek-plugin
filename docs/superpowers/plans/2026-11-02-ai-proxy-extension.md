# AI Proxy 插件端 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在 `ai-proxy/extension/` 实现单一 MV3 插件：按 hostname 自动选择供应商适配器，第一版仅 DeepSeek，负责向网页发送提示词、拦截流式回复并回传给本地代理。

**Architecture:** `background.js` 持有到 `ws://127.0.0.1:8787/bridge` 的 WebSocket；`content.js`（isolated world）按 hostname 查表选择页面适配器；`inject.js`（MAIN world）按 hostname 选择流拦截器。新增供应商只加适配器表项。

**Tech Stack:** Chrome MV3、原生 JavaScript（无构建步骤）。

## Global Constraints

- 代码根：`ai-proxy/extension/`；桌面端在 `ai-proxy/desktop/`。
- 第一版只匹配 `https://chat.deepseek.com/*`；适配器注册表结构必须支持后续域名。
- 不向用户手动聊天插入逻辑：仅处理代理发起的 `send_prompt`。
- 20 秒内未收到流开始即报错，避免代理永久等待。
- 不改动页面其他网络请求；仅拦截 `/api/v0/chat/(completion|regenerate|resume_stream)`。

## 文件结构

```text
ai-proxy/extension/
├── manifest.json
├── background.js
├── content.js
├── inject.js
└── README.md          # 安装与手工验收说明
```

---

### Task 1: 插件骨架与 manifest

**Files:** `manifest.json`、`README.md`

**Interfaces:** 产出可被 Chrome「加载已解压的扩展程序」识别的 MV3 插件。

- [ ] `manifest.json` 使用 `manifest_version: 3`，权限仅 `tabs`，`host_permissions` 为 `https://chat.deepseek.com/*`；两个 content script 都 `run_at: document_start`，其中 `inject.js` 的 `world` 为 `MAIN`。
- [ ] `README.md` 写明：安装插件、打开 chat.deepseek.com、启动桌面端、检查插件连接状态。
- [ ] 验证：Chrome 扩展页加载目录无报错。
- [ ] 提交：`git commit -m "feat(extension): MV3 插件骨架"`。

---

### Task 2: background WebSocket 客户端与路由

**Files:** `background.js`

**Interfaces:** 服务端消息 `{type:"send_prompt",id,prompt,new_chat,chat_url}`；回传 `stream_chunk` / `stream_end`（带 url）/ `stream_error`。

- [ ] 连接 `ws://127.0.0.1:8787/bridge`；`onclose` 2 秒后重连；每 20 秒发 `{type:"ping"}`。
- [ ] `forwardPrompt`：查 DeepSeek 标签页；`new_chat` 时导航到首页，`chat_url` 不同则导航到该对话；等 `tabs.onUpdated` 的 `status === "complete"` 后向 content 发送消息；20 秒超时。
- [ ] 无标签页、无响应、导航超时均回 `stream_error`，错误文本用中文。
- [ ] 验证：桌面端 `/status` 在插件加载后显示 `bridge:true`；关闭标签页后恢复打开能自动重连。
- [ ] 提交：`git commit -m "feat(extension): 代理桥接与标签页路由"`。

---

### Task 3: DeepSeek 页面适配器与流拦截

**Files:** `content.js`、`inject.js`

**Interfaces:** `content.js` 导出 `PAGE_ADAPTERS = { "chat.deepseek.com": {...} }`；`inject.js` 导出 `STREAM_ADAPTERS = { "chat.deepseek.com": {...} }`（同一文件内常量，按 hostname 查表）。

- [ ] `content.js`：监听 `window.postMessage` 且只接受 `e.source === window && e.data.__dsBridge`；非代理发起（`currentId` 为空）忽略；`stream_start` 记录 sessionId；`stream_end` 生成 `https://chat.deepseek.com/a/chat/s/<id>` 并回传；转发 chunk/error 给 background。
- [ ] `sendPrompt(text)`：`waitFor('textarea')`；用 `HTMLTextAreaElement.prototype.value` 原生 setter 赋值；派发 `input`；100ms 后派发 Enter `keydown`；发送后启动 20 秒看门狗，检测页面「频繁/稍后重试」以区分限流。
- [ ] `inject.js`：包装 `fetch`，匹配 `/api/v0/chat/(completion|regenerate|resume_stream)`；非 `event-stream` 响应读取错误文本并上报；正常响应 `body.tee()` 后逐块 postMessage；另包装 XHR 的 `open/send/progress/load/error` 作为备用通道。
- [ ] `send_prompt` 处理期间如已有生成中的请求，返回「上一条消息还在生成中」。
- [ ] 验证：手工在页面输入框观察消息自动填入并发送；插件控制台可见 `stream_chunk` 转发；用户手动发送消息不会触发代理转发。
- [ ] 提交：`git commit -m "feat(extension): DeepSeek 页面与流适配器"`。

---

### Task 4: 与桌面端联调验收

**Files:** `README.md`（补验收步骤）

- [ ] 启动桌面端与插件，`/status` 显示 `bridge:true`。
- [ ] `curl -N -X POST http://127.0.0.1:8787/test -H 'Content-Type: application/json' -d '{"prompt":"用一句话介绍你自己"}'` 能连续输出网页回复文本。
- [ ] Claude Code 手工配置 `ANTHROPIC_BASE_URL=http://127.0.0.1:8787` 与 token，跑一轮「读文件 → 修改 → 验证」完整工具调用。
- [ ] 记录问题到 README「已知限制」：页面改版时只需改 DeepSeek 适配器；用户需保持 DeepSeek 标签页登录。
- [ ] 提交：`git commit -m "docs(extension): 联调与验收说明"`。

## 验收

- 一插件多供应商架构就位，第一版只注册 DeepSeek。
- 插件能自动收发流、上报会话 URL、在未连接/无限流时给出中文错误。
- 用户手动聊天不被干扰；适配器改动范围限定在单文件表项。
