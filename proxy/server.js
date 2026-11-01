// 本地代理：Anthropic 兼容接口 + 给插件连接的 WebSocket。
//   POST /v1/messages   Anthropic Messages API（支持 stream）
//   POST /test          调试用：{"prompt":"..."}，原样返回 DeepSeek 的 SSE
//   GET  /status        查看插件连接状态
import http from 'node:http';
import { randomUUID } from 'node:crypto';
import { WebSocketServer } from 'ws';
import { createParser } from './deepseek-sse.js';
import { createToolParser } from './tool-parser.js';

const PORT = Number(process.env.PORT) || 8787;
let bridge = null;
const handlers = new Map(); // id -> { onChunk, onEnd }

// 一个标签页同时只能生成一条回复，所有请求排队执行
let queue = Promise.resolve();
function enqueue(task) {
  const run = queue.then(task, task);
  queue = run.catch(() => {});
  return run;
}

// 两次发送之间至少间隔这么久，避免触发网页端"消息发送过于频繁"
const MIN_INTERVAL = Number(process.env.MIN_INTERVAL_MS) || 3000;
// 被限流后的等待时间（毫秒），依次重试
const RATE_LIMIT_BACKOFF = [15000, 30000, 60000];
const isRateLimit = (msg) => /频繁|稍后重试|rate.?limit|HTTP 429/i.test(msg);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
let lastSend = 0;

// 把提示词发给网页。chatUrl 为空时新建对话，否则在该对话里继续。
// 结束时 resolve 为对话的 URL，供下一轮继续使用。
function askDeepSeek(prompt, onChunk, chatUrl = null) {
  return enqueue(async () => {
    for (let attempt = 0; ; attempt++) {
      const wait = lastSend + MIN_INTERVAL - Date.now();
      if (wait > 0) await sleep(wait);
      lastSend = Date.now();
      try {
        return await sendOnce(prompt, onChunk, chatUrl);
      } catch (e) {
        if (!isRateLimit(e.message) || attempt >= RATE_LIMIT_BACKOFF.length) throw e;
        const ms = RATE_LIMIT_BACKOFF[attempt];
        console.log(`[限流] ${e.message}，${ms / 1000} 秒后重试（第 ${attempt + 1} 次）`);
        await sleep(ms);
      }
    }
  });
}

function sendOnce(prompt, onChunk, chatUrl) {
  return new Promise((resolve, reject) => {
    if (!bridge) return reject(new Error('插件未连接，请打开 chat.deepseek.com'));
    const id = randomUUID();
    handlers.set(id, {
      onChunk,
      onEnd: (err, url) => { handlers.delete(id); err ? reject(new Error(err)) : resolve(url); },
    });
    bridge.send(JSON.stringify({ type: 'send_prompt', id, prompt, newChat: !chatUrl, chatUrl }));
  });
}

// 会话复用：一个 Claude Code 会话对应一个网页对话，用 system + 第一条 user 消息识别。
// Claude Code 每轮会微调历史（插入提醒、改写块），逐条比对很容易对不上，
// 所以只记录网页对话里已经有多少条消息，每次只发后面新增的部分。
const sessions = []; // { key, count, url }
const MAX_SESSIONS = 20;
const norm = (s) => s.replace(/\s+/g, ' ').trim();
const sessionKey = (body) => {
  const first = (body.messages || []).find((m) => m.role === 'user');
  return norm(contentText(body.system)).slice(0, 2000) + '\n' + norm(contentText(first?.content || '')).slice(0, 2000);
};

function findSession(body) {
  const key = sessionKey(body);
  const n = body.messages?.length || 0;
  for (let i = sessions.length - 1; i >= 0; i--) {
    const s = sessions[i];
    if (s.key !== key) continue;
    if (n > s.count) return { session: s, rest: body.messages.slice(s.count) };
    // 上一轮回复为空或出错，Claude Code 重发了同样的历史：把最后一条再发一次
    if (n === s.count - 1 && s.retryable) return { session: s, rest: body.messages.slice(-1) };
    return null; // 历史变短了（压缩或回退），开新对话
  }
  return null;
}

function saveSession(body, url, old, ok) {
  if (!url || !/\/chat\/s\//.test(url)) return;
  if (old) sessions.splice(sessions.indexOf(old), 1);
  // 网页对话里现在有：这次的全部消息 + 刚生成的回复
  sessions.push({ key: sessionKey(body), count: (body.messages?.length || 0) + 1, url, retryable: !ok });
  if (sessions.length > MAX_SESSIONS) sessions.shift();
}

function buildFollowUp(rest) {
  const parts = rest.map((m) => `<${m.role}>\n${contentText(m.content)}\n</${m.role}>`);
  parts.push('请继续以 assistant 的身份回复，需要时按约定格式调用工具。');
  return parts.join('\n\n');
}

const server = http.createServer(async (req, res) => {
  const url = req.url.split('?')[0];
  try {
    if (req.method === 'GET' && url === '/status') return json(res, 200, { bridge: !!bridge, pending: handlers.size });
    if (req.method === 'POST' && url === '/test') return await handleTest(req, res);
    if (req.method === 'POST' && url === '/v1/messages') return await handleMessages(req, res);
    if (req.method === 'POST' && url === '/v1/messages/count_tokens') {
      const body = JSON.parse(await readBody(req));
      return json(res, 200, { input_tokens: Math.ceil(buildPrompt(body).length / 3) });
    }
    json(res, 404, { type: 'error', error: { type: 'not_found_error', message: 'not found' } });
  } catch (e) {
    if (!res.headersSent) json(res, 500, { type: 'error', error: { type: 'api_error', message: e.message } });
    else res.end();
  }
});

async function handleTest(req, res) {
  const body = JSON.parse(await readBody(req));
  res.writeHead(200, { 'Content-Type': 'text/plain; charset=utf-8' });
  await askDeepSeek(body.prompt, (c) => res.write(c)).catch((e) => res.write(`\n[错误] ${e.message}\n`));
  res.end();
}

// 历史里的工具调用也写成 DSML，和提示词约定的格式保持一致，避免模型学着混用
function dsmlCall(name, input) {
  const params = Object.entries(input || {}).map(([k, v]) => typeof v === 'string'
    ? `<｜DSML｜parameter name="${k}" string="true">${v}</｜DSML｜parameter>`
    : `<｜DSML｜parameter name="${k}" string="false">${JSON.stringify(v)}</｜DSML｜parameter>`);
  return `<｜DSML｜function_calls>\n<｜DSML｜invoke name="${name}">\n${params.join('\n')}\n</｜DSML｜invoke>\n</｜DSML｜function_calls>`;
}

// Claude Code 每次都发完整历史，网页端每次新建对话，把历史拼成一条消息
function contentText(content) {
  if (typeof content === 'string') return content;
  return (content || []).map((b) => {
    if (b.type === 'text') return b.text;
    if (b.type === 'tool_use') return dsmlCall(b.name, b.input);
    if (b.type === 'tool_result') {
      const err = b.is_error ? ' error="true"' : '';
      return `<tool_result${err}>\n${contentText(b.content)}\n</tool_result>`;
    }
    if (b.type === 'image') return '[图片：网页端不支持，已省略]';
    return '';
  }).join('\n');
}

function toolsPrompt(tools) {
  if (!tools?.length) return '';
  const list = tools.map((t) =>
    `## ${t.name}\n${t.description || ''}\n参数 JSON Schema：${JSON.stringify(t.input_schema || {})}`).join('\n\n');
  return `<tools>
你可以调用以下工具。调用格式（严格遵守，开始和结束标签必须成对）：
<｜DSML｜function_calls>
<｜DSML｜invoke name="工具名">
<｜DSML｜parameter name="参数名" string="true">字符串参数原文</｜DSML｜parameter>
<｜DSML｜parameter name="参数名" string="false">数字、布尔或 JSON 值</｜DSML｜parameter>
</｜DSML｜invoke>
</｜DSML｜function_calls>

规则：
- 一次回复可以在同一个 function_calls 里放多个互不依赖的 invoke。调用完工具后立即停止输出，等待结果。
- Edit 的 old_string 必须和文件原文逐字一致（包括缩进和空格）。不确定时先用 Read 读取文件。
- 工具结果会在下一条 user 消息里以 <tool_result>...</tool_result> 给出。绝对不要自己编写 <tool_result>。
- 不需要工具时直接用文字回答。

${list}
</tools>`;
}

function buildPrompt(body) {
  const parts = [];
  const system = contentText(body.system);
  if (system) parts.push(`<system>\n${system}\n</system>`);
  const tools = toolsPrompt(body.tools);
  if (tools) parts.push(tools);
  for (const m of body.messages || []) parts.push(`<${m.role}>\n${contentText(m.content)}\n</${m.role}>`);
  parts.push('请以 assistant 的身份回复最后一条 user 消息，只输出回复内容，需要时按约定格式调用工具。');
  return parts.join('\n\n');
}

// 把文本 / 工具调用组装成 Anthropic content 块；sse 为 null 时只收集结果（非流式）
function createOutput(sse) {
  const content = [];
  let textOpen = false;
  let outLen = 0;

  function closeText() {
    if (!textOpen) return;
    textOpen = false;
    sse?.('content_block_stop', { index: content.length - 1 });
  }
  return {
    content,
    get outLen() { return outLen; },
    text(t) {
      outLen += t.length;
      if (!textOpen) {
        // 工具调用之间的纯空白不单独成块
        if (!t.trim()) return;
        content.push({ type: 'text', text: '' });
        textOpen = true;
        sse?.('content_block_start', { index: content.length - 1, content_block: { type: 'text', text: '' } });
      }
      content.at(-1).text += t;
      sse?.('content_block_delta', { index: content.length - 1, delta: { type: 'text_delta', text: t } });
    },
    toolUse({ name, input }) {
      closeText();
      const block = { type: 'tool_use', id: 'toolu_' + randomUUID().replace(/-/g, '').slice(0, 24), name, input };
      content.push(block);
      const index = content.length - 1;
      sse?.('content_block_start', { index, content_block: { type: 'tool_use', id: block.id, name, input: {} } });
      sse?.('content_block_delta', { index, delta: { type: 'input_json_delta', partial_json: JSON.stringify(input) } });
      sse?.('content_block_stop', { index });
    },
    finish() {
      closeText();
      return content.some((b) => b.type === 'tool_use') ? 'tool_use' : 'end_turn';
    },
  };
}

// Claude Code 的辅助请求（生成会话标题等）不带工具。标题请求在本地直接回答，
// 不占用网页对话，也不用排队等主对话。
function localReply(body) {
  if (body.tools?.length) return null;
  if (!/title/i.test(contentText(body.system))) return null;
  const firstUser = (body.messages || []).find((m) => m.role === 'user');
  const title = norm(contentText(firstUser?.content || '')).slice(0, 20) || '新会话';
  return JSON.stringify({ title });
}

async function handleMessages(req, res) {
  const body = JSON.parse(await readBody(req));
  const local = localReply(body);
  if (local !== null) return replyLocal(res, body, local);
  const found = findSession(body);
  const prompt = found ? buildFollowUp(found.rest) : buildPrompt(body);
  const msgId = 'msg_' + randomUUID().replace(/-/g, '');
  const model = body.model || 'deepseek-web';
  const usage = { input_tokens: Math.ceil(buildPrompt(body).length / 3), output_tokens: 0 };
  console.log(`[请求] ${body.messages?.length || 0} 条消息, ${body.tools?.length || 0} 个工具, ` +
    (found ? `续接对话, 新增 ${found.rest.length} 条, ${prompt.length} 字` : `新对话, 提示词 ${prompt.length} 字`));

  let sse = null;
  let ping = null;
  if (body.stream) {
    res.writeHead(200, { 'Content-Type': 'text/event-stream', 'Cache-Control': 'no-cache', Connection: 'keep-alive' });
    sse = (event, data) => res.write(`event: ${event}\ndata: ${JSON.stringify({ type: event, ...data })}\n\n`);
    sse('message_start', { message: { id: msgId, type: 'message', role: 'assistant', model, content: [], stop_reason: null, stop_sequence: null, usage } });
    ping = setInterval(() => sse('ping', {}), 10000);
  }

  const out = createOutput(sse);
  const tools = createToolParser({ onText: out.text, onToolUse: out.toolUse });
  let raw = '';
  const parser = createParser({ onDelta: (kind, t) => { if (kind === 'text') { raw += t; tools.push(t); } } });
  try {
    const url = await askDeepSeek(prompt, (c) => parser.push(c), found?.session.url);
    tools.end();
    out.finish();
    const ok = out.content.some((b) => b.type === 'tool_use' || b.text?.trim());
    if (!ok) console.log(`[空回复] 模型原文: ${JSON.stringify(raw.slice(0, 500))}`);
    saveSession(body, url, found?.session, ok);
  } catch (e) {
    clearInterval(ping);
    if (!sse) return json(res, 503, { type: 'error', error: { type: 'overloaded_error', message: e.message } });
    tools.end();
    out.text(`\n[代理错误] ${e.message}`);
  }
  clearInterval(ping);
  const stopReason = out.finish();
  usage.output_tokens = Math.ceil(out.outLen / 3);
  console.log(`[回复] ${out.content.map((b) => (b.type === 'tool_use' ? `工具:${b.name}` : `文字${b.text.length}`)).join(', ') || '空'}`);

  if (!sse) {
    if (!out.content.length) out.content.push({ type: 'text', text: '' });
    return json(res, 200, { id: msgId, type: 'message', role: 'assistant', model, content: out.content, stop_reason: stopReason, stop_sequence: null, usage });
  }
  sse('message_delta', { delta: { stop_reason: stopReason, stop_sequence: null }, usage: { output_tokens: usage.output_tokens } });
  sse('message_stop', {});
  res.end();
}

function replyLocal(res, body, text) {
  const msgId = 'msg_' + randomUUID().replace(/-/g, '');
  const model = body.model || 'deepseek-web';
  const usage = { input_tokens: 1, output_tokens: 1 };
  console.log(`[本地回复] ${text}`);
  if (!body.stream) {
    return json(res, 200, { id: msgId, type: 'message', role: 'assistant', model, content: [{ type: 'text', text }], stop_reason: 'end_turn', stop_sequence: null, usage });
  }
  res.writeHead(200, { 'Content-Type': 'text/event-stream', 'Cache-Control': 'no-cache' });
  const sse = (event, data) => res.write(`event: ${event}\ndata: ${JSON.stringify({ type: event, ...data })}\n\n`);
  sse('message_start', { message: { id: msgId, type: 'message', role: 'assistant', model, content: [], stop_reason: null, stop_sequence: null, usage } });
  sse('content_block_start', { index: 0, content_block: { type: 'text', text: '' } });
  sse('content_block_delta', { index: 0, delta: { type: 'text_delta', text } });
  sse('content_block_stop', { index: 0 });
  sse('message_delta', { delta: { stop_reason: 'end_turn', stop_sequence: null }, usage: { output_tokens: 1 } });
  sse('message_stop', {});
  res.end();
}

const wss = new WebSocketServer({ server, path: '/bridge' });
wss.on('connection', (ws, req) => {
  // 只接受 Chrome 插件的连接，防止任意网页连上来
  if (!String(req.headers.origin || '').startsWith('chrome-extension://')) return ws.close();
  bridge = ws;
  console.log('插件已连接');
  ws.on('close', () => {
    if (bridge !== ws) return;
    bridge = null;
    console.log('插件已断开');
    for (const h of handlers.values()) h.onEnd('插件已断开');
  });
  ws.on('message', (data) => {
    let msg;
    try { msg = JSON.parse(data); } catch { return; }
    const h = handlers.get(msg.id);
    if (!h) return;
    if (msg.type === 'stream_chunk') h.onChunk(msg.chunk);
    else if (msg.type === 'stream_end') h.onEnd(null, msg.url);
    else if (msg.type === 'stream_error') h.onEnd(msg.error || '未知错误');
  });
});

server.listen(PORT, '127.0.0.1', () => console.log(`代理已启动: http://127.0.0.1:${PORT}`));

function json(res, code, obj) {
  res.writeHead(code, { 'Content-Type': 'application/json; charset=utf-8' });
  res.end(JSON.stringify(obj));
}
function readBody(req) {
  return new Promise((ok, fail) => {
    let s = '';
    req.on('data', (c) => (s += c));
    req.on('end', () => ok(s));
    req.on('error', fail);
  });
}
