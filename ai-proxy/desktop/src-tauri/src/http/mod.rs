use crate::bridge::BridgeEvent;
use crate::state::AppState;
use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};

pub mod anthropic;
pub mod openai;
pub mod session_id;

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/status", get(status))
        .route("/logs", get(logs))
        .route("/bridge", get(ws_handler))
        .route("/test", post(test_prompt))
        .route("/v1/messages", post(anthropic::handle))
        .route("/v1/messages/count_tokens", post(anthropic::count_tokens))
        .route("/v1/chat/completions", post(openai::handle))
        .with_state(state)
}

async fn logs(State(state): State<AppState>) -> Json<Vec<String>> {
    Json(state.logs.snapshot())
}

async fn status(State(state): State<AppState>) -> Json<Value> {
    let s = state.settings.read().unwrap();
    Json(json!({
        "bridge": state.bridge.connected(),
        "pending": state.bridge.pending_count(),
        "provider": s.active_provider,
        "interval_ms": s
            .providers
            .get(&s.active_provider)
            .map(|p| p.min_interval_ms)
            .unwrap_or(5000),
    }))
}

async fn ws_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    let origin = headers
        .get("origin")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !origin.is_empty() && !origin.starts_with("chrome-extension://") {
        return StatusCode::FORBIDDEN.into_response();
    }
    ws.on_upgrade(move |socket| ws_loop(socket, state))
}

async fn ws_loop(socket: WebSocket, state: AppState) {
    let Some(mut out) = state.bridge.attach() else {
        return;
    };
    let (mut sink, mut stream) = socket.split();
    let writer = tokio::spawn(async move {
        while let Some(text) = out.recv().await {
            if sink.send(Message::Text(text.into())).await.is_err() {
                break;
            }
        }
    });
    while let Some(Ok(msg)) = stream.next().await {
        if let Message::Text(text) = msg {
            if let Ok(v) = serde_json::from_str::<Value>(&text) {
                if v.get("type").and_then(|x| x.as_str()) == Some("providers_status") {
                    state.update_provider_status(&v);
                }
                state.bridge.handle_client(v);
            }
        }
    }
    state.bridge.detach();
    writer.abort();
}

async fn test_prompt(State(state): State<AppState>, Json(body): Json<Value>) -> Response {
    let prompt = body.get("prompt").and_then(|x| x.as_str()).unwrap_or("");
    let mut rx = match state.bridge.ask(prompt, true, None) {
        Ok(rx) => rx,
        Err(e) => return (StatusCode::SERVICE_UNAVAILABLE, e).into_response(),
    };
    let mut out = String::new();
    while let Some(ev) = rx.recv().await {
        match ev {
            BridgeEvent::Chunk(c) => out.push_str(&c),
            BridgeEvent::End(_) => break,
            BridgeEvent::Error(e) => {
                out.push_str(&format!("\n[错误] {e}"));
                break;
            }
        }
    }
    (StatusCode::OK, out).into_response()
}
