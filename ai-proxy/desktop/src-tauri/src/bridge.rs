use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Mutex, RwLock};
use tokio::sync::mpsc;

#[derive(Debug)]
pub enum BridgeEvent {
    Chunk(String),
    End(Option<String>),
    Error(String),
}

#[derive(Default)]
pub struct BridgeHub {
    sink: RwLock<Option<mpsc::UnboundedSender<String>>>,
    pending: Mutex<HashMap<String, mpsc::UnboundedSender<BridgeEvent>>>,
}

impl BridgeHub {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn attach(&self) -> Option<mpsc::UnboundedReceiver<String>> {
        let mut sink = self.sink.write().unwrap();
        if sink.is_some() {
            return None;
        }
        let (tx, rx) = mpsc::unbounded_channel();
        *sink = Some(tx);
        Some(rx)
    }

    pub fn detach(&self) {
        self.sink.write().unwrap().take();
        let mut pending = self.pending.lock().unwrap();
        for (_, tx) in pending.drain() {
            let _ = tx.send(BridgeEvent::Error("插件已断开".into()));
        }
    }

    pub fn connected(&self) -> bool {
        self.sink.read().unwrap().is_some()
    }

    pub fn pending_count(&self) -> usize {
        self.pending.lock().unwrap().len()
    }

    pub fn notify(&self, msg: Value) -> bool {
        self.sink
            .read()
            .unwrap()
            .as_ref()
            .map(|tx| tx.send(msg.to_string()).is_ok())
            .unwrap_or(false)
    }

    pub fn ask(
        &self,
        prompt: &str,
        new_chat: bool,
        chat_url: Option<String>,
    ) -> Result<mpsc::UnboundedReceiver<BridgeEvent>, String> {
        let sink = self
            .sink
            .read()
            .unwrap()
            .clone()
            .ok_or_else(|| "插件未连接，请打开 chat.deepseek.com".to_string())?;
        let id = uuid::Uuid::new_v4().to_string();
        let (tx, rx) = mpsc::unbounded_channel();
        self.pending.lock().unwrap().insert(id.clone(), tx);
        let msg = json!({
            "type": "send_prompt",
            "id": id,
            "prompt": prompt,
            "new_chat": new_chat,
            "chat_url": chat_url,
        });
        if sink.send(msg.to_string()).is_err() {
            self.pending.lock().unwrap().remove(&id);
            return Err("插件未连接，请打开 chat.deepseek.com".into());
        }
        Ok(rx)
    }

    pub fn handle_client(&self, v: Value) {
        let Some(id) = v.get("id").and_then(|x| x.as_str()) else { return };
        let Some(tx) = self.pending.lock().unwrap().get(id).cloned() else { return };
        match v.get("type").and_then(|x| x.as_str()).unwrap_or("") {
            "stream_chunk" => {
                if let Some(c) = v.get("chunk").and_then(|x| x.as_str()) {
                    let _ = tx.send(BridgeEvent::Chunk(c.to_string()));
                }
            }
            "stream_end" => {
                self.pending.lock().unwrap().remove(id);
                let url = v.get("url").and_then(|x| x.as_str()).map(str::to_string);
                let _ = tx.send(BridgeEvent::End(url));
            }
            "stream_error" => {
                self.pending.lock().unwrap().remove(id);
                let e = v
                    .get("error")
                    .and_then(|x| x.as_str())
                    .unwrap_or("未知错误");
                let _ = tx.send(BridgeEvent::Error(e.to_string()));
            }
            _ => {}
        }
    }
}
