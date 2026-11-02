# Claude Code 与网页会话映射设计文档

- 日期：2026-11-02
- 状态：已与用户确认，待评审
- 范围：`ai-proxy/desktop` 的会话映射层

## 1. 背景与目标

现状用「system 提示词 + 首条 user 消息」的启发式 key 关联 Claude Code 会话与 DeepSeek 网页对话，内存保存，存在误关联、应用重启丢映射、无法恢复旧会话等问题。

已实测确认：Claude Code 每个请求都带 `x-claude-code-session-id` 请求头，`metadata.user_id` 的 JSON 字符串里也有同一个 `session_id`（UUID）。因此可以建立精确映射。

目标：

- 一个 Claude Code 会话精确对应一组网页对话（1:N 分段），支持 `claude --resume` 找回原网页对话。
- 网页端因对话过长自动开启新对话时，自动追加新分段，不影响后续续接。
- Claude 历史变短（压缩/回退）时，仍强行续接最新网页对话。

## 2. 范围与非目标

### 2.1 范围

- SQLite 映射表与读写逻辑。
- Anthropic 入口提取 CLI 会话 ID、查表续接、流结束写回。
- 历史变短时的强行续接策略。
- 无 CLI 会话 ID 的客户端（如部分 Codex 请求）继续使用现有启发式匹配作为兜底。

### 2.2 非目标

- 不做映射管理界面（查看/删除映射）——后续迭代。
- 不改插件协议、代理 HTTP 接口与供应商队列。

## 3. 数据结构

存储：应用配置目录下的 `sessions.db`（SQLite，`rusqlite` + `bundled`）。

```sql
CREATE TABLE IF NOT EXISTS session_segments (
  id             INTEGER PRIMARY KEY AUTOINCREMENT,
  cli_session_id TEXT NOT NULL,
  provider       TEXT NOT NULL,
  web_chat_id    TEXT NOT NULL,
  web_url        TEXT NOT NULL,
  synced_count   INTEGER NOT NULL,
  retryable      INTEGER NOT NULL DEFAULT 0,
  created_at     TEXT NOT NULL,
  updated_at     TEXT NOT NULL,
  UNIQUE (cli_session_id, provider, web_chat_id)
);
CREATE INDEX IF NOT EXISTS idx_segments_session
  ON session_segments (cli_session_id, provider, updated_at DESC);
```

- 一个 `cli_session_id` + provider 对应多条分段。
- 最新分段按 `updated_at` 倒序取第一条。
- `synced_count`：该网页对话已经同步到的 Claude 消息条数（含刚生成的 assistant 回复，即 n + 1）。
- `web_chat_id` 从流结束时上报的 URL `/a/chat/s/<uuid>` 解析。

## 4. 会话 ID 提取

顺序：

1. 请求头 `x-claude-code-session-id`。
2. `metadata.user_id` 为 JSON 字符串时，解析其中的 `session_id`。
3. 都没有 → 返回 `None`，走启发式兜底（现有 SessionStore）。

标题类请求（无 tools 且 system 含 title）与 count_tokens 不写映射。

## 5. 请求流程

1. 提取 `cli_session_id`；按当前 provider 查询最新分段。
2. 命中且 `n > synced_count`：导航到该分段 `web_url`，只发送 `messages[synced_count..n]`，正常续接。
3. 命中且 `n < synced_count`（历史变短）：仍导航到该分段 `web_url`，把 `build_prompt(body)` 的完整上下文作为一条消息发送（强行续旧对话），成功后 `synced_count = n + 1`。
4. 命中且 `n == synced_count - 1` 且 `retryable`：重发最后一条（失败重试）。
5. 未命中：新网页对话，发送完整提示词（`new_chat = true`）。
6. 流结束拿到 URL 与 web_chat_id：
   - 与请求开始时选中的分段相同 → 更新 `synced_count`、`retryable`、`updated_at`。
   - 不同（网页端自动分叉）→ 插入新分段，`synced_count = n + 1`，旧分段保留。
   - 新会话 → 插入第一条分段。

`claude --resume <uuid>` 时会话 ID 不变，命中同一组分段，取最新分段续接，即回到原来的网页对话；若期间网页端已分叉，则续接最新分段。

## 6. 供应商切换

查询键包含 provider。切到其他供应商时该 CLI 会话没有对应分段 → 新建网页对话；切回 DeepSeek 时原分段仍在，可继续。

## 7. 错误处理

- SQLite 打开/读写失败：记录日志，退回现有启发式匹配，保证请求不中断（fail-open）。
- URL 无法解析出 web_chat_id：本次不写映射，只记日志。
- 网页自动分叉但未上报新 URL：按原分段更新，下次仍续旧网页；已知限制。

## 8. 测试与验收

- 单元测试：提取 header/metadata 会话 ID；插入新会话；同 chat 更新；新 chat 追加分段；取最新分段；历史变短走完整上下文分支。
- 集成测试：两个不同的 `x-claude-code-session-id` 并发/交替请求，各自映射到不同网页对话；重复同 ID 命中同一分段。
- 手工验收：`claude --resume <旧会话 uuid>` 能回到原网页对话；长对话触发网页分叉后，下一轮进入新分段。
