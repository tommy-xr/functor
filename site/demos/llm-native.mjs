// Reproducible capture of the "LLM-Native" hero GIF.
//
// Runs a game with a HIDDEN window (a valid framebuffer, but nothing on screen)
// and the debug server, then drives + observes it entirely over HTTP the way a
// coding agent would: render frames on demand (POST /capture), pause the running
// game (POST /time set), and inspect its render tree as data (GET /scene). The
// left pane is that real terminal session; the right pane is the game, rendered
// only because the terminal asked for it. Everything shown is captured live.
//
// Prereqs: a current functor binary (npm run build:cli:debug — `run` does not
// rebuild the runtime), @playwright/test's chromium, and ffmpeg on PATH.
//
//   npm run demo:llm-native                 # -> site/demos/llm-native.gif
//   node site/demos/llm-native.mjs out.gif  # custom output path
import { spawn, execFileSync } from "node:child_process";
import { mkdtempSync, mkdirSync, rmSync, existsSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { Buffer } from "node:buffer";
import { chromium } from "@playwright/test";

const ROOT = fileURLToPath(new URL("../..", import.meta.url));
const PORT = 8077;
const HOST = `http://127.0.0.1:${PORT}`;
const GAME = process.env.DEMO_GAME || "examples/primitives";
const OUT = resolve(process.argv[2] || join(ROOT, "site/demos/llm-native.gif"));
const WIDTH = 1180;
const HEIGHT = 620;
const GIF_WIDTH = 900;
const FPS = Number(process.env.DEMO_FPS || 14);

const BIN = existsSync(join(ROOT, "target/debug/functor"))
  ? join(ROOT, "target/debug/functor")
  : join(ROOT, "target/release/functor");
if (!existsSync(BIN)) {
  console.error("missing the functor binary — build it:\n  npm run build:cli:debug");
  process.exit(1);
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const round2 = (n) => Math.round(n * 100) / 100;

// Every "Geometry" leaf, via recursive descent — computed exactly the way
// `jq '[.. | .Geometry? // empty]'` extracts it from the real /scene response,
// so the terminal shows the actual returned values, not a hand-made shape.
const geoms = (node, out = []) => {
  if (Array.isArray(node)) node.forEach((n) => geoms(n, out));
  else if (node && typeof node === "object") {
    if (typeof node.Geometry === "string") out.push(node.Geometry);
    Object.values(node).forEach((v) => geoms(v, out));
  }
  return out;
};

// Pretty-print the REAL response but abridged: number arrays inline, 4x4
// matrices compacted, and anything past SCENE_DEPTH replaced with "…". Every
// value shown is exactly what /scene returned; the "…" marks what we trimmed.
const SCENE_DEPTH = 3;
const abridge = (v, ind = 0, depth = 0) => {
  const pad = "  ".repeat(ind);
  const padc = "  ".repeat(ind + 1);
  if (v === null) return "null";
  if (typeof v === "number") return String(round2(v));
  if (typeof v !== "object") return JSON.stringify(v);
  if (Array.isArray(v)) {
    if (!v.length) return "[]";
    if (v.every((x) => typeof x === "number")) return "[" + v.map(round2).join(", ") + "]";
    if (v.every((x) => Array.isArray(x) && x.every((y) => typeof y === "number")))
      return "[ [" + v[0].map(round2).join(",") + "]" + (v.length > 1 ? ", …" : "") + " ]";
    if (depth >= SCENE_DEPTH) return "[ … ]";
    const items = [padc + abridge(v[0], ind + 1, depth + 1)];
    if (v.length > 1) items.push(padc + "…");
    return "[\n" + items.join(",\n") + "\n" + pad + "]";
  }
  if (depth >= SCENE_DEPTH) return "{ … }";
  const keys = Object.keys(v);
  return (
    "{\n" +
    keys.map((k) => padc + JSON.stringify(k) + ": " + abridge(v[k], ind + 1, depth + 1)).join(",\n") +
    "\n" +
    pad +
    "}"
  );
};

// --- Run the game (hidden window → /capture works) ---------------------------
const game = spawn(
  BIN,
  ["-d", GAME, "run", "native", "--hidden", "--debug-port", String(PORT)],
  { cwd: ROOT, stdio: "ignore" }
);
process.on("exit", () => game.kill());
for (let i = 0; ; i++) {
  try {
    if ((await fetch(`${HOST}/`)).ok) break;
  } catch {
    /* not up */
  }
  if (i > 60) throw new Error("game never came up");
  await sleep(150);
}
await sleep(1200);

// --- The terminal + game-view page -------------------------------------------
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
    #cap { padding:13px 22px; text-align:center; font-size:15px; color:#e9e6f2;
      background:#161226; border-bottom:1px solid #2b2542; opacity:0; transition:opacity 0.28s; }
    #cap.show { opacity:1; } #cap b { color:#41d8e6; font-weight:600; }
    #cols { flex:1; display:flex; min-height:0; }
    #term { flex:1; padding:18px 24px; overflow:hidden; font-size:13.5px; line-height:1.55;
      display:flex; flex-direction:column; justify-content:flex-end; }
    .ln { white-space:pre-wrap; } .pr { color:#41d8e6; font-weight:600; }
    .out { color:#9b94b3; } .out .k { color:#41d8e6; } .out .n { color:#b7a9e0; } .out .s { color:#6fdc92; }
    .cur { color:#41d8e6; }
    #stage { width:470px; display:flex; flex-direction:column; border-left:1px solid #2b2542; background:#0d0a18; }
    #shdr { padding:9px 14px; font-size:12px; color:#9b94b3; border-bottom:1px solid #2b2542;
      display:flex; align-items:center; gap:8px; }
    #dot { width:8px; height:8px; border-radius:50%; background:#6fdc92; }
    #shdr.paused #dot { background:#eec877; }
    #gv { flex:1; width:100%; min-height:0; object-fit:cover; background:#0d0221; display:block; }
  </style></head><body>
  <div id="wrap">
    <div id="cap"></div>
    <div id="cols">
      <div id="term"></div>
      <div id="stage"><div id="shdr"><span id="dot"></span><span id="slbl">off-screen render — POST /capture</span></div><img id="gv" /></div>
    </div>
  </div></body></html>`);
await page.evaluate(() => {
  const term = document.getElementById("term");
  window.__t = {
    cmd() {
      const ln = document.createElement("div");
      ln.className = "ln";
      ln.innerHTML = '<span class="pr">$</span> <span class="c"></span><span class="cur">▋</span>';
      term.appendChild(ln);
      window.__c = ln.querySelector(".c");
      window.__caret = ln.querySelector(".cur");
    },
    ch(c) { window.__c.textContent += c; },
    done() { window.__caret && window.__caret.remove(); },
    out(text) {
      const ln = document.createElement("div");
      ln.className = "ln out";
      ln.innerHTML = text
        .replace(/("[^"]*")(\s*:)/g, '<span class="k">$1</span>$2')
        .replace(/: (-?\d[\d.]*)/g, ': <span class="n">$1</span>')
        .replace(/("(?:Plane|Sphere|Cube|Quad)")/g, '<span class="s">$1</span>')
        .replace(/(#.*)$/g, '<span class="out">$1</span>');
      term.appendChild(ln);
    },
    paused(p) { document.getElementById("shdr").classList.toggle("paused", p);
      document.getElementById("slbl").textContent = p ? "off-screen render — paused by the agent" : "off-screen render — POST /capture"; },
  };
});

const framesDir = mkdtempSync(join(tmpdir(), "functor-ln-"));
let n = 0;
const snap = async () => {
  await page.screenshot({ path: join(framesDir, `f${String(n).padStart(4, "0")}.png`) });
  n++;
};
const hold = async (frames, ms = 70) => { for (let k = 0; k < frames; k++) { await snap(); await sleep(ms); } };
const caption = (html) => page.evaluate((h) => { const c = document.getElementById("cap"); c.innerHTML = h; c.classList.add("show"); }, html);
const label = (text) => page.evaluate((t) => { document.getElementById("slbl").textContent = t; }, text);
const capture = async () => {
  const buf = Buffer.from(await (await fetch(`${HOST}/capture`, { method: "POST" })).arrayBuffer());
  const url = "data:image/png;base64," + buf.toString("base64");
  await page.evaluate((u) => { document.getElementById("gv").src = u; }, url);
};
const typeCmd = async (text) => {
  await page.evaluate(() => window.__t.cmd());
  for (const c of text) {
    await page.evaluate((ch) => window.__t.ch(ch), c);
    if (" -/:".includes(c)) await snap();
    await sleep(12);
  }
  await page.evaluate(() => window.__t.done());
  await snap();
};
const out = async (lines) => { for (const l of lines) { await page.evaluate((x) => window.__t.out(x), l); await snap(); await sleep(80); } };
// Dump a whole block at once (like jq printing its result), then one frame.
const outBlock = async (lines) => { for (const l of lines) await page.evaluate((x) => window.__t.out(x), l); await snap(); };

// 0. Intro — a running game, rendering off-screen (nothing on screen).
await capture();
await caption("A game running <b>off-screen</b> — a live framebuffer, no window.");
await hold(7, 70);

// 1. Run it. The view is a LIVE feed — it animates because the game runs in real
//    time, not because we capture. Smooth frames make that obvious.
await typeCmd(`functor -d ${GAME} run native --hidden --debug-port ${PORT}`);
await out(["# --hidden: renders to an off-screen buffer — no window on screen"]);
await label("live · off-screen render");
for (let k = 0; k < 9; k++) { await capture(); await snap(); await sleep(105); }
await hold(3, 70);

// 2. Pause the running game — capture the instant it freezes, so the view stops
//    exactly at the pinned frame (no delayed jump when we capture later).
await caption("Pause the running game — <b>POST /time</b>.");
await typeCmd(`curl -sX POST :${PORT}/time -d '{"type":"set","tts":2.0}'`);
await fetch(`${HOST}/time`, { method: "POST", body: JSON.stringify({ type: "set", tts: 2.0 }) });
await capture();
await page.evaluate(() => window.__t.paused(true));
await out(["# clock pinned — the game is frozen where it stood"]);
await hold(6, 70);

// 3. Capture again — and nothing moves: while paused, /capture just re-reads the
//    same frame. It reads pixels; it does not advance the clock.
await caption("Now <b>POST /capture</b> — paused, so it's the same frame. Reading, not advancing.");
await typeCmd(`curl -sX POST :${PORT}/capture -o frame.png`);
await capture();
await hold(4, 70);

// 4. Inspect the render tree — the REAL /scene response, abridged with "…" for
//    what we trimmed. Nothing here is synthesised; it's what the game returned.
await caption("Inspect the <b>render tree</b> — the real /scene response, abridged.");
await typeCmd(`curl -s :${PORT}/scene | jq`);
const scene = await (await fetch(`${HOST}/scene`)).json();
await outBlock(abridge(scene).split("\n"));
await hold(7, 70);

// … and a plain jq filter pulls the shapes straight out of it — a real query.
await caption("Query it like any JSON — <b>jq '[.. | .Geometry?]'</b> lists every shape.");
await typeCmd(`curl -s :${PORT}/scene | jq '[.. | .Geometry? // empty]'`);
await out(["[ " + geoms(scene.scene).map((g) => `"${g}"`).join(", ") + " ]"]);
await hold(8, 70);
await caption("Driven and observed as <b>text</b> — your coding agent can build with you.");
await hold(22, 70);

await browser.close();
game.kill();

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
  { stdio: "inherit" }
);
rmSync(framesDir, { recursive: true, force: true });
console.log(`\nwrote ${OUT} (${n} frames)`);
