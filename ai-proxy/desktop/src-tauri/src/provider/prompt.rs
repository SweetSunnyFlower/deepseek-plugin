use crate::types::{Block, Message, ToolSpec};
use serde_json::Value;

pub fn content_text(blocks: &[Block]) -> String {
    blocks.iter().map(block_text).collect::<Vec<_>>().join("\n")
}

fn block_text(b: &Block) -> String {
    match b {
        Block::Text { text } => text.clone(),
        Block::ToolUse { name, input, .. } => tool_call_xml(name, input),
        Block::ToolResult { is_error, content } => {
            let err = if *is_error { " error=\"true\"" } else { "" };
            format!("<tool_result{}>\n{}\n</tool_result>", err, content)
        }
    }
}

fn tool_call_xml(name: &str, input: &Value) -> String {
    let mut lines = vec![format!("<tool_call name=\"{}\">", name)];
    if let Some(obj) = input.as_object() {
        for (k, v) in obj {
            if let Some(s) = v.as_str() {
                lines.push(format!("<param name=\"{}\" string=\"true\">{}</param>", k, s));
            } else {
                lines.push(format!("<param name=\"{}\" string=\"false\">{}</param>", k, v));
            }
        }
    }
    lines.push("</tool_call>".to_string());
    lines.join("\n")
}

pub fn tools_prompt(tools: &[ToolSpec]) -> String {
    if tools.is_empty() {
        return String::new();
    }
    let list = tools
        .iter()
        .map(|t| format!("## {}\n{}\n参数 JSON Schema：{}", t.name, t.description, t.input_schema))
        .collect::<Vec<_>>()
        .join("\n\n");
    format!(
        "<tools>\n你可以调用以下工具。调用格式：\n\
<tool_call name=\"工具名\">\n\
<param name=\"参数名\" string=\"true\">字符串参数原文</param>\n\
<param name=\"参数名\" string=\"false\">数字、布尔或 JSON 值</param>\n\
</tool_call>\n\n规则：\n\
- 一次回复可在多个 <tool_call> 里放互不依赖的调用。调用完立即停止，等待结果。\n\
- 工具结果会在下一条 user 消息里以 <tool_result>...</tool_result> 给出，不要自己编写。\n\
- Edit 的 old_string 必须和文件原文逐字一致。\n\
- 不需要工具时直接文字回答。\n\n\
{}\n</tools>",
        list
    )
}

pub fn build_prompt(system: Option<&str>, tools: &[ToolSpec], messages: &[Message]) -> String {
    let mut parts = Vec::new();
    if let Some(s) = system.filter(|s| !s.trim().is_empty()) {
        parts.push(format!("<system>\n{}\n</system>", s));
    }
    let tp = tools_prompt(tools);
    if !tp.is_empty() {
        parts.push(tp);
    }
    for m in messages {
        parts.push(format!("<{}>\n{}\n</{}>", m.role, content_text(&m.content), m.role));
    }
    parts.push(
        "请以 assistant 的身份回复最后一条 user 消息，只输出回复内容，需要时按约定格式调用工具。".into(),
    );
    parts.join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_contains_system_tools_and_messages() {
        let tools = vec![ToolSpec {
            name: "Read".into(),
            description: "读文件".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": { "file_path": { "type": "string" } }
            }),
        }];
        let msgs = vec![Message {
            role: "user".into(),
            content: vec![Block::Text { text: "看下 a.js".into() }],
        }];
        let p = build_prompt(Some("你是助手"), &tools, &msgs);
        assert!(p.contains("<system>\n你是助手\n</system>"));
        assert!(p.contains("## Read"));
        assert!(p.contains("<tool_call name=\"工具名\">"));
        assert!(p.contains("<user>\n看下 a.js\n</user>"));
        assert!(p.contains("请以 assistant 的身份回复最后一条 user 消息"));
        assert!(!p.contains("DSML"));
    }

    #[test]
    fn tool_results_render_with_error_flag() {
        let blocks = vec![
            Block::ToolResult { is_error: true, content: "坏了".into() },
            Block::ToolResult { is_error: false, content: "好了".into() },
        ];
        let t = content_text(&blocks);
        assert!(t.contains("<tool_result error=\"true\">\n坏了\n</tool_result>"));
        assert!(t.contains("<tool_result>\n好了\n</tool_result>"));
    }
}
