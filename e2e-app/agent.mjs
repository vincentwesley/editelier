// Client for the app's debug agent channel (D7): JSON-RPC 2.0 over the named pipe the app
// announces in %TEMP%\editelier-agent.json, with the per-session token on every request.
import { readFileSync } from "node:fs";
import net from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";

export const INFO = join(tmpdir(), "editelier-agent.json");
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

function open(pipe) {
  return new Promise((resolve, reject) => {
    const s = net.connect(pipe, () => resolve(s));
    s.once("error", reject);
  });
}

export async function connect(timeoutMs = 60_000) {
  const end = Date.now() + timeoutMs;
  for (;;) {
    try {
      const { pipe, token } = JSON.parse(readFileSync(INFO, "utf8"));
      const sock = await open(pipe);
      return client(sock, token);
    } catch (e) {
      if (Date.now() > end) throw new Error(`agent channel not reachable: ${e.message}`);
      await sleep(250);
    }
  }
}

function client(sock, token) {
  let buf = "";
  let id = 0;
  const waiting = new Map();
  sock.setEncoding("utf8");
  sock.on("data", (d) => {
    buf += d;
    let i;
    while ((i = buf.indexOf("\n")) >= 0) {
      const msg = JSON.parse(buf.slice(0, i));
      buf = buf.slice(i + 1);
      waiting.get(msg.id)?.(msg);
      waiting.delete(msg.id);
    }
  });
  // One request in flight at a time (the pipe serves requests in order anyway).
  let chain = Promise.resolve();
  const call = (method, params = {}) => {
    const run = () =>
      new Promise((resolve, reject) => {
        const n = ++id;
        waiting.set(n, (m) => (m.error ? reject(new Error(`${method}: ${m.error.message} (${m.error.data ?? m.error.code})`)) : resolve(m.result)));
        sock.write(JSON.stringify({ jsonrpc: "2.0", id: n, token, method, params }) + "\n");
      });
    const p = chain.then(run, run);
    chain = p.catch(() => {});
    return p;
  };
  return { call, close: () => sock.end() };
}
