use crate::bridge::BridgeEvent;
use crate::http::anthropic::{ask_provider, feed, Output};
use crate::parser::deepseek_sse::{DeepSeekSseParser, DeepSseEvent};
use crate::parser::xml_tool::XmlToolParser;
use crate::provider::prompt::{build_prompt, content_text};
use crate::provider::session::SessionStore;
use crate::state::AppState;
use crate::types::{Block, Message, ToolSpec};
use axum::{
    extract::State,
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio_stream::wrappers::UnboundedReceiverStream;

pub async fn handle(State(state): State<AppState>, Json(body): Json<Value>) -> Response {
    let stream = body.get("stream").and_then(|x| x.as_bool()).unwrap_or(false);
    let (system, messages, tools) = to_internal(&body);
    let model = body
        .get("model")
        .and_then(|x| x.as_str())
        .unwrap_or("deepseek")
        .to_string();

    let first_user = messages
        .iter()
        .find(|m| m.role == "user")
        .map(|m| content_text(&m.content))
        .unwrap_or_default();
    let key = SessionStore::key(&system, &first_user);
    let n = messages.len();
    let found = {
        let store = state.sessions.lock().unwrap();
        store.find(&key, n, &messages)
    };
    let (prompt, found_info) = match &found {
        Some((idx, url, rest)) => (build_follow_up(rest), Some((*idx, url.clone()))),
        None => (
            build_prompt(
                if system.is_empty() { None } else { Some(&system) },
                &tools,
                &messages,
            ),
            None,
        ),
    };
    let (new_chat, chat_url) = match &found_info {
        Some((_, url)) => (false, url.clone()),
        None => (true, None),
    };
    state.log(format!(
        "[openai 请求] {} 条消息, {} 个工具",
        messages.len(),
        tools.len()
    ));

    let rx = match ask_provider(&state, &prompt, new_chat, chat_url).await {
        Ok(rx) => rx,
        Err(e) => return openai_error(&e, stream),
    };

    let chat_id = format!("chatcmpl-{}", uuid::Uuid::new_v4().simple());
    let mut out = Output::new(None);
    let mut tool_ids = Vec::new();
    if stream {
        let (tx, rx_out) = mpsc::unbounded_channel::<Result<String, std::convert::Infallible>>();
        consume(rx, &mut out, &mut tool_ids, &chat_id, &model, Some(tx.clone())).await;
        let _ = tx.send(Ok(format!(
            "data: {}\n\n",
            json!({
                "id": chat_id, "object": "chat.completion.chunk", "created": 0, "model": model,
                "choices": [{ "index": 0, "delta": {}, "finish_reason": stop_to_openai(out.stop_reason()) }]
            })
        )));
        let _ = tx.send(Ok("data: [DONE]\n\n".to_string()));
        drop(tx);
        return Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "text/event-stream; charset=utf-8")
            .header(header::CACHE_CONTROL, "no-cache")
            .body(axum::body::Body::from_stream(UnboundedReceiverStream::new(rx_out)))
            .unwrap();
    }
    consume(rx, &mut out, &mut tool_ids, &chat_id, &model, None).await;

    if out.content.is_empty() {
        out.content.push(Block::Text { text: String::new() });
    }
    let (content, tool_calls) = openai_blocks(&out.content, &mut tool_ids);
    Json(json!({
        "id": chat_id,
        "object": "chat.completion",
        "created": 0,
        "model": model,
        "choices": [{
            "index": 0,
            "message": { "role": "assistant", "content": content, "tool_calls": tool_calls },
            "finish_reason": stop_to_openai(out.stop_reason()),
        }],
        "usage": {
            "prompt_tokens": 0,
            "completion_tokens": out.text_len / 3,
            "total_tokens": out.text_len / 3,
        },
    }))
    .into_response()
}

async fn consume(
    mut rx: mpsc::UnboundedReceiver<BridgeEvent>,
    out: &mut Output,
    tool_ids: &mut Vec<String>,
    chat_id: &str,
    model: &str,
    stream_tx: Option<mpsc::UnboundedSender<Result<String, std::convert::Infallible>>>,
) {
    let mut sse_parser = DeepSeekSseParser::new();
    let mut tool_parser = XmlToolParser::new();
    if let Some(tx) = stream_tx.clone() {
        let _ = tx.send(Ok(format!(
            "data: {}\n\n",
            json!({
                "id": chat_id, "object": "chat.completion.chunk", "created": 0, "model": model,
                "choices": [{ "index": 0, "delta": { "role": "assistant", "content": "" }, "finish_reason": null }]
            })
        )));
    }
    loop {
        match rx.recv().await {
            Some(BridgeEvent::Chunk(c)) => {
                for ev in sse_parser.push(&c) {
                    match ev {
                        DeepSseEvent::Text(t) => {
                            for pe in tool_parser.push(&t) {
                                feed(out, pe);
                            }
                        }
                        DeepSseEvent::Done => {
                            for pe in tool_parser.finish() {
                                feed(out, pe);
                            }
                        }
                    }
                }
            }
            Some(BridgeEvent::End(_)) | Some(BridgeEvent::Error(_)) | None => break,
        }
    }
    out.close_text();
    if let Some(tx) = stream_tx {
        let chat_id = chat_id.to_string();
        let model = model.to_string();
        for block in &out.content {
            match block {
                Block::Text { text } if !text.is_empty() => {
                    let _ = tx.send(Ok(format!(
                        "data: {}\n\n",
                        json!({
                            "id": chat_id, "object": "chat.completion.chunk", "created": 0, "model": model,
                            "choices": [{ "index": 0, "delta": { "content": text }, "finish_reason": null }]
                        })
                    )));
                }
                Block::ToolUse { id, name, input } => {
                    tool_ids.push(id.clone());
                    let _ = tx.send(Ok(format!(
                        "data: {}\n\n",
                        json!({
                            "id": chat_id, "object": "chat.completion.chunk", "created": 0, "model": model,
                            "choices": [{ "index": 0, "delta": { "tool_calls": [{
                                "index": tool_ids.len() - 1, "id": id, "type": "function",
                                "function": { "name": name, "arguments": input.to_string() }
                            }] }, "finish_reason": null }]
                        })
                    )));
                }
                _ => {}
            }
        }
    }
}

fn openai_blocks(blocks: &[Block], tool_ids: &mut Vec<String>) -> (Option<String>, Vec<Value>) {
    let mut text = String::new();
    let mut calls = Vec::new();
    for b in blocks {
        match b {
            Block::Text { text: t } => text.push_str(t),
            Block::ToolUse { id, name, input } => {
                tool_ids.push(id.clone());
                calls.push(json!({
                    "id": id,
                    "type": "function",
                    "function": { "name": name, "arguments": input.to_string() },
                }));
            }
            _ => {}
        }
    }
    (if text.is_empty() { None } else { Some(text) }, calls)
}

fn stop_to_openai(reason: &str) -> &'static str {
    if reason == "tool_use" { "tool_calls" } else { "stop" }
}

fn openai_error(e: &str, stream: bool) -> Response {
    if !stream {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "error": { "message": e, "type": "proxy_error" } })),
        )
            .into_response();
    }
    let (tx, rx) = mpsc::unbounded_channel::<Result<String, std::convert::Infallible>>();
    let _ = tx.send(Ok(format!(
        "data: {}\n\n",
        json!({ "error": { "message": e, "type": "proxy_error" } })
    )));
    let _ = tx.send(Ok("data: [DONE]\n\n".to_string()));
    drop(tx);
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/event-stream; charset=utf-8")
        .header(header::CACHE_CONTROL, "no-cache")
        .body(axum::body::Body::from_stream(UnboundedReceiverStream::new(rx)))
        .unwrap()
}

fn build_follow_up(rest: &[Message]) -> String {
    let mut parts: Vec<String> = rest
        .iter()
        .map(|m| format!("<{}>\n{}\n</{}>", m.role, content_text(&m.content), m.role))
        .collect();
    parts.push("请继续以 assistant 的身份回复，需要时按约定格式调用工具。".into());
    parts.join("\n\n")
}

fn to_internal(body: &Value) -> (String, Vec<Message>, Vec<ToolSpec>) {
    let mut system = String::new();
    let mut messages = Vec::new();
    if let Some(arr) = body.get("messages").and_then(|x| x.as_array()) {
        for m in arr {
            let role = m.get("role").and_then(|x| x.as_str()).unwrap_or("user");
            let mut content = Vec::new();
            match m.get("content") {
                Some(Value::String(s)) => content.push(Block::Text { text: s.clone() }),
                Some(Value::Array(parts)) => {
                    for p in parts {
                        if let Some(t) = p.get("text").and_then(|x| x.as_str()) {
                            content.push(Block::Text { text: t.to_string() });
                        }
                    }
                }
                _ => {}
            }
            if role == "system" {
                system = content
                    .iter()
                    .filter_map(|b| match b {
                        Block::Text { text } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                continue;
            }
            if role == "tool" {
                messages.push(Message {
                    role: "user".into(),
                    content: vec![Block::ToolResult {
                        is_error: false,
                        content: m
                            .get("content")
                            .and_then(|x| x.as_str())
                            .unwrap_or("")
                            .to_string(),
                    }],
                });
                continue;
            }
            if let Some(calls) = m.get("tool_calls").and_then(|x| x.as_array()) {
                for c in calls {
                    let f = &c["function"];
                    let args: Value = f
                        .get("arguments")
                        .and_then(|x| x.as_str())
                        .and_then(|s| serde_json::from_str(s).ok())
                        .unwrap_or_else(|| json!({}));
                    content.push(Block::ToolUse {
                        id: c.get("id").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                        name: f.get("name").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                        input: args,
                    });
                }
            }
            messages.push(Message {
                role: role.to_string(),
                content,
            });
        }
    }
    let tools = body
        .get("tools")
        .and_then(|x| x.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|t| {
                    let f = t.get("function")?;
                    Some(ToolSpec {
                        name: f.get("name").and_then(|x| x.as_str())?.to_string(),
                        description: f
                            .get("description")
                            .and_then(|x| x.as_str())
                            .unwrap_or("")
                            .to_string(),
                        input_schema: f.get("parameters").cloned().unwrap_or_else(|| json!({})),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    (system, messages, tools)
}
