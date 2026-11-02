// 运行在页面 MAIN world：拦截网页自己发出的流式回复请求，把原始数据块转给 content script。
// 按 hostname 查表选择流拦截器；新增供应商只加表项。

(() => {
  const TARGET = /\/api\/v0\/chat\/(completion|regenerate|resume_stream)/;
  const post = (type, data) =>
    window.postMessage({ __dsBridge: true, type, ...data }, "*");

  const STREAM_ADAPTERS = {
    "chat.deepseek.com": {
      target: TARGET,
      sessionFromBody(body) {
        try {
          return (
            JSON.parse(typeof body === "string" ? body : "{}")
              .chat_session_id || null
          );
        } catch {
          return null;
        }
      },
    },
  };

  const adapter = STREAM_ADAPTERS[location.hostname];
  if (!adapter) return;

  const origFetch = window.fetch;
  window.fetch = async function (input, init) {
    const res = await origFetch.apply(this, arguments);
    const url = typeof input === "string" ? input : input?.url || "";
    if (!adapter.target.test(url) || !res.body) return res;

    // 被限流等错误时返回的是 JSON 而不是 SSE，直接把错误内容报上去
    if (!/event-stream/.test(res.headers.get("content-type") || "")) {
      res
        .clone()
        .text()
        .then((t) =>
          post("stream_error", { error: `HTTP ${res.status}: ${t.slice(0, 300)}` })
        );
      return res;
    }

    const [mine, theirs] = res.body.tee();
    // 请求体里带着对话 id，比 location.href 可靠（URL 更新有延迟）
    const sessionId = adapter.sessionFromBody(init?.body);
    (async () => {
      const reader = mine.getReader();
      const decoder = new TextDecoder();
      post("stream_start", { url, sessionId });
      try {
        for (;;) {
          const { done, value } = await reader.read();
          if (done) break;
          post("stream_chunk", { chunk: decoder.decode(value, { stream: true }) });
        }
        post("stream_end", {});
      } catch (e) {
        post("stream_error", { error: String(e) });
      }
    })();
    return new Response(theirs, {
      status: res.status,
      statusText: res.statusText,
      headers: res.headers,
    });
  };

  // 备用：页面如果改用 XHR 发流式请求，也能拦截到
  const origOpen = XMLHttpRequest.prototype.open;
  const origSend = XMLHttpRequest.prototype.send;
  XMLHttpRequest.prototype.open = function (method, url) {
    this.__dsUrl = String(url);
    return origOpen.apply(this, arguments);
  };
  XMLHttpRequest.prototype.send = function () {
    if (adapter.target.test(this.__dsUrl || "")) {
      let seen = 0;
      post("stream_start", { url: this.__dsUrl });
      const flush = () => {
        const text = this.responseText || "";
        if (text.length > seen) {
          post("stream_chunk", { chunk: text.slice(seen) });
          seen = text.length;
        }
      };
      this.addEventListener("progress", flush);
      this.addEventListener("load", () => {
        flush();
        post("stream_end", {});
      });
      this.addEventListener("error", () =>
        post("stream_error", { error: "xhr error" })
      );
    }
    return origSend.apply(this, arguments);
  };
})();
