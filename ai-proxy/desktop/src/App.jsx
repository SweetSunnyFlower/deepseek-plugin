import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

const TABS = [
  ["status", "状态"],
  ["providers", "供应商"],
  ["tools", "编程工具"],
  ["logs", "日志"],
];

const COMING_SOON = [
  { name: "ChatGPT", note: "即将支持" },
  { name: "Gemini", note: "即将支持" },
];

export default function App() {
  const [tab, setTab] = useState("status");
  const [settings, setSettings] = useState(null);
  const [status, setStatus] = useState(null);
  const [logs, setLogs] = useState([]);
  const [tools, setTools] = useState([]);
  const [manual, setManual] = useState(null);
  const [busy, setBusy] = useState(false);

  async function refresh() {
    try {
      setSettings(await invoke("get_settings"));
      setStatus(await invoke("get_status"));
      setLogs(await invoke("get_logs"));
      setManual(await invoke("get_manual_config"));
    } catch (e) {
      console.error(e);
    }
  }

  useEffect(() => {
    refresh();
    const t = setInterval(refresh, 3000);
    return () => clearInterval(t);
  }, []);

  function notify(text) {
    alert(text);
  }

  async function setIntervalMs(ms) {
    try {
      setSettings(
        await invoke("update_provider", {
          provider: "deepseek",
          minIntervalMs: ms,
        })
      );
    } catch (e) {
      notify(String(e));
    }
  }

  async function setEnabled(enabled) {
    try {
      setSettings(
        await invoke("update_provider", { provider: "deepseek", enabled })
      );
    } catch (e) {
      notify(String(e));
    }
  }

  async function scan() {
    setTools(await invoke("detect_tools"));
  }

  async function takeover(tool) {
    if (busy) return;
    setBusy(true);
    try {
      notify("已接管，备份在：" + (await invoke("takeover_tool", { tool })));
      refresh();
    } catch (e) {
      notify(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function restore(tool) {
    if (busy) return;
    setBusy(true);
    try {
      await invoke("restore_tool", { tool });
      notify("已还原配置");
      refresh();
    } catch (e) {
      notify(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function installCodexEnv(append) {
    try {
      notify(await invoke("install_codex_env", { append }));
    } catch (e) {
      notify(String(e));
    }
  }

  async function copy(text) {
    try {
      await navigator.clipboard.writeText(text);
      notify("已复制");
    } catch (e) {
      notify(String(e));
    }
  }

  if (!settings || !status || !manual) {
    return <main>加载中…</main>;
  }
  const p = settings.providers.deepseek;

  return (
    <main>
      <header>
        <h1>AI Proxy</h1>
        <span className={status.bridge ? "dot on" : "dot off"}>
          {status.bridge ? "插件已连接" : "等待插件连接"}
        </span>
      </header>
      <nav>
        {TABS.map(([k, label]) => (
          <button
            key={k}
            className={tab === k ? "active" : ""}
            onClick={() => setTab(k)}
          >
            {label}
          </button>
        ))}
      </nav>

      {tab === "status" && (
        <section>
          <h2>运行状态</h2>
          <div className="card">
            <p>当前供应商：{status.provider}</p>
            <p>请求间隔：{Math.round(p.min_interval_ms / 1000)} 秒</p>
            <p>代理地址：{manual.base_url}</p>
            <p>
              Token：<code>{manual.token}</code>{" "}
              <button onClick={() => copy(manual.token)}>复制</button>
            </p>
            <p className="hint">
              插件连接后，在浏览器打开并登录 chat.deepseek.com 即可使用。
            </p>
          </div>
        </section>
      )}

      {tab === "providers" && (
        <section>
          <h2>供应商</h2>
          <div className="card">
            <h3>
              DeepSeek{" "}
              <span className="hint">{p.enabled ? "已开启" : "已关闭"}</span>
            </h3>
            <label>
              请求间隔（秒）
              <input
                type="number"
                min="1"
                max="30"
                value={Math.round(p.min_interval_ms / 1000)}
                onChange={(e) => setIntervalMs(Number(e.target.value) * 1000)}
              />
            </label>
            <button onClick={() => setEnabled(!p.enabled)}>
              {p.enabled ? "关闭" : "开启"}
            </button>
            <p className="hint">间隔太短容易触发网页限流，默认 5 秒。</p>
          </div>
          {COMING_SOON.map((c) => (
            <div className="card disabled" key={c.name}>
              <h3>
                {c.name} <span className="hint">{c.note}</span>
              </h3>
            </div>
          ))}
        </section>
      )}

      {tab === "tools" && (
        <section>
          <h2>编程工具</h2>
          <button onClick={scan}>重新检测</button>
          {tools.map((t) => (
            <div className="card" key={t.tool}>
              <strong>{t.tool}</strong>
              <span className="path">{t.path}</span>
              <button disabled={busy} onClick={() => takeover(t.tool)}>
                接管配置
              </button>
              <button disabled={busy} onClick={() => restore(t.tool)}>
                还原
              </button>
            </div>
          ))}
          <div className="card">
            <h3>手动配置</h3>
            <p className="hint">复制命令到终端执行即可，或用上方的一键接管。</p>
            {[
              ["Claude Code", manual.claude_cmd],
              ["Codex", manual.codex_cmd],
            ].map(([name, cmd]) => (
              <div className="cmd" key={name}>
                <strong>{name}</strong>
                <code className="cmdbox">{cmd}</code>
                <button onClick={() => copy(cmd)}>复制命令</button>
              </div>
            ))}
            <p>
              OpenAI 地址：{manual.openai_base_url}{" "}
              <button onClick={() => copy(manual.openai_base_url)}>复制</button>
            </p>
          </div>
          <div className="card">
            <h3>Codex 环境变量</h3>
            <button onClick={() => installCodexEnv(false)}>复制 export 行</button>
            <button onClick={() => installCodexEnv(true)}>
              写入 ~/.zshrc（自动备份）
            </button>
          </div>
        </section>
      )}

      {tab === "logs" && (
        <section>
          <h2>日志</h2>
          <pre>{logs.join("\n") || "暂无日志"}</pre>
        </section>
      )}
    </main>
  );
}
