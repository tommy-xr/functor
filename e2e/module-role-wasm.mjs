// A same-file MODULE entry role must boot in the browser, not just natively.
//
// `e2e/fixtures/module-role` declares one role — `{ "file": "game.fun",
// "module": "Server" }` — and its file deliberately has NO top-level
// `init`/`tick`/`draw`: if the page booted the plain contract it would fail to
// load, so reaching "[functor-lang] loaded" IS the proof the role resolved.
//
// The bundle is exported with `build wasm` and served from an ephemeral port
// rather than driven through `run wasm` (which hardcodes :8080): both render
// the SAME page from the same template — that `run wasm`'s served index
// carries the module is pinned by `substitutes_the_entry_module` in
// cli/src/util/wasm_dev_server.rs — and exporting keeps this check hermetic
// next to a dev server someone else is running.
//
//   npm run build:cli:debug   # once, so target/debug/functor embeds the runtime
//   node e2e/module-role-wasm.mjs
import { execFileSync } from "node:child_process";
import { createReadStream, statSync } from "node:fs";
import http from "node:http";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "@playwright/test";

const ROOT = fileURLToPath(new URL("..", import.meta.url));
const DIR = path.join(ROOT, "e2e", "fixtures", "module-role");
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

console.log(
  execFileSync(path.join(ROOT, "target/debug/functor"), ["-d", DIR, "build", "wasm"], {
    encoding: "utf8",
  })
);

const TYPES = {
  ".html": "text/html",
  ".js": "text/javascript",
  ".wasm": "application/wasm",
  ".fun": "text/plain",
};
const root = path.join(DIR, "dist", "web");
const server = http.createServer((req, res) => {
  let rel = decodeURIComponent(req.url.split("?")[0]);
  if (rel === "/") rel = "/index.html";
  const file = path.join(root, rel);
  try {
    statSync(file);
  } catch {
    res.writeHead(404).end("not found");
    return;
  }
  res.writeHead(200, {
    "Content-Type": TYPES[path.extname(file)] ?? "application/octet-stream",
  });
  createReadStream(file).pipe(res);
});
await new Promise((r) => server.listen(0, "127.0.0.1", r));
const { port } = server.address();

// Software WebGL2 (swiftshader) so the runtime's GL context comes up on any
// runner — no real GPU needed; this check compares no pixels.
const browser = await chromium.launch({
  args: [
    "--use-gl=angle",
    "--use-angle=swiftshader",
    "--enable-unsafe-swiftshader",
    "--ignore-gpu-blocklist",
  ],
});
let ok = false;
let reason = "";
try {
  const page = await browser.newPage({ viewport: { width: 640, height: 480 } });
  const log = [];
  page.on("console", (m) => log.push(`${m.type()}: ${m.text()}`));
  page.on("pageerror", (e) => log.push(`pageerror: ${e}`));
  await page.goto(`http://127.0.0.1:${port}/`);
  for (let i = 0; !log.some((m) => m.includes("[functor-lang] loaded")); i++) {
    if (i > 60) throw new Error(`never loaded:\n${log.join("\n")}`);
    await sleep(250);
  }
  const [bootModule, bootPrefix] = await page.evaluate(() => [
    window.__functorLangEntryModule,
    window.__functorLangEntryPrefix,
  ]);
  await sleep(1500); // run frames: a broken role would error per frame
  const errors = log.filter((l) => /\[functor-lang\].*error/.test(l));
  console.log(
    `boot config: module=${JSON.stringify(bootModule)} prefix=${JSON.stringify(bootPrefix)}`
  );
  console.log(log.filter((l) => l.includes("[functor-lang]")).join("\n"));
  ok = bootModule === "Server" && bootPrefix === "" && errors.length === 0;
  reason = ok ? "" : `module=${bootModule} prefix=${bootPrefix} errors=${errors.join(" | ")}`;
} catch (e) {
  reason = String(e);
} finally {
  await browser.close();
  server.close();
}
console.log(ok ? "PASS: the module role booted on wasm" : `FAIL: ${reason}`);
process.exit(ok ? 0 : 1);
