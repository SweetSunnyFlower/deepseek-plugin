# AI Proxy Bridge（Chrome 插件）

配合桌面应用 AI Proxy 使用：把本地代理的请求转发到已登录的 DeepSeek 网页会话，并把网页流式回复原样回传。

## 安装

1. 打开 Chrome 的「扩展程序」页面（chrome://extensions），开启右上角「开发者模式」。
2. 点击「加载已解压的扩展程序」，选择本目录（`ai-proxy/extension/`）。
3. 打开并登录 https://chat.deepseek.com ，保持标签页开启。
4. 启动桌面应用，确认「状态」页显示「插件已连接」。

## 使用

- 在桌面应用里开启 DeepSeek 供应商、接管或手动配置编程工具后，即可正常使用 Claude Code / Codex 等工具。
- 用户自己在网页里手动聊天不会受插件影响；只有代理发起的请求才会被拦截。

## 已知限制

- 需要保持至少一个 DeepSeek 标签页打开且已登录。
- 网页改版导致失效时，只需修改 `content.js` 与 `inject.js` 里的 `chat.deepseek.com` 适配器表项。

## 联调验收步骤

1. 启动桌面应用，加载本插件，`curl http://127.0.0.1:8787/status` 显示 `"bridge":true`。
2. 调试流：

```bash
curl -N -X POST http://127.0.0.1:8787/test \
  -H 'Content-Type: application/json' \
  -d '{"prompt":"用一句话介绍你自己"}'
```

应当能看到 DeepSeek 的流式文本逐段输出。

3. Claude Code：设置 `ANTHROPIC_BASE_URL=http://127.0.0.1:8787` 与 token（或桌面应用里直接「接管配置」），跑一轮「读文件 → 修改 → 验证」的完整工具调用。
4. Codex：provider 的 `base_url` 指向 `http://127.0.0.1:8787/v1`，API key 填 token（或桌面应用接管），跑一轮含工具调用的任务。
