// 运行在 isolated world：操作输入框发送消息，把 inject.js 拦截到的流转发给 background。
// 按 hostname 查表选择页面适配器；新增供应商只加表项。

const PAGE_ADAPTERS = {
  "chat.deepseek.com": {
    input: "textarea",
  },
};

let currentId = null;
let currentSession = null;
let watchdog = null;

const adapter = PAGE_ADAPTERS[location.hostname];
if (!adapter) {
  console.warn("[AI Proxy] 当前站点没有页面适配器:", location.hostname);
}

window.addEventListener("message", (e) => {
  if (e.source !== window || !e.data?.__dsBridge) return;
  if (!currentId) return; // 不是代理发起的对话（用户手动聊天），忽略
  const { type, chunk, error } = e.data;
  const id = currentId;
  if (type === "stream_start") {
    clearTimeout(watchdog);
    currentSession = e.data.sessionId;
    return;
  }
  if (type === "stream_end" || type === "stream_error") {
    clearTimeout(watchdog);
    currentId = null;
  }
  if (type === "stream_end") {
    // 上报对话 URL，代理下一轮据此续接
    const url = currentSession
      ? `https://chat.deepseek.com/a/chat/s/${currentSession}`
      : location.href;
    chrome.runtime.sendMessage({ from: "content", id, type, url });
    return;
  }
  chrome.runtime.sendMessage({ from: "content", id, type, chunk, error });
});

chrome.runtime.onMessage.addListener((msg, _sender, sendResponse) => {
  if (msg.type !== "send_prompt") return;
  (async () => {
    try {
      if (!adapter) throw new Error("当前站点不支持");
      if (currentId) throw new Error("上一条消息还在生成中");
      currentId = msg.id;
      currentSession = null;
      await sendPrompt(msg.prompt);
      // 网页被限流时可能只弹提示、根本不发请求，这里兜底，避免代理一直等
      const id = msg.id;
      watchdog = setTimeout(() => {
        if (currentId !== id) return;
        currentId = null;
        const limited = /频繁|稍后重试/.test(document.body.innerText);
        chrome.runtime.sendMessage({
          from: "content",
          id,
          type: "stream_error",
          error: limited
            ? "消息发送过于频繁，请稍后重试"
            : "发送后 20 秒没有收到回复，可能没发出去",
        });
      }, 20000);
      sendResponse({ ok: true });
    } catch (e) {
      currentId = null;
      sendResponse({ ok: false, error: String(e.message || e) });
    }
  })();
  return true; // 异步 sendResponse
});

async function waitFor(selector, timeout = 10000) {
  for (
    const start = Date.now();
    Date.now() - start < timeout;
    await new Promise((r) => setTimeout(r, 100))
  ) {
    const el = document.querySelector(selector);
    if (el) return el;
  }
  return null;
}

async function sendPrompt(text) {
  const box = await waitFor(adapter.input);
  if (!box) throw new Error("找不到输入框，检查 PAGE_ADAPTERS.input");
  box.focus();
  // React 受控组件：要用原生 setter 赋值再触发 input 事件，状态才会更新
  const setter = Object.getOwnPropertyDescriptor(
    HTMLTextAreaElement.prototype,
    "value"
  ).set;
  setter.call(box, text);
  box.dispatchEvent(new Event("input", { bubbles: true }));
  // 等 React 渲染后按回车发送
  setTimeout(() => {
    box.dispatchEvent(
      new KeyboardEvent("keydown", {
        key: "Enter",
        code: "Enter",
        keyCode: 13,
        bubbles: true,
      })
    );
  }, 100);
}
