use ai_proxy_desktop_lib::{http, state::AppState};
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::{connect_async, tungstenite::Message};

async fn start_server() -> String {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new(dir.path().to_path_buf());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let app = http::router(state);
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    addr
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
        format!(
            r#"{{"type":"stream_end","id":"{id}","url":"https://chat.deepseek.com/a/chat/s/t1"}}"#
        )
        .into(),
    ))
    .await
    .unwrap();
}

#[tokio::test]
async fn text_reply_round_trip() {
    let addr = start_server().await;
    tokio::spawn(ws_echo(
        addr.clone(),
        vec![
            "data: {\\\"v\\\":{\\\"response\\\":{\\\"fragments\\\":[{\\\"type\\\":\\\"RESPONSE\\\",\\\"content\\\":\\\"你好，我是\\\"}]}}}\\n\\n".into(),
            "data: {\\\"v\\\":\\\" DeepSeek。\\\"}\\n\\n".into(),
            "data: {\\\"p\\\":\\\"response/status\\\",\\\"o\\\":\\\"SET\\\",\\\"v\\\":\\\"FINISHED\\\"}\\n\\n".into(),
        ],
    ));
    let resp = reqwest::Client::new()
        .post(format!("http://{addr}/v1/messages"))
        .json(&serde_json::json!({
            "model": "deepseek-web",
            "max_tokens": 1024,
            "stream": false,
            "messages": [{ "role": "user", "content": "自我介绍" }]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let v: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(v["content"][0]["text"], "你好，我是 DeepSeek。");
    assert_eq!(v["stop_reason"], "end_turn");
}

#[tokio::test]
async fn tool_call_reply_round_trip() {
    let addr = start_server().await;
    tokio::spawn(ws_echo(
        addr.clone(),
        vec![
            "data: {\\\"v\\\":{\\\"response\\\":{\\\"fragments\\\":[{\\\"type\\\":\\\"RESPONSE\\\",\\\"content\\\":\\\"<tool_call name=\\\\\\\"Read\\\\\\\">\\\"}]}}}\\n\\n".into(),
            "data: {\\\"v\\\":\\\"<param name=\\\\\\\"file_path\\\\\\\" string=\\\\\\\"true\\\\\\\">/a.js</param>\\\"}\\n\\n".into(),
            "data: {\\\"v\\\":\\\"</tool_call>\\\"}\\n\\n".into(),
            "data: {\\\"p\\\":\\\"response/status\\\",\\\"o\\\":\\\"SET\\\",\\\"v\\\":\\\"FINISHED\\\"}\\n\\n".into(),
        ],
    ));
    let resp = reqwest::Client::new()
        .post(format!("http://{addr}/v1/messages"))
        .json(&serde_json::json!({
            "model": "deepseek-web",
            "max_tokens": 1024,
            "stream": false,
            "messages": [{ "role": "user", "content": "读 /a.js" }],
            "tools": [{ "name": "Read", "description": "读文件", "input_schema": { "type": "object" } }]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let v: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(v["content"][0]["type"], "tool_use");
    assert_eq!(v["content"][0]["name"], "Read");
    assert_eq!(v["content"][0]["input"]["file_path"], "/a.js");
    assert_eq!(v["stop_reason"], "tool_use");
}
