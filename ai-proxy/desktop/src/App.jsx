import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  Check,
  Copy,
  ExternalLink,
  FilePen,
  Loader2,
  RefreshCw,
  RotateCcw,
  Terminal,
  Unplug,
  Wifi,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { ScrollArea } from "@/components/ui/scroll-area";
import logoUrl from "../logo.jpg";

const PROVIDER_NAMES = { deepseek: "DeepSeek" };
const APP_NAMES = { claude: "Claude Code", codex: "Codex" };

export default function App() {
  const [view, setView] = useState("providers");
  const [providers, setProviders] = useState([]);
  const [status, setStatus] = useState(null);
  const [settings, setSettings] = useState(null);
  const [logs, setLogs] = useState([]);
  const [tools, setTools] = useState([]);
  const [manual, setManual] = useState(null);
  const [busy, setBusy] = useState("");

  async function refresh() {
    try {
      setProviders(await invoke("get_providers_status"));
      setStatus(await invoke("get_status"));
      setSettings(await invoke("get_settings"));
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

  async function copy(text) {
    try {
      await navigator.clipboard.writeText(text);
      notify("已复制");
    } catch (e) {
      notify(String(e));
    }
  }

  async function choose(id) {
    try {
      setSettings(await invoke("select_provider", { id }));
      refresh();
    } catch (e) {
      notify(String(e));
    }
  }

  async function setIntervalMs(provider, ms) {
    try {
      setSettings(
        await invoke("update_provider", { provider, minIntervalMs: ms })
      );
    } catch (e) {
      notify(String(e));
    }
  }

  async function setEnabled(provider, enabled) {
    try {
      setSettings(await invoke("update_provider", { provider, enabled }));
    } catch (e) {
      notify(String(e));
    }
  }

  async function refreshProviders() {
    await invoke("refresh_providers");
    setTimeout(refresh, 500);
  }

  async function scan() {
    setTools(await invoke("detect_tools"));
  }

  async function takeover(tool) {
    if (busy) return;
    setBusy(tool);
    try {
      notify("已完成，备份在：" + (await invoke("takeover_tool", { tool })));
      refresh();
    } catch (e) {
      notify(String(e));
    } finally {
      setBusy("");
    }
  }

  async function restore(tool) {
    if (busy) return;
    setBusy(tool);
    try {
      await invoke("restore_tool", { tool });
      notify("已还原配置");
      refresh();
    } catch (e) {
      notify(String(e));
    } finally {
      setBusy("");
    }
  }

  async function installCodexEnv(append) {
    try {
      notify(await invoke("install_codex_env", { append }));
    } catch (e) {
      notify(String(e));
    }
  }

  if (!status || !manual) {
    return (
      <div className="flex h-screen items-center justify-center text-muted-foreground">
        加载中…
      </div>
    );
  }

  const loggedIn = providers.filter((p) => p.logged_in);
  const activeName = PROVIDER_NAMES[status.provider] || status.provider;
  const isManaged = (tool) =>
    (settings?.managed_tools || []).some((m) => m.tool === tool);

  return (
    <div className="flex h-screen bg-background text-foreground">
      <aside className="flex w-52 flex-col border-r p-4">
        <div className="flex items-center gap-2 px-2 pb-6">
          <img src={logoUrl} alt="AI Proxy" className="h-8 w-8 rounded-md" />
          <span className="font-semibold">AI Proxy</span>
        </div>
        <nav className="flex flex-col gap-1 text-sm">
          {[
            ["providers", "供应商"],
            ["apps", "应用"],
            ["logs", "日志"],
            ["manual", "手动配置"],
          ].map(([key, label]) => (
            <Button
              key={key}
              variant={view === key ? "secondary" : "ghost"}
              className="justify-start"
              onClick={() => setView(key)}
            >
              {label}
            </Button>
          ))}
        </nav>
      </aside>

      <main className="flex flex-1 flex-col">
        <header className="flex items-center justify-between border-b px-6 py-3">
          <div className="flex items-center gap-3">
            <span
              className={
                "inline-flex items-center gap-1.5 text-sm " +
                (status.bridge ? "text-emerald-600" : "text-muted-foreground")
              }
            >
              {status.bridge ? <Wifi size={16} /> : <Unplug size={16} />}
              {status.bridge ? "插件已连接" : "等待插件连接"}
            </span>
            {status.provider ? (
              <Badge variant="outline">{activeName}</Badge>
            ) : (
              <Badge variant="outline">未选择供应商</Badge>
            )}
          </div>
          <Button variant="ghost" size="sm" onClick={refreshProviders}>
            <RefreshCw size={14} className="mr-1" />
            重新探测
          </Button>
        </header>

        <ScrollArea className="flex-1 p-6">
          {view === "providers" && (
            <ProvidersView
              providers={loggedIn}
              settings={settings}
              onChoose={choose}
              onInterval={setIntervalMs}
              onToggle={setEnabled}
            />
          )}
          {view === "apps" && (
            <AppsView
              tools={tools}
              manual={manual}
              busy={busy}
              managed={isManaged}
              onScan={scan}
              onTakeover={takeover}
              onRestore={restore}
              onCopy={copy}
              onCodexEnv={installCodexEnv}
            />
          )}
          {view === "logs" && <LogsView logs={logs} />}
          {view === "manual" && <ManualView manual={manual} onCopy={copy} />}
        </ScrollArea>
      </main>
    </div>
  );
}

function ProvidersView({ providers, settings, onChoose, onInterval, onToggle }) {
  if (providers.length === 0) {
    return (
      <div className="mx-auto max-w-md pt-16">
        <Card>
          <CardHeader>
            <CardTitle>没有检测到已登录的供应商</CardTitle>
          </CardHeader>
          <CardContent className="space-y-3 text-sm text-muted-foreground">
            <p>请先在浏览器登录网页，然后回来重新探测。</p>
            <Button onClick={() => window.open("https://chat.deepseek.com/")}>
              <ExternalLink size={14} className="mr-1" />
              去登录 DeepSeek
            </Button>
          </CardContent>
        </Card>
      </div>
    );
  }
  return (
    <div className="grid max-w-3xl gap-4">
      {providers.map((p) => {
        const cfg = settings?.providers?.[p.id];
        return (
          <Card key={p.id} className={p.active ? "ring-2 ring-primary" : ""}>
            <CardHeader className="flex-row items-center justify-between space-y-0">
              <div className="flex items-center gap-2">
                <CardTitle>{PROVIDER_NAMES[p.id] || p.id}</CardTitle>
                {p.active && (
                  <Badge variant="secondary">
                    <Check size={12} className="mr-1" />
                    当前使用
                  </Badge>
                )}
              </div>
              {!p.active ? (
                <Button size="sm" onClick={() => onChoose(p.id)}>
                  选择
                </Button>
              ) : null}
            </CardHeader>
            <CardContent className="space-y-3 text-sm">
              <div className="flex items-center gap-3">
                <Label htmlFor={`interval-${p.id}`}>请求间隔（秒）</Label>
                <Input
                  id={`interval-${p.id}`}
                  type="number"
                  min="1"
                  max="30"
                  className="w-20"
                  value={Math.round((cfg?.min_interval_ms || 5000) / 1000)}
                  onChange={(e) =>
                    onInterval(p.id, Number(e.target.value) * 1000)
                  }
                />
              </div>
              <div className="flex items-center gap-3">
                <Switch
                  id={`enabled-${p.id}`}
                  checked={!!cfg?.enabled}
                  onCheckedChange={(v) => onToggle(p.id, v)}
                />
                <Label htmlFor={`enabled-${p.id}`}>
                  {cfg?.enabled ? "已开启" : "已关闭"}
                </Label>
              </div>
              <p className="text-xs text-muted-foreground">
                间隔太短容易触发网页限流，默认 5 秒。
              </p>
            </CardContent>
          </Card>
        );
      })}
    </div>
  );
}

function AppsView({
  tools,
  manual,
  busy,
  managed,
  onScan,
  onTakeover,
  onRestore,
  onCopy,
  onCodexEnv,
}) {
  const cmds = { claude: manual?.claude_cmd, codex: manual?.codex_cmd };
  return (
    <div className="grid max-w-3xl gap-4">
      <div className="flex items-center justify-between">
        <h2 className="text-lg font-semibold">应用</h2>
        <Button variant="outline" size="sm" onClick={onScan}>
          <RefreshCw size={14} className="mr-1" />
          重新检测
        </Button>
      </div>
      {Object.entries(APP_NAMES).map(([tool, name]) => {
        const found = tools.find((t) => t.tool === tool);
        const isManaged = managed(tool);
        return (
          <Card key={tool}>
            <CardHeader className="flex-row items-center justify-between space-y-0">
              <div className="flex items-center gap-2">
                <CardTitle>{name}</CardTitle>
                {isManaged && <Badge>已接管</Badge>}
              </div>
              <span className="text-xs text-muted-foreground">
                {found ? found.path : "未检测到"}
              </span>
            </CardHeader>
            <CardContent className="space-y-3">
              {cmds[tool] && (
                <div className="flex items-center gap-2">
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={() => onCopy(cmds[tool])}
                  >
                    <Terminal size={14} className="mr-1" />
                    复制启动命令
                  </Button>
                  <code className="flex-1 truncate rounded bg-muted px-2 py-1 text-xs">
                    {cmds[tool]}
                  </code>
                </div>
              )}
              <div className="flex gap-2">
                <Button
                  size="sm"
                  disabled={!!busy}
                  onClick={() => onTakeover(tool)}
                >
                  {busy === tool ? (
                    <Loader2 size={14} className="mr-1 animate-spin" />
                  ) : (
                    <FilePen size={14} className="mr-1" />
                  )}
                  写入配置（自动备份）
                </Button>
                {isManaged && (
                  <Button
                    variant="destructive"
                    size="sm"
                    disabled={!!busy}
                    onClick={() => onRestore(tool)}
                  >
                    <RotateCcw size={14} className="mr-1" />
                    还原
                  </Button>
                )}
              </div>
              {tool === "codex" && (
                <div className="flex gap-2">
                  <Button variant="ghost" size="sm" onClick={() => onCodexEnv(false)}>
                    复制 export 行
                  </Button>
                  <Button variant="ghost" size="sm" onClick={() => onCodexEnv(true)}>
                    写入 ~/.zshrc（自动备份）
                  </Button>
                </div>
              )}
            </CardContent>
          </Card>
        );
      })}
    </div>
  );
}

function LogsView({ logs }) {
  return (
    <div className="max-w-3xl">
      <h2 className="mb-3 text-lg font-semibold">日志</h2>
      <pre className="h-[60vh] overflow-auto whitespace-pre-wrap rounded-lg border bg-card p-4 text-xs">
        {logs.join("\n") || "暂无日志"}
      </pre>
    </div>
  );
}

function ManualView({ manual, onCopy }) {
  return (
    <div className="grid max-w-3xl gap-4">
      <h2 className="text-lg font-semibold">手动配置</h2>
      <Card>
        <CardContent className="space-y-3 text-sm">
          <p>
            代理地址：<code className="rounded bg-muted px-1">{manual.base_url}</code>
            <Button variant="ghost" size="sm" onClick={() => onCopy(manual.base_url)}>
              <Copy size={12} className="mr-1" />
              复制
            </Button>
          </p>
          <p>
            Token：<code className="rounded bg-muted px-1">{manual.token}</code>
            <Button variant="ghost" size="sm" onClick={() => onCopy(manual.token)}>
              <Copy size={12} className="mr-1" />
              复制
            </Button>
          </p>
          {[
            ["Claude Code", manual.claude_cmd],
            ["Codex", manual.codex_cmd],
          ].map(([name, cmd]) => (
            <div key={name}>
              <p className="mb-1 font-medium">{name}</p>
              <div className="flex items-center gap-2">
                <code className="flex-1 whitespace-pre-wrap break-all rounded bg-muted px-2 py-1 text-xs">
                  {cmd}
                </code>
                <Button variant="outline" size="sm" onClick={() => onCopy(cmd)}>
                  <Copy size={12} className="mr-1" />
                  复制命令
                </Button>
              </div>
            </div>
          ))}
        </CardContent>
      </Card>
    </div>
  );
}
