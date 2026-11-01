// 从模型的流式文本里识别工具调用。DeepSeek 经常混用两种格式，甚至在一个调用里混着写：
//   <tool_use name="Read">{"file_path":"/a.js"}</tool_use>
//   <｜DSML｜function_calls>
//   <｜DSML｜invoke name="Write">
//   <｜DSML｜parameter name="file_path" string="true">/a.py</｜DSML｜parameter>
//   </｜DSML｜invoke>
//   </｜DSML｜function_calls>
// 所以这里不按严格的嵌套解析，而是按"标记"切分：
//   - 调用开始：<tool_use name="X"> 或 <｜DSML｜invoke name="X">
//   - 调用结束：</tool_use>、</｜DSML｜invoke>、下一个调用开始、calls 标签、或流结束
//   - calls 标签（不论有没有斜杠）和多余的结束标签直接丢弃
// 调用体里有 DSML parameter 就按参数解析，否则按 JSON 解析。
// 模型如果自己编造 <tool_result>，从那里开始的内容全部丢弃。

const P = '[｜|]+\\s*DSML\\s*[｜|]+\\s*';
const START = new RegExp(`^<(?:tool_use|${P}invoke)\\s+name\\s*=\\s*"([^"]+)"[^>]*>`);
const CALLS = new RegExp(`^<\\/?${P}(?:function_)?calls\\s*>`);
const STRAY_END = new RegExp(`^<\\/(?:tool_use|${P}invoke)\\s*>`);
const STOP = /^<tool_result/;
// 调用体在这些标记处结束（不消耗下一个调用的开始和 calls 标签）
const BODY_END = new RegExp(`<\\/(?:tool_use|${P}invoke)\\s*>|(?=<(?:tool_use|${P}invoke)\\s+name)|(?=<\\/?${P}(?:function_)?calls\\s*>)|(?=<tool_result)`);
const PARAM = new RegExp(`<${P}parameter\\s+name\\s*=\\s*"([^"]+)"([^>]*)>([\\s\\S]*?)(?:<\\/?${P}parameter\\s*>|(?=<${P}parameter\\s+name)|$)`, 'g');
// 可能是某个标记的前缀，需要等更多数据再判断
const MAYBE = /^<\/?(?:t(?:o(?:o(?:l(?:_(?:u(?:s(?:e)?)?|r(?:e(?:s(?:u(?:l(?:t)?)?)?)?)?)?)?)?)?)?|[｜|][^>]{0,40})?$/;

export function createToolParser({ onText, onToolUse }) {
  let buf = '';
  let stopped = false;
  let call = null; // { name, raw, body }

  const emit = (t) => { if (t) onText(t); };

  function finishCall(body) {
    const { name, raw } = call;
    call = null;
    let input;
    if (new RegExp(`<${P}parameter`).test(body)) {
      input = {};
      for (const [, key, attrs, value] of body.matchAll(PARAM)) {
        if (/string\s*=\s*"true"/.test(attrs)) { input[key] = value; continue; }
        try { input[key] = JSON.parse(value); } catch { input[key] = value; }
      }
    } else {
      const json = body.trim().replace(/^```(?:json)?\s*/, '').replace(/\s*```$/, '');
      try {
        input = json ? JSON.parse(json) : {};
        if (typeof input !== 'object' || Array.isArray(input) || input === null) throw new Error();
      } catch {
        return emit(raw + body); // 解析失败，按普通文字输出，不中断对话
      }
    }
    onToolUse({ name, input });
  }

  function run(final) {
    while (!stopped && buf) {
      if (call) {
        const m = buf.match(BODY_END);
        if (!m) {
          if (final) { finishCall(buf); buf = ''; }
          return;
        }
        const body = buf.slice(0, m.index);
        buf = buf.slice(m.index + m[0].length);
        finishCall(body);
        continue;
      }

      const i = buf.indexOf('<');
      if (i < 0) { emit(buf); buf = ''; return; }
      if (i > 0) { emit(buf.slice(0, i)); buf = buf.slice(i); }

      let m;
      if (STOP.test(buf)) { stopped = true; buf = ''; return; }
      if ((m = buf.match(START))) { call = { name: m[1], raw: m[0] }; buf = buf.slice(m[0].length); continue; }
      if ((m = buf.match(CALLS)) || (m = buf.match(STRAY_END))) { buf = buf.slice(m[0].length); continue; }
      // 可能是标记的前半截：没遇到 '>' 前先等着
      if (!final && !buf.includes('>') && buf.length < 60 && (MAYBE.test(buf) || /^<[｜|]|^<(?:tool_use|\/)/.test(buf))) return;
      emit('<');
      buf = buf.slice(1);
    }
  }

  return {
    push(text) { if (!stopped) { buf += text; run(false); } },
    end() { if (!stopped) run(true); },
  };
}
