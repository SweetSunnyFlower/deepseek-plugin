use ai_proxy_desktop_lib::{http, state::AppState};
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::{connect_async, tungstenite::Message};

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

async fn ws_echo(addr: String, payloads: Vec<String>) {
    let (mut ws, _) = connect_async(format!("ws://{addr}/bridge")).await.unwrap();
    let prompt_msg = tokio::time::timeout(std::time::Duration::from_secs(3), ws.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(prompt_msg.to_text().unwrap()).unwrap();
    let id = v["id"].as_str().unwrap();
    for p in payloads {
        ws.send(Message::Text(
            format!(r#"{{"type":"stream_chunk","id":"{id}","chunk":"{p}"}}"#).into(),
        ))
        .await
        .unwrap();
    }
    ws.send(Message::Text(
        format!(r#"{{"type":"stream_end","id":"{id}"}}"#).into(),
    ))
    .await
    .unwrap();
}

async fn wait_bridge(addr: &str) {
    let client = reqwest::Client::new();
    for _ in 0..100 {
        let v: serde_json::Value = client
            .get(format!("http://{addr}/status"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        if v["bridge"] == true {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("bridge 未在预期时间内连接");
}

#[tokio::test]
async fn tool_call_via_chat_completions() {
    let (addr, token) = start_server().await;
    tokio::spawn(ws_echo(
        addr.clone(),
        vec![
            "data: {\\\"v\\\":{\\\"response\\\":{\\\"fragments\\\":[{\\\"type\\\":\\\"RESPONSE\\\",\\\"content\\\":\\\"<tool_call name=\\\\\\\"Read\\\\\\\">\\\"}]}}}\\n\\n".into(),
            "data: {\\\"v\\\":\\\"<param name=\\\\\\\"file_path\\\\\\\" string=\\\\\\\"true\\\\\\\">/a.js</param>\\\"}\\n\\n".into(),
            "data: {\\\"v\\\":\\\"</tool_call>\\\"}\\n\\n".into(),
            "data: {\\\"p\\\":\\\"response/status\\\",\\\"o\\\":\\\"SET\\\",\\\"v\\\":\\\"FINISHED\\\"}\\n\\n".into(),
        ],
    ));
    wait_bridge(&addr).await;
    let resp = reqwest::Client::new()
        .post(format!("http://{addr}/v1/chat/completions"))
        .bearer_auth(&token)
        .json(&serde_json::json!({
            "model": "deepseek",
            "stream": false,
            "messages": [
                { "role": "system", "content": "sys" },
                { "role": "user", "content": "读 /a.js" }
            ],
            "tools": [{ "type": "function", "function": {
                "name": "Read", "description": "读文件",
                "parameters": { "type": "object" }
            }}]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let v: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(v["choices"][0]["message"]["tool_calls"][0]["function"]["name"], "Read");
    assert_eq!(
        v["choices"][0]["message"]["tool_calls"][0]["function"]["arguments"],
        "{\"file_path\":\"/a.js\"}"
    );
    assert_eq!(v["choices"][0]["finish_reason"], "tool_calls");
}

#[tokio::test]
async fn text_via_chat_completions() {
    let (addr, token) = start_server().await;
    tokio::spawn(ws_echo(
        addr.clone(),
        vec![
            "data: {\\\"v\\\":{\\\"response\\\":{\\\"fragments\\\":[{\\\"type\\\":\\\"RESPONSE\\\",\\\"content\\\":\\\"好的\\\"}]}}}\\n\\n".into(),
            "data: {\\\"p\\\":\\\"response/status\\\",\\\"o\\\":\\\"SET\\\",\\\"v\\\":\\\"FINISHED\\\"}\\n\\n".into(),
        ],
    ));
    wait_bridge(&addr).await;
    let resp = reqwest::Client::new()
        .post(format!("http://{addr}/v1/chat/completions"))
        .bearer_auth(&token)
        .json(&serde_json::json!({
            "model": "deepseek",
            "stream": false,
            "messages": [{ "role": "user", "content": "你好" }]
        }))
        .send()
        .await
        .unwrap();
    let v: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(v["choices"][0]["message"]["content"], "好的");
    assert_eq!(v["choices"][0]["finish_reason"], "stop");
}
