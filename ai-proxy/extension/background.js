// 持有到本地代理的 WebSocket，在代理和 DeepSeek 标签页之间转发消息。
// WebSocket 有活动时 service worker 不会被休眠；每 20 秒发一次心跳。

const PROXY_URL = "ws://127.0.0.1:8787/bridge";
let ws = null;

const DEEPSEEK_COOKIE_KEYS = ["ds_session_id"];

async function detectProviders() {
  const cookies = await chrome.cookies.getAll({ domain: "chat.deepseek.com" });
  const names = cookies.map((c) => c.name);
  const loggedIn = DEEPSEEK_COOKIE_KEYS.some((k) => names.includes(k));
  return {
    type: "providers_status",
    providers: [{ id: "deepseek", logged_in: loggedIn }],
  };
}

function connect() {
  if (ws && (ws.readyState === WebSocket.OPEN || ws.readyState === WebSocket.CONNECTING)) {
    return;
  }
  ws = new WebSocket(PROXY_URL);
  ws.onopen = async () => {
    console.log("[bridge] 已连接代理");
    send(await detectProviders());
  };
  ws.onclose = () => {
    ws = null;
    setTimeout(connect, 2000);
  };
  ws.onerror = () => {}; // onclose 负责重连
  ws.onmessage = async (e) => {
    let msg;
    try {
      msg = JSON.parse(e.data);
    } catch {
      return;
    }
    if (msg.type === "send_prompt") await forwardPrompt(msg);
    if (msg.type === "refresh_providers") send(await detectProviders());
  };
}

function send(obj) {
  if (ws?.readyState === WebSocket.OPEN) ws.send(JSON.stringify(obj));
}

async function forwardPrompt(msg) {
  const [tab] = await chrome.tabs.query({ url: "https://chat.deepseek.com/*" });
  if (!tab) {
    return send({ id: msg.id, type: "stream_error", error: "没有打开 chat.deepseek.com 标签页" });
  }
  try {
    if (msg.newChat) await openUrl(tab.id, "https://chat.deepseek.com/");
    else if (msg.chatUrl && tab.url !== msg.chatUrl) await openUrl(tab.id, msg.chatUrl);
    const res = await sendToTab(tab.id, msg);
    if (!res?.ok) {
      send({ id: msg.id, type: "stream_error", error: res?.error || "标签页无响应，请刷新页面" });
    }
  } catch (e) {
    send({ id: msg.id, type: "stream_error", error: "标签页无响应，请刷新页面: " + e.message });
  }
}

// 扩展重载后旧页面的 content script 会失效（Receiving end does not exist）。
// 自动重载标签页并重试一次，避免用户手动刷新。
async function sendToTab(tabId, msg) {
  try {
    return await chrome.tabs.sendMessage(tabId, msg);
  } catch (e) {
    const text = String(e?.message || e);
    if (!text.includes("Receiving end does not exist")) throw e;
    await reloadTab(tabId);
    return chrome.tabs.sendMessage(tabId, msg);
  }
}

function reloadTab(tabId) {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      done();
      reject(new Error("刷新标签页超时"));
    }, 20000);
    const listener = (id, info) => {
      if (id === tabId && info.status === "complete") {
        done();
        resolve();
      }
    };
    const done = () => {
      clearTimeout(timer);
      chrome.tabs.onUpdated.removeListener(listener);
    };
    chrome.tabs.onUpdated.addListener(listener);
    chrome.tabs.reload(tabId);
  });
}

// 跳到首页（新建对话）或指定对话；等页面加载完成，content script 才能接收消息
function openUrl(tabId, url) {
  return chrome.tabs.get(tabId).then((tab) => {
    if ((tab.url || "").split("#")[0] === url) return;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        done();
        reject(new Error("打开对话超时"));
      }, 20000);
      const listener = (id, info) => {
        if (id === tabId && info.status === "complete") {
          done();
          resolve();
        }
      };
      const done = () => {
        clearTimeout(timer);
        chrome.tabs.onUpdated.removeListener(listener);
      };
      chrome.tabs.onUpdated.addListener(listener);
      chrome.tabs.update(tabId, { url });
    });
  });
}

chrome.runtime.onMessage.addListener((msg) => {
  if (msg.from === "content") send(msg);
});

setInterval(() => send({ type: "ping" }), 20000);
chrome.cookies.onChanged.addListener(({ cookie, removed }) => {
  if (!cookie.domain.includes("chat.deepseek.com")) return;
  detectProviders().then(send);
});
// MV3 service worker 会被休眠：用 alarms 定期唤醒并确保 WebSocket 存活/重连
chrome.alarms.create("bridge-watchdog", { periodInMinutes: 0.5 });
chrome.alarms.onAlarm.addListener(() => connect());
chrome.runtime.onStartup.addListener(() => connect());
chrome.runtime.onInstalled.addListener(() => connect());
connect();
