// Sandbox multiplayer e2e: the SERVER PANE, end to end.
//
// e2e/net-coordinator.mjs proves the routing substrate on a bare fixture page.
// This one proves the PRODUCT: pick "Arena (client/server)" in the sandbox and
// you get a working session in the pane grid — a server pane plus N client
// panes, wired by the same host coordinator, with the clients actually joining
// the server's world.
//
// It asserts:
//
//   1. mp at the default count mounts a server pane beside ONE client (the
//      server is extra at every client count, not a client you gave up);
//   2. #clients=2 grows to server + two clients, and the pill counts them
//      honestly as "2+1 running";
//   3. both clients reach `status: "in-world"` — read from each pane's paused
//      inspector trace, the same path the net-coordinator e2e uses — so the
//      panes are one session, not three lonely sims;
//   4. every pane header shows its coordinator link state;
//   5. the GRID layout puts the clients in two columns with the server as a
//      full-width strip below them — and switching layout reloads no pane;
//   6. switching to a single-role example removes the server pane again.
//
// Run manually (needs the web-runtime wasm bundle):
//
//   wasm-pack build runtime/functor-runtime-web --target=web   # once
//   node e2e/sandbox-mp.mjs
import { spawn, spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { chromium } from "@playwright/test";

const PORT = Number(process.env.FUNCTOR_MP_PORT ?? 8125);
const BASE = `http://127.0.0.1:${PORT}`;
const ROOT = fileURLToPath(new URL("..", import.meta.url));

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

let failures = 0;
const check = (ok, what, detail = "") => {
  console.log(`${ok ? "  ok" : "FAIL"}  ${what}${detail ? ` — ${detail}` : ""}`);
  if (!ok) failures += 1;
};

const build = spawnSync("node", ["site/build.mjs"], { cwd: ROOT, stdio: "inherit" });
if (build.status !== 0) process.exit(build.status ?? 1);

try {
  await fetch(BASE);
  console.error(`port ${PORT} is already in use — kill the process on it first`);
  process.exit(1);
} catch {
  // Nothing listening: good.
}
const server = spawn("node", ["site/serve.mjs", "--port", String(PORT)], {
  cwd: ROOT,
  stdio: "ignore",
});
process.on("exit", () => server.kill());
for (let i = 0; ; i++) {
  try {
    await fetch(BASE);
    break;
  } catch {
    if (i > 50) throw new Error("site server never came up");
    await sleep(200);
  }
}

// The host-page probe: pane roles straight off the rendered chrome, plus the
// paused-inspector relay (a pane publishes its trace when it parks, and every
// entry point binds the model as `m`).
const installProbe = (page) =>
  page.evaluate(() => {
    const traces = new Map();
    const shells = () =>
      [...document.querySelectorAll(".mp-pane")].map((shell, index) => ({
        role: shell.classList.contains("server") ? "server" : `client ${index + 1}`,
        frame: shell.querySelector("iframe"),
        header: shell.querySelector(".mp-pane-hd").innerText.replace(/\s+/g, " ").trim(),
      }));
    window.addEventListener("message", (event) => {
      if (event.data?.type !== "functor-inspector-trace") return;
      const hit = shells().find((s) => s.frame.contentWindow === event.source);
      if (hit) traces.set(hit.role, event.data.trace);
    });
    window.__mpProbe = {
      roles: () => shells().map((s) => s.role),
      headers: () => shells().map((s) => s.header),
      pill: () => document.getElementById("status").textContent.trim(),
      pauseAll: () => document.getElementById("mp-pause").click(),
      layout: () => ({
        view: document.querySelector(".mp-grid").dataset.view,
        pressed: [...document.querySelectorAll(".mp-viewseg button")].map(
          (b) => `${b.id.replace("mp-view-", "")}:${b.getAttribute("aria-pressed")}`
        ),
      }),
      // Laid-out geometry per pane, plus the server's resolved grid placement.
      boxes: () =>
        shells().map((s) => {
          const shell = s.frame.closest(".mp-pane");
          const box = shell.getBoundingClientRect();
          return {
            role: s.role,
            x: Math.round(box.x),
            y: Math.round(box.y),
            w: Math.round(box.width),
            h: Math.round(box.height),
            gridColumn: getComputedStyle(shell).gridColumn,
          };
        }),
      // Document-lifetime markers for EVERY pane: a re-parented iframe reloads
      // into a fresh realm and loses the flag.
      markAll: () => {
        for (const s of shells()) s.frame.contentWindow.__mpAlive = true;
      },
      allAlive: () => shells().every((s) => s.frame.contentWindow?.__mpAlive === true),
      model: (role) => {
        const trace = traces.get(role);
        const invocations = trace?.invocations ?? [];
        const inv = invocations.find((i) => i.entry === "draw") ?? invocations[0];
        return inv?.bindings?.find((b) => b.name === "m")?.value ?? null;
      },
    };
  });

const browser = await chromium.launch();
try {
  // --- 1. One client + a server pane. -----------------------------------------
  {
    const page = await browser.newPage({ viewport: { width: 1440, height: 900 } });
    await page.goto(`${BASE}/sandbox.html?example=mp`);
    await page.waitForFunction(() => window.__sandbox?.status().state === "live", {
      timeout: 40000,
    });
    await installProbe(page);
    const roles = await page.evaluate(() => window.__mpProbe.roles());
    check(
      roles.length === 2 && roles[0] === "client 1" && roles[1] === "server",
      "mp at one client mounts a server pane beside it",
      roles.join(", ")
    );
    const clientsControl = await page.inputValue("#client-count");
    check(clientsControl === "1", "the CLIENTS control counts clients only", clientsControl);

    // Selecting a pane hands it the BROWSER keyboard, not just the chrome —
    // the regression was "⌨ you" showing while WASD still landed in the host
    // page until a second click inside the canvas. A REAL click matters:
    // mousedown's default action re-points focus after the handler, which a
    // synthetic dispatchEvent (no default action) can never catch.
    await page.click(".mp-pane .mp-pane-hd");
    await sleep(150);
    const keyboardOwner = await page.evaluate(() => {
      const active = document.activeElement;
      const shell = document.querySelector(".mp-pane");
      return { isPaneIframe: active === shell?.querySelector("iframe"), tag: active?.tagName };
    });
    check(
      keyboardOwner.isPaneIframe,
      "a real header click gives the pane the real keyboard (activeElement is its iframe)",
      keyboardOwner.tag ?? "null"
    );
    // The picker survives a claimed keyboard: with the pane's iframe focused,
    // `f` (typed INTO the pane's document) must still cycle the layout —
    // the pane documents carry the same picker handler as the host window.
    const viewBefore = await page.evaluate(
      () => document.querySelector(".mp-grid")?.getAttribute("data-view") ?? null
    );
    await page.keyboard.press("f");
    await sleep(200);
    const viewAfter = await page.evaluate(
      () => document.querySelector(".mp-grid")?.getAttribute("data-view") ?? null
    );
    check(
      viewBefore !== null && viewAfter !== null && viewAfter !== viewBefore,
      "picker keys still work while a pane owns the keyboard (f cycled the layout)",
      `${viewBefore} -> ${viewAfter}`
    );
    // Reset the layout so the geometry checks below see the default.
    if (viewAfter !== viewBefore) {
      await page.evaluate(() => document.querySelector("#mp-view-tiled")?.dispatchEvent(new MouseEvent("click", { bubbles: true })));
      await sleep(200);
    }

    // Grid with a LONE client: no peer to match, so it takes the whole row
    // rather than sitting beside an empty cell — the server strip below it.
    await page.click("#mp-view-grid");
    await sleep(400);
    const [client, server] = await page.evaluate(() => window.__mpProbe.boxes());
    check(
      Math.abs(client.w - server.w) < 2 && server.y >= client.y + client.h && server.h < client.h,
      "grid with one client stacks it full-width over the server strip",
      JSON.stringify([client, server])
    );
    await page.close();
  }

  // --- 2–4. Two clients + a server: one joined session. -----------------------
  {
    const page = await browser.newPage({ viewport: { width: 1440, height: 900 } });
    const consoleLog = [];
    page.on("console", (m) => consoleLog.push(m.text()));
    page.on("pageerror", (e) => consoleLog.push(`pageerror: ${e}`));
    await page.goto(`${BASE}/sandbox.html?example=mp#clients=2`);
    await page.waitForFunction(() => window.__sandbox?.status().state === "live", {
      timeout: 40000,
    });
    await installProbe(page);
    const roles = await page.evaluate(() => window.__mpProbe.roles());
    check(
      roles.join(", ") === "client 1, client 2, server",
      "#clients=2 runs two clients and one server, server last",
      roles.join(", ")
    );

    // The link indicator: every pane linked through the coordinator.
    const linked = await page
      .waitForFunction(
        () => window.__mpProbe.headers().every((h) => h.includes("linked")),
        null,
        { timeout: 20000 }
      )
      .then(() => true)
      .catch(() => false);
    const headers = await page.evaluate(() => window.__mpProbe.headers());
    check(linked, "every pane header shows its coordinator link state", headers.join(" | "));
    check(
      headers[2].startsWith("SERVER") && !headers[2].includes("⇅"),
      "the server pane is labelled SERVER in its chrome and carries no link chip",
      headers[2]
    );

    const pill = await page
      .waitForFunction(() => window.__mpProbe.pill().includes("2+1 running"), null, {
        timeout: 20000,
      })
      .then(() => page.evaluate(() => window.__mpProbe.pill()))
      .catch(() => page.evaluate(() => window.__mpProbe.pill()));
    check(pill.includes("2+1 running"), "the pill counts clients and server apart", pill);

    // Both clients joined the SERVER's world — the assertion that makes this a
    // session rather than three independent sims.
    await sleep(2500);
    await page.evaluate(() => window.__mpProbe.pauseAll());
    await sleep(1500);
    const models = await page.evaluate(() => ({
      c1: window.__mpProbe.model("client 1"),
      c2: window.__mpProbe.model("client 2"),
      server: window.__mpProbe.model("server"),
    }));
    check(
      typeof models.c1 === "string" &&
        typeof models.c2 === "string" &&
        models.c1.includes('status: "in-world"') &&
        models.c2.includes('status: "in-world"'),
      "both client panes joined the server pane's world",
      JSON.stringify(models.c1)
    );
    check(
      typeof models.server === "string" && (models.server.match(/cid: /g) ?? []).length === 2,
      "the server pane tracks both clients",
      String(models.server)
    );
    const errors = consoleLog.filter((line) => line.includes("[functor-lang]") && line.includes("error"));
    check(errors.length === 0, "no pane reported a runtime error", errors.slice(0, 3).join(" | "));
    await page.close();
  }

  // --- 4b. A LIVE client-count change leaves the authority running. -----------
  // Re-appending a mounted iframe reloads it, so growing the client set must
  // insert the new tile BEFORE the server's rather than moving the server to
  // the end — otherwise the world (and every client's join) is wiped whenever
  // someone changes the count.
  {
    const page = await browser.newPage({ viewport: { width: 1440, height: 900 } });
    await page.goto(`${BASE}/sandbox.html?example=mp`);
    await page.waitForFunction(() => window.__sandbox?.status().state === "live", {
      timeout: 40000,
    });
    await installProbe(page);
    await page.evaluate(() => {
      const serverWindow = () =>
        document.querySelector(".mp-pane.server")?.querySelector("iframe").contentWindow ?? null;
      window.__mpProbe.serverFrame = () => serverWindow()?.__scrub?.frame() ?? null;
      // A document-lifetime marker: a reloaded iframe gets a fresh realm and
      // loses it, which a frame counter cannot tell you (a rebooted pane
      // counts back up past whatever the old one had reached).
      window.__mpProbe.markServer = () => {
        const win = serverWindow();
        if (win) win.__mpAlive = true;
      };
      window.__mpProbe.serverAlive = () => serverWindow()?.__mpAlive === true;
    });
    await page.waitForFunction(() => window.__mpProbe.serverFrame() > 60, { timeout: 30000 });
    const before = await page.evaluate(() => {
      window.__mpProbe.markServer();
      return window.__mpProbe.serverFrame();
    });
    await page.selectOption("#client-count", "2");
    await sleep(3000);
    const roles = await page.evaluate(() => window.__mpProbe.roles());
    const state = await page.evaluate(() => ({
      alive: window.__mpProbe.serverAlive(),
      frame: window.__mpProbe.serverFrame(),
    }));
    check(
      roles.join(", ") === "client 1, client 2, server" && state.alive && state.frame > before,
      "growing the client set live keeps the server pane running (no reload)",
      `${roles.join(", ")}; alive=${state.alive}; server frame ${before} -> ${state.frame}`
    );
    await page.close();
  }

  // --- 4c. GRID layout: client cells over a server strip, no pane reloaded. ---
  // Switching layout must be a pure CSS/class change: reordering or re-parenting
  // a pane's node would reload its iframe and wipe the model, so the same
  // document-lifetime marker 4b uses on the server is applied to EVERY pane here.
  {
    const page = await browser.newPage({ viewport: { width: 1600, height: 1000 } });
    await page.goto(`${BASE}/sandbox.html?example=mp#clients=2`);
    await page.waitForFunction(() => window.__sandbox?.status().state === "live", {
      timeout: 40000,
    });
    await installProbe(page);
    const tiled = await page.evaluate(() => {
      window.__mpProbe.markAll();
      return window.__mpProbe.layout();
    });
    check(
      tiled.view === "tiled" && tiled.pressed.join(" ") === "tiled:true grid:false tabs:false",
      "the layout control starts on TILED with three options",
      `${tiled.view} — ${tiled.pressed.join(" ")}`
    );

    await page.click("#mp-view-grid");
    await sleep(400);
    const grid = await page.evaluate(() => window.__mpProbe.layout());
    check(
      grid.view === "grid" && grid.pressed.join(" ") === "tiled:false grid:true tabs:false",
      "clicking GRID switches the pane grid's layout",
      `${grid.view} — ${grid.pressed.join(" ")}`
    );

    const boxes = await page.evaluate(() => window.__mpProbe.boxes());
    const [c1, c2, server] = boxes;
    check(
      Math.abs(c1.y - c2.y) < 2 && c2.x > c1.x && Math.abs(c1.w - c2.w) < 2,
      "the two clients sit side by side in equal columns",
      JSON.stringify([c1, c2])
    );
    check(
      server.gridColumn === "1 / -1" && server.w > c1.w * 1.8 && server.y >= c1.y + c1.h,
      "the server pane spans every column as a strip below the clients",
      JSON.stringify(server)
    );
    check(
      server.h < c1.h,
      "the server strip is shorter than a client cell",
      `${server.h} vs ${c1.h}`
    );
    check(
      await page.evaluate(() => window.__mpProbe.allAlive()),
      "no pane's iframe was reloaded by the switch to grid"
    );

    // ...and back. Tiled is one row again, still the same three documents.
    await page.click("#mp-view-tiled");
    await sleep(400);
    const back = await page.evaluate(() => window.__mpProbe.layout());
    const backBoxes = await page.evaluate(() => window.__mpProbe.boxes());
    check(
      back.view === "tiled" &&
        back.pressed.join(" ") === "tiled:true grid:false tabs:false" &&
        backBoxes.every((b) => Math.abs(b.y - backBoxes[0].y) < 2),
      "switching back to TILED restores the single row",
      `${back.view} — ${JSON.stringify(backBoxes.map((b) => b.y))}`
    );
    check(
      await page.evaluate(() => window.__mpProbe.allAlive()),
      "no pane's iframe was reloaded by the switch back to tiled"
    );

    // `f` cycles the three layouts in the segmented control's order.
    const cycled = [];
    for (let i = 0; i < 3; i++) {
      await page.keyboard.press("f");
      await sleep(250);
      cycled.push(await page.evaluate(() => window.__mpProbe.layout().view));
    }
    check(
      cycled.join(" → ") === "grid → tabs → tiled",
      "f cycles tiled → grid → tabs → tiled",
      cycled.join(" → ")
    );
    check(
      await page.evaluate(() => window.__mpProbe.allAlive()),
      "no pane's iframe was reloaded by the f cycle"
    );

    // A third client makes the odd row: it keeps its half-width cell (its peers
    // are right above it, and the server strip still closes the layout below).
    await page.selectOption("#client-count", "3");
    await sleep(2500);
    await page.click("#mp-view-grid");
    await sleep(400);
    const three = await page.evaluate(() => window.__mpProbe.boxes());
    check(
      three.length === 4 &&
        Math.abs(three[2].w - three[0].w) < 2 &&
        three[2].x === three[0].x &&
        three[2].y >= three[0].y + three[0].h &&
        three[3].w > three[2].w * 1.8,
      "a third client keeps its half-width cell above the full-width strip",
      JSON.stringify(three.map((b) => [b.role, b.x, b.y, b.w, b.h]))
    );
    await page.close();
  }

  // --- 5. Switching away drops the server pane. -------------------------------
  {
    const page = await browser.newPage({ viewport: { width: 1440, height: 900 } });
    await page.goto(`${BASE}/sandbox.html?example=mp`);
    await page.waitForFunction(() => window.__sandbox?.status().state === "live", {
      timeout: 40000,
    });
    await installProbe(page);
    await page.selectOption("#example-picker", "counter");
    await page.waitForFunction(() => window.__sandbox?.status().state === "live", {
      timeout: 40000,
    });
    const gone = await page
      .waitForFunction(() => window.__mpProbe.roles().length === 1, null, { timeout: 10000 })
      .then(() => true)
      .catch(() => false);
    const roles = await page.evaluate(() => window.__mpProbe.roles());
    check(gone, "switching to a single-role example removes the server pane", roles.join(", "));

    // Grid with no authority: nothing closes the bottom row, so the odd last
    // client stretches rather than leaving a whole empty quadrant.
    await page.selectOption("#client-count", "3");
    await sleep(2500);
    await page.click("#mp-view-grid");
    await sleep(400);
    const solo = await page.evaluate(() => window.__mpProbe.boxes());
    check(
      solo.length === 3 && solo[2].w > solo[0].w * 1.8 && solo[2].y >= solo[0].y + solo[0].h,
      "in a single-role example the odd last client spans the bottom row",
      JSON.stringify(solo.map((b) => [b.role, b.x, b.y, b.w, b.h]))
    );
    await page.close();
  }
} finally {
  await browser.close();
  server.kill();
}

console.log(failures === 0 ? "\nALL CHECKS PASSED" : `\n${failures} CHECK(S) FAILED`);
process.exit(failures === 0 ? 0 : 1);
