// A stand-in for the OpenAI Responses API, so a real Codex CLI runs a
// scripted session, and fires its real hooks, with no account or model.
// scripts/codex-e2e.sh uses it.
//
//   node scripts/codex-mock-model.mjs <scenario.json> <port> [requests.jsonl]
//
// The scenario: { "<conversation key>": [step, step, ...] }. A request's key
// is the first scenario key found in its users' text ("main" otherwise), so
// a subagent's conversation can have its own; its step is how many tool
// outputs its input already has. A step is a list of output items:
// {"say": "text"}, or {"call": "name", "args": {...}, "namespace"?: "..."}.
// "$AGENT" in a call's arguments is the id the last spawn_agent returned.
import http from 'node:http';
import fs from 'node:fs';

const [file, port = '7911', requests] = process.argv.slice(2);
// What each request asked for, to see what Codex sent.
const log = requests ? fs.createWriteStream(requests, {flags: 'a'}) : {write() {}};

const text = (v) => (typeof v === 'string' ? v : JSON.stringify(v ?? ''));

http
  .createServer((req, res) => {
    let body = '';
    req.on('data', (c) => (body += c));
    req.on('end', () => {
      if (req.method === 'GET') {
        res.writeHead(200, {'content-type': 'application/json'});
        res.end(JSON.stringify({object: 'list', data: [], models: []}));
        return;
      }
      const scenario = JSON.parse(fs.readFileSync(file, 'utf8'));
      let request = {};
      try {
        request = JSON.parse(body);
      } catch {}
      const input = Array.isArray(request.input) ? request.input : [];
      const all = text(input);
      const userText = text(input.filter((i) => i.role === 'user'));
      const key = Object.keys(scenario).find((k) => k !== 'main' && userText.includes(k)) || 'main';
      const outputs = input.filter((i) => /_output$/.test(i.type || '')).length;
      const steps = scenario[key];
      const step = steps[Math.min(outputs, steps.length - 1)];
      log.write(JSON.stringify({key, outputs, tools: (request.tools || []).map((t) => t.name ? (t.type === "namespace" ? t.name + "[" + (t.tools || []).map((x) => x.name).join(",") + "]" : t.name) : t.type), last: input.slice(-2)}) + '\n');
      const id = `resp_${Date.now()}_${Math.random().toString(36).slice(2, 8)}`;
      const events = [{type: 'response.created', response: {id}}];
      step.forEach((item, n) => {
        if (item.say !== undefined) {
          events.push({
            type: 'response.output_item.done',
            item: {type: 'message', role: 'assistant', id: `msg_${id}_${n}`, content: [{type: 'output_text', text: item.say}]},
          });
        } else {
          // "$AGENT" in the arguments: the id the last spawn_agent returned.
          const spawned = [...all.matchAll(/agent_id\\*":\\*"([0-9a-f-]{36})/g)].pop();
          const args = JSON.stringify(item.args || {}).replaceAll('$AGENT', spawned ? spawned[1] : 'none');
          const call = {type: 'function_call', call_id: `call_${id}_${n}`, name: item.call, arguments: args};
          if (item.namespace) call.namespace = item.namespace;
          events.push({type: 'response.output_item.done', item: call});
        }
      });
      events.push({
        type: 'response.completed',
        response: {id, usage: {input_tokens: 0, input_tokens_details: null, output_tokens: 0, output_tokens_details: null, total_tokens: 0}},
      });
      res.writeHead(200, {'content-type': 'text/event-stream'});
      const delay = scenario.delayMs ? Number(scenario.delayMs) : 0;
      setTimeout(() => {
        for (const ev of events) res.write(`event: ${ev.type}\ndata: ${JSON.stringify(ev)}\n\n`);
        res.end();
      }, delay);
    });
  })
  .listen(Number(port), '127.0.0.1', () => console.log(`mock model on ${port}`));
