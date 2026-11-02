use crate::bridge::BridgeEvent;
use crate::parser::deepseek_sse::{DeepSeekSseParser, DeepSseEvent};
use crate::parser::xml_tool::{ParseEvent, XmlToolParser};
use crate::provider::prompt::{build_prompt, content_text};
use crate::provider::session::SessionStore;
use crate::state::AppState;
use crate::types::{Block, Message, ToolSpec};
use axum::{
    extract::State,
    http::{header, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_stream::wrappers::UnboundedReceiverStream;

pub async fn count_tokens(State(state): State<AppState>, Json(body): Json<Value>) -> Response {
    let system = system_text(&body);
    let messages = parse_messages(&body);
    let tools = parse_tools(&body);
    let prompt = build_prompt(
        if system.is_empty() { None } else { Some(&system) },
        &tools,
        &messages,
    );
    state.log(format!("count_tokens: {} 字符", prompt.len()));
    Json(json!({ "input_tokens": (prompt.len() as f64 / 3.0).ceil() as u64 })).into_response()
}

pub async fn handle(State(state): State<AppState>, Json(body): Json<Value>) -> Response {
    let stream = body.get("stream").and_then(|x| x.as_bool()).unwrap_or(false);
    let system = system_text(&body);
    let messages = parse_messages(&body);
    let tools = parse_tools(&body);
    let model = body
        .get("model")
        .and_then(|x| x.as_str())
        .unwrap_or("deepseek-web")
        .to_string();

    if tools.is_empty() && system.to_lowercase().contains("title") {
        return title_reply(&messages, stream, &model);
    }

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
            build_prompt(if system.is_empty() { None } else { Some(&system) }, &tools, &messages),
            None,
        ),
    };
    state.log(format!(
        "[请求] {} 条消息, {} 个工具, {}",
        messages.len(),
        tools.len(),
        if found_info.is_some() { "续接对话" } else { "新对话" }
    ));

    let (new_chat, chat_url) = match &found_info {
        Some((_, url)) => (false, url.clone()),
        None => (true, None),
    };

    let rx = {
        let active = {
            let s = state.settings.read().unwrap();
            s.active_provider.clone()
        };
        let controller = state
            .providers
            .get(&active)
            .cloned()
            .ok_or_else(|| format!("未知供应商: {active}"));
        match controller {
            Err(e) => {
                return stream_error(&e, stream);
            }
            Ok(controller) => {
                let bridge = state.bridge.clone();
                let prompt = prompt.clone();
                match controller
                    .enqueue(move || {
                        let bridge = bridge.clone();
                        let prompt = prompt.clone();
                        let url = chat_url.clone();
                        async move { bridge.ask(&prompt, new_chat, url) }
                    })
                    .await
                {
                    Ok(rx) => rx,
                    Err(e) => return stream_error(&e, stream),
                }
            }
        }
    };

    let msg_id = format!("msg_{}", uuid::Uuid::new_v4().simple());
    let (out, final_url, err, empty_ok) = if stream {
        let (tx, rx_out) = mpsc::unbounded_channel();
        let headers = [
            (header::CONTENT_TYPE, HeaderValue::from_static("text/event-stream; charset=utf-8")),
            (header::CACHE_CONTROL, HeaderValue::from_static("no-cache")),
            (header::CONNECTION, HeaderValue::from_static("keep-alive")),
        ];
        let sse_tx = tx.clone();
        let model_c = model.clone();
        let usage = json!({ "input_tokens": (prompt.len() as f64 / 3.0).ceil() as u64, "output_tokens": 0 });
        let _ = sse_tx.send(Ok(format!(
            "event: message_start\ndata: {}\n\n",
            json!({ "type": "message_start", "message": {
                "id": msg_id, "type": "message", "role": "assistant", "model": model_c,
                "content": [], "stop_reason": null, "stop_sequence": null, "usage": usage
            }})
        )));
        let ping_tx = tx.clone();
        let ping = tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                if ping_tx
                    .send(Ok("event: ping\ndata: {\"type\":\"ping\"}\n\n".to_string()))
                    .is_err()
                {
                    break;
                }
            }
        });
        let (out, final_url, err, empty_ok) =
            run_stream(rx, Some(sse_tx)).await;
        ping.abort();
        let usage = json!({ "output_tokens": out.text_len / 3 });
        let _ = tx.send(Ok(format!(
            "event: message_delta\ndata: {}\n\n",
            json!({ "type": "message_delta", "delta": { "stop_reason": out.stop_reason(), "stop_sequence": null }, "usage": usage })
        )));
        let _ = tx.send(Ok("event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n".to_string()));
        drop(tx);
        let resp = Response::builder()
            .status(StatusCode::OK);
        let resp = headers.iter().fold(resp, |b, (k, v)| b.header(k, v));
        return resp
            .body(axum::body::Body::from_stream(UnboundedReceiverStream::new(rx_out)))
            .unwrap();
    } else {
        run_stream(rx, None).await
    };

    finish_non_stream(state, body, found_info, key, n, final_url, err, empty_ok, out, model, msg_id).await
}

struct Output {
    content: Vec<Block>,
    text_len: usize,
    text_open: bool,
    sse: Option<Arc<dyn Fn(&str, Value) + Send + Sync>>,
}

impl Output {
    fn new(sse: Option<Arc<dyn Fn(&str, Value) + Send + Sync>>) -> Self {
        Self {
            content: Vec::new(),
            text_len: 0,
            text_open: false,
            sse,
        }
    }

    fn text(&mut self, t: &str) {
        if !self.text_open {
            if t.trim().is_empty() {
                return;
            }
            self.content.push(Block::Text { text: String::new() });
            self.text_open = true;
            let index = self.content.len() - 1;
            if let Some(f) = &self.sse {
                f(
                    "content_block_start",
                    json!({ "index": index, "content_block": { "type": "text", "text": "" } }),
                );
            }
        }
        self.text_len += t.len();
        let index = self.content.len() - 1;
        if let Block::Text { text } = self.content.last_mut().unwrap() {
            text.push_str(t);
        }
        if let Some(f) = &self.sse {
            f(
                "content_block_delta",
                json!({ "index": index, "delta": { "type": "text_delta", "text": t } }),
            );
        }
    }

    fn tool_use(&mut self, name: String, input: Value) {
        self.close_text();
        let id = format!(
            "toolu_{}",
            uuid::Uuid::new_v4().simple().to_string().chars().take(24).collect::<String>()
        );
        self.content.push(Block::ToolUse {
            id: id.clone(),
            name: name.clone(),
            input: input.clone(),
        });
        let index = self.content.len() - 1;
        if let Some(f) = &self.sse {
            f(
                "content_block_start",
                json!({ "index": index, "content_block": { "type": "tool_use", "id": id, "name": name, "input": {} } }),
            );
            f(
                "content_block_delta",
                json!({ "index": index, "delta": { "type": "input_json_delta", "partial_json": input.to_string() } }),
            );
            f("content_block_stop", json!({ "index": index }));
        }
    }

    fn close_text(&mut self) {
        if !self.text_open {
            return;
        }
        self.text_open = false;
        let index = self.content.len() - 1;
        if let Some(f) = &self.sse {
            f("content_block_stop", json!({ "index": index }));
        }
    }

    fn stop_reason(&self) -> &'static str {
        if self.content.iter().any(|b| matches!(b, Block::ToolUse { .. })) {
            "tool_use"
        } else {
            "end_turn"
        }
    }
}

async fn run_stream(
    mut rx: mpsc::UnboundedReceiver<BridgeEvent>,
    sse_tx: Option<mpsc::UnboundedSender<Result<String, std::convert::Infallible>>>,
) -> (Output, Option<String>, Option<String>, bool) {
    let sse: Option<Arc<dyn Fn(&str, Value) + Send + Sync>> = sse_tx.map(|tx| {
        Arc::new(move |event: &str, data: Value| {
            let _ = tx.send(Ok(format!("event: {event}\ndata: {data}\n\n")));
        }) as Arc<dyn Fn(&str, Value) + Send + Sync>
    });
    let mut out = Output::new(sse);
    let mut sse_parser = DeepSeekSseParser::new();
    let mut tool_parser = XmlToolParser::new();
    let mut final_url = None;
    let mut err = None;
    let mut raw = String::new();
    loop {
        match rx.recv().await {
            Some(BridgeEvent::Chunk(c)) => {
                raw.push_str(&c);
                for ev in sse_parser.push(&c) {
                    match ev {
                        DeepSseEvent::Text(t) => {
                            for pe in tool_parser.push(&t) {
                                feed(&mut out, pe);
                            }
                        }
                        DeepSseEvent::Done => {
                            for pe in tool_parser.finish() {
                                feed(&mut out, pe);
                            }
                        }
                    }
                }
            }
            Some(BridgeEvent::End(url)) => {
                final_url = url;
                break;
            }
            Some(BridgeEvent::Error(e)) => {
                err = Some(e);
                break;
            }
            None => {
                err = Some("插件连接中断".into());
                break;
            }
        }
    }
    out.close_text();
    let empty_ok = !out.content.is_empty();
    if !empty_ok && raw.len() > 500 {
        let _ = raw.split_off(500);
    }
    (out, final_url, err, empty_ok)
}

fn feed(out: &mut Output, pe: ParseEvent) {
    match pe {
        ParseEvent::Text(t) => out.text(&t),
        ParseEvent::ToolUse { name, input } => out.tool_use(name, input),
    }
}

async fn finish_non_stream(
    state: AppState,
    body: Value,
    found_info: Option<(usize, Option<String>)>,
    key: String,
    n: usize,
    final_url: Option<String>,
    err: Option<String>,
    empty_ok: bool,
    mut out: Output,
    model: String,
    msg_id: String,
) -> Response {
    if let Some(e) = err {
        state.log(format!("[错误] {e}"));
        save_session(&state, found_info, key, n, final_url, false);
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({
                "type": "error",
                "error": { "type": "overloaded_error", "message": e }
            })),
        )
            .into_response();
    }
    let _ = &body;
    if out.content.is_empty() {
        out.content.push(Block::Text { text: String::new() });
    }
    let stop_reason = out.stop_reason();
    let usage = json!({
        "input_tokens": 0,
        "output_tokens": out.text_len / 3,
    });
    state.log(format!(
        "[回复] {}",
        out.content
            .iter()
            .map(|b| match b {
                Block::ToolUse { name, .. } => format!("工具:{name}"),
                Block::Text { text } => format!("文字{}", text.len()),
                _ => "其他".into(),
            })
            .collect::<Vec<_>>()
            .join(", ")
    ));
    save_session(&state, found_info, key, n, final_url, empty_ok);
    Json(json!({
        "id": msg_id,
        "type": "message",
        "role": "assistant",
        "model": model,
        "content": out.content,
        "stop_reason": stop_reason,
        "stop_sequence": null,
        "usage": usage,
    }))
    .into_response()
}

fn save_session(
    state: &AppState,
    found_info: Option<(usize, Option<String>)>,
    key: String,
    n: usize,
    final_url: Option<String>,
    ok: bool,
) {
    let url = final_url.filter(|u| u.contains("/chat/s/"));
    let mut store = state.sessions.lock().unwrap();
    store.save(
        found_info.map(|(i, _)| i),
        key,
        n + 1,
        url,
        !ok,
    );
}

fn stream_error(e: &str, stream: bool) -> Response {
    if !stream {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "type": "error", "error": { "type": "overloaded_error", "message": e } })),
        )
            .into_response();
    }
    let (tx, rx) = mpsc::unbounded_channel::<Result<String, std::convert::Infallible>>();
    let headers = [
        (header::CONTENT_TYPE, HeaderValue::from_static("text/event-stream; charset=utf-8")),
        (header::CACHE_CONTROL, HeaderValue::from_static("no-cache")),
    ];
    let body_text = format!("\n[代理错误] {e}");
    let msg_id = format!("msg_{}", uuid::Uuid::new_v4().simple());
    let _ = tx.send(Ok(format!(
        "event: message_start\ndata: {}\n\n",
        json!({ "type": "message_start", "message": {
            "id": msg_id, "type": "message", "role": "assistant", "model": "deepseek-web",
            "content": [], "stop_reason": null, "stop_sequence": null,
            "usage": { "input_tokens": 0, "output_tokens": 0 }
        }})
    )));
    let _ = tx.send(Ok(format!(
        "event: content_block_start\ndata: {}\n\n",
        json!({ "type": "content_block_start", "index": 0, "content_block": { "type": "text", "text": "" } })
    )));
    let _ = tx.send(Ok(format!(
        "event: content_block_delta\ndata: {}\n\n",
        json!({ "type": "content_block_delta", "index": 0, "delta": { "type": "text_delta", "text": body_text } })
    )));
    let _ = tx.send(Ok(
        "event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\n".to_string(),
    ));
    let _ = tx.send(Ok(
        "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\",\"stop_sequence\":null},\"usage\":{\"output_tokens\":1}}\n\n".to_string(),
    ));
    let _ = tx.send(Ok("event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n".to_string()));
    drop(tx);
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/event-stream; charset=utf-8")
        .header(header::CACHE_CONTROL, "no-cache")
        .body(axum::body::Body::from_stream(UnboundedReceiverStream::new(rx)))
        .unwrap()
}

fn title_reply(messages: &[Message], stream: bool, model: &str) -> Response {
    let first_user = messages
        .iter()
        .find(|m| m.role == "user")
        .map(|m| content_text(&m.content))
        .unwrap_or_default();
    let title: String = first_user.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(20).collect();
    let title = if title.is_empty() { "新会话".to_string() } else { title };
    let text = serde_json::to_string(&serde_json::json!({ "title": title })).unwrap();
    if !stream {
        return Json(json!({
            "id": format!("msg_{}", uuid::Uuid::new_v4().simple()),
            "type": "message",
            "role": "assistant",
            "model": model,
            "content": [{ "type": "text", "text": text }],
            "stop_reason": "end_turn",
            "stop_sequence": null,
            "usage": { "input_tokens": 1, "output_tokens": 1 },
        }))
        .into_response();
    }
    let (tx, rx) = mpsc::unbounded_channel::<Result<String, std::convert::Infallible>>();
    let msg_id = format!("msg_{}", uuid::Uuid::new_v4().simple());
    let _ = tx.send(Ok(format!(
        "event: message_start\ndata: {}\n\n",
        json!({ "type": "message_start", "message": {
            "id": msg_id, "type": "message", "role": "assistant", "model": model,
            "content": [], "stop_reason": null, "stop_sequence": null,
            "usage": { "input_tokens": 1, "output_tokens": 1 }
        }})
    )));
    let _ = tx.send(Ok(format!(
        "event: content_block_start\ndata: {}\n\n",
        json!({ "type": "content_block_start", "index": 0, "content_block": { "type": "text", "text": "" } })
    )));
    let _ = tx.send(Ok(format!(
        "event: content_block_delta\ndata: {}\n\n",
        json!({ "type": "content_block_delta", "index": 0, "delta": { "type": "text_delta", "text": text } })
    )));
    let _ = tx.send(Ok(
        "event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\n".to_string(),
    ));
    let _ = tx.send(Ok(
        "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\",\"stop_sequence\":null},\"usage\":{\"output_tokens\":1}}\n\n".to_string(),
    ));
    let _ = tx.send(Ok("event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n".to_string()));
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

fn system_text(body: &Value) -> String {
    let system = body.get("system").cloned().unwrap_or(Value::Null);
    if let Some(s) = system.as_str() {
        return s.to_string();
    }
    system
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|b| b.get("text").and_then(|x| x.as_str()))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

fn parse_messages(body: &Value) -> Vec<Message> {
    body.get("messages")
        .and_then(|x| x.as_array())
        .map(|arr| {
            arr.iter()
                .map(|m| Message {
                    role: m.get("role").and_then(|x| x.as_str()).unwrap_or("user").to_string(),
                    content: blocks_from_content(m.get("content").unwrap_or(&Value::Null)),
                })
                .collect()
        })
        .unwrap_or_default()
}

fn blocks_from_content(v: &Value) -> Vec<Block> {
    if let Some(s) = v.as_str() {
        return vec![Block::Text { text: s.to_string() }];
    }
    v.as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|b| match b.get("type").and_then(|x| x.as_str()) {
                    Some("text") => Some(Block::Text {
                        text: b.get("text").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                    }),
                    Some("tool_use") => Some(Block::ToolUse {
                        id: b.get("id").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                        name: b.get("name").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                        input: b.get("input").cloned().unwrap_or_else(|| json!({})),
                    }),
                    Some("tool_result") => Some(Block::ToolResult {
                        is_error: b.get("is_error").and_then(|x| x.as_bool()).unwrap_or(false),
                        content: plain_text(b.get("content").unwrap_or(&Value::Null)),
                    }),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

fn plain_text(v: &Value) -> String {
    if let Some(s) = v.as_str() {
        return s.to_string();
    }
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|b| b.get("text").and_then(|x| x.as_str()))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

fn parse_tools(body: &Value) -> Vec<ToolSpec> {
    body.get("tools")
        .and_then(|x| x.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|t| {
                    Some(ToolSpec {
                        name: t.get("name").and_then(|x| x.as_str())?.to_string(),
                        description: t
                            .get("description")
                            .and_then(|x| x.as_str())
                            .unwrap_or("")
                            .to_string(),
                        input_schema: t.get("input_schema").cloned().unwrap_or_else(|| json!({})),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}
