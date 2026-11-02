use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

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
