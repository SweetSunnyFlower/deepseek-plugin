use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
pub enum DeepSseEvent {
    Text(String),
    Done,
}

struct Frag {
    kind: String,
    content: String,
}

pub struct DeepSeekSseParser {
    buf: String,
    event: String,
    last_path: Option<String>,
    fragments: Vec<Frag>,
    finished: bool,
}

impl Default for DeepSeekSseParser {
    fn default() -> Self {
        Self::new()
    }
}

impl DeepSeekSseParser {
    pub fn new() -> Self {
        Self {
            buf: String::new(),
            event: "message".into(),
            last_path: None,
            fragments: Vec::new(),
            finished: false,
        }
    }

    pub fn push(&mut self, chunk: &str) -> Vec<DeepSseEvent> {
        let mut out = Vec::new();
        self.buf.push_str(chunk);
        loop {
            let Some(i) = self.buf.find('\n') else { break };
            let line = self.buf[..i].trim_end_matches('\r').to_string();
            self.buf.drain(..i + 1);
            if line.is_empty() {
                self.event = "message".into();
                continue;
            }
            if let Some(ev) = line.strip_prefix("event:") {
                self.event = ev.trim().to_string();
                continue;
            }
            let Some(data) = line.strip_prefix("data:") else { continue };
            if let Some(ev) = self.handle_data(data.trim()) {
                let done = ev == DeepSseEvent::Done;
                out.push(ev);
                if done {
                    return out;
                }
            }
        }
        out
    }

    fn handle_data(&mut self, raw: &str) -> Option<DeepSseEvent> {
        if self.event == "close" {
            self.finished = true;
            return Some(DeepSseEvent::Done);
        }
        if self.event != "message" {
            return None;
        }
        let v: Value = serde_json::from_str(raw).ok()?;
        if let Some(p) = v.get("p").and_then(|x| x.as_str()) {
            self.last_path = Some(p.to_string());
        }
        if v.get("p").is_none() {
            if let Some(resp) = v.get("v").and_then(|x| x.get("response")) {
                self.fragments.clear();
                self.last_path = Some("response/fragments/-1/content".into());
                if let Some(list) = resp.get("fragments").and_then(|x| x.as_array()) {
                    return self.add_fragments(list);
                }
                return None;
            }
        }
        self.handle_patch(
            v.get("o").and_then(|x| x.as_str()).unwrap_or("APPEND"),
            v.get("v"),
        )
    }

    fn add_fragments(&mut self, list: &[Value]) -> Option<DeepSseEvent> {
        let mut first = None;
        for f in list {
            let kind = f
                .get("type")
                .and_then(|x| x.as_str())
                .unwrap_or("RESPONSE")
                .to_string();
            let mut frag = Frag {
                kind: kind.clone(),
                content: String::new(),
            };
            if let Some(c) = f.get("content").and_then(|x| x.as_str()) {
                frag.content.push_str(c);
                if kind != "THINK" && first.is_none() {
                    first = Some(DeepSseEvent::Text(c.to_string()));
                }
            }
            self.fragments.push(frag);
        }
        first
    }

    fn handle_patch(&mut self, op: &str, v: Option<&Value>) -> Option<DeepSseEvent> {
        let path = self.last_path.clone()?;
        let p = path.as_str();
        if p == "response/fragments" {
            if let Some(arr) = v.and_then(|x| x.as_array()) {
                return self.add_fragments(arr);
            }
            return None;
        }
        if p == "response" && op == "BATCH" {
            if let Some(arr) = v.and_then(|x| x.as_array()) {
                let mut out = None;
                for item in arr {
                    let ip = item.get("p").and_then(|x| x.as_str()).unwrap_or("");
                    self.last_path = Some(ip.to_string());
                    if let Some(e) = self.handle_patch(
                        item.get("o").and_then(|x| x.as_str()).unwrap_or("SET"),
                        item.get("v"),
                    ) {
                        out = Some(e);
                    }
                }
                return out;
            }
        }
        if (p == "response/status" || p == "response/quasi_status")
            && v.and_then(|x| x.as_str()) == Some("FINISHED")
        {
            self.finished = true;
            return Some(DeepSseEvent::Done);
        }
        if let Some(idx) = p
            .strip_prefix("response/fragments/")
            .and_then(|r| r.strip_suffix("/content"))
        {
            if let Some(text) = v.and_then(|x| x.as_str()) {
                let i: isize = idx.parse().ok()?;
                let fi = if i < 0 {
                    self.fragments.len().saturating_sub(1)
                } else {
                    i as usize
                };
                let frag = self.fragments.get_mut(fi)?;
                frag.content.push_str(text);
                if frag.kind != "THINK" {
                    return Some(DeepSseEvent::Text(text.to_string()));
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_then_append() {
        let mut p = DeepSeekSseParser::new();
        let mut ev = p.push(
            "data: {\"v\":{\"response\":{\"fragments\":[{\"type\":\"RESPONSE\",\"content\":\"你好\"}]}}}\n\n",
        );
        ev.extend(p.push(
            "data: {\"p\":\"response/fragments/-1/content\",\"o\":\"APPEND\",\"v\":\"，世界\"}\n\n",
        ));
        ev.extend(p.push("data: {\"v\":\"！\"}\n\n"));
        ev.extend(p.push("data: {\"p\":\"response/status\",\"o\":\"SET\",\"v\":\"FINISHED\"}\n\n"));
        assert_eq!(
            ev,
            vec![
                DeepSseEvent::Text("你好".into()),
                DeepSseEvent::Text("，世界".into()),
                DeepSseEvent::Text("！".into()),
                DeepSseEvent::Done,
            ]
        );
    }

    #[test]
    fn split_lines_are_buffered() {
        let mut p = DeepSeekSseParser::new();
        assert!(p
            .push("data: {\"v\":{\"response\":{\"fragments\":[{\"type\":\"RESPONSE\",\"content\":\"a")
            .is_empty());
        let ev = p.push("b\"}]}}}\n\n");
        assert_eq!(ev, vec![DeepSseEvent::Text("ab".into())]);
    }

    #[test]
    fn malformed_json_is_ignored() {
        let mut p = DeepSeekSseParser::new();
        assert!(p.push("data: not-json\n\n").is_empty());
    }

    #[test]
    fn think_fragments_are_silent() {
        let mut p = DeepSeekSseParser::new();
        let ev = p.push(
            "data: {\"v\":{\"response\":{\"fragments\":[{\"type\":\"THINK\",\"content\":\"在想…\"},{\"type\":\"RESPONSE\",\"content\":\"答\"}]}}}\n\n",
        );
        assert_eq!(ev, vec![DeepSseEvent::Text("答".into())]);
    }
}
