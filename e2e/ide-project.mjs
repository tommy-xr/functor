// IDE project-seam e2e: the multi-file postMessage protocol, headless — the
// seam the site IDE builds on (player.html?project=inline +
// functor_lang_set_project), with no IDE UI involved:
//
//   1. player.html?project=inline announces `functor-lang-project-waiting`
//      and boots nothing until files arrive;
//   2. a pushed two-file project (game.fun referencing Pieces.*) boots from
//      MEMORY — no .fun is fetched — and reports `functor-lang-preview-ready`;
//   3. a `functor-lang-set-project` push editing the SIBLING module hot-swaps
//      (the thing `functor-lang-set-source` cannot do) and echoes the push id;
//   4. a broken sibling push reports the error and keeps the old program;
//   5. a good push after the broken one recovers and clears the overlay.
//
// Run manually (needs the wasm bundle):
//
//   wasm-pack build runtime/functor-runtime-web --target=web   # once
//   node e2e/ide-project.mjs
import { spawn, spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { chromium } from "@playwright/test";

const PORT = 8124;
const BASE = `http://127.0.0.1:${PORT}`;
const ROOT = fileURLToPath(new URL("..", import.meta.url));

const PIECES_V1 = `let speed = 1.0
let glow = 0.2
`;
const PIECES_V2 = `let speed = 3.0
let glow = 0.9
`;
const PIECES_BROKEN = `let speed =
`;
const GAME = `let init = { t: 0.0 }
let tick = (model, dt: Float, tts: Float) => { model with t: model.t + dt * Pieces.speed }
let draw = (model, tts: Float) =>
  Frame.create(
    Camera.lookAt(Vec3.make(0.0, 0.0, -6.0), Vec3.make(0.0, 0.0, 0.0)),
    Scene.sphere() |> Scene.emissive(Color.rgb(0.1, 1.0, Pieces.glow)) |> Scene.scale(2.0))
`;

const project = (pieces) => [
  { path: "game.fun", source: GAME },
  { path: "pieces.fun", source: pieces },
];

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const fail = (message) => {
  console.error(`FAIL: ${message}`);
  process.exitCode = 1;
};

// Build the site fresh so dist matches the working tree.
const built = spawnSync("node", ["site/build.mjs"], { cwd: ROOT, stdio: "inherit" });
if (built.status !== 0) {
  console.error("FAIL: site build failed");
  process.exit(1);
}

const server = spawn("node", ["site/serve.mjs", "--port", String(PORT)], {
  cwd: ROOT,
  stdio: "ignore",
});
const browser = await chromium.launch({
  args: [
    "--use-gl=angle",
    "--use-angle=swiftshader",
    "--enable-unsafe-swiftshader",
    "--ignore-gpu-blocklist",
  ],
});

try {
  const page = await browser.newPage({ viewport: { width: 640, height: 480 } });
  const log = [];
  page.on("console", (m) => log.push(m.text()));
  // Load the player in an IFRAME on a same-origin host — exactly how the IDE
  // hosts it. The player posts its announcements/results to `window.parent`
  // (the top frame), where the init-script listener collects them; our pushes
  // go the other way, to the iframe's contentWindow, arriving with
  // event.source === window.parent as the player's handler requires.
  await page.addInitScript(() => {
    window.__msgs = [];
    window.addEventListener("message", (e) => {
      if (e.data && typeof e.data.type === "string") window.__msgs.push(e.data);
    });
  });

  // A CLEAN same-origin host (not the landing page — its live hero player
  // emits the very protocol messages we assert on, which would pollute the
  // top-window collection). Route a blank document so only our iframe speaks.
  await page.route(`${BASE}/__ide_host`, (route) =>
    route.fulfill({ contentType: "text/html", body: "<!doctype html><title>ide host</title>" })
  );
  // Wait for the dev server, then mount the host page + player iframe.
  for (let i = 0; i < 60; i++) {
    try {
      await page.goto(`${BASE}/__ide_host`);
      break;
    } catch {
      await sleep(500);
    }
  }
  await page.evaluate((base) => {
    const f = document.createElement("iframe");
    f.id = "player";
    f.style.cssText = "width:640px;height:480px;border:0";
    f.src = `${base}/player.html?project=inline`;
    document.body.appendChild(f);
  }, BASE);
  const post = (msg) =>
    page.evaluate(
      (m) => document.getElementById("player").contentWindow.postMessage(m, "*"),
      msg
    );

  const waitForMsg = async (predicate, what, timeoutMs = 20000) => {
    const deadline = Date.now() + timeoutMs;
    while (Date.now() < deadline) {
      const found = await page.evaluate(
        (pred) => window.__msgs.find((m) => new Function("m", `return ${pred}`)(m)) ?? null,
        predicate
      );
      if (found) return found;
      await sleep(100);
    }
    fail(`timed out waiting for ${what}`);
    return null;
  };

  // 1. The inline player waits — it must announce, and must NOT have booted.
  if (!(await waitForMsg(`m.type === "functor-lang-project-waiting"`, "project-waiting"))) {
    throw new Error("no waiting announcement");
  }
  if (log.some((l) => l.includes("[functor-lang] loaded"))) {
    fail("player booted before any project was pushed");
  }

  // 1b. A MALFORMED first push is rejected but must NOT consume the boot
  // handshake — a subsequent valid push still boots (a broken editor state
  // shouldn't wedge the preview forever).
  await post({ type: "functor-lang-set-project", files: [{}], id: 1 });
  const rejected = await waitForMsg(
    `m.type === "functor-lang-set-source-result" && m.id === 1`,
    "malformed-first-push rejection"
  );
  if (rejected?.ok) fail("a malformed first project push was accepted");
  else if (rejected) console.log("malformed first push rejected, handshake kept ✓");
  if (log.some((l) => l.includes("[functor-lang] loaded"))) {
    fail("player booted from a malformed first push");
  }

  // 2. Push the two-file project; it boots from memory.
  await post({ type: "functor-lang-set-project", files: project(PIECES_V1) });
  await waitForMsg(`m.type === "functor-lang-preview-ready"`, "preview-ready");
  if (!log.some((l) => l.includes("[functor-lang] loaded game.fun"))) {
    fail(`no "[functor-lang] loaded game.fun" in console:\n${log.join("\n")}`);
  } else {
    console.log("boot from pushed project ✓");
  }

  const pushProject = async (pieces, id) => {
    await post({ type: "functor-lang-set-project", files: project(pieces), id });
    return waitForMsg(`m.type === "functor-lang-set-source-result" && m.id === ${id}`, `result #${id}`);
  };

  // 3. Hot-swap the SIBLING module (set-source can't do this) — id echoed.
  const swap = await pushProject(PIECES_V2, 2);
  if (swap && !swap.ok) fail(`sibling hot-swap rejected: ${swap.message}`);
  if (swap?.ok && !/2 file/.test(swap.message)) {
    fail(`expected a 2-file project reload status, got: ${swap.message}`);
  }
  if (swap?.ok) console.log(`sibling hot-swap ✓ (${swap.message})`);

  // 4. A broken sibling keeps the old program (result not ok).
  const broken = await pushProject(PIECES_BROKEN, 3);
  if (broken?.ok) fail("a broken sibling push was accepted");
  else if (broken) console.log(`broken push rejected ✓ (${broken.message.slice(0, 60)}…)`);

  // 5. Recovery clears the overlay.
  const recovered = await pushProject(PIECES_V1, 4);
  if (recovered && !recovered.ok) fail(`recovery push rejected: ${recovered.message}`);
  await sleep(500);
  // The error overlay lives inside the player iframe, so probe its document.
  const overlayVisible = await page.frameLocator("#player").locator("#functor-lang-error").isVisible().catch(() => false);
  if (overlayVisible) fail("error overlay still visible after recovery");
  else if (recovered?.ok) console.log("recovery ✓ (overlay cleared)");

  console.log(process.exitCode ? "RESULT: FAIL" : "RESULT: PASS");
} finally {
  await browser.close();
  server.kill();
}
