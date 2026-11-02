use crate::config_manager::{claude, codex, detect};
use crate::settings::{save, AppSettings, ManagedTool};
use crate::state::AppState;
use serde_json::{json, Value};
use std::path::PathBuf;
use tauri::State;

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> AppSettings {
    state.settings.read().unwrap().clone()
}

#[tauri::command]
pub fn get_providers_status(state: State<'_, AppState>) -> Vec<Value> {
    let s = state.settings.read().unwrap();
    let status = state.provider_status.read().unwrap();
    s.providers
        .iter()
        .map(|(id, p)| {
            json!({
                "id": id,
                "logged_in": status.get(id).copied().unwrap_or(false),
                "enabled": p.enabled,
                "min_interval_ms": p.min_interval_ms,
                "active": s.active_provider == *id,
            })
        })
        .collect()
}

#[tauri::command]
pub fn select_provider(state: State<'_, AppState>, id: String) -> Result<AppSettings, String> {
    let status = state.provider_status.read().unwrap().clone();
    let mut s = state.settings.write().unwrap();
    crate::settings::apply_selection(&mut s, &status, &id)?;
    let snapshot = s.clone();
    drop(s);
    save(&state.config_dir, &snapshot).map_err(|e| e.to_string())?;
    state.log(format!("[选择] 当前供应商: {id}"));
    Ok(snapshot)
}

#[tauri::command]
pub fn refresh_providers(state: State<'_, AppState>) -> bool {
    state.bridge.notify(json!({ "type": "refresh_providers" }))
}

#[tauri::command]
pub fn update_provider(
    state: State<'_, AppState>,
    provider: String,
    enabled: Option<bool>,
    min_interval_ms: Option<u64>,
) -> Result<AppSettings, String> {
    let mut s = state.settings.write().unwrap();
    let p = s.providers.get_mut(&provider).ok_or("未知供应商")?;
    if let Some(v) = enabled {
        p.enabled = v;
    }
    if let Some(v) = min_interval_ms {
        if !(1000..=30000).contains(&v) {
            return Err("请求间隔需在 1–30 秒之间".into());
        }
        p.min_interval_ms = v;
    }
    let snapshot = s.clone();
    drop(s);
    save(&state.config_dir, &snapshot).map_err(|e| e.to_string())?;
    state.log(format!(
        "[设置] {} 间隔 {}ms 开关 {}",
        provider,
        snapshot.providers[&provider].min_interval_ms,
        snapshot.providers[&provider].enabled
    ));
    Ok(snapshot)
}

#[tauri::command]
pub fn get_status(state: State<'_, AppState>) -> Value {
    let s = state.settings.read().unwrap();
    let interval = s
        .providers
        .get(&s.active_provider)
        .map(|p| p.min_interval_ms)
        .unwrap_or(5000);
    json!({
        "bridge": state.bridge.connected(),
        "pending": state.bridge.pending_count(),
        "provider": s.active_provider,
        "interval_ms": interval,
    })
}

#[tauri::command]
pub fn get_logs(state: State<'_, AppState>) -> Vec<String> {
    state.logs.snapshot()
}

#[tauri::command]
pub fn detect_tools() -> Vec<detect::DetectedTool> {
    detect::detect_all()
}

#[tauri::command]
pub fn takeover_tool(state: State<'_, AppState>, tool: String) -> Result<String, String> {
    let s = state.settings.read().unwrap().clone();
    let base = format!("http://127.0.0.1:{}", s.port);
    let home = home_dir()?;
    let backup: Option<std::path::PathBuf> = match tool.as_str() {
        "claude" => claude::takeover(&home, &base, &s.token)?,
        "codex" => codex::takeover(&home, &format!("{base}/v1"), &s.token)?,
        _ => return Err("暂不支持该工具".into()),
    };
    let Some(backup) = backup else {
        state.log(format!("[接管] {tool} 已接管，仅更新 token"));
        return Ok("已接管（未新建备份）".into());
    };
    let mut settings = state.settings.write().unwrap();
    settings.managed_tools.push(ManagedTool {
        tool,
        backup: backup.to_string_lossy().to_string(),
        taken_at: chrono::Local::now().to_rfc3339(),
    });
    let snapshot = settings.clone();
    drop(settings);
    save(&state.config_dir, &snapshot).map_err(|e| e.to_string())?;
    state.log(format!("[接管] {} 备份到 {}", snapshot.managed_tools.last().unwrap().tool, snapshot.managed_tools.last().unwrap().backup));
    Ok(backup.to_string_lossy().to_string())
}

#[tauri::command]
pub fn restore_tool(state: State<'_, AppState>, tool: String) -> Result<(), String> {
    let backup = {
        let s = state.settings.read().unwrap();
        s.managed_tools
            .iter()
            .rev()
            .find(|m| m.tool == tool)
            .map(|m| PathBuf::from(&m.backup))
            .ok_or("没有找到该工具的备份")?
    };
    let home = home_dir()?;
    match tool.as_str() {
        "claude" => claude::restore(&home, &backup)?,
        "codex" => codex::restore(&home, &backup)?,
        _ => return Err("暂不支持该工具".into()),
    }
    let mut s = state.settings.write().unwrap();
    s.managed_tools.retain(|m| m.tool != tool);
    let snapshot = s.clone();
    drop(s);
    save(&state.config_dir, &snapshot).map_err(|e| e.to_string())?;
    state.log(format!("[还原] {tool} 已还原"));
    Ok(())
}

#[tauri::command]
pub fn get_manual_config(state: State<'_, AppState>) -> Value {
    let s = state.settings.read().unwrap();
    let base = format!("http://127.0.0.1:{}", s.port);
    let openai = format!("{base}/v1");
    let token = s.token.as_str();
    json!({
        "base_url": base,
        "openai_base_url": openai,
        "token": token,
        "claude_cmd": format!(
            "ANTHROPIC_BASE_URL=\"{base}\" ANTHROPIC_AUTH_TOKEN=\"{token}\" claude"
        ),
        "codex_cmd": format!(
            "grep -qs '[model_providers.bridge]' ~/.codex/config.toml || printf '\\n[model_providers.bridge]\\nname = \"bridge\"\\nbase_url = \"{openai}\"\\nenv_key = \"DEEPSEEK_BRIDGE_TOKEN\"\\nmodel = \"bridge\"\\n' >> ~/.codex/config.toml; DEEPSEEK_BRIDGE_TOKEN=\"{token}\" codex"
        ),
    })
}

#[tauri::command]
pub fn install_codex_env(state: State<'_, AppState>, append: bool) -> Result<String, String> {
    let token = state.settings.read().unwrap().token.clone();
    let line = format!("export DEEPSEEK_BRIDGE_TOKEN=\"{token}\"");
    if !append {
        return Ok(line);
    }
    let rc = home_dir()?.join(".zshrc");
    let existing = if rc.exists() {
        std::fs::read_to_string(&rc).map_err(|e| e.to_string())?
    } else {
        String::new()
    };
    if existing.contains("DEEPSEEK_BRIDGE_TOKEN") {
        return Ok("已存在，无需重复写入".into());
    }
    crate::config_manager::backup(&rc).map_err(|e| e.to_string())?;
    let mut out = existing;
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(&line);
    out.push('\n');
    std::fs::write(&rc, out).map_err(|e| e.to_string())?;
    state.log("[设置] 已写入 .zshrc 的 DEEPSEEK_BRIDGE_TOKEN");
    Ok("已写入 ~/.zshrc".into())
}

fn home_dir() -> Result<PathBuf, String> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .ok_or_else(|| "无法读取 HOME 目录".to_string())
}
