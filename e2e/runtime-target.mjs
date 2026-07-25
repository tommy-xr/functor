// Browser-runtime link e2e. A hermetic fake debug runtime verifies that BOTH
// editor surfaces:
//   - load the whole current project on the first explicit push;
//   - hot-reload later edits with the model preserved;
//   - fresh-load resets/restarts;
//   - poll state and render a binary capture.
//
// Run manually (needs the two wasm bundles used by site:build):
//   npm run test:runtime-target

import { spawn, spawnSync } from "node:child_process";
import { createServer } from "node:http";
import { fileURLToPath } from "node:url";
import { chromium } from "@playwright/test";

const SITE_PORT = 8136;
const RUNTIME_PORT = 8137;
const BASE = `http://127.0.0.1:${SITE_PORT}`;
const RUNTIME = `http://127.0.0.1:${RUNTIME_PORT}`;
const ROOT = fileURLToPath(new URL("..", import.meta.url));
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

// Valid 1×1 PNG. The transport and browser image decode are what this suite
// exercises; real stereo pixels are covered by the Quest device loop.
const PNG = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=",
  "base64"
);

let failures = 0;
const check = (name, ok, detail = "") => {
  console.log(`${ok ? "PASS" : "FAIL"}: ${name}${ok || !detail ? "" : ` — ${detail}`}`);
  if (!ok) failures += 1;
};

const requests = [];
let frame = 40;
let delayNextLoadMs = 0;
let delayNextReloadMs = 0;
let delayNextCaptureMs = 0;
let delayNextAssetMs = 0;
let dropNextReload = false;
let rejectNextProjectStatus = 0;

const readBody = async (request) => {
  const chunks = [];
  for await (const chunk of request) chunks.push(chunk);
  return Buffer.concat(chunks);
};

const uploadedAssetPath = (body) => {
  const pathLength = body.readUInt32BE(0);
  return body.subarray(4, 4 + pathLength).toString("utf8");
};

const runtimeServer = createServer(async (request, response) => {
  const origin = request.headers.origin;
  if (origin) {
    response.setHeader("Access-Control-Allow-Origin", origin);
    response.setHeader("Access-Control-Allow-Methods", "GET, POST, OPTIONS");
    response.setHeader("Access-Control-Allow-Headers", "Content-Type");
    response.setHeader("Access-Control-Allow-Private-Network", "true");
  }
  if (request.method === "OPTIONS") {
    response.writeHead(204).end();
    return;
  }

  const rawBody = await readBody(request);
  const body = rawBody.toString("utf8");
  requests.push({ method: request.method, path: request.url, body, rawBody });

  if (request.method === "GET" && request.url === "/") {
    response
      .writeHead(200, { "Content-Type": "application/json" })
      .end(
        JSON.stringify({
          service: "functor debug runtime",
          protocol_version: 2,
          endpoints: {},
        })
      );
  } else if (request.method === "GET" && request.url === "/state") {
    frame += 1;
    response
      .writeHead(200, { "Content-Type": "application/json" })
      .end(
        JSON.stringify({
          frame,
          tts: frame / 90,
          viewport: { width: 2048, height: 1024 },
          views: [
            { name: "left", viewport: { width: 1024, height: 1024 } },
            { name: "right", viewport: { width: 1024, height: 1024 } },
          ],
          model: `{ ticks: ${frame} }`,
          input: { held_keys: [], mouse: { x: 0, y: 0 } },
        })
      );
  } else if (
    request.method === "POST" &&
    (request.url === "/load-project" || request.url === "/reload-project")
  ) {
    if (request.url === "/reload-project" && dropNextReload) {
      dropNextReload = false;
      request.socket.destroy();
      return;
    }
    if (rejectNextProjectStatus !== 0) {
      const status = rejectNextProjectStatus;
      rejectNextProjectStatus = 0;
      response
        .writeHead(status, { "Content-Type": "text/plain; charset=utf-8" })
        .end("project rejected");
      return;
    }
    const delayMs =
      request.url === "/load-project" ? delayNextLoadMs : delayNextReloadMs;
    delayNextLoadMs = 0;
    delayNextReloadMs = 0;
    if (delayMs > 0) await sleep(delayMs);
    response
      .writeHead(200, { "Content-Type": "text/plain; charset=utf-8" })
      .end(request.url === "/load-project" ? "loaded fresh model" : "reloaded; model preserved");
  } else if (request.method === "POST" && request.url === "/capture") {
    const delayMs = delayNextCaptureMs;
    delayNextCaptureMs = 0;
    if (delayMs > 0) await sleep(delayMs);
    response.writeHead(200, { "Content-Type": "image/png" }).end(PNG);
  } else if (request.method === "POST" && request.url === "/reload-asset") {
    const delayMs = delayNextAssetMs;
    delayNextAssetMs = 0;
    if (delayMs > 0) await sleep(delayMs);
    response.writeHead(200, { "Content-Type": "text/plain; charset=utf-8" }).end("asset updated");
  } else if (request.method === "POST" && request.url === "/sync-assets") {
    response.writeHead(200, { "Content-Type": "text/plain; charset=utf-8" }).end("assets synced");
  } else {
    response.writeHead(404).end("not found");
  }
});

const listen = (server, port) =>
  new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(port, "127.0.0.1", resolve);
  });

const waitForSite = async () => {
  for (let attempt = 0; attempt < 60; attempt += 1) {
    try {
      const response = await fetch(BASE);
      if (response.ok) return;
    } catch {}
    await sleep(250);
  }
  throw new Error(`site did not start at ${BASE}`);
};

const nextRequest = async (path, start) => {
  const deadline = Date.now() + 10_000;
  while (Date.now() < deadline) {
    const match = requests.slice(start).find((request) => request.path === path);
    if (match) return match;
    await sleep(50);
  }
  throw new Error(`timed out waiting for ${path}; saw ${JSON.stringify(requests.slice(start))}`);
};

const nextRequests = async (path, start, count) => {
  const deadline = Date.now() + 10_000;
  while (Date.now() < deadline) {
    const matches = requests.slice(start).filter((request) => request.path === path);
    if (matches.length >= count) return matches;
    await sleep(50);
  }
  throw new Error(
    `timed out waiting for ${count} ${path} requests; saw ${JSON.stringify(requests.slice(start))}`
  );
};

const openAndPush = async (page) => {
  await page.click("[data-runtime-summary]");
  await page.fill("[data-runtime-endpoint]", RUNTIME);
  await page.click("[data-runtime-push]");
  try {
    await page.waitForFunction(
      () => document.querySelector("[data-runtime-status]")?.dataset.state === "live",
      undefined,
      { timeout: 10_000 }
    );
  } catch {
    const state = await page.evaluate(async () => ({
      target: window.__sandbox?.runtimeTarget?.() ?? window.__ide?.runtimeTarget?.(),
      text: document.querySelector("[data-runtime-status]")?.textContent,
      disabled: document.querySelector("[data-runtime-push]")?.disabled,
      secure: window.isSecureContext,
      permission: await navigator.permissions
        .query({ name: "local-network-access" })
        .then((result) => result.state)
        .catch((error) => error.message),
      plainFetch: await fetch(document.querySelector("[data-runtime-endpoint]").value)
        .then((result) => `HTTP ${result.status}`)
        .catch((error) => `${error.name}: ${error.message}`),
    }));
    throw new Error(
      `runtime target did not become live: ${JSON.stringify(state)}; ` +
        `requests=${JSON.stringify(requests)}`
    );
  }
};

const build = spawnSync("node", ["site/build.mjs"], { cwd: ROOT, stdio: "inherit" });
if (build.status !== 0) process.exit(build.status ?? 1);

await listen(runtimeServer, RUNTIME_PORT);
const runtimeProbe = await fetch(RUNTIME);
if (!runtimeProbe.ok) throw new Error(`fake runtime probe failed: HTTP ${runtimeProbe.status}`);
const siteServer = spawn("node", ["site/serve.mjs", "--port", String(SITE_PORT)], {
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
  await waitForSite();
  const context = await browser.newContext({ viewport: { width: 1180, height: 720 } });
  await context.grantPermissions(["local-network-access"], { origin: BASE });

  // Sandbox: a single-file project, live reload, fresh example reset, state,
  // and capture.
  {
    const page = await context.newPage();
    await page.goto(`${BASE}/sandbox.html`);
    await page.waitForFunction(() => window.__sandbox?.status().state === "live", {
      timeout: 20_000,
    });

    let cursor = requests.length;
    delayNextLoadMs = 500;
    const initialPush = openAndPush(page);
    const initial = await nextRequest("/load-project", cursor);
    const initialFiles = JSON.parse(initial.body);
    check(
      "sandbox first push fresh-loads game.fun",
      initialFiles.length === 1 && initialFiles[0][0] === "game.fun"
    );
    await page.evaluate(() =>
      window.__sandbox.setSource(`${window.__sandbox.getSource()}\n// edit during first load`)
    );
    await initialPush;
    const initialFollowup = await nextRequest("/reload-project", cursor);
    check(
      "sandbox queues edits made during its initial load",
      JSON.parse(initialFollowup.body)[0][1].includes("// edit during first load")
    );

    cursor = requests.length;
    await page.evaluate(() =>
      window.__sandbox.setSource(`${window.__sandbox.getSource()}\n// device live edit`)
    );
    const reload = await nextRequest("/reload-project", cursor);
    check(
      "sandbox edits hot-reload with model preservation",
      JSON.parse(reload.body)[0][1].includes("// device live edit")
    );

    dropNextReload = true;
    cursor = requests.length;
    await page.evaluate(() =>
      window.__sandbox.setSource(`${window.__sandbox.getSource()}\n// retry after reconnect`)
    );
    await nextRequest("/reload-project", cursor);
    const retriedReload = (await nextRequests("/reload-project", cursor, 2))[1];
    check(
      "sandbox retries an unsent edit after transport recovery",
      JSON.parse(retriedReload.body)[0][1].includes("// retry after reconnect")
    );

    await page.waitForFunction(
      () => document.querySelector("[data-runtime-views]")?.textContent === "left + right",
      { timeout: 10_000 }
    );
    check("sandbox renders stereo state telemetry", true);

    cursor = requests.length;
    await page.click("[data-runtime-capture]");
    await nextRequest("/capture", cursor);
    await page.waitForFunction(
      () => {
        const image = document.querySelector("[data-runtime-image]");
        return image?.complete && image.naturalWidth === 1;
      },
      { timeout: 10_000 }
    );
    check("sandbox displays the runtime PNG capture", true);

    // Reset during an in-flight hot reload must not lose its fresh-model
    // intent when the older response completes.
    delayNextReloadMs = 500;
    cursor = requests.length;
    await page.evaluate(() =>
      window.__sandbox.setSource(`${window.__sandbox.getSource()}\n// edit before reset`)
    );
    await nextRequest("/reload-project", cursor);
    cursor = requests.length;
    await page.click("#reset");
    await nextRequest("/load-project", cursor);
    check("sandbox reset queues a fresh load behind an in-flight edit", true);

    await page.waitForFunction(
      () => document.querySelector("[data-runtime-status]")?.dataset.state === "live",
      { timeout: 10_000 }
    );
    await sleep(200);
    const statePollsBefore = requests.filter((request) => request.path === "/state").length;
    await sleep(2_250);
    const statePollsAfter = requests.filter((request) => request.path === "/state").length;
    check(
      "sandbox keeps one state polling chain",
      statePollsAfter - statePollsBefore <= 3,
      `${statePollsAfter - statePollsBefore} polls in 2.25s`
    );

    cursor = requests.length;
    await page.selectOption("#example-picker", "mario");
    const marioLoad = await nextRequest("/load-project", cursor);
    const marioFiles = JSON.parse(marioLoad.body);
    const marioAssetUploads = requests
      .slice(cursor)
      .filter((request) => request.path === "/reload-asset");
    const marioManifest = requests
      .slice(cursor)
      .find((request) => request.path === "/sync-assets");
    check(
      "sandbox example push includes sibling modules",
      marioFiles.map(([path]) => path).join(",") === "game.fun,assets.fun" &&
        marioFiles[1][1].includes("Asset.texture"),
      marioFiles.map(([path]) => path).join(",")
    );
    check(
      "sandbox uploads assets before source and finalizes its manifest after",
      marioAssetUploads.map((request) => uploadedAssetPath(request.rawBody)).join(",") ===
        "ground.png,hero-atlas.png" &&
        JSON.parse(marioManifest?.body ?? "[]").join(",") === "ground.png,hero-atlas.png" &&
        marioAssetUploads.every((request) => requests.indexOf(request) < requests.indexOf(marioLoad)) &&
        requests.indexOf(marioLoad) < requests.indexOf(marioManifest),
      marioAssetUploads.map((request) => uploadedAssetPath(request.rawBody)).join(",")
    );

    rejectNextProjectStatus = 413;
    cursor = requests.length;
    await page.selectOption("#example-picker", "hero");
    await nextRequest("/load-project", cursor);
    await page.waitForFunction(
      () => document.querySelector("[data-runtime-summary-state]")?.textContent === "sync error",
      undefined,
      { timeout: 10_000 }
    );
    check(
      "sandbox preserves the last-good asset manifest when new source is rejected",
      !requests.slice(cursor).some((request) => request.path === "/sync-assets")
    );
    check("sandbox labels every HTTP project rejection as a sync error", true);

    cursor = requests.length;
    await page.selectOption("#example-picker", "mario");
    await nextRequest("/load-project", cursor);
    await page.waitForFunction(
      () => document.querySelector("[data-runtime-status]")?.dataset.state === "live",
      undefined,
      { timeout: 10_000 }
    );

    // A newer reset that lands during asset transfer must retain its own fresh
    // model request after the older reset completes.
    delayNextAssetMs = 500;
    cursor = requests.length;
    await page.click("#reset");
    await nextRequest("/reload-asset", cursor);
    await page.click("#reset");
    const resetLoads = await nextRequests("/load-project", cursor, 2);
    check(
      "sandbox preserves a fresh reset queued during asset transfer",
      resetLoads.length >= 2
    );
    await page.waitForFunction(
      () => document.querySelector("[data-runtime-status]")?.dataset.state === "live",
      undefined,
      { timeout: 10_000 }
    );

    // Quest capture can take seconds. An edit made during it must wait and then
    // send the latest source, rather than racing the single runtime server.
    delayNextCaptureMs = 1_500;
    cursor = requests.length;
    await page.click("[data-runtime-capture]");
    await nextRequest("/capture", cursor);
    const captureRequestIndex = requests.findIndex(
      (request, index) => index >= cursor && request.path === "/capture"
    );
    await page.evaluate(() =>
      window.__sandbox.setSource(`${window.__sandbox.getSource()}\n// edit during capture`)
    );
    await sleep(1_200);
    check(
      "sandbox does not push concurrently with capture",
      !requests
        .slice(captureRequestIndex + 1)
        .some((request) => request.path === "/reload-project")
    );
    check(
      "sandbox suspends state polling during capture",
      !requests
        .slice(captureRequestIndex + 1)
        .some((request) => request.path === "/state")
    );
    cursor = requests.length;
    const postCaptureReload = await nextRequest("/reload-project", cursor);
    check(
      "sandbox flushes the latest edit after capture",
      JSON.parse(postCaptureReload.body)[0][1].includes("// edit during capture")
    );
    await page.setViewportSize({ width: 390, height: 844 });
    await page.click("[data-runtime-close]");
    check(
      "sandbox panel closes from its in-panel control at 390px",
      !(await page.locator(".runtime-target").evaluate((details) => details.open))
    );
    await page.close();
  }

  // IDE: the complete in-memory sibling-module project, live sibling edit, and
  // explicit restart.
  {
    const page = await context.newPage();
    await page.addInitScript(() => localStorage.removeItem("functor-ide-project-v1"));
    await page.goto(`${BASE}/ide.html`);
    await page.waitForFunction(() => window.__ide?.status().state === "live", {
      timeout: 20_000,
    });

    let cursor = requests.length;
    await openAndPush(page);
    const initial = await nextRequest("/load-project", cursor);
    const files = JSON.parse(initial.body);
    check(
      "IDE first push includes entry and sibling modules",
      files.map(([path]) => path).join(",") === "game.fun,palette.fun"
    );

    cursor = requests.length;
    await page.evaluate(() => window.__ide.openFile("palette.fun"));
    await page.evaluate(() =>
      window.__ide.setActiveSource("let glow = 0.42\nlet sky = 0.24\n")
    );
    const reload = await nextRequest("/reload-project", cursor);
    const changedFiles = JSON.parse(reload.body);
    check(
      "IDE sibling edits hot-reload the complete project",
      changedFiles.find(([path]) => path === "palette.fun")?.[1].includes("0.42")
    );

    cursor = requests.length;
    await page.click("#restart");
    await nextRequest("/load-project", cursor);
    check("IDE restart fresh-loads the linked runtime", true);

    await page.waitForFunction(
      () => document.querySelector("[data-runtime-status]")?.dataset.state === "live",
      { timeout: 10_000 }
    );
    const target = await page.evaluate(() => window.__ide.runtimeTarget());
    check(
      "IDE exposes a live linked-target status",
      target.connected && target.live && target.status === "live",
      JSON.stringify(target)
    );
    await page.close();
  }

  await context.close();
} finally {
  await browser.close();
  siteServer.kill();
  await new Promise((resolve) => runtimeServer.close(resolve));
}

console.log(failures === 0 ? "\nALL CHECKS PASSED" : `\n${failures} CHECK(S) FAILED`);
process.exit(failures === 0 ? 0 : 1);
