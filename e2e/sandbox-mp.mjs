// Sandbox multiplayer e2e: the SERVER PANE, end to end.
//
// e2e/net-coordinator.mjs proves the routing substrate on a bare fixture page.
// This one proves the PRODUCT: pick "Orbs (multiplayer)" in the sandbox and
// you get a working session in the pane grid — a server pane plus N client
// panes, wired by the same host coordinator, with the clients actually seated
// in the server's arena.
//
// Orbs keeps BOTH roles in one `game.fun` as inline `module Client` /
// `module Server` blocks, so a pane's ROLE — not its file — is what
// distinguishes it, and it travels as `?module=`.
//
// It asserts:
//
//   1. orbs at the default count mounts a server pane beside ONE client (the
//      server is extra at every client count, not a client you gave up);
//   2. #clients=2 grows to server + two clients, and the pill counts them
//      honestly as "2+1 running";
//   3. every pane boots the SAME file at its own inline module, and both
//      clients are seated by the authority (a pid off the wire) — read from
//      each pane's paused inspector trace, the same path the net-coordinator
//      e2e uses — so the panes are one session, not three lonely sims;
//   4. every pane header shows its coordinator link state;
//   5. the GRID layout puts the clients in two columns with the server as a
//      full-width strip below them — and switching layout reloads no pane;
//   6. switching to a single-role example removes the server pane again;
//   7. every pane CARD letterboxes to a 16:9 body inside its cell in the grid
//      and network views, and the network wires anchor to the cards rather
//      than to the cells;
//   8. parked, the wires and the pinned wire log replay the packet LOG at the
//      playhead rather than showing a frozen live feed;
//   9. the WIRE TAB — the same traffic monitor docked in the status bar — is
//      present (and counting) in every layout, filters by link and direction,
//      opens payloads into the value tree, stays scrub-locked, and is absent
//      from a single-role example;
//  10. LINK IMPAIRMENT: a pane's link chip really delays that client's packets
//      (sent → delivered on every row), only that client's, never out of order,
//      and without breaking convergence — while loss stays a labelled datagram
//      setting (design Addendum 8.2).
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
 * box, not its laid-out one.
 */
const settle = (page) =>
  page.waitForFunction(
    () =>
      [...document.querySelectorAll(".mp-pane")].every(
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
    // One row of traffic, as both surfaces render it: the SENT frame, the
    // DELIVERED frame beside it (impairment — Addendum 8.2), and whether the
    // playhead has it mid-flight.
    const readRow = (row) => {
      const stamps = [...row.querySelector(".f").textContent.matchAll(/-?\d+/g)].map((m) =>
        Number(m[0])
      );
      return {
        frame: stamps[0] ?? NaN,
        delivered: stamps[1] ?? stamps[0] ?? NaN,
        flight: row.classList.contains("flight"),
        link: row.querySelector(".l")?.textContent.trim() ?? null,
        dir: row.querySelector(".d").textContent.trim(),
        payload: row.querySelector(".payload").textContent.trim(),
        bytes: row.querySelector(".n").textContent.trim(),
        at: row.classList.contains("at"),
      };
    };
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
          rows: [...panel.querySelectorAll(".mp-wl")].map(readRow),
        };
      },
      /** Open the FATTEST payload on the pinned log — the snapshot, i.e. the
       * row whose one-line rendering elides the most. Returns its compact
       * text, so the test can compare it with what the tree then shows. */
      openBiggestRow: () => {
        let button = null;
        let best = -1;
        for (const row of document.querySelectorAll(".mp-wl")) {
          const bytes = Number((row.querySelector(".n").textContent.match(/\d+/) ?? [0])[0]);
          const control = row.querySelector("button.payload");
          if (control && bytes > best) {
            best = bytes;
            button = control;
          }
        }
        if (!button) return null;
        const compact = button.textContent.trim();
        button.click();
        return compact;
      },
      /** Open the shallowest still-CLOSED node labelled `label` (the child
       * index / field name), so repeating a label drills down a level rather
       * than closing what the last call opened. */
      openNode: (label) => {
        const node = [...document.querySelectorAll("button.mp-vt-row")].find(
          (row) =>
            row.querySelector(".mp-vt-k")?.textContent === label &&
            row.getAttribute("aria-expanded") !== "true"
        );
        if (!node) return false;
        node.click();
        return true;
      },
      /** The open row's tree, flattened: every visible node's label, its
       * rendered value, and whether it is open. */
      valueTree: () => {
        const tree = document.querySelector(".mp-wl-tree");
        const open = document.querySelectorAll('.mp-wl .payload[aria-expanded="true"]').length;
        if (!tree) return { mounted: false, openRows: open, nodes: [] };
        return {
          mounted: true,
          openRows: open,
          // The pinned panel's footer, when it is the tree's host; the wire tab
          // has its own (`.wire-ft`, read by wireTab()).
          footer: document.querySelector(".mp-wl-ft")?.textContent.trim() ?? "",
          nodes: [...tree.querySelectorAll(".mp-vt-row")]
            // A node inside a CLOSED parent is in the DOM but not on screen.
            .filter((row) => !row.closest(".mp-vt-kids[hidden]"))
            .map((row) => ({
              key: row.querySelector(".mp-vt-k")?.textContent ?? "",
              value: row.querySelector(".mp-vt-v")?.textContent ?? "",
              open: row.getAttribute("aria-expanded") === "true",
            })),
        };
      },
      closeOpenRow: () => {
        const button = document.querySelector('.mp-wl .payload[aria-expanded="true"]');
        if (!button) return false;
        button.click();
        return true;
      },
      clickEdge: (index) => document.querySelectorAll(".mp-edge-chip")[index].click(),
      /** A client pane's link chip: what it says, and what it promises. */
      linkChip: (index) => {
        const chip = document.querySelectorAll(".mp-pane .mp-link-chip")[index];
        return chip ? { label: chip.textContent.trim(), title: chip.title } : null;
      },
      /** Set a client's link from the chip menu — the reader's own path: open
       * the chip, press a preset, close it again. */
      setLink: (index, name) => {
        const chip = document.querySelectorAll(".mp-pane .mp-link-chip")[index];
        if (!chip) return false;
        chip.click();
        const menu = chip.closest(".mp-link-host").querySelector(".mp-link-menu");
        const preset = [...menu.querySelectorAll(".mp-link-presets button")].find(
          (one) => one.dataset.name === name
        );
        if (!preset) return false;
        preset.click();
        chip.click();
        return true;
      },
      /** The link menu's loss field and the label that says who reads it. */
      lossField: (index) => {
        const menu = document
          .querySelectorAll(".mp-pane .mp-link-host")
          [index]?.querySelector(".mp-link-menu");
        if (!menu) return null;
        return {
          value: menu.querySelector(".mp-l-loss").value,
          badge: menu.querySelector(".mp-link-soon")?.textContent.trim() ?? "",
          note: menu.querySelector(".mp-link-note").textContent.replace(/\s+/g, " ").trim(),
        };
      },
      /** The WIRE TAB in the status bar: the docked traffic monitor. Read from
       * the bar's own chrome, so "present" means the tab a reader can click. */
      wireTab: () => {
        const tab = document.querySelector('.statusbar-tab[data-tab="wire"]');
        const badge = tab?.querySelector(".wire-badge");
        const list = document.querySelector(".wire-list");
        return {
          present: !!tab,
          active: !!tab?.classList.contains("active"),
          badge: badge?.textContent?.trim() ?? "",
          badgeHidden: !badge || badge.hidden,
          // The panel is toggled with `display`, like every other tab's list.
          open: !!list && getComputedStyle(list).display !== "none",
          // Which layout the panes are in — the whole point is that this works
          // outside the network view.
          view: document.querySelector(".mp-grid").dataset.view,
          chips: [...document.querySelectorAll(".wire-filters .wire-chip")].map(
            (chip) => `${chip.textContent.trim()}:${chip.getAttribute("aria-pressed")}`
          ),
          footer: document.querySelector(".wire-ft")?.textContent.trim() ?? "",
          rows: [...document.querySelectorAll(".wire-rows .mp-wl")].map(readRow),
        };
      },
      openWireTab: () => document.querySelector('.statusbar-tab[data-tab="wire"]').click(),
      /** Pick a filter chip by its label prefix (`c2`, `intent`, `all`). */
      pickWireChip: (label) => {
        const chip = [...document.querySelectorAll(".wire-filters .wire-chip")].find((one) =>
          one.textContent.trim().startsWith(label)
        );
        if (!chip) return false;
        chip.click();
        return true;
      },
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
      // A keydown/keyup straight into a pane's window — how a client pane is
      // given real intent to put on the wire.
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
      // Reload markers per pane: a hot-swap records one on that runtime's own
      // timeline, so this counts who actually took a pushed edit.
      reloads: () =>
        [...document.querySelectorAll(".mp-pane")].map((shell, index) => {
          const role = shell.classList.contains("server") ? "server" : `client ${index + 1}`;
          const seam = shell.querySelector("iframe")?.contentWindow?.__scrub;
          const markers = seam?.view()?.eventMarkers ?? [];
          const count = markers
            .filter((m) => m.category === "reload")
            .reduce((total, m) => total + (m.count ?? 1), 0);
          return { role, count };
        }),
    };
  });

const browser = await chromium.launch();
try {
  // --- 1. One client + a server pane. -----------------------------------------
  {
    const page = await browser.newPage({ viewport: { width: 1440, height: 900 } });
    await page.goto(`${BASE}/sandbox.html?example=orbs`);
    await page.waitForFunction(() => window.__sandbox?.status().state === "live", {
      timeout: 40000,
    });
    await installProbe(page);
    const roles = await page.evaluate(() => window.__mpProbe.roles());
    check(
      roles.length === 2 && roles[0] === "client 1" && roles[1] === "server",
      "orbs at one client mounts a server pane beside it",
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

    // The "+" affordances are retired: the CLIENTS dropdown is the one way to
    // change the count, and no ghost seat/tab renders in any view.
    const addAffordances = await page.evaluate(() => ({
      tile: !!document.querySelector(".mp-add-tile"),
      tab: !!document.querySelector(".mp-tab.add"),
    }));
    check(
      !addAffordances.tile && !addAffordances.tab,
      "no + tile or + tab renders (the CLIENTS dropdown is the only count control)",
      JSON.stringify(addAffordances)
    );
    await page.selectOption("#client-count", "2");
    await sleep(2500);
    const afterAdd = await page.evaluate(() => ({
      roles: window.__mpProbe.roles(),
      hash: window.location.hash,
    }));
    check(
      afterAdd.roles.join(", ") === "client 1, client 2, server" &&
        afterAdd.hash.includes("clients=2"),
      "the CLIENTS dropdown adds a client (roles + hash follow)",
      JSON.stringify(afterAdd)
    );
    await page.selectOption("#client-count", "1");
    await sleep(1500);

    // Grid with a LONE client: the client takes the whole row (no peer to
    // match), over the authority strip.
    await page.click("#mp-view-grid");
    await settle(page);
    const [client, server] = await page.evaluate(() => window.__mpProbe.boxes());
    check(
      server.gridColumn === "1 / -1" &&
        server.y >= client.y + client.h &&
        server.h < client.h,
      "grid keeps the lone client over the authority strip",
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
    await page.goto(`${BASE}/sandbox.html?example=orbs#clients=2`);
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

    // The ROLE seam. Orbs' two roles are inline modules of ONE file, so a pane
    // that lost its `?module=` would boot the file's (nonexistent) plain
    // top-level contract — silent-ish rather than a 404. Assert the params
    // themselves, then that all three panes really are the same file.
    const srcs = await page.evaluate(() => window.__mpProbe.srcs());
    const params = srcs.map((src) => new URLSearchParams(src.split("?")[1]));
    check(
      params.length === 3 &&
        params.every((p) => !p.has("prefix")) &&
        params[0].get("module") === "Client" &&
        params[1].get("module") === "Client" &&
        params[2].get("module") === "Server",
      "every pane boots its role as a MODULE — clients Client, authority Server",
      params.map((p) => `${p.get("game")}?module=${p.get("module")}`).join(" | ")
    );
    check(
      params.every((p) => p.get("game") === params[0].get("game")),
      "all three panes boot the same file",
      params.map((p) => p.get("game")).join(" | ")
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

    // Both clients are in the SERVER's arena — the assertion that makes this a
    // session rather than three independent sims. A client's `myPid` starts at
    // -1 and only a Welcome off the wire replaces it, so a seated pid is the
    // authority's answer travelling back.
    await sleep(2500);
    await page.evaluate(() => window.__mpProbe.pauseAll());
    await sleep(1500);
    const models = await page.evaluate(() => ({
      c1: window.__mpProbe.model("client 1"),
      c2: window.__mpProbe.model("client 2"),
      server: window.__mpProbe.model("server"),
    }));
    // A pid off the wire AND a board that actually arrived: `ships: []` would
    // satisfy a mere `includes("ships: [")`, so count the ships (each renders
    // `{ pid: …, x: …, y: …, rot: … }`; `myPid` is capitalised, orbs carry
    // `id:`/`owner:`, so lowercase `pid: ` counts ships alone).
    const seated = (model) =>
      typeof model === "string" &&
      /myPid: \d/.test(model) &&
      (model.match(/\bpid: /g) ?? []).length === 2;
    check(
      seated(models.c1) && seated(models.c2),
      "both client panes were seated by the server pane (a pid off the wire)",
      String(models.c1).slice(0, 160)
    );

    // A pushed edit reaches EVERY runtime, the authority included. Both roles
    // are modules of the buffer being edited, and each pane re-resolves its own
    // role on reload — so a session-wide hot swap is one edit, not one edit
    // plus a reset. (Before the authority was a push target this held for the
    // clients alone, and the server pane sat on its served source.)
    const before = await page.evaluate(() => window.__mpProbe.reloads());
    await page.evaluate(() => {
      const src = window.__sandbox.source();
      window.__sandbox.setSource(src.replace("let halfW = 13.0", "let halfW = 12.0"));
    });
    await sleep(3500);
    const after = await page.evaluate(() => window.__mpProbe.reloads());
    const took = (role) => {
      const b = before.find((r) => r.role === role)?.count ?? 0;
      const a = after.find((r) => r.role === role)?.count ?? 0;
      return a > b;
    };
    check(
      took("server") && took("client 1") && took("client 2"),
      "one edit hot-reloads every pane, the authority included",
      JSON.stringify({ before, after })
    );
    // A SEAT, not just a pilot: the world's pilots carry a `pid` too, so only
    // the cid→pid pairing proves the authority bound each connection.
    check(
      typeof models.server === "string" &&
        (models.server.match(/cid: \d+, pid: \d+/g) ?? []).length === 2,
      "the server pane holds a seat for each client",
      String(models.server).slice(0, 160)
    );
    // A boot failure can surface as an uncaught exception rather than a
    // `[functor-lang]` line, so the pageerror relay counts too.
    const errors = consoleLog.filter(
      (line) =>
        line.startsWith("pageerror:") ||
        (line.includes("[functor-lang]") && line.includes("error"))
    );
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
    await page.goto(`${BASE}/sandbox.html?example=orbs`);
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
    await page.goto(`${BASE}/sandbox.html?example=orbs#clients=2`);
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
    // The strip stays a STRIP at max count: shorter than a client card (the
    // data-clients="3" span override in styles.css exists for exactly this).
    check(
      three[3].h < three[0].h,
      "the server strip stays shorter than a client card at three clients",
      `server ${three[3].h} vs client ${three[0].h}`
    );
    await page.close();
  }

  // --- 4d. NETWORK layout: the hub, its measured wires, and live packets. -----
  // The graph is drawn from MEASURED pane boxes, so the assertions here are
  // about geometry (who sits where, how many wires exist and that each one has
  // a real length) rather than about hard-coded coordinates.
  {
    const page = await browser.newPage({ viewport: { width: 1600, height: 1000 } });
    await page.goto(`${BASE}/sandbox.html?example=orbs#clients=2`);
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
      graph.chips.length === 2 && graph.chips.every((c) => c.startsWith("LAN · 8ms <1f")),
      "each edge carries its client's link chip mid-wire, sub-frame pace and all",
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
      // Orbs' client streams a Steer every tick regardless, but driving client
      // 1 puts real intent on the wire; a dot lives 600ms, so sample faster
      // than that.
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
    await page.goto(`${BASE}/sandbox.html?example=orbs`);
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

  // --- 4f. Parked, the wires replay the LOG at the playhead. ------------------
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

    // --- the value tree: a row opens into its whole payload ------------------
    // The row's one line is a summary and says so (`…`); opening it must show
    // the fields that line elided, with nothing elided in their place.
    const compact = await page.evaluate(() => window.__mpProbe.openBiggestRow());
    const tree = await page.evaluate(() => window.__mpProbe.valueTree());
    check(
      tree.mounted && tree.openRows === 1 && tree.nodes.length > 1,
      "opening a payload mounts its value tree in the row",
      `${tree.nodes.length} nodes from "${compact}"`
    );
    // Into the snapshot's first list, then its first entry: the ship record's
    // own fields, which the row could never have shown.
    await page.evaluate(() => window.__mpProbe.openNode("0"));
    await sleep(50);
    await page.evaluate(() => window.__mpProbe.openNode("0"));
    await sleep(50);
    const deep = await page.evaluate(() => window.__mpProbe.valueTree());
    const leaves = deep.nodes.filter((node) => /^(pid|x|y|rot|id|owner)$/.test(node.key));
    check(
      leaves.length >= 4 && leaves.every((node) => !node.value.includes("…")),
      "expanding reaches the record's own fields, each rendered whole",
      JSON.stringify(leaves.slice(0, 6))
    );
    // The live tail turns over ten times a second: the open row survives it
    // (opening holds the viewport — the panel says so).
    await sleep(400);
    const survived = await page.evaluate(() => window.__mpProbe.valueTree());
    check(
      survived.mounted &&
        survived.openRows === 1 &&
        survived.nodes.length === deep.nodes.length &&
        /tail is held/.test(survived.footer),
      "the open row survives the live tail, with its opened nodes still open",
      `${survived.nodes.length} nodes (was ${deep.nodes.length}) — "${survived.footer}"`
    );
    const closed = await page
      .evaluate(() => window.__mpProbe.closeOpenRow())
      .then(() => sleep(100))
      .then(() => page.evaluate(() => window.__mpProbe.valueTree()));
    check(
      !closed.mounted && closed.openRows === 0,
      "closing the row puts the panel back to one line per packet",
      JSON.stringify(closed)
    );

    await page.evaluate(() => window.__mpProbe.pauseAll());
    await sleep(600);
    await page.focus("#mp-playhead");
    await page.keyboard.press("End");
    await sleep(400);
    // Since impairment (#657) a parked frame shows exactly the packets in the
    // air THEN — an unimpaired dot's drawn flight is only the ~2-frame
    // visibility floor, and the pause can land a few frames after the last
    // packet the coordinator routed (Packet.frame is measured once per host
    // rAF, so it lags the recorded head under load). The head may therefore
    // legitimately be past every flight. The unconditional claim is that a
    // parked wire carries no LIVE dot; replay dots are required exactly when
    // the log actually has a packet in the air at the head — read off the
    // pinned wire log, the product's own record.
    const head = await page.evaluate(() => ({
      ...window.__mpProbe.network(),
      log: window.__mpProbe.wireLog(),
      label: window.__mpProbe.railFrame(),
    }));
    const playhead = Number(head.label.match(/-?\d+/)?.[0] ?? NaN);
    const inFlightAtHead = head.log.rows.some(
      (row) => Number.isFinite(row.frame) && playhead - row.frame >= 0 && playhead - row.frame <= 1
    );
    check(
      head.live === 0 && (!inFlightAtHead || head.replay > 0),
      "parked at the head, every dot on the wires comes from the packet log",
      `replay=${head.replay} live=${head.live} at #f ${head.label}` +
        (inFlightAtHead
          ? ""
          : ` (nothing in flight at the head — last logged #f ${head.log.rows.at(-1)?.frame ?? "none"})`)
    );

    // SCRUB-LOCKED: parked, the panel's viewport sits at the playhead and marks
    // the nearest row — the rail and the log are one instrument.
    const parkedLog = head.log;
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

  // --- 4g. The WIRE TAB: the traffic monitor, docked. -------------------------
  // Addendum 8.2. The pinned panel above lives in the network view and hangs off
  // one wire; this is the same rows as a permanent tab in the bottom panel, so
  // the assertions here are the ones the pinned panel cannot make: the tab is
  // there while the panes are in TILED, its badge counts packets while it is
  // closed, the link filter narrows the list, and the rows are still
  // scrub-locked to the rail.
  {
    const page = await browser.newPage({ viewport: { width: 1600, height: 1000 } });
    await page.goto(`${BASE}/sandbox.html?example=orbs#clients=2`);
    await page.waitForFunction(() => window.__sandbox?.status().state === "live", {
      timeout: 40000,
    });
    await installProbe(page);

    // CLOSED, in the default layout: the tab exists and its badge is counting.
    const counting = await page
      .waitForFunction(() => Number(window.__mpProbe.wireTab().badge) > 0, null, {
        timeout: 25000,
      })
      .then(() => page.evaluate(() => window.__mpProbe.wireTab()))
      .catch(() => page.evaluate(() => window.__mpProbe.wireTab()));
    check(
      counting.present && counting.view === "tiled" && !counting.open && !counting.badgeHidden,
      "the wire tab sits in the bottom panel from the start, closed, in TILED",
      JSON.stringify({ view: counting.view, open: counting.open, badge: counting.badge })
    );
    check(
      Number(counting.badge) > 0,
      "its badge counts routed packets while the panel is closed",
      counting.badge
    );

    // OPEN: rows, in a layout that draws no wires at all — the whole point.
    await page.evaluate(() => window.__mpProbe.openWireTab());
    await sleep(400);
    const opened = await page.evaluate(() => window.__mpProbe.wireTab());
    check(
      opened.open && opened.active && opened.view === "tiled" && opened.rows.length > 0,
      "opening it shows traffic in TILED — no network view needed",
      `${opened.rows.length} rows in ${opened.view}`
    );
    check(
      opened.badgeHidden,
      "the count steps aside once the rows are on screen",
      `badge "${opened.badge}"`
    );
    check(
      opened.rows.some((row) => /Snapshot|Steer|Welcome|Join|Claim/.test(row.payload)) &&
        opened.rows.every((row) => /^\d+ B$/.test(row.bytes)),
      "the rows are the same grammar as the pinned panel — decoded values and bytes",
      `e.g. "${opened.rows.at(-1)?.payload ?? ""}" ${opened.rows.at(-1)?.bytes ?? ""}`
    );
    // ALL is the default, so every row names the link it crossed and both
    // clients are in the one list.
    const linksSeen = new Set(opened.rows.map((row) => row.link));
    check(
      opened.chips[0] === "all links:true" &&
        opened.rows.every((row) => row.link) &&
        linksSeen.size > 1,
      "ALL interleaves every link, each row naming the wire it crossed",
      `${[...linksSeen].join(", ")} — chips ${opened.chips.join(" | ")}`
    );
    check(
      /showing the last \d+ of \d+ rows|^\d+ rows?/.test(opened.footer),
      "the footer says how much of the log it is showing",
      opened.footer
    );

    // The LINK FILTER narrows the list to one wire (and drops the now-redundant
    // link column with it).
    await page.evaluate(() => window.__mpProbe.pickWireChip("c2"));
    await sleep(400);
    const filtered = await page.evaluate(() => window.__mpProbe.wireTab());
    check(
      filtered.rows.length > 0 &&
        filtered.rows.every((row) => /c2/.test(row.dir)) &&
        filtered.rows.every((row) => row.link === null) &&
        filtered.chips.includes("c2 ↔ srv:true"),
      "picking a link narrows the monitor to that wire",
      `${filtered.rows.length} rows: ${[...new Set(filtered.rows.map((r) => r.dir))].join(" ")}`
    );
    // ...and the direction filter drops half the conversation.
    await page.evaluate(() => window.__mpProbe.pickWireChip("intent"));
    await sleep(400);
    const intent = await page.evaluate(() => window.__mpProbe.wireTab());
    check(
      intent.rows.length > 0 && intent.rows.every((row) => row.dir === "c2→srv"),
      "the direction filter keeps intent only",
      [...new Set(intent.rows.map((r) => r.dir))].join(" ")
    );
    await page.evaluate(() => window.__mpProbe.pickWireChip("both"));
    await sleep(300);

    // The value tree opens in the tab exactly as it does in the panel.
    const compact = await page.evaluate(() => window.__mpProbe.openBiggestRow());
    await sleep(200);
    const tree = await page.evaluate(() => window.__mpProbe.valueTree());
    check(
      tree.mounted && tree.openRows === 1 && tree.nodes.length > 1,
      "a payload opens into its value tree inside the tab",
      `${tree.nodes.length} nodes from "${compact}"`
    );
    const heldFooter = (await page.evaluate(() => window.__mpProbe.wireTab())).footer;
    check(
      /tail is held/.test(heldFooter),
      "opening a row holds the tail, and the footer says so",
      heldFooter
    );
    // A held tail is a FROZEN list, so changing the filter under one would keep
    // showing rows the pressed chip excludes. Changing a filter closes the row.
    await page.evaluate(() => window.__mpProbe.pickWireChip("intent"));
    await sleep(400);
    const refiltered = await page.evaluate(() => ({
      ...window.__mpProbe.wireTab(),
      tree: window.__mpProbe.valueTree(),
    }));
    check(
      !refiltered.tree.mounted &&
        refiltered.tree.openRows === 0 &&
        refiltered.rows.length > 0 &&
        refiltered.rows.every((row) => row.dir === "c2→srv"),
      "changing a filter while a row is open closes it — the held rows are the old filter's",
      `${refiltered.rows.length} rows: ${[...new Set(refiltered.rows.map((r) => r.dir))].join(" ")}`
    );
    await page.evaluate(() => window.__mpProbe.pickWireChip("both"));
    await sleep(300);

    // SCRUB-LOCKED: parked, the tab's viewport follows the rail and marks the
    // row nearest the playhead — the same promise the pinned panel makes.
    await page.evaluate(() => window.__mpProbe.pauseAll());
    await sleep(600);
    await page.focus("#mp-playhead");
    await page.keyboard.press("End");
    await sleep(500);
    const head = await page.evaluate(() => ({
      ...window.__mpProbe.wireTab(),
      label: window.__mpProbe.railFrame(),
    }));
    const playhead = Number(head.label.match(/-?\d+/)?.[0] ?? NaN);
    const marked = head.rows.find((row) => row.at);
    check(
      !!marked && Math.abs(marked.frame - playhead) <= 60,
      "parked, the tab marks the row nearest the playhead",
      `playhead #f ${playhead}; marked ${JSON.stringify(marked ?? null)}`
    );
    await page.keyboard.press("Home");
    await sleep(500);
    const start = await page.evaluate(() => window.__mpProbe.wireTab());
    check(
      start.rows.length === 0 ||
        start.rows.at(-1).frame < (head.rows.at(-1)?.frame ?? Infinity),
      "seeking sweeps the tab's viewport with the playhead",
      `head ended at #f ${head.rows.at(-1)?.frame}, start ends at #f ${start.rows.at(-1)?.frame}`
    );
    await page.close();
  }

  // --- 4h. LINK IMPAIRMENT: the chips are controls now. -----------------------
  // Addendum 8.2. A client's link chip schedules that client's packets in the
  // coordinator — latency plus a jitter draw, and NOTHING else: `Sub.connect`
  // is reliable-ordered, so no packet may be dropped or overtaken. The
  // assertions are exactly those two halves. The delay is REAL (mobile's 132ms
  // is eight frames of the timeline, and the rows print sent → delivered), and
  // the guarantees are intact (FIFO per link per direction; the session still
  // converges; the untouched client is still on LAN, i.e. same-frame).
  {
    const page = await browser.newPage({ viewport: { width: 1600, height: 1000 } });
    await page.goto(`${BASE}/sandbox.html?example=orbs#clients=2`);
    await page.waitForFunction(() => window.__sandbox?.status().state === "live", {
      timeout: 40000,
    });
    await installProbe(page);
    await page.evaluate(() => window.__mpProbe.openWireTab());
    await page.waitForFunction(() => window.__mpProbe.wireTab().rows.length > 4, null, {
      timeout: 25000,
    });

    // The default is LAN, and LAN is a decision (mp-panes' DEFAULT_LINK): 8ms
    // rounds to zero frames, so an untouched session behaves exactly as it did
    // before impairment existed.
    const before = await page.evaluate(() => window.__mpProbe.wireTab());
    check(
      before.rows.length > 4 && before.rows.every((row) => row.delivered === row.frame),
      "the default LAN link delivers in the frame it was sent — impairment is opt-in",
      `deltas ${[...new Set(before.rows.map((row) => row.delivered - row.frame))].join(", ")}`
    );

    // Now slow client 1's link down, through the chip a reader would use.
    const set = await page.evaluate(() => window.__mpProbe.setLink(0, "mobile"));
    const chip = await page.evaluate(() => window.__mpProbe.linkChip(0));
    check(
      set && chip.label === "⇅ mobile ▾" && /132ms/.test(chip.title),
      "picking `mobile` on a pane's chip re-labels it with what it now does",
      `${chip?.label} — "${chip?.title}"`
    );
    // Long enough that every row in the 60-row viewport was routed AFTER the
    // change: orbs puts ~240 packets a second on the wires at two clients.
    await sleep(2500);
    const after = await page.evaluate(() => window.__mpProbe.wireTab());
    const on = (id) => after.rows.filter((row) => row.link === id);
    const deltas = (id) => on(id).map((row) => row.delivered - row.frame);
    // 132ms is 7.92 frames of the timeline → 8, and 38ms of jitter is 2 more at
    // most. Nothing may arrive EARLY: jitter is extra delay, never a head start.
    const mobile = deltas("c1");
    check(
      mobile.length > 4 && mobile.every((d) => d >= 8 && d <= 10),
      "a mobile link delays client 1's packets by its latency, plus a jitter draw",
      `${mobile.length} rows, deltas ${[...new Set(mobile)].sort().join(", ")}`
    );
    check(
      new Set(mobile).size > 1,
      "the jitter draw actually varies packet to packet",
      `deltas ${[...new Set(mobile)].sort().join(", ")}`
    );
    // The other client never changed, and is still same-frame: one chip impairs
    // one link.
    const lan = deltas("c2");
    check(
      lan.length > 4 && lan.every((d) => d === 0),
      "client 2's LAN link is untouched — same frame, still sub-frame-ish",
      `${lan.length} rows, deltas ${[...new Set(lan)].sort().join(", ")}`
    );

    // FIFO, the invariant impairment may not break: on one link in one
    // direction, a later-sent packet never lands before an earlier one, however
    // the jitter drew.
    const ordered = [];
    const seen = new Map();
    for (const row of after.rows) {
      const key = `${row.link}|${row.dir}`;
      const previous = seen.get(key);
      if (previous !== undefined && row.delivered < previous) {
        ordered.push(`${key}: ${previous} then ${row.delivered}`);
      }
      seen.set(key, row.delivered);
    }
    check(
      after.rows.length > 8 && ordered.length === 0,
      "no packet overtakes an earlier one on its link (FIFO under jitter)",
      ordered.slice(0, 3).join(" | ") || `${after.rows.length} rows in order`
    );

    // Loss stays a SETTING, labelled for the channel that will read it.
    const loss = await page.evaluate(() => window.__mpProbe.lossField(0));
    check(
      loss?.badge === "datagrams" && /Net\.Udp/.test(loss.note) && /reliable-ordered/.test(loss.note),
      "the loss field says it applies to datagrams, and why nothing is dropped here",
      `badge "${loss?.badge}" — "${loss?.note}"`
    );

    // PACE IS A MEASUREMENT (Addendum 5a): the impaired edge's dots fly its real
    // delay, and the LAN edge — whose delay is below the visibility floor — says
    // on its chip how much it is being stretched.
    await page.click("#mp-view-network");
    await settle(page);
    const chips = await page.evaluate(() => window.__mpProbe.network().chips);
    check(
      chips.length === 2 &&
        /^mobile · 132ms/.test(chips[0]) &&
        !/<1f/.test(chips[0]) &&
        /^LAN · 8ms <1f/.test(chips[1]),
      "the mid-edge chips show the real profile, and flag the edge whose pace is floored",
      chips.join(" | ")
    );

    // CONVERGENCE: eight frames of delay is a delay, not a break — both clients
    // still hold both ships.
    await page.evaluate(() => window.__mpProbe.pauseAll());
    await sleep(1500);
    const boards = await page.evaluate(() => ({
      c1: window.__mpProbe.model("client 1"),
      c2: window.__mpProbe.model("client 2"),
    }));
    const ships = (text) => (text?.match(/pid: /g) ?? []).length;
    check(
      ships(boards.c1) >= 2 && ships(boards.c2) >= 2,
      "the session still converges over an impaired link — orbs plays, delayed",
      `client 1 holds ${ships(boards.c1)} ships, client 2 ${ships(boards.c2)}`
    );

    // Parked, a packet SENT before the playhead but delivered after it is drawn
    // as what it is: still crossing. The window keys on the sent frame (that is
    // the row's place on the rail), so the in-flight rows are the newest ones.
    await page.evaluate(() => window.__mpProbe.openWireTab());
    await page.focus("#mp-playhead");
    await page.keyboard.press("End");
    await sleep(600);
    const parked = await page.evaluate(() => ({
      ...window.__mpProbe.wireTab(),
      label: window.__mpProbe.railFrame(),
    }));
    const playhead = Number(parked.label.match(/-?\d+/)?.[0] ?? NaN);
    const flying = parked.rows.filter((row) => row.flight);
    check(
      flying.length > 0 &&
        flying.every(
          (row) => row.link === "c1" && row.frame <= playhead && row.delivered > playhead
        ),
      "parked, a packet sent before the playhead and delivered after it reads as in flight",
      `playhead #f ${playhead}; ${flying.length} in flight: ${flying
        .slice(0, 3)
        .map((row) => `${row.link} ${row.frame}→${row.delivered}`)
        .join(", ")}`
    );
    await page.close();
  }

  // --- 5. Switching away drops the server pane. -------------------------------
  {
    const page = await browser.newPage({ viewport: { width: 1440, height: 900 } });
    await page.goto(`${BASE}/sandbox.html?example=orbs`);
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
    // Same gate for the docked monitor: no authority, no links, no coordinator
    // log — so the tab is absent rather than an empty panel.
    const soloWire = await page.evaluate(() => window.__mpProbe.wireTab());
    check(
      !soloWire.present && !soloWire.open,
      "a single-role example has no wire tab at all",
      JSON.stringify(soloWire)
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
