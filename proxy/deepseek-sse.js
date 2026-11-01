// 解析 DeepSeek 网页端的 SSE 流（JSON patch 风格），输出文本增量。
//
// 格式要点（实测）：
//   data: {"v":{"response":{..., "fragments":[{"type":"RESPONSE","content":"这是一个"}]}}}  初始快照
//   data: {"p":"response/fragments/-1/content","o":"APPEND","v":"最"}                      带路径的追加
//   data: {"v":"经典的"}                                                                    省略 p/o，沿用上一条的路径
//   data: {"p":"response/fragments","o":"APPEND","v":[{...}]}                               新增 fragment（如思考 → 回答）
//   data: {"p":"response/status","o":"SET","v":"FINISHED"}                                  结束
//
// onDelta(kind, text)：kind 为 'text'（RESPONSE 片段）或 'thinking'（THINK 片段）

export function createParser({ onDelta, onDone, onError }) {
  let buf = '';
  let event = 'message';
  let lastPath = null;
  let fragments = [];
  let finished = false;

  const kindOf = (frag) => (frag?.type === 'THINK' ? 'thinking' : 'text');

  function addFragments(list) {
    for (const f of list) {
      fragments.push({ type: f.type, content: '' });
      if (f.content) appendTo(-1, f.content);
    }
  }

  function appendTo(index, text) {
    const frag = fragments.at(index);
    if (!frag || typeof text !== 'string' || !text) return;
    frag.content += text;
    onDelta(kindOf(frag), text);
  }

  function finish() {
    if (finished) return;
    finished = true;
    onDone?.();
  }

  function handlePatch(p, o, v) {
    if (p === 'response' && o === 'BATCH' && Array.isArray(v)) {
      for (const item of v) handlePatch('response/' + item.p, item.o || 'SET', item.v);
      return;
    }
    if (p === 'response/fragments' && Array.isArray(v)) return addFragments(v);
    const m = p?.match(/^response\/fragments\/(-?\d+)\/content$/);
    if (m && typeof v === 'string') return appendTo(Number(m[1]), v);
    if ((p === 'response/status' || p === 'response/quasi_status') && v === 'FINISHED') return finish();
  }

  function handleData(raw) {
    let msg;
    try { msg = JSON.parse(raw); } catch { return; }

    if (event === 'close') return finish();
    if (event !== 'message') return; // ready / update_session / title 等事件忽略

    if (msg.p === undefined && msg.v && typeof msg.v === 'object' && msg.v.response) {
      fragments = [];
      addFragments(msg.v.response.fragments || []);
      lastPath = 'response/fragments/-1/content';
      return;
    }
    if (msg.p !== undefined) lastPath = msg.p;
    handlePatch(lastPath, msg.o || 'APPEND', msg.v);
  }

  return {
    // 数据块可能在任意位置截断，按行缓冲
    push(chunk) {
      buf += chunk;
      let i;
      while ((i = buf.indexOf('\n')) >= 0) {
        const line = buf.slice(0, i).replace(/\r$/, '');
        buf = buf.slice(i + 1);
        if (line === '') { event = 'message'; continue; }
        if (line.startsWith('event:')) event = line.slice(6).trim();
        else if (line.startsWith('data:')) handleData(line.slice(5).trim());
      }
    },
    end() { finish(); },
    error(e) { onError?.(e); },
    text() { return fragments.filter((f) => f.type !== 'THINK').map((f) => f.content).join(''); },
  };
}
