use crate::config_manager::backup;
use std::fs;
use std::path::{Path, PathBuf};

pub fn config_path(home: &Path) -> PathBuf {
    home.join(".codex/config.toml")
}

pub fn takeover(home: &Path, base_url: &str, _token: &str) -> Result<Option<PathBuf>, String> {
    let path = config_path(home);
    let text = if path.exists() {
        fs::read_to_string(&path).map_err(|e| e.to_string())?
    } else {
        String::new()
    };
    let mut doc: toml_edit::DocumentMut = text
        .parse()
        .map_err(|e| format!("config.toml 解析失败: {e}"))?;
    let already = doc
        .get("model_providers")
        .and_then(|t| t.get("bridge"))
        .and_then(|t| t.get("base_url"))
        .and_then(|v| v.as_str())
        == Some(base_url);
    let saved = if already {
        None
    } else {
        Some(Some(backup(&path).map_err(|e| e.to_string())?))
    };
    let root = doc.as_table_mut();
    if !root.contains_key("model_providers") {
        let mut mprov = toml_edit::Table::new();
        mprov.set_implicit(true);
        root.insert("model_providers", toml_edit::Item::Table(mprov));
    }
    let providers = root
        .get_mut("model_providers")
        .and_then(|i| i.as_table_mut())
        .ok_or("model_providers 格式异常")?;
    let mut table = toml_edit::Table::new();
    table.set_implicit(false);
    table["base_url"] = toml_edit::value(base_url);
    table["env_key"] = toml_edit::value("DEEPSEEK_BRIDGE_TOKEN");
    providers.insert("bridge", toml_edit::Item::Table(table));
    root.insert("model", toml_edit::value("bridge"));
    fs::write(&path, doc.to_string()).map_err(|e| e.to_string())?;
    Ok(saved.unwrap_or(None))
}

pub fn restore(home: &Path, backup: &Path) -> Result<(), String> {
    if !backup.exists() {
        return Err("备份文件不存在".into());
    }
    fs::copy(backup, config_path(home)).map_err(|e| e.to_string())?;
    Ok(())
}
