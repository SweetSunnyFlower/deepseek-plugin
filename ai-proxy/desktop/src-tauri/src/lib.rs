pub mod auth;
pub mod bridge;
pub mod config_manager;
pub mod http;
pub mod logs;
pub mod parser;
pub mod provider;
pub mod settings;
pub mod state;
pub mod types;

use state::AppState;
use tauri::Manager;

pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let dir = app.path().app_config_dir()?;
            let state = AppState::new(dir);
            let port = state.settings.read().unwrap().port;
            let router = http::router(state.clone());
            tauri::async_runtime::spawn(async move {
                let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
                let listener = tokio::net::TcpListener::bind(addr)
                    .await
                    .expect("端口 8787 被占用");
                axum::serve(listener, router).await.expect("代理启动失败");
            });
            app.manage(state);
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("Tauri 启动失败");
}
