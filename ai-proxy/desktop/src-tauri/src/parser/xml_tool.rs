use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
pub enum ParseEvent {
    Text(String),
    ToolUse { name: String, input: Value },
}

pub struct XmlToolParser {
    buf: String,
    stopped: bool,
    call: Option<Call>,
}

struct Call {
    name: String,
    open: String,
    body: String,
}

impl Default for XmlToolParser {
    fn default() -> Self {
        Self::new()
    }
}

impl XmlToolParser {
    pub fn new() -> Self {
        Self {
            buf: String::new(),
            stopped: false,
            call: None,
        }
    }

    pub fn push(&mut self, chunk: &str) -> Vec<ParseEvent> {
        self.buf.push_str(chunk);
        self.run(false)
    }

    pub fn finish(&mut self) -> Vec<ParseEvent> {
        self.run(true)
    }

    fn run(&mut self, final_: bool) -> Vec<ParseEvent> {
        let mut out = Vec::new();
        loop {
            if self.stopped {
                self.buf.clear();
                return out;
            }
            if self.call.is_some() {
                let mut call = self.call.take().unwrap();
                match find_body_end(&self.buf) {
                    Some(idx) => {
                        let tail = &self.buf[idx..];
                        if tail.starts_with("</tool_call") && !tail.contains('>') && !final_ {
                            self.call = Some(call);
                            return out;
                        }
                        call.body.push_str(&self.buf[..idx]);
                        let len = close_len(tail);
                        self.buf.drain(..idx + len);
                        self.emit_call(&mut out, call);
                    }
                    None if final_ => {
                        call.body.push_str(&self.buf);
                        self.buf.clear();
                        self.emit_call(&mut out, call);
                    }
                    None => {
                        self.call = Some(call);
                        return out;
                    }
                }
                continue;
            }
            match self.buf.find('<') {
                None => {
                    let t = std::mem::take(&mut self.buf);
                    if !t.is_empty() {
                        out.push(ParseEvent::Text(t));
                    }
                    return out;
                }
                Some(0) => {
                    if self.buf.starts_with("<tool_result") {
                        self.stopped = true;
                        self.buf.clear();
                        return out;
                    }
                    if self.buf.starts_with("</tool_call") {
                        drop_to_gt(&mut self.buf);
                        continue;
                    }
                    if let Some((len, name)) = parse_start(&self.buf) {
                        let open = self.buf[..len].to_string();
                        self.buf.drain(..len);
                        self.call = Some(Call {
                            name,
                            open,
                            body: String::new(),
                        });
                        continue;
                    }
                    if !final_ && is_partial_marker(&self.buf) {
                        return out;
                    }
                    out.push(ParseEvent::Text("<".to_string()));
                    self.buf.drain(..1);
                }
                Some(i) => {
                    let t: String = self.buf.drain(..i).collect();
                    out.push(ParseEvent::Text(t));
                }
            }
        }
    }

    fn emit_call(&mut self, out: &mut Vec<ParseEvent>, call: Call) {
        match parse_body(&call.body) {
            Some(input) => out.push(ParseEvent::ToolUse {
                name: call.name,
                input,
            }),
            None => out.push(ParseEvent::Text(format!("{}{}", call.open, call.body))),
        }
    }
}

fn find_body_end(s: &str) -> Option<usize> {
    ["</tool_call", "<tool_call", "<tool_result"]
        .iter()
        .filter_map(|m| s.find(m))
        .min()
}

fn close_len(s: &str) -> usize {
    if s.starts_with("</tool_call") {
        s.find('>').map(|i| i + 1).unwrap_or(s.len())
    } else {
        0
    }
}

fn drop_to_gt(buf: &mut String) {
    let n = buf.find('>').map(|i| i + 1).unwrap_or(buf.len());
    buf.drain(..n);
}

fn parse_start(s: &str) -> Option<(usize, String)> {
    if !s.starts_with("<tool_call") {
        return None;
    }
    let rest = s["<tool_call".len()..].strip_prefix(|c: char| c.is_whitespace())?;
    let rest = rest.strip_prefix("name")?;
    let rest = rest.trim_start_matches(char::is_whitespace).strip_prefix('=')?;
    let rest = rest.trim_start_matches(char::is_whitespace).strip_prefix('"')?;
    let end = rest.find('"')?;
    let name = rest[..end].to_string();
    let after = &rest[end + 1..];
    let gt = after.find('>')?;
    Some((s.len() - after.len() + gt + 1, name))
}

fn is_partial_marker(s: &str) -> bool {
    if s.len() > 60 || s.contains('>') {
        return false;
    }
    ["<tool_call", "</tool_call", "<tool_result"]
        .iter()
        .any(|m| m.starts_with(s) || s.starts_with(m))
}

fn parse_body(body: &str) -> Option<Value> {
    let mut s = body.trim();
    if s.starts_with("```") {
        s = s.trim_start_matches("```").trim_start_matches("json").trim();
        s = s.trim_end_matches("```").trim();
    }
    if let Some(v) = parse_params(s) {
        return Some(v);
    }
    if s.is_empty() {
        return Some(serde_json::json!({}));
    }
    serde_json::from_str::<Value>(s).ok().filter(|v| v.is_object())
}

fn parse_params(s: &str) -> Option<Value> {
    let mut map = serde_json::Map::new();
    let mut rest = s;
    let mut found = false;
    loop {
        let Some(i) = rest.find("<param") else { break };
        rest = &rest[i..];
        let Some(gt) = rest.find('>') else { break };
        let tag = &rest[..gt + 1];
        let Some(name) = attr_value(tag, "name") else { break };
        let string_true = attr_value(tag, "string").map(|v| v == "true").unwrap_or(false);
        rest = &rest[gt + 1..];
        let end = ["</param", "<param"]
            .iter()
            .filter_map(|m| rest.find(m))
            .min()
            .unwrap_or(rest.len());
        let val = rest[..end].trim().to_string();
        rest = &rest[end..];
        let v = if string_true {
            Value::String(val)
        } else {
            serde_json::from_str::<Value>(&val).unwrap_or_else(|_| Value::String(val))
        };
        map.insert(name, v);
        found = true;
        if rest.starts_with("</param") {
            let c = rest.find('>').map(|i| i + 1).unwrap_or(rest.len());
            rest = &rest[c..];
        }
    }
    if found {
        Some(Value::Object(map))
    } else {
        None
    }
}

fn attr_value(tag: &str, name: &str) -> Option<String> {
    let pat = format!("{}=\"", name);
    let i = tag.find(&pat)?;
    let rest = &tag[i + pat.len()..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: &str) -> ParseEvent {
        ParseEvent::Text(s.to_string())
    }

    #[test]
    fn plain_text_passes_through() {
        let mut p = XmlToolParser::new();
        assert_eq!(p.push("hello "), vec![text("hello ")]);
        assert_eq!(p.push("world"), vec![text("world")]);
    }

    #[test]
    fn parses_full_tool_call() {
        let src = "<tool_call name=\"Read\">\n<param name=\"file_path\" string=\"true\">/a.js</param>\n<param name=\"offset\" string=\"false\">3</param>\n</tool_call>";
        let mut p = XmlToolParser::new();
        let ev = p.push(src);
        assert_eq!(ev.len(), 1);
        match &ev[0] {
            ParseEvent::ToolUse { name, input } => {
                assert_eq!(name, "Read");
                assert_eq!(input["file_path"], "/a.js");
                assert_eq!(input["offset"], 3);
            }
            _ => panic!("expected tool use"),
        }
    }

    #[test]
    fn handles_text_between_calls() {
        let src = "先读文件。<tool_call name=\"Read\"><param name=\"file_path\" string=\"true\">/a</param></tool_call>然后结束。";
        let mut p = XmlToolParser::new();
        let ev = p.push(src);
        assert_eq!(
            ev,
            vec![
                text("先读文件。"),
                ParseEvent::ToolUse {
                    name: "Read".into(),
                    input: serde_json::json!({"file_path": "/a"}),
                },
                text("然后结束。"),
            ]
        );
    }

    #[test]
    fn parses_when_stream_is_split_anywhere() {
        let src = "<tool_call name=\"Edit\"><param name=\"old_string\" string=\"true\">a</param></tool_call>";
        let mut p = XmlToolParser::new();
        let mut ev = Vec::new();
        for ch in src.chars() {
            ev.extend(p.push(&ch.to_string()));
        }
        ev.extend(p.finish());
        assert_eq!(
            ev,
            vec![ParseEvent::ToolUse {
                name: "Edit".into(),
                input: serde_json::json!({"old_string": "a"}),
            }]
        );
    }

    #[test]
    fn fenced_json_body_is_accepted() {
        let src = "<tool_call name=\"Read\">\n```json\n{\"file_path\":\"/b\"}\n```\n</tool_call>";
        let mut p = XmlToolParser::new();
        assert_eq!(
            p.push(src),
            vec![ParseEvent::ToolUse {
                name: "Read".into(),
                input: serde_json::json!({"file_path": "/b"}),
            }]
        );
    }

    #[test]
    fn dsml_is_plain_text() {
        let src = "<｜DSML｜invoke name=\"Read\">x</｜DSML｜invoke>";
        let mut p = XmlToolParser::new();
        let mut ev = p.push(src);
        ev.extend(p.finish());
        let joined: String = ev
            .iter()
            .map(|e| match e {
                ParseEvent::Text(t) => t.clone(),
                _ => panic!("DSML 不应被解析为工具调用"),
            })
            .collect();
        assert_eq!(joined, src);
    }

    #[test]
    fn fabricated_tool_result_is_dropped() {
        let src = "done<tool_result>fake</tool_result>tail";
        let mut p = XmlToolParser::new();
        let mut ev = p.push(src);
        ev.extend(p.finish());
        assert_eq!(ev, vec![text("done")]);
    }
}
