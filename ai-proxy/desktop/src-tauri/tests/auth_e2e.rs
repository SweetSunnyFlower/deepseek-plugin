use ai_proxy_desktop_lib::{http, state::AppState};

async fn start_server() -> (String, String) {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new(dir.path().to_path_buf());
    let token = state.settings.read().unwrap().token.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let app = http::router(state);
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (addr, token)
}

#[tokio::test]
async fn rejects_without_token() {
    let (addr, _) = start_server().await;
    let resp = reqwest::Client::new()
        .post(format!("http://{addr}/v1/messages"))
        .json(&serde_json::json!({
            "model": "deepseek-web",
            "messages": [{ "role": "user", "content": "hi" }]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn rejects_wrong_token() {
    let (addr, _) = start_server().await;
    let resp = reqwest::Client::new()
        .post(format!("http://{addr}/v1/chat/completions"))
        .bearer_auth("wrong-token")
        .json(&serde_json::json!({
            "model": "deepseek",
            "messages": [{ "role": "user", "content": "hi" }]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn correct_token_without_bridge_is_503() {
    let (addr, token) = start_server().await;
    let resp = reqwest::Client::new()
        .post(format!("http://{addr}/v1/messages"))
        .header("x-api-key", &token)
        .json(&serde_json::json!({
            "model": "deepseek-web",
            "messages": [{ "role": "user", "content": "hi" }]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 503);
}
