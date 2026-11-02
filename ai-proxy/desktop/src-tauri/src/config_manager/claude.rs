use crate::config_manager::backup;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

pub fn settings_path(home: &Path) -> PathBuf {
    home.join(".claude/settings.json")
}

pub fn takeover(home: &Path, base_url: &str, token: &str) -> Result<PathBuf, String> {
    let path = settings_path(home);
    let mut v: Value = if path.exists() {
        let text = fs::read_to_string(&path).map_err(|e| e.to_string())?;
        serde_json::from_str(&text).map_err(|e| format!("settings.json 解析失败: {e}"))?
    } else {
        json!({})
    };
    if !v.is_object() {
        return Err("settings.json 顶层必须是对象".into());
    }
    let env = v
        .as_object_mut()
        .unwrap()
        .entry("env".to_string())
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or("env 字段必须是对象")?;
    let already = env.get("ANTHROPIC_BASE_URL").and_then(|x| x.as_str()) == Some(base_url)
        && env.get("ANTHROPIC_AUTH_TOKEN").and_then(|x| x.as_str()) == Some(token);
    let saved = if already {
        None
    } else {
        Some(backup(&path).map_err(|e| e.to_string())?)
    };
    env.insert("ANTHROPIC_BASE_URL".into(), json!(base_url));
    env.insert("ANTHROPIC_AUTH_TOKEN".into(), json!(token));
    fs::write(
        &path,
        serde_json::to_string_pretty(&v).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    Ok(saved.unwrap_or(path))
}

pub fn restore(home: &Path, backup: &Path) -> Result<(), String> {
    if !backup.exists() {
        return Err("备份文件不存在".into());
    }
    fs::copy(backup, settings_path(home)).map_err(|e| e.to_string())?;
    Ok(())
}
