# Claude Code 与网页会话映射 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 用 Claude Code 的 `x-claude-code-session-id` 建立 SQLite 一对多会话分段映射，实现新会话新建网页对话、`--resume` 回到旧网页对话、网页分叉自动追加分段、历史变短强行续旧对话。

**Architecture:** 新增 `SessionDb`（rusqlite，应用配置目录 `sessions.db`），Anthropic 入口优先用 CLI 会话 ID 查询最新分段；无 ID 时退回现有内存启发式。流结束后按上报 URL 的 web_chat_id 决定更新分段还是追加分段。

**Tech Stack:** 现有 Tauri 2 + Rust；新增 `rusqlite`（`bundled` 特性）、`chrono`（已有）。

## Global Constraints

- 只改 `ai-proxy/desktop`；代理接口、插件协议、供应商队列不变。
- 数据库位置：`app_config_dir/sessions.db`。
- 标题类请求与 count_tokens 不写映射。
- 数据库任何失败都 fail-open：记录日志并退回启发式匹配，请求不中断。
- 无 CLI 会话 ID 的请求保持现有 SessionStore 行为。
- 每个任务 `cargo test` 通过后单独提交。

---

### Task 1: SessionDb 与表结构

**Files:**
- Modify: `ai-proxy/desktop/src-tauri/Cargo.toml`
- Create: `ai-proxy/desktop/src-tauri/src/session_db.rs`
- Modify: `ai-proxy/desktop/src-tauri/src/state.rs`
- Modify: `ai-proxy/desktop/src-tauri/src/lib.rs`

**Interfaces:**
- Produces: `SessionDb::open(path: &Path) -> rusqlite::Result<Self>`。
- Produces: `SessionDb::upsert_segment(&self, seg: &Segment) -> Result<(), String>`。
- Produces: `SessionDb::latest_segment(&self, cli_session_id: &str, provider: &str) -> Result<Option<Segment>, String>`。
- Produces: `Segment { cli_session_id, provider, web_chat_id, web_url, synced_count, retryable }`。
- Produces: `AppState.session_db: Arc<SessionDb>`。

- [ ] **Step 1: 加依赖**

`Cargo.toml` 的 `[dependencies]` 增加：

```toml
rusqlite = { version = "0.32", features = ["bundled"] }
```

- [ ] **Step 2: 写失败测试**（session_db.rs 内）

```rust
#[test]
fn inserts_and_reads_latest_segment() {
    let db = SessionDb::open_in_memory().unwrap();
    db.upsert_segment(&Segment::new("cli-1", "deepseek", "chat-a", "https://chat.deepseek.com/a/chat/s/chat-a", 2)).unwrap();
    db.upsert_segment(&Segment::new("cli-1", "deepseek", "chat-b", "https://chat.deepseek.com/a/chat/s/chat-b", 5)).unwrap();
    let latest = db.latest_segment("cli-1", "deepseek").unwrap().unwrap();
    assert_eq!(latest.web_chat_id, "chat-b");
    assert_eq!(latest.synced_count, 5);
}

#[test]
fn same_chat_id_updates_instead_of_duplicating() {
    // upsert 同 (cli, provider, chat) 两次 → 只有一行，synced_count 取最新
}
```

Run: `cd ai-proxy/desktop/src-tauri && cargo test session_db`
Expected: 编译失败，模块不存在。

- [ ] **Step 3: 实现**

- 建表 SQL 按设计文档第 3 节，`updated_at` 使用 `chrono::Utc::now().to_rfc3339()`。
- `upsert_segment` 用 `INSERT ... ON CONFLICT(cli_session_id, provider, web_chat_id) DO UPDATE SET synced_count=?, retryable=?, updated_at=?`。
- `latest_segment` 按 `updated_at DESC, id DESC` 取第一条。
- `open_in_memory()` 仅测试使用；生产用 `open(path)`。
- `AppState::new` 打开 `<config_dir>/sessions.db`；打开失败则记录日志，`session_db: Option<Arc<SessionDb>>` 或内存降级（选择 Option，fail-open）。

- [ ] **Step 4: 跑测试并提交**

Run: `cargo test`
Expected: 全部 PASS。

```bash
git add ai-proxy/desktop/src-tauri
git commit -m "feat(desktop): 会话分段 SQLite 存储"
```

---

### Task 2: 提取 Claude Code 会话 ID

**Files:**
- Create: `ai-proxy/desktop/src-tauri/src/http/session_id.rs`
- Modify: `ai-proxy/desktop/src-tauri/src/http/mod.rs`

**Interfaces:**
- Produces: `fn cli_session_id(headers: &HeaderMap, body: &Value) -> Option<String>`。

- [ ] **Step 1: 写测试**

```rust
#[test]
fn prefers_header() {
    // HeaderMap 设 x-claude-code-session-id: abc；body metadata 给 def → 返回 abc
}

#[test]
fn falls_back_to_metadata_user_id() {
    // 无 header；metadata.user_id = "{\"session_id\":\"def\"}" → 返回 def
}

#[test]
fn returns_none_without_id() {
    // 空 header + 无 metadata → None
}
```

- [ ] **Step 2: 实现**

- header 优先，trim 后非空即返回。
- 否则取 `metadata.user_id` 字符串，`serde_json::from_str` 解析，读 `session_id`。
- 解析失败/为空返回 None。

- [ ] **Step 3: 跑测试并提交**

```bash
git add ai-proxy/desktop/src-tauri
git commit -m "feat(desktop): 提取 Claude Code 会话 ID"
```

---

### Task 3: Anthropic 入口接入精确映射

**Files:**
- Modify: `ai-proxy/desktop/src-tauri/src/http/anthropic.rs`

**Interfaces:**
- Consumes: `session_id::cli_session_id`、`SessionDb`、现有 `SessionStore`。
- 行为：见设计文档第 5 节。

- [ ] **Step 1: 决策函数抽纯逻辑并测试**

在 anthropic.rs（或新模块）抽出：

```rust
enum SessionMode { New, Continue, ForceContinue, Retry }

fn decide_mode(segment: Option<&Segment>, n: usize) -> SessionMode {
    match segment {
        None => SessionMode::New,
        Some(s) if n + 1 == s.synced_count && s.retryable => SessionMode::Retry,
        Some(s) if n > s.synced_count => SessionMode::Continue,
        Some(s) if n < s.synced_count => SessionMode::ForceContinue,
        Some(_) => SessionMode::New,
    }
}
```

测试四个分支的边界。

- [ ] **Step 2: 接入 handle()**

- 提取 `cli_session_id`；若为 Some，查询 `state.session_db` 最新分段并 `decide_mode`。
- `New`：完整 `build_prompt`，`new_chat=true`。
- `Continue`：`build_follow_up(messages[s.synced_count..])`，`chat_url=Some(s.web_url)`，`new_chat=false`。
- `ForceContinue`：完整 `build_prompt(body)`，`chat_url=Some(s.web_url)`，`new_chat=false`。
- `Retry`：只发最后一条，`chat_url=Some(s.web_url)`。
- 无 ID 或 DB 失败：走现有 `SessionStore` 启发式路径，行为不变。

- [ ] **Step 3: 流结束写回**

- 从最终 URL 解析 `web_chat_id`（`/a/chat/s/<id>` 的最后一段）。
- 若有 CLI session id 且 URL 有效：
  - 若本次请求开始时选中的分段与最终 web_chat_id 相同 → upsert 更新 `synced_count = n + 1`、`retryable = !ok`。
  - 不同或没有分段 → upsert 新分段（同表天然去重）。
  - 同时**不要**再用启发式 SessionStore 保存该请求。
- 无 CLI id → 保持现有 `save_session`。
- 日志：`[映射] cli=<id> provider=deepseek chat=<id> synced=<n>`；新增分段时加「网页已分叉」。

- [ ] **Step 4: 集成测试**

用假插件 WS（复用 anthropic_e2e 模式）：

- 两个不同 `x-claude-code-session-id` 的请求 → 两个不同 web_chat_id（回传 URL 不同）→ 数据库两行。
- 同一 session id 第二次带更多消息 → WS 收到的 prompt 只包含新增消息，且 chat_url 指向第一次的 URL。
- 第二次回传不同 URL → 追加第二条分段，latest 变为新 chat。
- 历史更短时 → WS 收到的 prompt 含完整 system/历史（断言包含 system 文本）。
- 同一 CLI id 在 provider 维度下互不影响：查询 `chatgpt` 时返回 None，`deepseek` 仍命中。

Run: `cargo test`
Expected: 全部 PASS。

```bash
git add ai-proxy/desktop/src-tauri
git commit -m "feat(desktop): Claude Code 会话精确映射"
```

---

### Task 4: 真机验收

- [ ] 启动应用 + 插件，新开 Claude Code 会话问一句 → 网页端新建对话，日志出现 `[映射]`。
- [ ] 同会话继续 → 无新网页对话，网页里追加消息。
- [ ] 退出后用 `claude --resume <uuid>` 恢复 → 回到原网页对话。
- [ ] 长对话触发网页自动分叉 → 日志显示「网页已分叉」，下一轮进入新分段。
- [ ] 手动构造历史变短请求（或删除/压缩上下文后继续）→ 旧网页对话收到完整上下文，不新建对话。
- [ ] 把结果补充到验收记录并提交文档变更。

## 验收

- 同一 CLI 会话始终命中同组分段；不同 CLI 会话互不串线。
- `--resume` 能找回旧网页对话；网页分叉后新分段生效。
- 历史变短时强行续旧对话。
- DB 失败不影响正常请求（回退启发式）。
- `cargo test` 全绿。

## 执行状态（2026-11-02）

- Task 1–3 已完成并通过自动化测试（共 56 个用例，含精确映射集成测试）。
- Task 4 真机验收暂缓：网页账号当前受限，解禁后补做 `--resume`、网页分叉与历史变短的真实链路验证。
- 额外提供 `GET /sessions` 调试接口，按当前供应商返回最近 200 条映射分段，便于验收时核对。
