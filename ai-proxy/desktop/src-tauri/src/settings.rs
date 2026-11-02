use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderSettings {
    pub enabled: bool,
    pub min_interval_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagedTool {
    pub tool: String,
    pub backup: String,
    pub taken_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    pub port: u16,
    pub token: String,
    pub active_provider: String,
    pub providers: BTreeMap<String, ProviderSettings>,
    pub managed_tools: Vec<ManagedTool>,
}

impl Default for AppSettings {
    fn default() -> Self {
        let mut providers = BTreeMap::new();
        providers.insert(
            "deepseek".into(),
            ProviderSettings {
                enabled: true,
                min_interval_ms: 5000,
            },
        );
        Self {
            port: 8787,
            token: uuid::Uuid::new_v4().to_string(),
            active_provider: "deepseek".into(),
            providers,
            managed_tools: Vec::new(),
        }
    }
}

pub fn load(dir: &Path) -> AppSettings {
    let path = dir.join("settings.json");
    fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str::<AppSettings>(&s).ok())
        .unwrap_or_default()
}

pub fn save(dir: &Path, settings: &AppSettings) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    let text = serde_json::to_string_pretty(settings)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
    fs::write(dir.join("settings.json"), text)
}

pub fn apply_selection(
    settings: &mut AppSettings,
    status: &std::collections::HashMap<String, bool>,
    id: &str,
) -> Result<(), String> {
    if !settings.providers.contains_key(id) {
        return Err("未知供应商".into());
    }
    if !status.get(id).copied().unwrap_or(false) {
        return Err("该供应商未登录，无法选择".into());
    }
    settings.active_provider = id.to_string();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = AppSettings::default();
        s.providers.get_mut("deepseek").unwrap().min_interval_ms = 7000;
        save(dir.path(), &s).unwrap();
        let loaded = load(dir.path());
        assert_eq!(loaded.providers["deepseek"].min_interval_ms, 7000);
        assert_eq!(loaded.port, 8787);
        assert_eq!(loaded.token, s.token);
    }

    #[test]
    fn missing_settings_creates_default() {
        let dir = tempfile::tempdir().unwrap();
        let s = load(dir.path());
        assert_eq!(s.port, 8787);
        assert_eq!(s.providers["deepseek"].min_interval_ms, 5000);
        assert!(!s.token.is_empty());
    }

    #[test]
    fn apply_selection_rejects_unknown() {
        let mut s = AppSettings::default();
        let status = std::collections::HashMap::new();
        assert!(apply_selection(&mut s, &status, "chatgpt").is_err());
    }

    #[test]
    fn apply_selection_rejects_not_logged_in() {
        let mut s = AppSettings::default();
        let mut status = std::collections::HashMap::new();
        status.insert("deepseek".into(), false);
        assert!(apply_selection(&mut s, &status, "deepseek").is_err());
        assert_eq!(s.active_provider, "deepseek");
    }

    #[test]
    fn apply_selection_sets_active_provider() {
        let mut s = AppSettings::default();
        let mut status = std::collections::HashMap::new();
        status.insert("deepseek".into(), true);
        apply_selection(&mut s, &status, "deepseek").unwrap();
        assert_eq!(s.active_provider, "deepseek");
    }
}
