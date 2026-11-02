# AI Proxy 供应商登录探测与 UI 重构设计文档

- 日期：2026-11-02
- 状态：已与用户确认，待评审
- 范围：在 `ai-proxy/`（desktop + extension）上做供应商登录探测与桌面端交互重构

## 1. 背景与目标

现状：桌面端是「状态 / 供应商 / 编程工具 / 日志」四 tab，供应商列表里固定显示 DeepSeek（另两家灰显），「当前供应商」概念对普通用户难以理解；登录状态只能靠用户自己知道。

目标：

- 打开应用后通过插件探测哪些供应商已登录，只展示已登录的供应商，用户点击即选择。
- 「编程工具」改名为「应用」，每款应用提供两个动作：复制启动命令（粘贴到终端即用）、写入配置文件（写入前自动备份、可一键还原）。
- UI 组件库换成 shadcn/ui，重构桌面端界面与交互。
- 本次迭代只做登录探测与 UI 重构，供应商仍只有 DeepSeek。

## 2. 范围与非目标

### 2.1 本次范围

- 插件：`cookies` 权限、登录状态探测、经 WS 上报、cookie 变化实时推送。
- 桌面端：状态接收与缓存、`get_providers_status` / `select_provider` / `refresh_providers` 三个命令、shadcn/ui 界面重构、应用页动作改造。
- 代理接口（/v1/messages、/v1/chat/completions 等）与供应商队列逻辑不变。

### 2.2 非目标

- 不接入 ChatGPT、Gemini 供应商。
- 不改变配置接管（备份/写入/还原）的底层逻辑，只改交互入口。
- 不做打包分发、自动更新。

## 3. 登录探测与数据流

探测方式：Cookie 探测（已确认）。插件 background 用 `chrome.cookies.getAll({ domain })` 检查各供应商域名下的会话 cookie，存在即判定为已登录。

- DeepSeek 判定键名：实现第一步在已登录环境下 `chrome.cookies.getAll` dump 实际 cookie 后确定常量列表；规则为「存在任一候选键即已登录」。
- 上报时机：WS 连接建立时全量推；`chrome.cookies.onChanged` 触发时增量推；桌面端点「重新探测」时发送 `{"type":"refresh_providers"}`，插件回一份全量状态。
- 消息形状（插件 → 桌面端）：`{"type":"providers_status","providers":[{"id":"deepseek","logged_in":true}]}`。最新状态覆盖旧状态，无请求/响应对应关系。
- 桌面端把状态缓存进 `AppState`，与 settings 里的 `providers` 合并展示（是否登录来自插件，间隔/开关来自本地配置）。
- 当前选中的供应商（`active_provider`）若登录状态变为 false，自动清空并在界面提示重新选择。

## 4. 界面结构（shadcn/ui）

布局从四 tab 改为两步流 + 轻量侧栏：

- 顶部状态条：应用 logo、插件连接状态点、当前供应商徽标。
- 第一屏「供应商」：只渲染已登录的供应商卡片；点击卡片即选中（高亮当前 `active_provider`）；卡片下方带每供应商独立的请求间隔输入（1–30 秒整数）与开关；全部未登录时显示空态卡片和「去登录」按钮（打开对应网站）。
- 第二屏「应用」（原「编程工具」）：每款应用一张卡片，显示应用名与检测到的安装路径（未检测到也显示卡片）；两个主按钮「复制启动命令」「写入配置」；已接管时卡片显示「已接管」徽标并出现「还原」按钮；还原逻辑沿用现有实现。
- 侧栏：日志（现有日志页迁入）、手动配置信息（地址/token 复制）作为二级入口。

组件清单：Button、Card、Badge、Input、Label、Switch、ScrollArea、Tabs，图标用 lucide-react。移除全部手写 CSS，改用 Tailwind。

## 5. 技术落地

前端：

- 引入 Tailwind CSS 与 shadcn/ui（Button/Card/Badge/Input/Label/Switch/ScrollArea/Tabs），建立 `components/ui/`、`lib/utils.ts`；重写 `App.jsx` 与入口样式。
- 具体脚手架命令与版本以实现时 Context7 查询到的 shadcn/ui 最新文档为准。

后端（Rust，改动最小）：

- `BridgeHub` 接收 `providers_status` 消息，写入 `AppState.providers_status`（`HashMap<String, bool>`）。
- 新命令：`get_providers_status()`、`select_provider(id)`（仅允许选中已登录供应商，持久化 `active_provider`）、`refresh_providers()`（向插件发 `refresh_providers` 消息）。
- 复用 `settings.providers` 与 `active_provider`，不新增配置字段。

插件：

- `manifest.json` 增加 `cookies` 权限；`background.js` 增加探测、`onChanged` 监听与 `refresh_providers` 处理；其余不动。

## 6. 错误处理

- 插件未连接：供应商页显示「插件未连接」提示与状态点，隐藏登录判断（不显示任何供应商为已登录）。
- cookie 存在但会话已过期：判定为已登录属于已知偏差，用户选中后请求会以网页端错误体现；后续迭代可加端点校验兜底。
- 选中供应商登出：清空 `active_provider`，UI 提示重新选择，避免请求发往失效会话。

## 7. 测试与验收

- 后端：`providers_status` 消息解析与状态更新测试；`select_provider` 对未登录供应商拒绝、对已登录供应商持久化的测试。
- 前端：手工走查登录态展示、登出后消失、选择生效、复制命令、写入配置与还原。
- 联调：真实浏览器登录/登出，验证桌面端实时更新。
