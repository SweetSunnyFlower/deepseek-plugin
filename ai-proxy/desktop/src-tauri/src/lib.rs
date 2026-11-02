pub mod logs;
pub mod parser;
pub mod provider;
pub mod settings;
pub mod types;

use axum::{routing::get, Json, Router};
use serde_json::{json, Value};

async fn status() -> Json<Value> {
    Json(json!({ "bridge": false, "pending": 0 }))
}

pub fn run() {
    tauri::Builder::default()
        .setup(|_app| {
            tauri::async_runtime::spawn(async {
                let app = Router::new().route("/status", get(status));
                let listener = tokio::net::TcpListener::bind("127.0.0.1:8787")
                    .await
                    .expect("端口 8787 被占用");
                axum::serve(listener, app).await.expect("代理启动失败");
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("Tauri 启动失败");
}
