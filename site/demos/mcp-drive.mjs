// Reproducible capture of the "AI Native" GIF: an MCP client playing a game.
//
// Everything on the left is a REAL MCP exchange. The script spawns
// `functor mcp` (docs/mcp.md) and speaks raw JSON-RPC 2.0 over its stdio — the
// same wire a coding agent's client uses, no MCP library in between — then
// prints each call and the server's own answer. The right pane is not a
// recording of a window: it is the PNG `capture_frame` returned, frame by
// frame, from a game running with no visible window at all.
//
// The loop it shows is the documented one: launch_game → pause → get_state →
// send_input → step → capture_frame. Because the clock is pinned and every
// step advances a fixed dts, the run is reproducible: the same model numbers
// and the same poses come back every time. The only values that move between
// captures are `frame`/`tts`, which count from wherever the game had got to
// when the agent paused it (mario stands still until it is driven, so the
// simulation itself starts from the same place either way).
//
// Prereqs: a current functor binary (cargo build -p functor-cli
// --no-default-features, or npm run build:cli:debug), @playwright/test's
// chromium, and ffmpeg on PATH.
//
//   npm run demo:mcp-drive                  # -> site/media/feature-mcp-drive.gif (+ .png)
//   node site/demos/mcp-drive.mjs out.gif   # custom output path
import { spawn, execFileSync } from "node:child_process";
import { mkdtempSync, mkdirSync, rmSync, copyFileSync, existsSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { Buffer } from "node:buffer";

import { chromium } from "@playwright/test";

const ROOT = fileURLToPath(new URL("../..", import.meta.url));
const GAME = process.env.DEMO_GAME || "examples/mario";
const OUT = resolve(process.argv[2] || join(ROOT, "site/media/feature-mcp-drive.gif"));
const STILL = OUT.replace(/\.gif$/, ".png");
const WIDTH = 1180;
const HEIGHT = 620;
const GIF_WIDTH = 560; // the committed feature-GIF width on the landing page
const FPS = Number(process.env.DEMO_FPS || 14);
const DTS = 0.016; // the step size every `step` call advances, per frame
// mario's chasm spans x ∈ [-3, 3] and its default jump carries ~6.9 units, so
// leaving from here clears it with room to spare (examples/mario/game.fun).
const JUMP_FROM = -3.6;

const BIN = existsSync(join(ROOT, "target/debug/functor"))
  ? join(ROOT, "target/debug/functor")
  : join(ROOT, "target/release/functor");
if (!existsSync(BIN)) {
  console.error("missing the functor binary — build it:\n  cargo build -p functor-cli --no-default-features");
  process.exit(1);
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const round = (n, p = 2) => Math.round(n * 10 ** p) / 10 ** p;

// --- A line-delimited JSON-RPC client over the server's stdio -----------------
class Rpc {
  constructor(proc) {
    this.proc = proc;
    this.pending = new Map();
    this.nextId = 1;
    this.buffer = "";
    proc.stdout.setEncoding("utf8");
    proc.stdout.on("data", (chunk) => {
      this.buffer += chunk;
      let index;
      while ((index = this.buffer.indexOf("\n")) >= 0) {
        const line = this.buffer.slice(0, index).trim();
        this.buffer = this.buffer.slice(index + 1);
        if (!line) continue;
        const message = JSON.parse(line);
        const waiter = this.pending.get(message.id);
        if (!waiter) continue;
        this.pending.delete(message.id);
        message.error ? waiter.reject(new Error(JSON.stringify(message.error))) : waiter.resolve(message.result);
      }
    });
  }

  notify(method, params) {
    this.proc.stdin.write(`${JSON.stringify({ jsonrpc: "2.0", method, params })}\n`);
  }

  request(method, params) {
    const id = this.nextId++;
    this.proc.stdin.write(`${JSON.stringify({ jsonrpc: "2.0", id, method, params })}\n`);
    return new Promise((resolve_, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id);
        reject(new Error(`${method} timed out`));
      }, 60000);
      this.pending.set(id, { resolve: (v) => (clearTimeout(timer), resolve_(v)), reject });
    });
  }

  /** Call a tool; return { json, image } — whichever content blocks came back. */
  async call(name, args = {}) {
    const result = await this.request("tools/call", { name, arguments: args });
    const text = result.content.filter((b) => b.type === "text").map((b) => b.text).join("");
    if (result.isError) throw new Error(`${name} failed: ${text}`);
    const image = result.content.find((b) => b.type === "image");
    let json = null;
    try {
      json = JSON.parse(text);
    } catch {
      json = text;
    }
    return { json, image };
  }
}

const server = spawn(BIN, ["mcp"], { cwd: ROOT, stdio: ["pipe", "pipe", "ignore"] });
process.on("exit", () => server.kill());
const rpc = new Rpc(server);
await rpc.request("initialize", {
  protocolVersion: "2025-06-18",
  capabilities: {},
  clientInfo: { name: "functor-demo", version: "0" },
});
rpc.notify("notifications/initialized", {});

// --- The page: an MCP transcript beside the frames it asked for --------------
let browser;
try {
  browser = await chromium.launch();
} catch {
  browser = await chromium.launch({ channel: "chrome" });
}
const page = await browser.newPage({ viewport: { width: WIDTH, height: HEIGHT } });
await page.setContent(`<!doctype html><html><head>
  <link rel="preconnect" href="https://fonts.googleapis.com" />
  <link href="https://fonts.googleapis.com/css2?family=JetBrains+Mono:wght@400;600&display=swap" rel="stylesheet" />
  <style>
    html,body { margin:0; height:100%; background:#0f0c1d;
      font-family:'JetBrains Mono', ui-monospace, monospace; color:#e9e6f2; }
    #wrap { height:100vh; display:flex; flex-direction:column; }
    #cap { padding:13px 22px; text-align:center; font-size:15px;
      background:#161226; border-bottom:1px solid #2b2542; opacity:0; transition:opacity 0.28s; }
    #cap.show { opacity:1; } #cap b { color:#41d8e6; font-weight:600; }
    #cols { flex:1; display:flex; min-height:0; }
    #term { flex:1; padding:18px 24px; overflow:hidden; font-size:13.5px; line-height:1.6;
      display:flex; flex-direction:column; justify-content:flex-end; }
    .ln { white-space:pre-wrap; }
    .call .ar { color:#e858b8; font-weight:600; }
    .call .tool { color:#41d8e6; font-weight:600; }
    .res { color:#9b94b3; } .res .ar { color:#6fdc92; }
    .res .k { color:#41d8e6; } .res .n { color:#b7a9e0; } .res .b { color:#6fdc92; }
    .note { color:#6b6383; }
    .cur { color:#e858b8; }
    #stage { width:470px; display:flex; flex-direction:column; border-left:1px solid #2b2542; background:#0d0a18; }
    #shdr { padding:9px 14px; font-size:12px; color:#9b94b3; border-bottom:1px solid #2b2542;
      display:flex; align-items:center; gap:8px; }
    #dot { width:8px; height:8px; border-radius:50%; background:#6b6383; }
    #shdr.paused #dot { background:#eec877; }
    #gv { flex:1; width:100%; min-height:0; object-fit:cover; background:#0d0221; display:block; }
  </style></head><body>
  <div id="wrap">
    <div id="cap"></div>
    <div id="cols">
      <div id="term"></div>
      <div id="stage"><div id="shdr"><span id="dot"></span><span id="slbl">no window — nothing on screen yet</span></div><img id="gv" /></div>
    </div>
  </div></body></html>`);
await page.evaluate(() => {
  const term = document.getElementById("term");
  const add = (cls, html) => {
    const ln = document.createElement("div");
    ln.className = `ln ${cls}`;
    ln.innerHTML = html;
    term.appendChild(ln);
    return ln;
  };
  const paint = (text) =>
    text
      .replace(/("[^"]*")(\s*:)/g, '<span class="k">$1</span>$2')
      .replace(/: (-?\d[\d.]*)/g, ': <span class="n">$1</span>')
      .replace(/: (true|false)/g, ': <span class="b">$1</span>');
  window.__t = {
    call(tool) {
      const ln = add("call", `<span class="ar">→</span> <span class="tool">${tool}</span> <span class="a"></span><span class="cur">▋</span>`);
      window.__a = ln.querySelector(".a");
      window.__caret = ln.querySelector(".cur");
    },
    ch(c) { window.__a.textContent += c; },
    done() { window.__caret && window.__caret.remove(); },
    res(text) { add("res", `<span class="ar">←</span> ${paint(text)}`); },
    note(text) { add("note", text); },
    stage(label, paused) {
      document.getElementById("shdr").classList.toggle("paused", !!paused);
      document.getElementById("slbl").textContent = label;
    },
  };
});

const framesDir = mkdtempSync(join(tmpdir(), "functor-mcp-"));
let n = 0;
let stillFrame = null;
const snap = async () => {
  const path = join(framesDir, `f${String(n).padStart(4, "0")}.png`);
  await page.screenshot({ path });
  n++;
  return path;
};
const hold = async (frames, ms = 70) => {
  for (let k = 0; k < frames; k++) {
    await snap();
    await sleep(ms);
  }
};
const caption = (html) =>
  page.evaluate((h) => {
    const c = document.getElementById("cap");
    c.innerHTML = h;
    c.classList.add("show");
  }, html);
const stage = (label, paused) => page.evaluate(([l, p]) => window.__t.stage(l, p), [label, paused]);
const res = async (text) => {
  await page.evaluate((t) => window.__t.res(t), text);
  await snap();
};
const note = async (text) => {
  await page.evaluate((t) => window.__t.note(t), text);
  await snap();
};

// Type the tool call out, then actually make it. `quiet` performs the call
// without showing it — used for the per-step `capture_frame`, whose result IS
// the game pane (the call itself is shown the first time it appears).
const call = async (tool, args, { quiet = false } = {}) => {
  if (quiet) return rpc.call(tool, args);
  const text = JSON.stringify(args).replace(/,"/g, ', "').replace(/":/g, '": ');
  await page.evaluate((t) => window.__t.call(t), tool);
  for (const [i, c] of [...text].entries()) {
    await page.evaluate((ch) => window.__t.ch(ch), c);
    if (i % 7 === 6) await snap();
    await sleep(6);
  }
  await page.evaluate(() => window.__t.done());
  await snap();
  return rpc.call(tool, args);
};

/** Show the PNG `capture_frame` just returned; report its real dimensions. */
const showFrame = async (image) => {
  await page.evaluate((u) => {
    document.getElementById("gv").src = u;
  }, `data:${image.mimeType};base64,${image.data}`);
  const png = Buffer.from(image.data, "base64");
  return `${png.readUInt32BE(16)}×${png.readUInt32BE(20)}`;
};

/** A one-line view of a real response object, keeping only `keys`. */
/** The server's answer as one line: its own text, or a compact object view. */
const answer = (json, keys) => (typeof json === "string" ? json : pick(json, keys));

const pick = (json, keys) =>
  "{ " + keys.filter((k) => json[k] !== undefined).map((k) => `"${k}": ${JSON.stringify(json[k])}`).join(", ") + " }";

const modelLine = (state) => {
  const m = state.model;
  return `{ "frame": ${state.frame}, "tts": ${round(state.tts)}, "model": { "x": ${round(m.x)}, "y": ${round(m.y)}, "grounded": ${m.grounded} } }`;
};

// --- 1. Launch a game, through the MCP server -------------------------------
await caption("A coding agent's MCP client, driving a real game — <b>functor mcp</b>.");
await hold(4);

const launched = await call("launch_game", { dir: GAME, mode: "hidden" });
const session = launched.json.session;
// The response's `port` is deliberately not shown: it is a free port the
// server picks, and it is the one value in the whole run that would differ
// between captures.
await res(answer(launched.json, ["session", "mode", "owned"]));
await note("  # a real runtime — a GL context in a window that is never shown");
await stage(`session ${session} — running`, false);

let shot = await call("capture_frame", { session });
await res(`[image] image/png, ${await showFrame(shot.image)}`);
await hold(5);

// --- 2. Pause: the clock is the agent's now ---------------------------------
await caption("Pin the clock — <b>pause</b>. Nothing advances unless the agent asks.");
const paused = await call("pause", { session });
await res(answer(paused.json, ["mode", "tts"]));
await stage(`session ${session} — paused`, true);
let state = (await call("get_state", { session })).json;
await res(modelLine(state));
await note("  # the model itself, as structured JSON — not a debug string");
shot = await call("capture_frame", { session });
await showFrame(shot.image);
await snap();
await hold(5);

// --- 3. Play it: hold Right, and step the world by hand ----------------------
await caption("Play it — <b>send_input</b> holds a key, <b>step</b> advances exact frames.");
const injected = await call("send_input", { session, command: { type: "key", key: "Right", down: true } });
await res(answer(injected.json));
await note("  # input is level state: it stays held across steps");

// Run to the lip of the chasm (it spans x ∈ [-3, 3]) — the model says when to
// stop, so the demo does not depend on counting frames by hand.
for (let k = 0; k < 12 && state.model.x < JUMP_FROM; k++) {
  state = (await call("step", { session, frames: 4, dts: DTS })).json;
  await res(modelLine(state));
  shot = await call("capture_frame", { session }, { quiet: true });
  await showFrame(shot.image);
  await snap();
}
await hold(3);

// --- 4. Jump the chasm, one injected key and the frames it takes -------------
await caption("One injected <b>Space</b>, then frame-by-frame — the jump, on demand.");
const jumped = await call("send_input", { session, command: { type: "key", key: "Space", down: true } });
await res(answer(jumped.json));
await call("send_input", { session, command: { type: "key", key: "Space", down: false } }, { quiet: true });

// Step until the model says it landed, so the closing caption is the game's
// own answer rather than a guess about how long the arc takes. The cap is a
// guard: a jump that never lands means the shot list no longer holds.
let airborne = 0;
for (; airborne < 16 && !(airborne > 0 && state.model.grounded); airborne++) {
  state = (await call("step", { session, frames: 6, dts: DTS })).json;
  await res(modelLine(state));
  shot = await call("capture_frame", { session }, { quiet: true });
  await showFrame(shot.image);
  const path = await snap();
  if (airborne === 4) stillFrame = path;
}
// mario respawns on the left platform if it falls in, and that also reads as
// `grounded: true` — so the far side is what proves the jump cleared.
if (!state.model.grounded || state.model.x < 3) {
  throw new Error(
    `the jump did not clear the chasm after ${airborne} steps (x = ${state.model.x}) — retune the shot list`,
  );
}

await caption("Landed — <b>grounded: true</b>. Every frame here was asked for over MCP.");
await hold(14);

await rpc.call("stop_game", { session });
await browser.close();
server.stdin.end();

// --- Render ------------------------------------------------------------------
mkdirSync(dirname(OUT), { recursive: true });
execFileSync(
  "ffmpeg",
  [
    "-y",
    "-framerate", String(FPS),
    "-i", join(framesDir, "f%04d.png"),
    "-vf",
    `scale=${GIF_WIDTH}:-1:flags=lanczos,split[s0][s1];[s0]palettegen=stats_mode=diff[p];[s1][p]paletteuse=dither=bayer:bayer_scale=3`,
    "-loop", "0",
    OUT,
  ],
  { stdio: "inherit" },
);
copyFileSync(stillFrame ?? join(framesDir, `f${String(n - 1).padStart(4, "0")}.png`), STILL);
rmSync(framesDir, { recursive: true, force: true });
console.log(`\nwrote ${OUT} (${n} frames) and ${STILL}`);
