use ai_proxy_desktop_lib::{http, state::AppState};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async, tungstenite::Message};

async fn start_server() -> (String, String, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new(dir.path().to_path_buf());
    let token = state.settings.read().unwrap().token.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let app = http::router(state);
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (addr, token, dir)
}

async fn wait_bridge(addr: &str) {
    let client = reqwest::Client::new();
    for _ in 0..100 {
        let v: Value = client
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
    panic!("bridge 未连接");
}

async fn ws_responder(addr: String, urls: Vec<String>, captured: mpsc::UnboundedSender<Value>) {
    let (mut ws, _) = connect_async(format!("ws://{addr}/bridge")).await.unwrap();
    let mut idx = 0;
    while let Some(Ok(Message::Text(text))) = ws.next().await {
        let Ok(v) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        if v["type"].as_str() != Some("send_prompt") {
            continue;
        }
        captured.send(v.clone()).unwrap();
        let id = v["id"].as_str().unwrap();
        let chunk = "data: {\"v\":{\"response\":{\"fragments\":[{\"type\":\"RESPONSE\",\"content\":\"好\"}]}}}\n\n";
        ws.send(Message::Text(
            json!({ "type": "stream_chunk", "id": id, "chunk": chunk }).to_string().into(),
        ))
        .await
        .unwrap();
        let url = urls
            .get(idx)
            .cloned()
            .unwrap_or_else(|| urls.last().unwrap().clone());
        idx += 1;
        ws.send(Message::Text(
            json!({ "type": "stream_end", "id": id, "url": url }).to_string().into(),
        ))
        .await
        .unwrap();
    }
}

async fn send(addr: &str, token: &str, cli: &str, messages: Value) -> Value {
    let resp = reqwest::Client::new()
        .post(format!("http://{addr}/v1/messages"))
        .header("x-api-key", token)
        .header("x-claude-code-session-id", cli)
        .json(&json!({
            "model": "glm-5.1",
            "max_tokens": 1024,
            "stream": false,
            "messages": messages,
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    resp.json().await.unwrap()
}

#[tokio::test]
async fn precise_mapping_continue_force_and_fork() {
    let (addr, token, _dir) = start_server().await;
    let (tx, mut rx) = mpsc::unbounded_channel();
    let chat_a = "https://chat.deepseek.com/a/chat/s/chat-a";
    tokio::spawn(ws_responder(
        addr.clone(),
        vec![
            chat_a.to_string(),
            chat_a.to_string(),
            "https://chat.deepseek.com/a/chat/s/chat-b".to_string(),
            "https://chat.deepseek.com/a/chat/s/chat-c".to_string(),
        ],
        tx,
    ));
    wait_bridge(&addr).await;

    let first = json!([{ "role": "user", "content": "第一轮问题" }]);
    send(&addr, &token, "cli-a", first).await;
    let p1 = rx.recv().await.unwrap();
    assert_eq!(p1["new_chat"], true);
    assert!(p1["prompt"].as_str().unwrap().contains("第一轮问题"));

    let resumed = json!([
        { "role": "user", "content": "第一轮问题" },
        { "role": "assistant", "content": "已回复" },
        { "role": "user", "content": "第二轮问题" }
    ]);
    send(&addr, &token, "cli-a", resumed.clone()).await;
    let p2 = rx.recv().await.unwrap();
    assert_eq!(p2["new_chat"], false);
    assert_eq!(p2["chat_url"], chat_a);
    let prompt2 = p2["prompt"].as_str().unwrap();
    assert!(!prompt2.contains("第一轮问题"));
    assert!(prompt2.contains("第二轮问题"));

    send(&addr, &token, "cli-a", resumed).await;
    let p3 = rx.recv().await.unwrap();
    assert_eq!(p3["new_chat"], false);
    assert_eq!(p3["chat_url"], chat_a);
    let prompt3 = p3["prompt"].as_str().unwrap();
    assert!(prompt3.contains("<system>") || prompt3.contains("第一轮问题"));

    send(
        &addr,
        &token,
        "cli-b",
        json!([{ "role": "user", "content": "另一个会话" }]),
    )
    .await;
    let p4 = rx.recv().await.unwrap();
    assert_eq!(p4["new_chat"], true);

    let logs: Vec<String> = reqwest::get(format!("http://{addr}/logs"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let joined = logs.join("\n");
    assert!(joined.contains("cli=cli-a"));
    assert!(joined.contains("网页已分叉"));
    assert!(joined.contains("cli=cli-b"));
}
