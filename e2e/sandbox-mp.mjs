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
//   6. switching to a single-role example removes the server pane again;
//   7. every pane CARD letterboxes to a 16:9 body inside its cell in the grid
//      and network views, the "+" seat is last in reading order, and the
//      network wires anchor to the cards rather than to the cells;
//   8. ORBS — the same-file sample — boots every pane of the ONE file at its
//      own inline module (`?module=Client` / `?module=Server`), and its real
//      wire carries packets both ways between the blocks.
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

/**
 * Wait for the view transition to finish. Switching layout FLIPs the cards
 * (mp-panes.ts, 420ms), and a transform moves a card's measured box without
 * resizing anything — so geometry read mid-flight is the card's *animated*
 * box, not its laid-out one. The "+" seat FLIPs alongside the cards and its
 * band geometry is asserted too, so it has to be waited on as well.
 */
const settle = (page) =>
  page.waitForFunction(
    () =>
      [...document.querySelectorAll(".mp-pane, .mp-add-tile")].every(
        (node) => node.getAnimations().length === 0
      ),
    null,
    { timeout: 5000 }
  );

/** A letterboxed card sits centred on an axis when its two margins match. */
const centred = (b, axis, size) =>
  Math.abs(b[axis] - b.cell[axis] - (b.cell[axis] + b.cell[size] - (b[axis] + b[size]))) <= 2;

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
      // Each pane's player URL — the seam a same-file sample's ROLE travels
      // through (`?module=Client` / `?module=Server`).
      srcs: () => shells().map((s) => s.frame.getAttribute("src")),
      // The link indicator, read as STATE rather than as prose: the dot is what
      // survives when a thumbnail header clips the word off (see headerParts).
      links: () =>
        [...document.querySelectorAll(".mp-pane .mp-conn")].map(
          (conn) => `${conn.hidden ? "hidden" : conn.dataset.linked}:${conn.title.slice(0, 20)}`
        ),
      pill: () => document.getElementById("status").textContent.trim(),
      /** The chrono rail's own label — reference-clock frames, the axis the
       * packet log is keyed to. */
      railFrame: () => document.getElementById("mp-frame").textContent.trim(),
      /** The pinned wire log: what it says it is showing, and what it shows. */
      wireLog: () => {
        const panel = document.querySelector(".mp-wirelog");
        const selected = document.querySelectorAll(".mp-wire.selected").length;
        if (!panel || panel.hidden) return { pinned: false, rows: [], selected };
        const box = panel.getBoundingClientRect();
        return {
          pinned: true,
          selected,
          link: panel.querySelector(".mp-wl-link").textContent.trim(),
          footer: panel.querySelector(".mp-wl-ft").textContent.trim(),
          tethered: !document.querySelector(".mp-tether").hasAttribute("visibility"),
          box: {
            x: Math.round(box.x),
            y: Math.round(box.y),
            w: Math.round(box.width),
            h: Math.round(box.height),
          },
          rows: [...panel.querySelectorAll(".mp-wl")].map((row) => ({
            frame: Number((row.querySelector(".f").textContent.match(/-?\d+/) ?? [NaN])[0]),
            dir: row.querySelector(".d").textContent.trim(),
            payload: row.querySelector(".payload").textContent.trim(),
            bytes: row.querySelector(".n").textContent.trim(),
            at: row.classList.contains("at"),
          })),
        };
      },
      clickEdge: (index) => document.querySelectorAll(".mp-edge-chip")[index].click(),
      pauseAll: () => document.getElementById("mp-pause").click(),
      layout: () => ({
        view: document.querySelector(".mp-grid").dataset.view,
        pressed: [...document.querySelectorAll(".mp-viewseg button")].map(
          (b) => `${b.id.replace("mp-view-", "")}:${b.getAttribute("aria-pressed")}`
        ),
      }),
      // The NETWORK view's graph layer: one measured wire per client↔server
      // link, its mid-edge chip, and the packets currently in flight.
      network: () => {
        const wires = [...document.querySelectorAll(".mp-wire")];
        return {
          wires: wires.length,
          drawn: wires.filter((w) => (w.getAttribute("d") ?? "").length > 0).length,
          lengths: wires.map((w) => Math.round(w.getTotalLength())),
          chips: [...document.querySelectorAll(".mp-edge-chip")].map((c) =>
            c.textContent.trim()
          ),
          dots: document.querySelectorAll(".mp-packet").length,
          replay: document.querySelectorAll(".mp-packet.replay").length,
          live: document.querySelectorAll(".mp-packet:not(.replay)").length,
          up: document.querySelectorAll(".mp-packet.up").length,
          down: document.querySelectorAll(".mp-packet.down").length,
          // The legend + demoted pane readouts now live on the status bar's
          // view strip: the legend only in network, the readouts in every
          // multi-pane view (the headers clip by their own width).
          legend: getComputedStyle(document.querySelector(".mp-vs-legend")).display,
          stripShown: !document.querySelector(".statusbar-view").hidden,
          strip: document.querySelector(".mp-vs-panes").innerText.replace(/\s+/g, " ").trim(),
          networkDisabled: document.getElementById("mp-view-network").disabled,
          networkTitle: document.getElementById("mp-view-network").title,
        };
      },
      /**
       * Each wire's endpoints in PAGE coordinates, so they can be compared with
       * the pane boxes: a wire has to leave the letterboxed CARD, not the cell
       * the card floats in (Addendum 5b.3).
       */
      wireEnds: () => {
        const origin = document.querySelector(".mp-net-layer").getBoundingClientRect();
        return [...document.querySelectorAll(".mp-wire")].map((wire) => {
          const at = (length) => {
            const point = wire.getPointAtLength(length);
            return { x: point.x + origin.x, y: point.y + origin.y };
          };
          return { from: at(0), to: at(wire.getTotalLength()) };
        });
      },
      // What a pane header still shows at its current width: the priority clip
      // keeps identity + link state and drops the frame counter first.
      headerParts: (index) => {
        const shell = [...document.querySelectorAll(".mp-pane")][index];
        const shown = (selector) => {
          const node = shell.querySelector(selector);
          return !!node && getComputedStyle(node).display !== "none";
        };
        return {
          width: Math.round(shell.getBoundingClientRect().width),
          digit: shown(".mp-digit"),
          dot: shown(".mp-conn-d"),
          word: shown(".mp-conn-t"),
          frame: shown(".mp-pf"),
        };
      },
      // The "+" affordances (tile in tiled/grid, tab in tabs).
      addTile: () => {
        const tile = document.querySelector(".mp-add-tile");
        return {
          present: !!tile,
          hidden: !tile || tile.hidden,
          display: tile ? getComputedStyle(tile).display : "none",
          tag: tile?.tagName,
          // Addendum 5b.1: the seat is the LAST thing in the arrangement.
          last: !!tile && tile.parentElement.lastElementChild === tile,
          tabDisplay: getComputedStyle(document.querySelector(".mp-tab.add")).display,
        };
      },
      clickAddTile: () => document.querySelector(".mp-add-tile").click(),
      // Click a view button and read back, IN THE SAME TASK, how many pane
      // cards are mid-animation — the FLIP has to have started before the
      // browser has painted anything.
      switchAndCountAnimations: (id) => {
        document.getElementById(id).click();
        return [...document.querySelectorAll(".mp-pane")].reduce(
          (total, shell) => total + shell.getAnimations().length,
          0
        );
      },
      // Laid-out geometry per pane: the CARD (the letterboxed pane itself, and
      // what the wires anchor to), the CELL it sits in (what the layout
      // places), and the body's aspect — 16:9 wherever the view letterboxes.
      boxes: () =>
        shells().map((s) => {
          const shell = s.frame.closest(".mp-pane");
          const cell = shell.closest(".mp-cell");
          const box = shell.getBoundingClientRect();
          const cellBox = cell.getBoundingClientRect();
          const body = shell.querySelector(".mp-pane-body").getBoundingClientRect();
          return {
            role: s.role,
            x: Math.round(box.x),
            y: Math.round(box.y),
            w: Math.round(box.width),
            h: Math.round(box.height),
            cell: {
              x: Math.round(cellBox.x),
              y: Math.round(cellBox.y),
              w: Math.round(cellBox.width),
              h: Math.round(cellBox.height),
            },
            bodyRatio: body.width / body.height,
            /** How much of the cell the card left over, UNROUNDED: the
             * letterbox arithmetic is exact to the pixel, and rounding would
             * hide a card that overflows its cell by a fraction of one. */
            slack: { w: cellBox.width - box.width, h: cellBox.height - box.height },
            /** What the header actually needs, against the 22px the letterbox
             * arithmetic budgets for it. */
            headerScroll: shell.querySelector(".mp-pane-hd").scrollHeight,
            gridColumn: getComputedStyle(cell).gridColumn,
          };
        }),
      // Document-lifetime markers for EVERY pane: a re-parented iframe reloads
      // into a fresh realm and loses the flag.
      markAll: () => {
        for (const s of shells()) s.frame.contentWindow.__mpAlive = true;
      },
      allAlive: () => shells().every((s) => s.frame.contentWindow?.__mpAlive === true),
      // A keydown/keyup straight into a pane's window — the mp client only
      // sends to the server when its input changes, so client → server traffic
      // has to be provoked.
      key: (role, code, down) => {
        const win = shells().find((s) => s.role === role)?.frame.contentWindow;
        if (!win) return false;
        win.dispatchEvent(new win.KeyboardEvent(down ? "keydown" : "keyup", { code }));
        return true;
      },
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

    // The "+" tile: the spatial way to add a client, same path as the dropdown.
    const tile = await page.evaluate(() => window.__mpProbe.addTile());
    check(
      tile.present && !tile.hidden && tile.display !== "none" && tile.tag === "BUTTON",
      "a + tile sits after the client panes, as a real button",
      JSON.stringify(tile)
    );
    check(
      tile.last,
      "the seat is LAST in reading order, after every occupied cell"
    );
    await page.evaluate(() => window.__mpProbe.clickAddTile());
    await sleep(2500);
    const afterAdd = await page.evaluate(() => ({
      roles: window.__mpProbe.roles(),
      hash: window.location.hash,
      control: document.getElementById("client-count").value,
    }));
    check(
      afterAdd.roles.join(", ") === "client 1, client 2, server" &&
        afterAdd.control === "2" &&
        afterAdd.hash.includes("clients=2"),
      "clicking + adds a client through the same path as the CLIENTS dropdown",
      JSON.stringify(afterAdd)
    );
    await page.selectOption("#client-count", "3");
    await sleep(2500);
    const atMax = await page.evaluate(() => window.__mpProbe.addTile());
    // The ATTRIBUTE is not the picture: the seat sets its own `display`, which
    // outranks the UA's `[hidden]` rule — so assert what is painted.
    check(
      atMax.hidden && atMax.display === "none",
      "the + tile is hidden at MAX_CLIENTS",
      JSON.stringify(atMax)
    );
    await page.selectOption("#client-count", "1");
    await sleep(1500);

    // Grid with a LONE client: the client takes the whole row (no peer to match)
    // and the open seat closes the layout as a band UNDER everything — never
    // between the client and the authority (Addendum 5b.1).
    await page.click("#mp-view-grid");
    await settle(page);
    const [client, server] = await page.evaluate(() => window.__mpProbe.boxes());
    const seat = await page.evaluate(() => {
      const box = document.querySelector(".mp-add-tile").getBoundingClientRect();
      return { x: Math.round(box.x), y: Math.round(box.y), w: Math.round(box.width), h: Math.round(box.height) };
    });
    check(
      seat.y >= server.y + server.h &&
        Math.abs(seat.w - client.cell.w) < 2 &&
        seat.h < client.h &&
        server.gridColumn === "1 / -1" &&
        server.y >= client.y + client.h &&
        server.h < client.h,
      "grid closes with the open seat as a band under the client and the strip",
      JSON.stringify([client, seat, server])
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
        () => window.__mpProbe.links().every((state) => state.startsWith("true")),
        null,
        { timeout: 20000 }
      )
      .then(() => true)
      .catch(() => false);
    const headers = await page.evaluate(() => window.__mpProbe.headers());
    check(
      linked,
      "every pane header shows its coordinator link state",
      (await page.evaluate(() => window.__mpProbe.links())).join(" | ")
    );
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
      tiled.view === "tiled" &&
        tiled.pressed.join(" ") === "tiled:true grid:false network:false tabs:false",
      "the layout control starts on TILED with four options",
      `${tiled.view} — ${tiled.pressed.join(" ")}`
    );

    await page.click("#mp-view-grid");
    await sleep(400);
    const grid = await page.evaluate(() => window.__mpProbe.layout());
    check(
      grid.view === "grid" &&
        grid.pressed.join(" ") === "tiled:false grid:true network:false tabs:false",
      "clicking GRID switches the pane grid's layout",
      `${grid.view} — ${grid.pressed.join(" ")}`
    );

    await settle(page);
    const boxes = await page.evaluate(() => window.__mpProbe.boxes());
    const [c1, c2, server] = boxes;
    check(
      Math.abs(c1.y - c2.y) < 2 && c2.x > c1.x && Math.abs(c1.w - c2.w) < 2,
      "the two clients sit side by side in equal columns",
      JSON.stringify([c1, c2])
    );
    check(
      server.gridColumn === "1 / -1" &&
        server.cell.w > c1.cell.w * 1.8 &&
        server.y >= c1.y + c1.h,
      "the server pane spans every column as a strip below the clients",
      JSON.stringify(server)
    );
    check(
      server.h < c1.h,
      "the server strip is shorter than a client cell",
      `${server.h} vs ${c1.h}`
    );
    // Addendum 5b.2: every card in GRID letterboxes its body to 16:9 inside
    // its cell — a game window reads as a game window rather
    // than as whatever shape the track handed it. The card fitting INSIDE its
    // cell is asserted exactly (`<=`, unrounded slack): the arithmetic budgets
    // for the header row AND the card's own borders, and a card a fraction
    // taller than its cell is that budget silently going wrong.
    check(
      boxes.every((b) => Math.abs(b.bodyRatio - 16 / 9) < 0.02) &&
        boxes.every(
          (b) => b.slack.w >= 0 && b.slack.h >= 0 && centred(b, "x", "w")
        ) &&
        boxes.some((b) => b.slack.w > 8 || b.slack.h > 8) &&
        // The pinned header row the letterbox arithmetic subtracts: if the
        // header ever needs more than that, the cards silently outgrow their
        // cells — so assert the budget rather than only its symptom.
        boxes.every((b) => b.headerScroll <= 22),
      "every grid card letterboxes to a 16:9 body, centred across its cell",
      JSON.stringify(boxes.map((b) => [b.role, +b.bodyRatio.toFixed(3), b.w, b.h, b.cell.w, b.cell.h]))
    );
    // Vertically the slack goes OUTWARD, not between the players and the
    // authority: the client cards hug the foot of their cells, the strip the
    // head of its band, so the two meet.
    check(
      Math.abs(c1.y + c1.h - (c1.cell.y + c1.cell.h)) <= 2 &&
        Math.abs(server.y - server.cell.y) <= 2,
      "the clients hug the foot of their cells and the strip the head of its band",
      JSON.stringify([c1, server].map((b) => [b.role, b.y, b.h, b.cell.y, b.cell.h]))
    );
    check(
      await page.evaluate(() => window.__mpProbe.allAlive()),
      "no pane's iframe was reloaded by the switch to grid"
    );

    // ...and back. Tiled is one row again, still the same three documents.
    await page.click("#mp-view-tiled");
    await sleep(400);
    const back = await page.evaluate(() => window.__mpProbe.layout());
    await settle(page);
    const backBoxes = await page.evaluate(() => window.__mpProbe.boxes());
    check(
      back.view === "tiled" &&
        back.pressed.join(" ") === "tiled:true grid:false network:false tabs:false" &&
        backBoxes.every((b) => Math.abs(b.y - backBoxes[0].y) < 2),
      "switching back to TILED restores the single row",
      `${back.view} — ${JSON.stringify(backBoxes.map((b) => b.y))}`
    );
    check(
      await page.evaluate(() => window.__mpProbe.allAlive()),
      "no pane's iframe was reloaded by the switch back to tiled"
    );

    // FLIP: the switch ANIMATES the pane cards (Web Animations on the same
    // nodes — never a re-parent), and the document-lifetime markers have to
    // survive an animated switch exactly as they survive an instant one.
    const running = await page.evaluate(() =>
      window.__mpProbe.switchAndCountAnimations("mp-view-network")
    );
    check(running > 0, "switching view animates the pane cards in place (FLIP)", `${running} animations`);
    check(
      await page.evaluate(() => window.__mpProbe.allAlive()),
      "no pane's iframe was reloaded mid-transition"
    );
    await sleep(700);
    check(
      await page.evaluate(
        () =>
          window.__mpProbe.allAlive() &&
          [...document.querySelectorAll(".mp-pane")].every(
            (shell) => getComputedStyle(shell).transform === "none"
          )
      ),
      "the transition settles with every pane still alive and untransformed"
    );
    await page.click("#mp-view-tiled");
    await sleep(700);

    // `f` cycles the four layouts in the segmented control's order.
    const cycled = [];
    for (let i = 0; i < 4; i++) {
      await page.keyboard.press("f");
      await sleep(250);
      cycled.push(await page.evaluate(() => window.__mpProbe.layout().view));
    }
    check(
      cycled.join(" → ") === "grid → network → tabs → tiled",
      "f cycles tiled → grid → network → tabs → tiled",
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
    await settle(page);
    const three = await page.evaluate(() => window.__mpProbe.boxes());
    check(
      three.length === 4 &&
        Math.abs(three[2].w - three[0].w) < 2 &&
        three[2].x === three[0].x &&
        three[2].y >= three[0].y + three[0].h &&
        three[3].cell.w > three[2].cell.w * 1.8,
      "a third client keeps its half-width cell above the full-width strip",
      JSON.stringify(three.map((b) => [b.role, b.x, b.y, b.w, b.h]))
    );
    await page.close();
  }

  // --- 4d. NETWORK layout: the hub, its measured wires, and live packets. -----
  // The graph is drawn from MEASURED pane boxes, so the assertions here are
  // about geometry (who sits where, how many wires exist and that each one has
  // a real length) rather than about hard-coded coordinates.
  {
    const page = await browser.newPage({ viewport: { width: 1600, height: 1000 } });
    await page.goto(`${BASE}/sandbox.html?example=mp#clients=2`);
    await page.waitForFunction(() => window.__sandbox?.status().state === "live", {
      timeout: 40000,
    });
    await installProbe(page);
    await page.evaluate(() => window.__mpProbe.markAll());

    await page.click("#mp-view-network");
    await sleep(600);
    const layout = await page.evaluate(() => window.__mpProbe.layout());
    check(
      layout.view === "network" &&
        layout.pressed.join(" ") === "tiled:false grid:false network:true tabs:false",
      "clicking NETWORK switches the pane grid's layout",
      `${layout.view} — ${layout.pressed.join(" ")}`
    );

    await settle(page);
    const [c1, c2, server] = await page.evaluate(() => window.__mpProbe.boxes());
    check(
      c1.x + c1.w <= server.x + 2 &&
        c2.x >= server.x + server.w - 2 &&
        server.h < c1.h &&
        server.w < c1.w,
      "the server is a compact hub between its two clients — the smallest card",
      JSON.stringify([c1, server, c2].map((b) => [b.role, b.x, b.y, b.w, b.h]))
    );

    // Addendum 5b.2/5b.3: the clients are 16:9 cards flanking the hub, not
    // full-height towers, and the hub is still the smallest card of the three.
    const netBoxes = [c1, c2, server];
    check(
      netBoxes.every((b) => Math.abs(b.bodyRatio - 16 / 9) < 0.02) &&
        netBoxes.every(
          (b) =>
            b.slack.h > 8 &&
            b.slack.w >= 0 &&
            centred(b, "x", "w") &&
            centred(b, "y", "h") &&
            b.headerScroll <= 22
        ),
      "every network card letterboxes to a 16:9 body, centred in its cell",
      JSON.stringify(netBoxes.map((b) => [b.role, +b.bodyRatio.toFixed(3), b.w, b.h, b.cell.w, b.cell.h]))
    );
    check(
      Math.abs(c1.y - c2.y) > c1.h / 2,
      "the two clients are staggered, so no wire shares a centreline",
      `${c1.y} vs ${c2.y} (card height ${c1.h})`
    );

    const graph = await page.evaluate(() => window.__mpProbe.network());
    check(
      graph.wires === 2 && graph.drawn === 2 && graph.lengths.every((l) => l > 50),
      "one measured wire per client↔server link",
      JSON.stringify(graph.lengths)
    );

    // The wires anchor to the LETTERBOXED CARD, never to the cell it floats in:
    // every endpoint has to sit on the edge of some card's box, and a card is
    // strictly inside its cell here, so an endpoint on a cell edge is a miss.
    const ends = await page.evaluate(() => window.__mpProbe.wireEnds());
    const onEdge = (point, box) => {
      const inX = point.x >= box.x - 2 && point.x <= box.x + box.w + 2;
      const inY = point.y >= box.y - 2 && point.y <= box.y + box.h + 2;
      const atX = Math.abs(point.x - box.x) <= 2 || Math.abs(point.x - (box.x + box.w)) <= 2;
      const atY = Math.abs(point.y - box.y) <= 2 || Math.abs(point.y - (box.y + box.h)) <= 2;
      return (inX && inY && (atX || atY));
    };
    // Topology as well as anchoring: each wire runs between the HUB's card and
    // ONE client's, and every client is served exactly once — so a wire that
    // happened to touch the right kind of edge on the wrong pane still fails.
    const anchoredTo = (point) =>
      onEdge(point, server) ? "server" : [c1, c2].findIndex((b) => onEdge(point, b));
    const served = ends.map((wire) => {
      const from = anchoredTo(wire.from);
      const to = anchoredTo(wire.to);
      if (from === "server" && typeof to === "number" && to >= 0) return to;
      if (to === "server" && typeof from === "number" && from >= 0) return from;
      return -1;
    });
    const anchored = served.sort().join() === "0,1";
    // The cards take their cell's WIDTH here and letterbox on height, so a
    // side departure is the same point either way — the endpoint that tells
    // card-anchored from cell-anchored apart is client 2's, which leaves the
    // TOP of a card sitting low in a tall cell. Off the cell it would be a
    // hundred-odd pixels higher, in the dead space above the card.
    const fromCardTop = ends.some((wire) =>
      [wire.from, wire.to].some(
        (point) =>
          Math.abs(point.y - c2.y) <= 2 &&
          point.x >= c2.x - 2 &&
          point.x <= c2.x + c2.w + 2 &&
          c2.y - c2.cell.y > 8
      )
    );
    check(
      ends.length === 2 && anchored && fromCardTop,
      "each wire leaves a card's own edge, not its cell's",
      JSON.stringify(ends.map((w) => [Math.round(w.from.x), Math.round(w.from.y), Math.round(w.to.x), Math.round(w.to.y)]))
    );
    check(
      graph.chips.length === 2 && graph.chips.every((c) => c.includes("Wi-Fi")),
      "each edge carries its client's link chip mid-wire",
      graph.chips.join(" | ")
    );
    check(
      graph.legend === "flex" && /1\s*#f/.test(graph.strip) && /SRV\s*#f/.test(graph.strip),
      "the network status strip carries the legend and every pane's frame",
      `${graph.legend} — "${graph.strip}"`
    );

    // Header priority at thumbnail width: identity + link state survive, the
    // frame counter (now on the strip) is the first thing dropped.
    const header = await page.evaluate(() => window.__mpProbe.headerParts(0));
    check(
      header.digit && header.dot && !header.frame,
      "a thumbnail pane header keeps its digit and link dot, drops the frame counter",
      JSON.stringify(header)
    );

    // Packets: the coordinator's live feed, sampled onto the wires. Both
    // directions must appear — the shapes are what carry direction.
    let flight = await page.evaluate(() => window.__mpProbe.network());
    let flowing = false;
    for (let i = 0; i < 40 && !flowing; i++) {
      // Drive client 1 so it has intent to send; a dot lives 600ms, so sample
      // faster than that.
      await page.evaluate((down) => window.__mpProbe.key("client 1", "KeyW", down), i % 2 === 0);
      await sleep(150);
      flight = await page.evaluate(() => window.__mpProbe.network());
      flowing = flight.up > 0 && flight.down > 0;
    }
    check(
      flowing,
      "packets fly both ways along the wires (circles up, snapshots down)",
      `up=${flight.up} down=${flight.down}`
    );
    check(
      flight.dots <= 260,
      "concurrent dots stay under the cap",
      String(flight.dots)
    );
    check(
      await page.evaluate(() => window.__mpProbe.allAlive()),
      "no pane's iframe was reloaded by the switch to network"
    );

    // Leaving the view is inert again: nothing keeps flying off-screen.
    await page.click("#mp-view-tiled");
    await sleep(400);
    const parked = await page.evaluate(() => window.__mpProbe.network());
    check(
      parked.dots === 0 && parked.legend === "none",
      "leaving network clears the dots and hides the legend",
      `${parked.dots} dots, legend ${parked.legend}`
    );
    // ...but the pane readouts stay: TILED at four cells clips its headers too,
    // so the frame counters still need somewhere to live.
    check(
      parked.stripShown && /1\s*#f/.test(parked.strip),
      "the pane readouts stay on the strip outside the network view",
      `shown=${parked.stripShown} "${parked.strip}"`
    );

    // Three clients: two flanking the hub, the third filling the quadrant they
    // leave open — at a PLAYER's size, since the channel belongs to the hub.
    await page.click("#mp-view-network");
    await page.selectOption("#client-count", "3");
    await sleep(2500);
    await settle(page);
    const three = await page.evaluate(() => window.__mpProbe.boxes());
    const graph3 = await page.evaluate(() => window.__mpProbe.network());
    const hub = three[3];
    check(
      three.length === 4 &&
        three[0].x < hub.x &&
        three[1].x > hub.x &&
        // Upper right: right of the hub, above client 2 — and clients 1 and 2
        // have not moved to make room for it.
        three[2].x > hub.x &&
        three[2].y + three[2].h <= three[1].y + 2 &&
        graph3.wires === 3 &&
        // Every client is a player-sized window; the hub stays the smallest.
        three.slice(0, 3).every((client) => hub.w < client.w && hub.h < client.h),
      "a third client fills the open quadrant at a player's size, with its own wire",
      `${JSON.stringify(three.map((b) => [b.role, b.x, b.y]))} — ${graph3.wires} wires`
    );
    await page.close();
  }

  // --- 4e. A LATE-JOINED client scrubs with everyone else. --------------------
  // Pane frame indices are boot-relative, so a client added mid-session counts
  // from 0 while the rest are in the hundreds. A broadcast seek sent as a bare
  // number therefore asks that client for a frame it has never reached — it
  // clamps to the end of its recording and sits still while everyone else
  // rewinds. The host measures each pane's OFFSET against the reference clock
  // (the server pane) and translates every seek through it.
  //
  // The assertion is a differential one, and it needs no knowledge of absolute
  // frames: park the session, seek back one second on the rail, and every pane
  // must move back one second OF ITS OWN CLOCK — and the late client's offset
  // to the server must be exactly what it was before the seek (the panes still
  // correspond). Untranslated, the late client's delta is ~0: it is asked for a
  // frame past its own end. The measured offset is asserted non-trivial first,
  // so a session that happened to boot in lockstep cannot pass vacuously.
  {
    const page = await browser.newPage({ viewport: { width: 1440, height: 900 } });
    await page.goto(`${BASE}/sandbox.html?example=mp`);
    await page.waitForFunction(() => window.__sandbox?.status().state === "live", {
      timeout: 40000,
    });
    await installProbe(page);
    await page.evaluate(() => {
      // Every pane's frame, sampled in ONE task — the same rule the host's own
      // measurement follows.
      window.__mpProbe.frames = () =>
        [...document.querySelectorAll(".mp-pane")].map((shell) => {
          const win = shell.querySelector("iframe").contentWindow;
          return {
            role: shell.classList.contains("server") ? "server" : "client",
            frame: win?.__scrub?.frame() ?? null,
          };
        });
    });
    // Let the session get a few seconds ahead, THEN add the late client.
    await page.waitForFunction(() => (window.__mpProbe.frames()[1]?.frame ?? 0) > 180, {
      timeout: 30000,
    });
    await page.selectOption("#client-count", "2");
    await sleep(4000);
    const joined = await page.evaluate(() => window.__mpProbe.frames());
    const [c1, c2, srv] = joined.map((p) => p.frame);
    const offset = c2 - srv;
    check(
      joined.length === 3 && offset < -60,
      "the late-joined client's clock really is offset from the reference",
      `client 1 #${c1}, client 2 #${c2}, server #${srv} → offset ${offset}`
    );

    await page.evaluate(() => window.__mpProbe.pauseAll());
    await sleep(1200);
    const before = (await page.evaluate(() => window.__mpProbe.frames())).map((p) => p.frame);
    // The product path: focus the rail's playhead and page back one second.
    await page.focus("#mp-playhead");
    await page.keyboard.press("PageDown");
    await sleep(1500);
    const after = (await page.evaluate(() => window.__mpProbe.frames())).map((p) => p.frame);
    const moved = before.map((frame, index) => frame - after[index]);
    check(
      moved.every((delta) => Math.abs(delta - 60) <= 8),
      "a broadcast seek moves every pane back the same second of ITS OWN clock",
      `${JSON.stringify(before)} → ${JSON.stringify(after)} (deltas ${JSON.stringify(moved)})`
    );
    check(
      Math.abs(after[1] - after[2] - offset) <= 8,
      "the late client and the server still correspond after the seek",
      `offset was ${offset}, now ${after[1] - after[2]}`
    );
    await page.close();
  }

  // --- 4f. Orbs: both roles are MODULES of one file, over the real wire. ------
  // examples/mp puts its roles in separate FILES; orbs puts them in one buffer
  // as `module Client` / `module Server`, so the ROLE — not the file — is what
  // distinguishes the panes. That makes this the only section where booting
  // the wrong param is silent-ish rather than a 404: a pane that lost its
  // `?module=` would boot the file's (nonexistent) plain top-level contract, so
  // reaching `live` at all is already an assertion, and the packet flow below
  // proves the two blocks are really talking to each other.
  {
    const page = await browser.newPage({ viewport: { width: 1600, height: 1000 } });
    const consoleLog = [];
    page.on("console", (m) => consoleLog.push(m.text()));
    page.on("pageerror", (e) => consoleLog.push(`pageerror: ${e}`));
    await page.goto(`${BASE}/sandbox.html?example=orbs#clients=2`);
    await page.waitForFunction(() => window.__sandbox?.status().state === "live", {
      timeout: 40000,
    });
    await installProbe(page);
    const roles = await page.evaluate(() => window.__mpProbe.roles());
    check(
      roles.join(", ") === "client 1, client 2, server",
      "orbs mounts two client panes and an authority pane",
      roles.join(", ")
    );
    const srcs = await page.evaluate(() => window.__mpProbe.srcs());
    const params = srcs.map((src) => new URLSearchParams(src.split("?")[1]));
    check(
      params.length === 3 &&
        params.every((p) => !p.has("prefix")) &&
        params[0].get("module") === "Client" &&
        params[1].get("module") === "Client" &&
        params[2].get("module") === "Server",
      "every orbs pane boots its role as a MODULE — clients Client, authority Server",
      params.map((p) => `${p.get("game")}?module=${p.get("module")}`).join(" | ")
    );
    // One file, one project: the panes differ ONLY by the role they resolve.
    check(
      params.every((p) => p.get("game") === params[0].get("game")),
      "all three panes boot the same file",
      params.map((p) => p.get("game")).join(" | ")
    );

    const pill = await page
      .waitForFunction(() => window.__mpProbe.pill().includes("2+1 running"), null, {
        timeout: 20000,
      })
      .then(() => page.evaluate(() => window.__mpProbe.pill()))
      .catch(() => page.evaluate(() => window.__mpProbe.pill()));
    check(pill.includes("2+1 running"), "the orbs pill counts clients and authority apart", pill);

    // THE TRANSPORT. Orbs speaks a real wire: each client dials the authority
    // (`Sub.connect`), the authority listens (`Sub.listen`) and answers a
    // Welcome, then snapshots stream back every tick. Packets on the hub's
    // wires — both directions — are that traffic, sampled live.
    await page.click("#mp-view-network");
    await sleep(600);
    const graph = await page.evaluate(() => window.__mpProbe.network());
    check(
      graph.wires === 2 && graph.drawn === 2,
      "the hub view wires each orbs client to the authority",
      `${graph.wires} wires`
    );
    let flight = graph;
    let flowing = false;
    for (let i = 0; i < 40 && !flowing; i++) {
      await sleep(150);
      flight = await page.evaluate(() => window.__mpProbe.network());
      flowing = flight.up > 0 && flight.down > 0;
    }
    check(
      flowing,
      "orbs packets fly both ways — Steer up, Snapshot down",
      `up=${flight.up} down=${flight.down}`
    );

    // …and the traffic MEANS something: the authority seated both connections
    // and each client is flying a pid the server handed it (init's -1).
    await page.click("#mp-view-tiled");
    await sleep(2500);
    await page.evaluate(() => window.__mpProbe.pauseAll());
    await sleep(1500);
    const models = await page.evaluate(() => ({
      c1: window.__mpProbe.model("client 1"),
      c2: window.__mpProbe.model("client 2"),
      server: window.__mpProbe.model("server"),
    }));
    const seated = (model) =>
      typeof model === "string" && /myPid: (?!-1)\d/.test(model) && model.includes("ships: [");
    check(
      seated(models.c1) && seated(models.c2),
      "both orbs clients were seated by the authority (a pid off the wire)",
      String(models.c1).slice(0, 120)
    );
    // A SEAT, not just a pilot: the world's pilots carry a `pid` too, so only
    // the cid→pid pairing proves the authority bound each connection.
    check(
      typeof models.server === "string" &&
        (models.server.match(/cid: \d+, pid: \d+/g) ?? []).length === 2,
      "the authority holds a seat for each client pane",
      String(models.server).slice(0, 160)
    );
    // A boot failure can surface as an uncaught exception rather than a
    // `[functor-lang]` line, so the pageerror relay counts too.
    const errors = consoleLog.filter(
      (line) =>
        line.startsWith("pageerror:") ||
        (line.includes("[functor-lang]") && line.includes("error"))
    );
    check(errors.length === 0, "no orbs pane reported a runtime error", errors.slice(0, 3).join(" | "));
    await page.close();
  }

  // --- 4g. Parked, the wires replay the LOG at the playhead. ------------------
  // Traffic is keyed to the timeline (Packet.frame, the reference clock), so a
  // parked session does not show a frozen live feed — it shows the packets that
  // actually crossed around the frame the rail is parked at. Two assertions
  // make that real: at the HEAD there is replayed traffic and no live dot at
  // all, and at the very start of the recording — before the connect handshake
  // — there is nothing on the wires, because nothing had been sent yet.
  //
  // The session is parked as soon as it is linked, which freezes the recorded
  // window at its first ~900 frames: `Home` therefore still lands on the boot
  // frames, whatever else this suite does afterwards.
  {
    const page = await browser.newPage({ viewport: { width: 1600, height: 1000 } });
    await page.goto(`${BASE}/sandbox.html?example=orbs#clients=2`);
    await page.waitForFunction(() => window.__sandbox?.status().state === "live", {
      timeout: 40000,
    });
    await installProbe(page);
    await page.click("#mp-view-network");
    await settle(page);
    // The server broadcasts a snapshot per tick, so traffic appears as soon as
    // the panes are linked — no input needed to provoke it.
    const flowed = await page
      .waitForFunction(() => window.__mpProbe.network().down > 0, null, { timeout: 25000 })
      .then(() => true)
      .catch(() => false);
    check(flowed, "orbs' authority traffic reaches the wires live");

    // --- the pinned wire log: click a link, read its traffic as VALUES -------
    await page.evaluate(() => window.__mpProbe.clickEdge(0));
    await sleep(400);
    const pinned = await page.evaluate(() => window.__mpProbe.wireLog());
    const boxes = await page.evaluate(() => window.__mpProbe.boxes());
    check(
      pinned.pinned && pinned.link === "c1 ↔ srv" && pinned.selected === 1 && pinned.tethered,
      "clicking a link pins its wire log, tethered, with that wire selected",
      JSON.stringify({ link: pinned.link, selected: pinned.selected, tether: pinned.tethered })
    );
    // orbs' protocol is a typed ADT, so the rows must read as its constructors —
    // the whole point of decoding rather than showing bytes.
    const decoded = pinned.rows.filter((row) => /Snapshot|Steer|Welcome|Join|Claim/.test(row.payload));
    check(
      pinned.rows.length > 0 && decoded.length > 0,
      "the rows are decoded Wire values, not bytes",
      `${pinned.rows.length} rows; e.g. "${pinned.rows.at(-1)?.payload ?? ""}"`
    );
    check(
      pinned.rows.some((row) => row.dir === "c1→srv") &&
        pinned.rows.some((row) => row.dir === "srv→c1"),
      "both directions are interleaved in one list",
      pinned.rows.map((row) => row.dir).join(" ")
    );
    // The panel parks in a free quadrant: it may not cover the hub it describes.
    // The overlap arithmetic is written out here rather than imported: this is
    // the check ON the production placement, so it must not share its maths.
    const hub = boxes[2];
    const overlaps = (a, b) =>
      Math.max(0, Math.min(a.x + a.w, b.x + b.w) - Math.max(a.x, b.x)) *
      Math.max(0, Math.min(a.y + a.h, b.y + b.h) - Math.max(a.y, b.y));
    check(
      overlaps(pinned.box, hub) === 0,
      "the panel never covers the hub",
      `${JSON.stringify(pinned.box)} vs hub ${JSON.stringify([hub.x, hub.y, hub.w, hub.h])}`
    );

    await page.evaluate(() => window.__mpProbe.pauseAll());
    await sleep(600);
    await page.focus("#mp-playhead");
    await page.keyboard.press("End");
    await sleep(400);
    const head = await page.evaluate(() => ({
      ...window.__mpProbe.network(),
      label: window.__mpProbe.railFrame(),
    }));
    check(
      head.replay > 0 && head.live === 0,
      "parked at the head, every dot on the wires comes from the packet log",
      `replay=${head.replay} live=${head.live} at #f ${head.label}`
    );

    // SCRUB-LOCKED: parked, the panel's viewport sits at the playhead and marks
    // the nearest row — the rail and the log are one instrument.
    const parkedLog = await page.evaluate(() => window.__mpProbe.wireLog());
    const playhead = Number(head.label.match(/-?\d+/)?.[0] ?? NaN);
    const marked = parkedLog.rows.find((row) => row.at);
    check(
      !!marked && Math.abs(marked.frame - playhead) <= 60,
      "parked, the log marks the row nearest the playhead",
      `playhead #f ${playhead}; marked ${JSON.stringify(marked ?? null)}`
    );

    // Resuming FROM THE HEAD hands the wires back to the live feed, and the
    // replayed dots go with the mode rather than lingering as traffic that is
    // not happening. (From the head deliberately: resuming from a rewound frame
    // is a live session again, but the panes' connections do not survive the
    // rewind — a coordinator-barrier problem, not this view's.)
    await page.evaluate(() => window.__mpProbe.pauseAll());
    const resumed = await page
      .waitForFunction(() => window.__mpProbe.network().live > 0, null, { timeout: 15000 })
      .then(() => page.evaluate(() => window.__mpProbe.network()))
      .catch(() => page.evaluate(() => window.__mpProbe.network()));
    check(
      resumed.live > 0 && resumed.replay === 0,
      "resuming returns the wires to the live feed",
      `live=${resumed.live} replay=${resumed.replay}`
    );

    // Park again and scrub to the very start of the recording.
    await page.evaluate(() => window.__mpProbe.pauseAll());
    await sleep(600);
    await page.focus("#mp-playhead");
    await page.keyboard.press("Home");
    await sleep(400);
    const start = await page.evaluate(() => ({
      ...window.__mpProbe.network(),
      label: window.__mpProbe.railFrame(),
    }));
    // The recording may begin BEFORE the connect handshake (slow boot: the
    // wires are empty here) or right on top of it (fast boot: the handshake
    // packets replay). The invariant is the same either way: parked at the
    // start, everything on the wires is replayed history — nothing is live.
    check(
      start.live === 0 && (start.dots === 0 || start.replay === start.dots),
      "parked at the recording's start, only replayed history is on the wires",
      `${start.dots} dots (${start.replay} replay, ${start.live} live) at #f ${start.label}`
    );

    // Scrubbing sweeps the rows with the dots: the window at the start of the
    // recording is not the window at the head.
    const startLog = await page.evaluate(() => window.__mpProbe.wireLog());
    check(
      startLog.pinned &&
        (startLog.rows.length === 0 ||
          startLog.rows.at(-1).frame < (parkedLog.rows.at(-1)?.frame ?? Infinity)),
      "scrubbing back sweeps the log's viewport with the playhead",
      `head ended at #f ${parkedLog.rows.at(-1)?.frame}, start ends at #f ${startLog.rows.at(-1)?.frame}`
    );

    // Escape clears the pin (the last layer in the cascade — no popover is open
    // and no event is selected here), and another link re-pins in place.
    await page.keyboard.press("Escape");
    await sleep(200);
    const cleared = await page.evaluate(() => window.__mpProbe.wireLog());
    check(
      !cleared.pinned && cleared.selected === 0,
      "esc clears the pinned panel and deselects the wire",
      JSON.stringify(cleared)
    );
    await page.evaluate(() => window.__mpProbe.clickEdge(1));
    await sleep(300);
    const repinned = await page.evaluate(() => window.__mpProbe.wireLog());
    check(
      repinned.pinned && repinned.link === "c2 ↔ srv" && repinned.selected === 1,
      "clicking another link re-pins the panel to it",
      `${repinned.link} — ${repinned.selected} selected`
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
    // Enter the hub view FIRST: dropping the authority must not strand the
    // session in a layout that no longer has a hub.
    await page.click("#mp-view-network");
    await sleep(400);
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
    check(
      (await page.evaluate(() => window.__mpProbe.layout().view)) !== "network",
      "losing the authority falls the layout back out of NETWORK",
      await page.evaluate(() => window.__mpProbe.layout().view)
    );

    // No authority, no hub: NETWORK is disabled with a tooltip that says why,
    // and `f` steps over it instead of stalling the cycle.
    await page.selectOption("#client-count", "2");
    await sleep(2500);
    const soloGraph = await page.evaluate(() => window.__mpProbe.network());
    check(
      soloGraph.networkDisabled && soloGraph.networkTitle.includes("no server role"),
      "a single-role example disables NETWORK and explains why",
      `disabled=${soloGraph.networkDisabled}; "${soloGraph.networkTitle}"`
    );
    const soloCycle = [];
    for (let i = 0; i < 3; i++) {
      await page.keyboard.press("f");
      await sleep(250);
      soloCycle.push(await page.evaluate(() => window.__mpProbe.layout().view));
    }
    check(
      soloCycle.join(" → ") === "grid → tabs → tiled",
      "f skips NETWORK when there is no authority",
      soloCycle.join(" → ")
    );

    // Grid with no authority: nothing closes the bottom row, so the odd last
    // client stretches rather than leaving a whole empty quadrant.
    await page.selectOption("#client-count", "3");
    await sleep(2500);
    await page.click("#mp-view-grid");
    await settle(page);
    const solo = await page.evaluate(() => window.__mpProbe.boxes());
    check(
      solo.length === 3 &&
        solo[2].cell.w > solo[0].cell.w * 1.8 &&
        solo[2].y >= solo[0].y + solo[0].h,
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
