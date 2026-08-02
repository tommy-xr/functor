// gamepad wasm e2e: the browser Gamepad API reaches a game's `sampledInput`
// through the web runtime's per-frame poll, with the standard-mapping
// conversion intact (analog trigger value + up-positive stick Y).
//
// The pad is FAKED by overriding `navigator.getGamepads` before the runtime
// boots — headless CI has no controller, and a fake is also what pins exact
// axis values. The fixture game (e2e/fixtures/gamepad) logs one
// "e2e-gamepad" line the first time it samples a pad with south held,
// echoing leftStick and rightTrigger; the test asserts that line and its
// converted values (browser Y -1 → domain +1).
//
// Run manually (needs the built CLI so target/debug/functor embeds the web
// runtime with pad polling):
//
//   npm run build:cli:debug
//   node e2e/gamepad-wasm.mjs
import { spawn } from "node:child_process";
import net from "node:net";
import { fileURLToPath } from "node:url";
import { chromium } from "@playwright/test";

const ROOT = fileURLToPath(new URL("..", import.meta.url));
const PORT = 8080;
const BASE = `http://127.0.0.1:${PORT}`;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

function portInUse() {
  return new Promise((resolve) => {
    const sock = net
      .connect(PORT, "127.0.0.1")
      .on("connect", () => {
        sock.destroy();
        resolve(true);
      })
      .on("error", () => resolve(false));
  });
}

async function waitPortFree(timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (!(await portInUse())) return true;
    await sleep(200);
  }
  return false;
}

async function main() {
  if (!(await waitPortFree(15000))) {
    throw new Error(`:${PORT} is in use — another dev server is running`);
  }
  const server = spawn(
    "./target/debug/functor",
    ["-d", "e2e/fixtures/gamepad", "run", "wasm", "--no-open"],
    { cwd: ROOT, stdio: "ignore" },
  );
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
    page.on("pageerror", (e) => log.push(`pageerror: ${e}`));

    // A standard-mapping pad: south held, left stick pushed right+up (browser
    // Y is down-positive, so up is -1), right trigger analog at 0.25. Installed
    // before any runtime script runs, so the first poll already sees it.
    await page.addInitScript(() => {
      const button = (pressed, value) => ({ pressed, touched: pressed, value });
      const buttons = Array.from({ length: 17 }, () => button(false, 0));
      buttons[0] = button(true, 1); // south
      buttons[7] = button(false, 0.25); // right trigger's analog value
      const pad = {
        id: "e2e-fake-pad",
        index: 0,
        connected: true,
        mapping: "standard",
        timestamp: 0,
        axes: [0.5, -1.0, 0.0, 0.0],
        buttons,
      };
      navigator.getGamepads = () => [pad];
    });

    // Wait for the dev server, then load and let the game sample a few frames.
    const deadline = Date.now() + 30000;
    let up = false;
    while (Date.now() < deadline && !up) {
      up = await portInUse();
      if (!up) await sleep(200);
    }
    if (!up) throw new Error("dev server never came up");
    await page.goto(BASE, { waitUntil: "domcontentloaded" });

    const found = async (pattern) => log.some((line) => pattern.test(line));
    const waitFor = async (pattern, what) => {
      const until = Date.now() + 20000;
      while (Date.now() < until) {
        if (await found(pattern)) return;
        await sleep(200);
      }
      throw new Error(`timed out waiting for ${what}\n--- console ---\n${log.join("\n")}`);
    };

    await waitFor(/\[functor-lang\] loaded/, "the game to load");
    await waitFor(/e2e-gamepad/, "the sampled pad to reach the game");

    const line = log.find((l) => l.includes("e2e-gamepad"));
    const expect = (cond, what) => {
      console.log(`  ${cond ? "✓" : "✗"} ${what}`);
      if (!cond) process.exitCode = 1;
    };
    expect(line.includes("x=0.5"), `stick X crossed raw (${line})`);
    expect(line.includes("y=1"), "browser down-positive Y negated to up-positive");
    expect(line.includes("rt=0.25"), "analog trigger value (not the digital shadow)");

    const functorLangErrors = log.filter((l) => /\[functor-lang\].*error/.test(l));
    expect(functorLangErrors.length === 0, "no [functor-lang] errors during the run");
  } finally {
    await browser.close();
    server.kill("SIGKILL");
    await waitPortFree(10000);
  }
}

main().then(
  () => {
    console.log(process.exitCode ? "✗ gamepad wasm e2e FAILED" : "✓ gamepad wasm e2e passed");
  },
  (err) => {
    console.error(`✗ gamepad wasm e2e FAILED: ${err.message}`);
    process.exitCode = 1;
  },
);
