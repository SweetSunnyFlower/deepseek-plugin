use crate::bridge::BridgeHub;
use crate::logs::LogBuffer;
use crate::provider::controller::ProviderController;
use crate::provider::session::SessionStore;
use crate::session_db::SessionDb;
use crate::settings::{self, AppSettings};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

#[derive(Clone)]
pub struct AppState {
    pub settings: Arc<RwLock<AppSettings>>,
    pub bridge: Arc<BridgeHub>,
    pub providers: Arc<BTreeMap<String, Arc<ProviderController>>>,
    pub sessions: Arc<std::sync::Mutex<SessionStore>>,
    pub provider_status: Arc<RwLock<HashMap<String, bool>>>,
    pub session_db: Option<Arc<SessionDb>>,
    pub logs: Arc<LogBuffer>,
    pub config_dir: PathBuf,
}

impl AppState {
    pub fn new(config_dir: PathBuf) -> Self {
        let loaded = settings::load(&config_dir);
        let _ = settings::save(&config_dir, &loaded);
        let settings = Arc::new(RwLock::new(loaded));
        let logs = Arc::new(LogBuffer::new(500));
        let session_db = match SessionDb::open(&config_dir.join("sessions.db")) {
            Ok(db) => Some(Arc::new(db)),
            Err(e) => {
                logs.push(format!("[会话库] 打开失败，退回启发式匹配: {e}"));
                None
            }
        };
        let providers = {
            let s = settings.read().unwrap();
            s.providers
                .keys()
                .map(|k| {
                    (
                        k.clone(),
                        Arc::new(ProviderController::new(settings.clone(), k)),
                    )
                })
                .collect()
        };
        Self {
            settings,
            bridge: Arc::new(BridgeHub::new()),
            providers: Arc::new(providers),
            sessions: Arc::new(std::sync::Mutex::new(SessionStore::new(20))),
            provider_status: Arc::new(RwLock::new(HashMap::new())),
            session_db,
            logs,
            config_dir,
        }
    }

    pub fn log(&self, msg: impl Into<String>) {
        self.logs.push(msg);
    }

    pub fn update_provider_status(&self, v: &serde_json::Value) {
        let Some(list) = v.get("providers").and_then(|x| x.as_array()) else {
            return;
        };
        let mut status = self.provider_status.write().unwrap();
        for item in list {
            let Some(id) = item.get("id").and_then(|x| x.as_str()) else {
                continue;
            };
            let logged_in = item.get("logged_in").and_then(|x| x.as_bool()).unwrap_or(false);
            let prev = status.insert(id.to_string(), logged_in);
            let active = {
                let s = self.settings.read().unwrap();
                s.active_provider.clone()
            };
            if active == id && !logged_in {
                self.settings.write().unwrap().active_provider.clear();
                self.log(format!("供应商 {id} 已登出，已取消选中"));
            }
            if prev != Some(logged_in) {
                self.log(format!(
                    "[登录状态] {id} {}",
                    if logged_in { "已登录" } else { "未登录" }
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_providers_status_message() {
        let dir = tempfile::tempdir().unwrap();
        let state = AppState::new(dir.path().to_path_buf());
        let msg = serde_json::json!({
            "type": "providers_status",
            "providers": [{ "id": "deepseek", "logged_in": true }]
        });
        state.update_provider_status(&msg);
        assert_eq!(
            *state.provider_status.read().unwrap().get("deepseek").unwrap(),
            true
        );
    }

    #[test]
    fn logout_clears_active_provider() {
        let dir = tempfile::tempdir().unwrap();
        let state = AppState::new(dir.path().to_path_buf());
        {
            let mut s = state.settings.write().unwrap();
            s.active_provider = "deepseek".into();
        }
        state.update_provider_status(&serde_json::json!({
            "type": "providers_status",
            "providers": [{ "id": "deepseek", "logged_in": false }]
        }));
        assert!(state.settings.read().unwrap().active_provider.is_empty());
    }
}
