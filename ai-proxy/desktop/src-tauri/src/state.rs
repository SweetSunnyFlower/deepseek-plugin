use crate::bridge::BridgeHub;
use crate::logs::LogBuffer;
use crate::provider::controller::ProviderController;
use crate::provider::session::SessionStore;
use crate::settings::{self, AppSettings};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

#[derive(Clone)]
pub struct AppState {
    pub settings: Arc<RwLock<AppSettings>>,
    pub bridge: Arc<BridgeHub>,
    pub providers: Arc<BTreeMap<String, Arc<ProviderController>>>,
    pub sessions: Arc<std::sync::Mutex<SessionStore>>,
    pub logs: Arc<LogBuffer>,
    pub config_dir: PathBuf,
}

impl AppState {
    pub fn new(config_dir: PathBuf) -> Self {
        let loaded = settings::load(&config_dir);
        let _ = settings::save(&config_dir, &loaded);
        let settings = Arc::new(RwLock::new(loaded));
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
            logs: Arc::new(LogBuffer::new(500)),
            config_dir,
        }
    }

    pub fn log(&self, msg: impl Into<String>) {
        self.logs.push(msg);
    }
}
