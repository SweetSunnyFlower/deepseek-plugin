use ai_proxy_desktop_lib::{http, state::AppState};
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::{connect_async, tungstenite::Message};

#[tokio::test]
async fn bridge_receives_prompt_and_streams_response() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new(dir.path().to_path_buf());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = http::router(state);
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let (mut ws, _) = connect_async(format!("ws://{addr}/bridge")).await.unwrap();
    let client = reqwest::Client::new();
    let post = tokio::spawn({
        let client = client.clone();
        async move {
            client
                .post(format!("http://{addr}/test"))
                .json(&serde_json::json!({"prompt": "hi"}))
                .send()
                .await
                .unwrap()
        }
    });
    let prompt_msg = tokio::time::timeout(std::time::Duration::from_secs(3), ws.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let text = prompt_msg.to_text().unwrap();
    assert!(text.contains("send_prompt"));
    let v: serde_json::Value = serde_json::from_str(text).unwrap();
    let id = v["id"].as_str().unwrap();

    ws.send(Message::Text(
        format!(r#"{{"type":"stream_chunk","id":"{id}","chunk":"hello"}}"#).into(),
    ))
    .await
    .unwrap();
    ws.send(Message::Text(
        format!(
            r#"{{"type":"stream_end","id":"{id}","url":"https://chat.deepseek.com/a/chat/s/x"}}"#
        )
        .into(),
    ))
    .await
    .unwrap();

    let resp = post.await.unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.text().await.unwrap(), "hello");
}
