// Site e2e: the sandbox's editor → runtime live-reload loop, headless — the
// site-shaped sibling of functor-lang-preview-reload.mjs (which drives the CLI dev
// server directly). Builds the site, serves dist with site/serve.mjs, then
// drives headless Chromium through:
//
//   1. the landing page's hero iframe renders (a live Functor Lang scene);
//   2. the sandbox loads its default example and reports "live";
//   3. an edit via the editor seam hot-swaps the scene (pixels change to the
//      pushed unmistakable green) and the status stays "live";
//   4. a broken edit reports the parse error and the old frame keeps
//      rendering;
//   5. a good edit after the broken one recovers;
//   6. every example in the picker loads to "live" and ticks cleanly (the
//      repo examples are copied in at build time — this catches one breaking
//      on wasm);
//   7. the docs page highlights its Functor Lang blocks, and a "try it" button's
//      program loads live in the sandbox (the #src= → player ?src= data-URL
//      path, fresh init);
//   7b. the API reference is readable with NO JavaScript — a plain fetch of
//      /docs/ returns every module and declaration, and /docs/api.json,
//      /docs/api.md and /llms.txt mirror it;
//   8. an inline #src= program with its OWN model shape truly fresh-inits (its
//      init runs — no model carried over from the default example) and ticks
//      cleanly.
//
// Run manually (needs the wasm bundle):
//
//   wasm-pack build runtime/functor-runtime-web --target=web   # once
//   node e2e/site-sandbox.mjs
import { spawn, spawnSync } from "node:child_process";
import { access, readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { chromium } from "@playwright/test";

const PORT = Number(process.env.FUNCTOR_SITE_PORT ?? 8123);
const BASE = `http://127.0.0.1:${PORT}`;
const ROOT = fileURLToPath(new URL("..", import.meta.url));

const GREEN = `let init = { t: 0.0 }
let tick = (model, dt: float, tts: float) => { model with t: model.t + dt }
let draw = (model, tts: float) =>
  Frame.create(
    Camera3D.lookAt(Vec3.make(0.0, 0.0, -6.0), Vec3.make(0.0, 0.0, 0.0)),
    Scene.sphere() |> Scene.emissive(Color.rgb(0.1, 1.0, 0.2)) |> Scene.scale(2.0))
`;
const BROKEN = "let init = {\n";

// An inline program whose model shape matches NO served example (`spin` —
// read in both tick and draw): only a fresh `init` runs it cleanly, so this
// catches the sandbox hot-swapping an inline program onto a foreign model.
const INLINE_SPIN = `let init = { spin: 0.0 }
let tick = (model, dt: float, tts: float) => { model with spin: model.spin + dt }
let draw = (model, tts: float) =>
  Frame.create(
    Camera3D.lookAt(Vec3.make(0.0, 0.0, -6.0), Vec3.make(0.0, 0.0, 0.0)),
    Scene.cube() |> Scene.rotateY(Angle.radians(model.spin)) |> Scene.emissive(Color.rgb(1.0, 0.2, 0.8)))
`;

// A model that deliberately retains a module-bound closure. Its old snapshots
// must not cross a hot reload, but the timeline should keep its frame/viewport
// and show the unavailable prefix rather than collapsing or disappearing.
const CLOSURE_HISTORY = `let offset = (k) => (x) => x + k
let init = { t: 0.0, behavior: offset(1.0) }
let tick = (model, dt: float, tts: float) => { model with t: model.t + dt }
let draw = (model, tts: float) =>
  Frame.create(
    Camera3D.lookAt(Vec3.make(0.0, 0.0, -6.0), Vec3.make(0.0, 0.0, 0.0)),
    Scene.cube() |> Scene.rotateY(Angle.radians(model.t)) |> Scene.emissive(Color.rgb(0.2, 0.8, 1.0)))
`;

// A float-model program for the language-intelligence checks: every top-level
// def has a knowable type (no record-typed `init` — a record's type stays
// Unknown and earns no lens), so all four defs get a signature codelens; the
// two unannotated `model` params get inlay hints; and `speed` hovers to its
// type. Loaded via #src= so it fresh-inits (its float model runs cleanly —
// a hot-swap onto the record-model default would throw at draw).
const INTEL_SRC = `let speed = 2.0
let init = 0.0
let tick = (model, dt: float, tts: float) => model + dt
let draw = (model, tts: float) =>
  Frame.create(
    Camera3D.lookAt(Vec3.make(0.0, 0.0, -6.0), Vec3.make(0.0, 0.0, 0.0)),
    Scene.sphere() |> Scene.emissive(Color.rgb(0.1, 1.0, 0.2))
      |> Scene.rotateY(Angle.radians(model)) |> Scene.scale(speed))
`;

let failures = 0;
const check = (name, ok, detail = "") => {
  console.log(`${ok ? "PASS" : "FAIL"}: ${name}${ok || !detail ? "" : ` — ${detail}`}`);
  if (!ok) failures += 1;
};
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// Build, then serve.
const build = spawnSync("node", ["site/build.mjs"], { cwd: ROOT, stdio: "inherit" });
if (build.status !== 0) process.exit(build.status ?? 1);

// site:build creates this gitignored artifact from the embedded prelude. Read
// expected totals only after that clean-checkout generation step.
const API_REFERENCE = JSON.parse(
  await readFile(`${ROOT}/site/generated/api-reference.json`, "utf8")
);
const API_MODULE_COUNT = API_REFERENCE.modules.length;
const API_ITEM_COUNT = API_REFERENCE.modules.reduce(
  (total, module) => total + module.items.length,
  0
);

// The language-intel pkg is REQUIRED by this suite. build.mjs treats it as
// optional (a site can ship without analysis), but the checks below must not
// silently skip — that is exactly how the editor once shipped degraded while
// CI stayed green.
try {
  await access(`${ROOT}site/dist/pkg/functor_lang_wasm.js`);
} catch {
  console.error(
    "site/dist/pkg/functor_lang_wasm.js missing — build it first: npm run build:lang-wasm"
  );
  process.exit(1);
}

// A occupied port would make serve.mjs die while the readiness probe below
// happily talks to whatever else is listening — fail loud instead.
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

const browser = await chromium.launch();

// Sample the center pixel of a WebGL canvas inside `frame`, copied in a rAF
// callback so it reads the just-rendered buffer (no preserveDrawingBuffer).
const centerPixel = (frame) =>
  frame.evaluate(
    () =>
      new Promise((resolve) => {
        requestAnimationFrame(() => {
          const gl = document.getElementById("canvas");
          const c = document.createElement("canvas");
          c.width = gl.width;
          c.height = gl.height;
          const ctx = c.getContext("2d");
          ctx.drawImage(gl, 0, 0);
          const d = ctx.getImageData((c.width / 2) | 0, (c.height / 2) | 0, 1, 1).data;
          resolve([d[0], d[1], d[2]]);
        });
      })
  );

// Hash a 32×32 downscale of the whole player canvas (drawn in a rAF callback so
// it reads the just-rendered buffer). Two equal hashes ~300ms apart = the frame
// is frozen; a change = it's animating.
const regionHash = (frame) =>
  frame.evaluate(
    () =>
      new Promise((resolve) => {
        requestAnimationFrame(() => {
          const gl = document.getElementById("canvas");
          const c = document.createElement("canvas");
          c.width = 32;
          c.height = 32;
          const ctx = c.getContext("2d");
          ctx.drawImage(gl, 0, 0, 32, 32);
          const d = ctx.getImageData(0, 0, 32, 32).data;
          let h = 0;
          for (let i = 0; i < d.length; i++) h = (h * 31 + d[i]) >>> 0;
          resolve(h);
        });
      })
  );

const playerFrame = (page) => {
  const frame = page.frames().find((f) => f.url().includes("player.html"));
  if (!frame) throw new Error("player iframe not found");
  return frame;
};

// --- 1. Landing page: the hero scene actually renders. ------------------------
{
  const page = await browser.newPage({ viewport: { width: 1024, height: 640 } });
  // Force recurring multi-step render frames while the hero stages. The baked
  // input script must be scheduled inside the runtime's fixed-step loop; a
  // parent-page rAF observer cannot reliably see frames 18/19 under this load.
  await page.addInitScript(() => {
    if (window !== window.top) return;
    window.__heroFrameStall = window.setInterval(() => {
      const until = performance.now() + 55;
      while (performance.now() < until) {
        // Deliberately occupy the page thread.
      }
    }, 80);
  });
  const consoleLog = [];
  page.on("console", (m) => consoleLog.push(m.text()));
  await page.goto(BASE);
  for (let i = 0; !consoleLog.some((m) => m.includes("[functor-lang] loaded")); i++) {
    if (i > 100) throw new Error(`hero never loaded:\n${consoleLog.join("\n")}`);
    await sleep(200);
  }
  await page.waitForFunction(() => window.__hero?.staged(), null, { timeout: 15000 });
  await page.evaluate(() => window.clearInterval(window.__heroFrameStall));
  const heroPlayer = playerFrame(page);
  const pixel = await centerPixel(heroPlayer);
  // Anything the platformer draws at center (sky / hills) differs from the GL
  // clear color rgb(26, 51, 77); "not clear color" = the scene rendered.
  const rendered = Math.abs(pixel[0] - 26) + Math.abs(pixel[1] - 51) + Math.abs(pixel[2] - 77) > 30;
  check("landing hero scene renders", rendered, `center = rgb(${pixel})`);
  const tagline = await page.locator(".hero-sub").textContent();
  check(
    "landing hero uses the engine-wide tagline",
    tagline.trim() === "A free, open-source game engine.",
    JSON.stringify(tagline.trim())
  );

  // The shared player carries the scrubber into the hero iframe too (hidden
  // until history, but the element is present).
  const heroHasScrubber = await heroPlayer.evaluate(
    () => !!document.getElementById("scrubber")
  );
  check("landing hero player has the scrubber element", heroHasScrubber);

  // The hero mini-sandbox: a live editor over the whole tunables trio.
  await page.waitForFunction(
    () => window.__hero && window.__hero.region().includes("let jumpVelocity"),
    { timeout: 10000 }
  );
  const region = await page.evaluate(() => window.__hero.region());
  const excerptHeading = await page.locator(".hero-editor-heading").textContent();
  const excerptLayout = await page.evaluate(() => {
    const scroller = document.querySelector(".hero-editor .cm-scroller");
    const viewport = document.querySelector(".hero-editor-code").getBoundingClientRect();
    const lines = [...document.querySelectorAll(".hero-editor .cm-line")];
    const lastLine = lines.findLast((line) => line.textContent.trim() !== "");
    const lastLineRect = lastLine.getBoundingClientRect();
    const statusRect = document.querySelector(".hero-status").getBoundingClientRect();
    const statusOverlapsLastLine =
      statusRect.left < lastLineRect.right &&
      statusRect.right > lastLineRect.left &&
      statusRect.top < lastLineRect.bottom &&
      statusRect.bottom > lastLineRect.top;
    // The three tunables and their inviting lead comment are the panel's
    // headline: they must sit fully inside the viewport at rest, never
    // needing a scroll to be read.
    const tunableLines = lines.filter((line) =>
      /🔮|let runSpeed =|let jumpVelocity =|let gravity =|let chasmHalf =/.test(
        line.textContent
      )
    );
    const tunablesVisible =
      tunableLines.length === 5 &&
      tunableLines.every((line) => {
        const rect = line.getBoundingClientRect();
        return rect.top >= viewport.top - 1 && rect.bottom <= viewport.bottom + 1;
      });
    return {
      tunablesVisible,
      verticallyVisible: lastLineRect.bottom <= viewport.bottom + 1,
      horizontallyVisible: scroller.scrollWidth <= scroller.clientWidth + 1,
      statusOverlapsLastLine,
    };
  });
  check(
    "hero editor clearly labels a real world-building excerpt",
    excerptHeading.toLowerCase().includes("live excerpt") &&
      excerptHeading.includes("examples/mario/game.fun") &&
      region.includes("let runSpeed") &&
      region.includes("let jumpVelocity") &&
      region.includes("let gravity") &&
      region.includes("let chasmHalf") &&
      // The DERIVED ground geometry stays outside the editable region: the
      // drawn platforms and the collided ground must keep deriving from one
      // chasmHalf, never drift apart under an edit.
      !region.includes("let leftGroundWidth") &&
      region.includes("let world = (model, tts) =>") &&
      region.includes("Sprite.group") &&
      !region.includes("let init"),
    region.slice(0, 40)
  );
  const sourceLink = await page.evaluate(() => {
    const link = document.querySelector(".hero-editor-source");
    return link && { href: link.getAttribute("href"), title: link.title, text: link.textContent.trim() };
  });
  check(
    "hero excerpt label deep-links into the sandbox with this example",
    sourceLink?.href === "sandbox.html?example=mario" &&
      sourceLink.text === "examples/mario/game.fun" &&
      sourceLink.title.length > 0,
    JSON.stringify(sourceLink)
  );
  check(
    "hero editor shows the tunables uncropped at rest",
    excerptLayout.tunablesVisible,
    JSON.stringify(excerptLayout)
  );
  check(
    "hero editor shows every meaningful excerpt line without overflow",
    excerptLayout.verticallyVisible &&
      excerptLayout.horizontallyVisible &&
      !excerptLayout.statusOverlapsLastLine,
    JSON.stringify(excerptLayout)
  );

  // The loader has already driven the checked-in input script, paused, and
  // parked immediately before Up-down. Extrapolation remains OFF so the real
  // crystal button is the promised one-click reveal.
  const staged = await heroPlayer.evaluate(() => {
    const events = window.__scrub.events();
    const jump = events.find((event) => event.label === "Up down");
    return {
      paused: window.__scrub.paused(),
      frame: window.__scrub.frame(),
      labels: events.map((event) => event.label),
      jumpFrame: jump?.frame,
      preview: window.__scrub.model().preview,
    };
  });
  check(
    "landing hero bakes the complete jump input into its timeline",
    ["Right down", "Up down", "Up up", "Right up"].every((label) =>
      staged.labels.includes(label)
    ),
    JSON.stringify(staged)
  );
  check(
    "landing hero parks two frames before takeoff with extrapolation off",
    staged.paused &&
      staged.jumpFrame !== undefined &&
      staged.frame === staged.jumpFrame - 2 &&
      staged.preview.enabled === false &&
      staged.preview.seconds === 1.3 &&
      staged.preview.rate === 8,
    JSON.stringify(staged)
  );

  // The bar's grammar: transport on the left of the rail, ways to LOOK at the
  // parked frame on its right — 📷 immediately before 🔮. The hero also trades
  // ⏭ (which reads as fast-forward here) for ↺, its re-park control.
  const bar = await heroPlayer.evaluate(() => {
    const camera = document.getElementById("scrub-camera");
    const step = document.getElementById("scrub-step");
    const reset = document.getElementById("scrub-reset");
    return {
      afterCamera: camera.nextElementSibling?.id,
      cameraHidden: camera.hidden,
      stepHidden: step.hidden,
      resetHidden: reset.hidden,
      resetGlyph: reset.textContent,
      attention: document
        .getElementById("scrub-extrapolate")
        .classList.contains("attention"),
    };
  });
  check(
    "scrubber puts the debug camera immediately left of extrapolation",
    !bar.cameraHidden && bar.afterCamera === "scrub-extrapolate",
    JSON.stringify(bar)
  );
  check(
    "hero bar offers ↺ reset instead of ⏭ step",
    bar.stepHidden && !bar.resetHidden && bar.resetGlyph === "↺",
    JSON.stringify(bar)
  );
  check("staged hero pulses attention on 🔮", bar.attention, JSON.stringify(bar));

  // Game input against a paused clock says so instead of doing nothing — but
  // only for keys that would have reached the game. A nudge on the timeline
  // handle is chrome, not game input.
  const pressInPlayer = (code, onPlayhead, repeat = false) =>
    heroPlayer.evaluate(
      ([keyCode, chrome, held]) => {
        const target = chrome ? document.getElementById("scrub-playhead") : document.body;
        target.dispatchEvent(
          new KeyboardEvent("keydown", { code: keyCode, bubbles: true, repeat: held })
        );
        const raised = document.getElementById("scrub-toast").classList.contains("show");
        // Release it: the host page delivers held keys to the game, and a
        // press with no matching release would leave the character running.
        target.dispatchEvent(new KeyboardEvent("keyup", { code: keyCode, bubbles: true }));
        return raised;
      },
      [code, onPlayhead, repeat]
    );
  // Past the staging-silence window: the notice deliberately says nothing for
  // 300ms after a pause EDGE, so the hero's own programmatic park never flashes
  // it. Staging finished moments ago, so settle before pressing.
  await sleep(400);
  const toastOnChrome = await pressInPlayer("ArrowRight", true);
  const toastOnGameKey = await pressInPlayer("ArrowRight", false);
  check(
    "paused game input raises the scrubber's paused notice",
    !toastOnChrome && toastOnGameKey,
    JSON.stringify({ toastOnChrome, toastOnGameKey })
  );
  await sleep(2000);
  check(
    "the paused notice dismisses itself",
    !(await heroPlayer.evaluate(() =>
      document.getElementById("scrub-toast").classList.contains("show")
    ))
  );
  // Auto-repeat is one held press, not a stream: it must not re-arm the notice
  // (which would pin it open while the key is down). A letter also counts —
  // the host router delivers every letter and digit, not just WASD/arrows.
  const toastOnRepeat = await pressInPlayer("KeyQ", false, true);
  const toastOnLetter = await pressInPlayer("KeyQ", false);
  check(
    "the paused notice ignores auto-repeat but covers any game key",
    !toastOnRepeat && toastOnLetter,
    JSON.stringify({ toastOnRepeat, toastOnLetter })
  );

  await heroPlayer.evaluate(() => document.getElementById("scrub-extrapolate").click());
  check(
    "using 🔮 retires its attention pulse for good",
    await heroPlayer.evaluate(() => {
      window.__scrub.setAttention({ extrapolate: true });
      return !document.getElementById("scrub-extrapolate").classList.contains("attention");
    })
  );
  await heroPlayer.waitForFunction(() => window.__scrub.model().preview.enabled, {
    timeout: 3000,
  });
  await sleep(500);
  const strongJumpHash = await regionHash(heroPlayer);
  check(
    "one click enables the platformer's extrapolation",
    await heroPlayer.evaluate(() => window.__scrub.view().previewFrames > 0)
  );

  // Push an edited region and wait for the runtime to accept it (a fresh
  // reload-ok marker) and the panel to report live again.
  const applyHeroRegion = async (src) => {
    const reloadsBefore = await heroPlayer.evaluate(
      () => window.__scrub.events().filter((event) => event.kind === "reload-ok").length
    );
    await page.evaluate((s) => window.__hero.setRegion(s), src);
    await heroPlayer.waitForFunction(
      (before) =>
        window.__scrub.events().filter((event) => event.kind === "reload-ok").length > before,
      reloadsBefore,
      { timeout: 8000 }
    );
    await page.waitForFunction(() => window.__hero.status().state === "live", null, {
      timeout: 8000,
    });
    await sleep(700);
  };

  // Reload juice reports EDITS, not history. Staging replayed a whole input
  // script and loaded the program before any of this ran, so nothing may have
  // flashed yet — the overlay is built lazily on the first real flash, so its
  // absence is the baseline guard.
  check(
    "hero staging lands without flashing the viewport",
    await heroPlayer.evaluate(() => !document.querySelector(".scrub-reload-juice")),
    await heroPlayer.evaluate(
      () => document.querySelector(".scrub-reload-juice")?.className ?? "absent"
    )
  );

  // Weakening the jump rebuilds the predicted (pink) future under the edited
  // code. The anchor stays parked, so the recorded (cyan) past is unchanged;
  // only the extrapolated trajectory ahead of it moves.
  const weakRegion = region.replace("let jumpVelocity = 13.0", "let jumpVelocity = 10.0");
  const rangeBeforeWeak = await heroPlayer.evaluate(() => Array.from(window.__scrub.range()));
  await applyHeroRegion(weakRegion);
  const weakJumpHash = await regionHash(heroPlayer);
  check(
    "editing jumpVelocity redraws the projected trajectory",
    weakJumpHash !== strongJumpHash,
    `${strongJumpHash} -> ${weakJumpHash}`
  );

  check(
    "an accepted hero edit flashes the viewport cyan",
    await heroPlayer.evaluate(() => {
      const overlay = document.querySelector(".scrub-reload-juice");
      return !!overlay && overlay.classList.contains("live") && !overlay.classList.contains("rejected");
    }),
    await heroPlayer.evaluate(
      () => document.querySelector(".scrub-reload-juice")?.className ?? "absent"
    )
  );

  // The clustering case, and the reason the glow is driven by hand: this second
  // edit lands at the SAME parked frame, so the timeline folds it into the
  // existing reload marker instead of inserting a node. A CSS insertion
  // animation would never run again — clearing `born` first proves the code
  // re-triggers it on the reused node.
  const markersBeforeReglow = await heroPlayer.evaluate(() => {
    for (const node of document.querySelectorAll(".scrub-event.born")) {
      node.classList.remove("born");
    }
    return document.querySelectorAll(".scrub-event.reload").length;
  });

  // gravity is editable in the same region: it must move the projection too.
  const heavyRegion = weakRegion.replace("let gravity = 30.0", "let gravity = 45.0");
  await applyHeroRegion(heavyRegion);
  const reglow = await heroPlayer.evaluate(() => {
    const born = [...document.querySelectorAll(".scrub-event.born")];
    // The glow must land on the cluster holding the newest reload, not merely
    // on some reload marker: with one marker on the rail a "nearest" search
    // can't be wrong, so pin the identity explicitly.
    const reloads = window.__scrub.events().filter((event) => event.kind.startsWith("reload-"));
    const newestFrame = reloads.length ? reloads[reloads.length - 1].frame : null;
    const cluster = window.__scrub
      .view()
      .eventMarkers.find((m) => m.category === "reload" && m.lastFrame === newestFrame);
    const glowedId = born[0]?.closest("[data-event-id]")?.dataset.eventId;
    return {
      born: born.length,
      markers: document.querySelectorAll(".scrub-event.reload").length,
      newestFrame,
      clusterId: cluster ? String(cluster.id) : null,
      glowedId: glowedId ?? null,
    };
  });
  check(
    "a repeat hero edit re-glows its clustered reload marker",
    reglow.born === 1 &&
      reglow.markers === markersBeforeReglow &&
      reglow.clusterId !== null &&
      reglow.glowedId === reglow.clusterId,
    JSON.stringify({ markersBeforeReglow, ...reglow })
  );
  const heavyJumpHash = await regionHash(heroPlayer);
  check(
    "editing gravity redraws the projected trajectory",
    heavyJumpHash !== weakJumpHash,
    `${weakJumpHash} -> ${heavyJumpHash}`
  );

  // chasmHalf is the level's one upstream geometry knob: widening it must move
  // the drawn platforms AND the ground the projection falls through, together.
  const wideRegion = heavyRegion.replace("let chasmHalf = 3.0", "let chasmHalf = 4.5");
  await applyHeroRegion(wideRegion);
  const wideJumpHash = await regionHash(heroPlayer);
  check(
    "editing chasmHalf redraws the level and its projection",
    wideJumpHash !== heavyJumpHash,
    `${heavyJumpHash} -> ${wideJumpHash}`
  );

  // A broken edit (unbalanced paren): error surfaced, old preview keeps drawing.
  await page.evaluate((s) => window.__hero.setRegion(s), `${wideRegion}\n(`);
  await page.waitForFunction(() => window.__hero.status().state === "error", {
    timeout: 8000,
  });
  await sleep(300);
  const brokenJumpHash = await regionHash(heroPlayer);
  check(
    "hero broken edit keeps the last good projected trajectory",
    brokenJumpHash === wideJumpHash,
    `${wideJumpHash} -> ${brokenJumpHash}`
  );
  // The rejection marker publishes on the runtime's next poll, a frame or two
  // after the panel reports the error; wait for the flash rather than racing it.
  await heroPlayer
    .waitForFunction(
      () => document.querySelector(".scrub-reload-juice")?.classList.contains("rejected"),
      { timeout: 5000 }
    )
    .catch(() => {});
  check(
    "a rejected hero edit flashes the viewport red",
    await heroPlayer.evaluate(() => {
      const overlay = document.querySelector(".scrub-reload-juice");
      return !!overlay && overlay.classList.contains("rejected") && !overlay.classList.contains("live");
    }),
    await heroPlayer.evaluate(
      () => document.querySelector(".scrub-reload-juice")?.className ?? "absent"
    )
  );

  // Recover with the original tunables. The deliberately paused timeline
  // remains parked and retains the whole recorded script across every safe
  // reload.
  await applyHeroRegion(region);
  const recovered = await heroPlayer.evaluate(() => ({
    paused: window.__scrub.paused(),
    frame: window.__scrub.frame(),
    range: Array.from(window.__scrub.range()),
    previewEnabled: window.__scrub.model().preview.enabled,
  }));
  check(
    "hero edits preserve the staged paused timeline",
    recovered.paused &&
      recovered.frame === staged.frame &&
      recovered.range[0] === rangeBeforeWeak[0] &&
      recovered.range[1] >= rangeBeforeWeak[1] &&
      recovered.previewEnabled,
    JSON.stringify({ staged, rangeBeforeWeak, recovered })
  );

  // A rejected scheduler batch is transactional: the valid first edge must
  // not leak through when a later frame cannot be represented by the runtime.
  await heroPlayer.evaluate((end) => window.__scrub.seek(end), recovered.range[1]);
  await heroPlayer.waitForFunction(
    (end) => window.__scrub.frame() === end,
    recovered.range[1],
    { timeout: 8000 }
  );
  const beforeRejectedBatch = await heroPlayer.evaluate(() => ({
    frame: window.__scrub.frame(),
    inputs: window.__scrub.events().filter((event) => event.kind === "input").length,
    accepted: window.__scrub.scheduleKeyInputs([
      { frame: 0, code: 30, isDown: true },
      { frame: 1e30, code: 30, isDown: false },
    ]),
  }));
  await heroPlayer.evaluate(() => window.__scrub.togglePause());
  await heroPlayer.waitForFunction(
    (frame) => window.__scrub.frame() >= frame + 4,
    beforeRejectedBatch.frame,
    { timeout: 8000 }
  );
  await heroPlayer.evaluate(() => window.__scrub.togglePause());
  const afterRejectedBatch = await heroPlayer.evaluate(
    () => window.__scrub.events().filter((event) => event.kind === "input").length
  );
  check(
    "rejected hero input batches enqueue no partial key edges",
    !beforeRejectedBatch.accepted && afterRejectedBatch === beforeRejectedBatch.inputs,
    JSON.stringify({ beforeRejectedBatch, afterRejectedBatch })
  );

  // ↺ re-parks the demo after the visitor has scrubbed (and edited) away: it
  // seeks back to the staged anchor, paused, over the live edited program.
  const scrubbedAway = await heroPlayer.evaluate(() => {
    const range = window.__scrub.range();
    window.__scrub.seek(range[1]);
    return range[1];
  });
  await heroPlayer.waitForFunction(
    (end) => window.__scrub.frame() === end,
    scrubbedAway,
    { timeout: 8000 }
  );
  await heroPlayer.evaluate(() => document.getElementById("scrub-reset").click());
  await heroPlayer
    .waitForFunction(
      (anchor) => window.__scrub.paused() && window.__scrub.frame() === anchor,
      staged.frame,
      { timeout: 8000 }
    )
    .catch(() => {});
  const afterReset = await heroPlayer.evaluate(() => ({
    frame: window.__scrub.frame(),
    paused: window.__scrub.paused(),
  }));
  check(
    "hero ↺ re-parks the staged moment after scrubbing away",
    afterReset.paused && afterReset.frame === staged.frame,
    JSON.stringify({ scrubbedAway, staged: staged.frame, afterReset })
  );

  // The visitor's other path: RESUME, let it run, then ↺. Reset must land
  // paused two frames before a real takeoff — the staged anchor while it is
  // still recorded, a freshly re-recorded one once the ring has dropped it.
  // Either way the demo is parked and ready for 🔮, never on a stray frame.
  await heroPlayer.evaluate(() => window.__scrub.togglePause());
  await heroPlayer.waitForFunction(() => !window.__scrub.paused(), { timeout: 8000 });
  await sleep(2500);
  await heroPlayer.evaluate(() => document.getElementById("scrub-reset").click());
  await heroPlayer
    .waitForFunction(
      () => {
        if (!window.__scrub.paused()) return false;
        const range = window.__scrub.range();
        const jumps = window.__scrub
          .events()
          .filter((event) => event.label === "Up down" && event.frame >= range[0]);
        return jumps.some((jump) => window.__scrub.frame() === Math.max(0, jump.frame - 2));
      },
      null,
      { timeout: 30000 }
    )
    .catch(() => {});
  const afterRunReset = await heroPlayer.evaluate(() => {
    const range = Array.from(window.__scrub.range());
    return {
      frame: window.__scrub.frame(),
      paused: window.__scrub.paused(),
      range,
      jumps: window.__scrub
        .events()
        .filter((event) => event.label === "Up down" && event.frame >= range[0])
        .map((event) => event.frame),
    };
  });
  check(
    "hero ↺ re-parks before a recorded takeoff after the demo has run",
    afterRunReset.paused &&
      afterRunReset.jumps.some((jump) => afterRunReset.frame === Math.max(0, jump - 2)),
    JSON.stringify(afterRunReset)
  );

  await page.close();
}

// --- 2–5. Sandbox: load, live edit, broken edit, recover. ---------------------
{
  const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
  await page.goto(`${BASE}/sandbox.html`);

  await page.waitForFunction(
    () => window.__sandbox && window.__sandbox.status().state === "live",
    { timeout: 30000 }
  );
  check("sandbox loads the default example to live", true);

  // Live edit → unmistakable green sphere.
  await page.evaluate((s) => window.__sandbox.setSource(s), GREEN);
  await page.waitForFunction(
    () => window.__sandbox.status().message.includes("model preserved"),
    { timeout: 5000 }
  );
  await sleep(400);
  const green = await centerPixel(playerFrame(page));
  check("live edit repaints the scene green", green[1] > 150 && green[0] < 100, `center = rgb(${green})`);

  // Broken edit → error surfaced, old frame keeps rendering.
  await page.evaluate((s) => window.__sandbox.setSource(s), BROKEN);
  // The preview holds an error back (~4s grace) before surfacing it.
  await page.waitForFunction(() => window.__sandbox.status().state === "error", {
    timeout: 8000,
  });
  const status = await page.evaluate(() => window.__sandbox.status());
  check("broken edit surfaces the parse error", /cannot .*:\d+:\d+/.test(status.message), status.message);
  await sleep(400);
  const still = await centerPixel(playerFrame(page));
  check("old program keeps rendering after a broken edit", still[1] > 150 && still[0] < 100, `center = rgb(${still})`);

  // Recovery.
  await page.evaluate((s) => window.__sandbox.setSource(s), GREEN);
  await page.waitForFunction(() => window.__sandbox.status().state === "live", {
    timeout: 5000,
  });
  check("edit after a broken edit recovers to live", true);

  await page.close();
}

// --- 6. Every example loads and ticks cleanly. ---------------------------------
// Derived from the live picker (not a hardcoded list) so a newly-added example
// is covered automatically — this is the guard that a repo example still runs
// on wasm once it's wired into the sandbox dropdown.
const examples = await (async () => {
  const page = await browser.newPage();
  await page.goto(`${BASE}/sandbox.html`);
  await page.waitForFunction(() => document.getElementById("example-picker")?.options.length > 0);
  const ids = await page.evaluate(() =>
    Array.from(document.getElementById("example-picker").options)
      .map((o) => o.value)
      .filter((v) => v !== "__inline")
  );
  await page.close();
  return ids;
})();
check("picker exposes the expanded example set", examples.length >= 10, examples.join(", "));
// Duplicate ids would silently overwrite each other's dist/examples/<id>.fun and
// under-test the set — a unique-count mismatch is a real drift bug, not a nit.
check("picker example ids are unique", new Set(examples).size === examples.length, examples.join(", "));
for (const example of examples) {
  const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
  const consoleLog = [];
  page.on("console", (m) => consoleLog.push(m.text()));
  await page.goto(`${BASE}/sandbox.html?example=${example}`);
  try {
    await page.waitForFunction(
      () => window.__sandbox && window.__sandbox.status().state === "live",
      { timeout: 30000 }
    );
    await sleep(700);
    const errors = consoleLog.filter((m) => m.includes("[functor-lang]") && m.includes("error"));
    check(`example '${example}' loads live and ticks cleanly`, errors.length === 0, errors.join("\n"));
    if (example === "animation") {
      const player = playerFrame(page);
      check(
        "head-look example keeps an absolute visible pointer",
        player.url().includes("cursor=visible"),
        player.url()
      );
      await player.evaluate(() => {
        const rect = document.getElementById("canvas").getBoundingClientRect();
        window.dispatchEvent(new MouseEvent("mousemove", {
          clientX: rect.left + rect.width * 0.9,
          clientY: rect.top + rect.height * 0.25,
        }));
      });
      await player.waitForFunction(
        () => window.__scrub.events().some((event) => event.kind === "mouse-move"),
        { timeout: 3000 }
      );
      const mouseMove = await player.evaluate(() =>
        window.__scrub.events().findLast((event) => event.kind === "mouse-move")
      );
      check(
        "visible pointer movement reaches the head-look runtime",
        mouseMove?.label.startsWith("mouse move ("),
        JSON.stringify(mouseMove)
      );
    }
  } catch {
    check(`example '${example}' loads live and ticks cleanly`, false, consoleLog.slice(-5).join("\n"));
  }
  await page.close();
}

// --- 7. Manual + generated API reference. -------------------------------------
{
  const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
  await page.goto(`${BASE}/manual/`);
  const highlighted = await page.locator("pre.functor-lang span.tok-k").count();
  const tryButtons = await page.locator("a.try-button").count();
  check("manual highlights Functor Lang blocks", highlighted > 10, `${highlighted} keyword spans`);
  check("manual offers try-it buttons", tryButtons >= 4, `${tryButtons} buttons`);

  // Follow the first try-it link in THIS page (target=_blank would detach).
  const href = await page.locator("a.try-button").first().getAttribute("href");
  await page.goto(`${BASE}/${href}`);
  try {
    await page.waitForFunction(
      () => window.__sandbox && window.__sandbox.status().state === "live",
      { timeout: 30000 }
    );
    await sleep(400);
    const pixel = await centerPixel(playerFrame(page));
    // The first runnable is the magenta spinning cube on the GL clear color —
    // just assert something got drawn (not solid clear color everywhere is
    // hard to probe; the live status is the main assertion).
    check("manual try-it program loads live in the sandbox", true, `center = rgb(${pixel})`);
  } catch {
    check("manual try-it program loads live in the sandbox", false, href);
  }

  await page.goto(`${BASE}/docs/`);
  await page.waitForSelector(".api-item");
  const modules = await page.locator(".api-module").count();
  const declarations = await page.locator(".api-item").count();
  check(
    "API reference renders every generated module",
    modules === API_MODULE_COUNT,
    `${modules}/${API_MODULE_COUNT} modules`
  );
  check(
    "API reference renders every generated declaration",
    declarations === API_ITEM_COUNT,
    `${declarations}/${API_ITEM_COUNT} declarations`
  );
  const inlineCode = await page.locator("#api-scene-littexture p code").allTextContents();
  check(
    "API reference renders Markdown backticks as inline code",
    inlineCode.includes("Texture.t") && inlineCode.includes("Asset.Texture"),
    inlineCode.join(", ")
  );
  await page.locator("#api-search").fill("Scene.rotateY");
  const visibleDeclarations = await page.locator(".api-item:visible").count();
  const targetVisible = await page.locator("#api-scene-rotatey:visible").count();
  check(
    "API reference search finds the target and narrows declarations",
    targetVisible === 1 && visibleDeclarations > 0 && visibleDeclarations < declarations,
    `${visibleDeclarations}/${declarations} visible; target=${targetVisible}`
  );

  await page.goto(`${BASE}/docs.html#get-started`);
  await page.waitForURL(/\/manual\/#get-started$/);
  check("legacy docs.html preserves manual anchors", true, page.url());
  await page.close();
}

// --- 7b. The API reference is readable with NO JavaScript. --------------------
// Functor is meant to be LLM-native, and an agent reads the docs with a plain
// HTTP GET. These use `fetch`, not the browser, so they fail if the reference
// ever regresses to being assembled client-side.
{
  // An absolute floor first: the count assertions below compare the page
  // against the same JSON it was rendered from, so an empty reference would
  // satisfy them (0 === 0) while shipping a blank page.
  check(
    "the generated reference is non-trivial",
    API_MODULE_COUNT > 10 && API_ITEM_COUNT > 50,
    `${API_MODULE_COUNT} modules, ${API_ITEM_COUNT} declarations`
  );

  const docsHtml = await (await fetch(`${BASE}/docs/`)).text();
  const staticModules = docsHtml.match(/class="api-module"/g)?.length ?? 0;
  const staticItems = docsHtml.match(/class="api-declaration"/g)?.length ?? 0;
  check(
    "no-JS GET of /docs/ contains every module",
    staticModules === API_MODULE_COUNT,
    `${staticModules}/${API_MODULE_COUNT} modules in the raw HTML`
  );
  check(
    "no-JS GET of /docs/ contains every declaration",
    staticItems === API_ITEM_COUNT,
    `${staticItems}/${API_ITEM_COUNT} declarations in the raw HTML`
  );
  // A signature an agent would actually look up, present as real text.
  check(
    "no-JS GET of /docs/ contains real signatures",
    docsHtml.includes("Scene.rotateY") && docsHtml.includes("Angle.t"),
    "Scene.rotateY / Angle.t"
  );

  for (const [path, verify] of [
    [
      "/docs/api.json",
      (body) => JSON.parse(body).modules.length === API_MODULE_COUNT,
    ],
    ["/docs/api.md", (body) => body.includes("# Functor API reference") && body.includes("## Scene")],
    ["/llms.txt", (body) => body.includes("/docs/api.md") && body.includes("/docs/api.json")],
  ]) {
    const response = await fetch(`${BASE}${path}`);
    const body = await response.text();
    let ok = false;
    try {
      ok = response.ok && verify(body);
    } catch {}
    check(`${path} serves the machine-readable reference`, ok, `HTTP ${response.status}`);
  }
}

// --- 8. Inline #src= program with its OWN model shape fresh-inits. -------------
{
  const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
  const consoleLog = [];
  page.on("console", (m) => consoleLog.push(m.text()));
  const b64u = Buffer.from(INLINE_SPIN).toString("base64url");
  await page.goto(`${BASE}/sandbox.html#src=${b64u}`);
  const name = "inline program with its own model shape fresh-inits and ticks cleanly";
  try {
    await page.waitForFunction(
      () => window.__sandbox && window.__sandbox.status().state === "live",
      { timeout: 30000 }
    );
    await sleep(700);
    // A hot-swap onto the default example's model would blow up on
    // `model.spin` every frame; a fresh init ticks with no runtime errors.
    const errors = consoleLog.filter((m) => m.includes("[functor-lang]") && m.includes("error"));
    check(name, errors.length === 0, errors.join("\n"));
  } catch {
    check(name, false, consoleLog.slice(-5).join("\n"));
  }
  await page.close();
}

// --- 9. Time-travel scrubber drives/observes the player via __scrub. ----------
{
  const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
  const scrubConsole = [];
  page.on("console", (message) => scrubConsole.push(message.text()));
  page.on("pageerror", (error) => scrubConsole.push(`pageerror: ${error.message}`));
  await page.goto(`${BASE}/sandbox.html?example=bounce`);
  await page.waitForFunction(
    () => window.__sandbox && window.__sandbox.status().state === "live",
    { timeout: 30000 }
  );
  const player = playerFrame(page);

  // The seam appears once the scrubber is wired; history then accrues as the
  // scene ticks. The sandbox player boots with ?scrubber=hidden — the seam
  // without the bar — and the HOST page's chrono bar is the one transport.
  await player.waitForFunction(() => window.__scrub, { timeout: 10000 });
  const chronoReplacesScrubber = await player.evaluate(
    () => !!window.__scrub && !document.getElementById("scrubber")
  );
  const chronoVisible = await page.evaluate(
    () => getComputedStyle(document.querySelector(".mp-chrono")).display === "flex"
  );
  check(
    "sandbox chrono bar replaces the in-frame scrubber",
    chronoReplacesScrubber && chronoVisible,
    JSON.stringify({ chronoReplacesScrubber, chronoVisible })
  );
  const customHandles = await page.evaluate(() =>
    ["mp-playhead", "mp-preview-handle"].every((id) => {
      const handle = document.getElementById(id);
      return handle?.getAttribute("role") === "slider" && handle.tabIndex === 0;
    })
  );
  check("chrono bar exposes two keyboard-focusable slider handles", customHandles);
  // Same grammar as the in-frame scrubber: transport left of the rail, the
  // two ways to LOOK at the parked frame right of it.
  const chronoOrder = await page.evaluate(() => {
    const ids = [...document.querySelectorAll(".mp-chrono > button")].map((b) => b.id);
    return { ids, afterCamera: document.getElementById("mp-camera").nextElementSibling?.id };
  });
  check(
    "chrono bar orders the debug camera immediately left of extrapolation",
    chronoOrder.afterCamera === "mp-extrap" &&
      chronoOrder.ids.indexOf("mp-step") < chronoOrder.ids.indexOf("mp-camera"),
    JSON.stringify(chronoOrder)
  );
  const handleColors = await page.evaluate(() => ({
    playhead: getComputedStyle(document.getElementById("mp-playhead")).backgroundColor,
    preview: getComputedStyle(document.getElementById("mp-preview-handle")).backgroundColor,
  }));
  check(
    "chrono bar handles keep their solid cyan and pink fills",
    handleColors.playhead === "rgb(65, 216, 230)" &&
      handleColors.preview === "rgb(232, 88, 184)",
    JSON.stringify(handleColors)
  );

  await player.waitForFunction(() => window.__scrub.range().length === 2, {
    timeout: 10000,
  });

  // The recorded range grows while running.
  const r0 = await player.evaluate(() => window.__scrub.range());
  await sleep(500);
  const r1 = await player.evaluate(() => window.__scrub.range());
  check("scrubber range grows while running", r1[1] > r0[1], `${r0} -> ${r1}`);

  // Extrapolation is a live mode: its second handle follows the advancing tail
  // by a fixed logical window. Pausing freezes the anchor; it does not enable
  // the control or the renderer.
  await page.evaluate(() => document.getElementById("mp-extrap").click());
  await player.evaluate(() => window.__scrub.setPreview({ seconds: 2 }));
  await page.waitForFunction(
    () => getComputedStyle(document.getElementById("mp-preview-handle")).display === "block",
    null,
    { timeout: 3000 }
  );
  const livePreview0 = await player.evaluate(() => window.__scrub.view());
  await sleep(300);
  const livePreview1 = {
    view: await player.evaluate(() => window.__scrub.view()),
    ...(await page.evaluate(() => ({
      handleVisible:
        getComputedStyle(document.getElementById("mp-preview-handle")).display === "block",
      endpointClipped: document
        .getElementById("mp-preview-handle")
        .classList.contains("fully-clipped"),
    }))),
  };
  check(
    "live extrapolation keeps its pink endpoint tracking the live tail",
    !livePreview0.paused &&
      !livePreview1.view.paused &&
      livePreview1.handleVisible &&
      livePreview1.endpointClipped &&
      livePreview1.view.selectedFrame > livePreview0.selectedFrame &&
      livePreview0.previewEndFrame - livePreview0.selectedFrame === 120 &&
      livePreview1.view.previewEndFrame - livePreview1.view.selectedFrame === 120,
    JSON.stringify({ livePreview0, livePreview1 })
  );

  // Markers come from the authoritative runtime log: a real recorded key edge,
  // followed by a real hot-reload boundary from the editor bridge.
  await player.evaluate(() => {
    window.dispatchEvent(new KeyboardEvent("keydown", { code: "Space" }));
    window.dispatchEvent(new KeyboardEvent("keyup", { code: "Space" }));
  });
  await player.waitForFunction(
    () => window.__scrub.events().some((event) => event.kind === "key-down"),
    { timeout: 3000 }
  );
  const inputMarker = await page
    .waitForFunction(() => !!document.querySelector("#mp-markers .mp-evt.input"), null, {
      timeout: 3000,
    })
    .then(() => true)
    .catch(() => false);
  check("timeline renders recorded input markers", inputMarker);
  const accessibleInputMarkers = await page
    .getByRole("button", { name: /frame \d+ · Space down/ })
    .count();
  check(
    "timeline markers are present in the accessibility tree",
    accessibleInputMarkers > 0
  );

  const rangeBeforeSafeReload = await player.evaluate(() => window.__scrub.range());
  await page.evaluate(() =>
    window.__sandbox.setSource(`${window.__sandbox.getSource()}\n// timeline reload marker`)
  );
  await player.waitForFunction(
    () => window.__scrub.events().some((event) => event.kind === "reload-ok"),
    { timeout: 5000 }
  );
  const reloadMarker = await page
    .waitForFunction(() => !!document.querySelector("#mp-markers .mp-evt.reload"), null, {
      timeout: 3000,
    })
    .then(() => true)
    .catch(() => false);
  check("timeline renders successful hot-reload boundaries", reloadMarker);
  // The viewport flash belongs to the SEAM, not the bar: this player mounts
  // with ?scrubber=hidden (no chrome, no bar stylesheet), and the sandbox edit
  // above must still light its pane. Proves the flash carries its own styling.
  const sandboxFlash = await player
    .waitForFunction(
      () => document.querySelector(".scrub-reload-juice")?.classList.contains("live"),
      { timeout: 5000 }
    )
    .then(() => true)
    .catch(() => false);
  const sandboxFlashStyled = await player.evaluate(() => {
    const overlay = document.querySelector(".scrub-reload-juice");
    if (!overlay) return null;
    const style = getComputedStyle(overlay);
    return { position: style.position, shadow: style.boxShadow.includes("rgb") };
  });
  check(
    "a sandbox edit flashes its hidden-mounted pane",
    sandboxFlash &&
      sandboxFlashStyled?.position === "fixed" &&
      sandboxFlashStyled?.shadow === true,
    JSON.stringify({ sandboxFlash, sandboxFlashStyled })
  );
  const rangeAfterSafeReload = await player.evaluate(() => window.__scrub.range());
  check(
    "plain-data history remains seekable across a hot reload",
    rangeAfterSafeReload[0] === rangeBeforeSafeReload[0] &&
      rangeAfterSafeReload[1] >= rangeBeforeSafeReload[1],
    `${rangeBeforeSafeReload} -> ${rangeAfterSafeReload}`
  );
  await player.waitForFunction(
    () => {
      const range = window.__scrub.range();
      return range.length === 2 && range[1] - range[0] >= 30;
    },
    { timeout: 3000 }
  );

  // Pause freezes both the frame counter AND the pixels.
  await player.evaluate(() => window.__scrub.togglePause());
  await player.waitForFunction(() => window.__scrub.paused(), { timeout: 3000 });
  const f0 = await player.evaluate(() => window.__scrub.frame());
  const h0 = await regionHash(player);
  await sleep(300);
  const f1 = await player.evaluate(() => window.__scrub.frame());
  const h1 = await regionHash(player);
  check("pause freezes the frame counter", f0 === f1, `${f0} -> ${f1}`);
  check("pause freezes the pixels", h0 === h1, `hash ${h0} -> ${h1}`);

  // Preview duration changes the logical second endpoint, but never the paused
  // viewport. At the live tail the endpoint is clipped and advertised as such.
  const previewBefore = await player.evaluate(() => window.__scrub.view());
  await player.evaluate(() => window.__scrub.setPreview({ enabled: true, seconds: 5 }));
  const previewAfter = await player.evaluate(() => window.__scrub.view());
  check(
    "preview changes keep the paused timeline domain fixed",
    previewBefore.viewport.lo === previewAfter.viewport.lo &&
      previewBefore.viewport.hi === previewAfter.viewport.hi,
    `${JSON.stringify(previewBefore.viewport)} -> ${JSON.stringify(previewAfter.viewport)}`
  );
  check(
    "off-rail extrapolation is clipped without shortening the logical preview",
    previewAfter.previewEndFrame > previewAfter.viewport.hi && previewAfter.previewClippedFrames > 0,
    JSON.stringify(previewAfter)
  );
  const transportAccessibility = await page.evaluate(() => ({
    pause: document.getElementById("mp-pause").getAttribute("aria-label"),
    extrapolating: document.getElementById("mp-extrap").getAttribute("aria-pressed"),
  }));
  check(
    "transport and extrapolation expose their current state accessibly",
    transportAccessibility.pause === "Resume" && transportAccessibility.extrapolating === "true",
    JSON.stringify(transportAccessibility)
  );
  // (#mp-playhead is pointer-events:none, so it can never be the hit itself —
  // the invariant is that the clipped endpoint does not sit over it.)
  const clippedHandlesRemainIndependent = await page.evaluate(() => {
    const playhead = document.getElementById("mp-playhead");
    const preview = document.getElementById("mp-preview-handle");
    const rect = playhead.getBoundingClientRect();
    return (
      preview.classList.contains("fully-clipped") &&
      document.elementFromPoint(rect.left + rect.width / 2, rect.top + rect.height / 2) !== preview
    );
  });
  check(
    "a fully clipped preview endpoint does not cover the playhead",
    clippedHandlesRemainIndependent
  );

  const frozenBeforeStep = await player.evaluate(() => window.__scrub.view());
  await player.evaluate(() => window.__scrub.step());
  await player.waitForFunction(
    (frame) => window.__scrub.frame() === frame + 1,
    frozenBeforeStep.selectedFrame,
    { timeout: 3000 }
  );
  const frozenAfterStep = await player.evaluate(() => window.__scrub.view());
  check(
    "step advances logically without moving the frozen paused endpoint",
    frozenAfterStep.selectedFrame === frozenBeforeStep.selectedFrame + 1 &&
      frozenAfterStep.viewport.hi === frozenBeforeStep.viewport.hi &&
      frozenAfterStep.playheadClippedAfter,
    JSON.stringify(frozenAfterStep)
  );

  // Markers have generous invisible hit targets, expose hover detail, and seek
  // when selected.
  const markerDetail = await page.evaluate(() => {
    const marker = document.querySelector("#mp-markers .mp-evt");
    marker.dispatchEvent(new MouseEvent("mouseenter"));
    return document.getElementById("mp-evt-tip").textContent;
  });
  check("hovering a marker exposes its frame and label", markerDetail.includes("frame"), markerDetail);
  const selectedMarkerFrame = await page.evaluate(() => {
    const marker = document.querySelector("#mp-markers .mp-evt");
    const labelled = marker.getAttribute("aria-label").match(/frame (\d+)/);
    marker.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    return labelled ? Number(labelled[1]) : -1;
  });
  // The click must SELECT the event on the seam (highlight/activeEvent latch),
  // not merely seek — the assertion the original in-frame check carried.
  const markerSelected = await player.evaluate(
    () => window.__scrub.model().selectedEventId !== null
  );
  await player.waitForFunction(
    (frame) => Math.abs(window.__scrub.frame() - frame) <= 1,
    selectedMarkerFrame,
    { timeout: 3000 }
  );
  check(
    "selecting a marker seeks to its frame",
    markerSelected && selectedMarkerFrame >= 0,
    JSON.stringify({ markerSelected, selectedMarkerFrame })
  );

  // Seek snaps to a frame within the range.
  const rng = await player.evaluate(() => window.__scrub.range());
  const target = Math.round((rng[0] + rng[1]) / 2);
  await player.evaluate((f) => window.__scrub.seek(f), target);
  await sleep(150);
  const seeked = await player.evaluate(() => window.__scrub.frame());
  check(
    "seek snaps to a frame within range",
    seeked >= rng[0] && seeked <= rng[1] && Math.abs(seeked - target) <= 1,
    `target ${target}, got ${seeked}, range ${rng}`
  );

  // Step advances the frame by exactly 1 while paused.
  const before = await player.evaluate(() => window.__scrub.frame());
  await player.evaluate(() => window.__scrub.step());
  await sleep(150);
  const after = await player.evaluate(() => window.__scrub.frame());
  const afterStepView = await player.evaluate(() => window.__scrub.view());
  check(
    "step advances the frame by exactly 1 while paused",
    after === before + 1,
    `${before} -> ${after}`
  );
  check(
    "stepping from history marks the discarded future as unrecorded",
    afterStepView.recordedEndUnit < 1,
    JSON.stringify(afterStepView)
  );

  // Resume: frames advance again.
  await player.evaluate(() => window.__scrub.togglePause());
  const rf0 = await player.evaluate(() => window.__scrub.frame());
  await sleep(400);
  const rf1 = await player.evaluate(() => window.__scrub.frame());
  check("resume advances frames again", rf1 > rf0, `${rf0} -> ${rf1}`);

  // Rewinding and resuming replaces the first frame of the discarded future.
  // Its markers must be rebuilt from the new branch, not retained from the old
  // history or skipped by the publication cursor.
  await player.evaluate(() => {
    window.dispatchEvent(new KeyboardEvent("keydown", { code: "Space" }));
  });
  await player.waitForFunction(
    () => window.__scrub.events().some((event) => event.label === "Space down"),
    { timeout: 3000 }
  );
  const oldBranchFrame = await player.evaluate(
    () => window.__scrub.events().findLast((event) => event.label === "Space down").frame
  );
  await player.waitForFunction(
    (frame) => window.__scrub.range()[1] >= frame + 4,
    oldBranchFrame,
    { timeout: 3000 }
  );
  await player.evaluate(() => window.__scrub.togglePause());
  await player.waitForFunction(() => window.__scrub.paused(), { timeout: 3000 });
  await player.evaluate((frame) => window.__scrub.seek(frame - 1), oldBranchFrame);
  await player.waitForFunction(
    (frame) => window.__scrub.frame() === frame - 1,
    oldBranchFrame,
    { timeout: 3000 }
  );
  await player.evaluate(() => {
    window.__scrub.togglePause();
    window.dispatchEvent(new KeyboardEvent("keyup", { code: "Space" }));
  });
  await player.waitForFunction(
    (frame) =>
      window.__scrub.events().some((event) => event.frame === frame && event.label === "Space up"),
    oldBranchFrame,
    { timeout: 3000 }
  );
  const branchMarkersAreAuthoritative = await player.evaluate(
    (frame) => {
      const atBranch = window.__scrub.events().filter((event) => event.frame === frame);
      return (
        atBranch.some((event) => event.label === "Space up") &&
        !atBranch.some((event) => event.label === "Space down")
      );
    },
    oldBranchFrame
  );
  check(
    "branching replaces discarded-future markers with authoritative inputs",
    branchMarkersAreAuthoritative
  );

  // A safe reload while scrubbed is non-destructive: it keeps the selected
  // cursor AND the complete recorded future. Step/Resume branches later.
  await player.waitForFunction(() => !window.__scrub.paused(), { timeout: 3000 });
  await player.waitForFunction(() => {
    const range = window.__scrub.range();
    return range.length === 2 && range[1] - range[0] >= 4;
  });
  await player.evaluate(() => window.__scrub.togglePause());
  await player.waitForFunction(() => window.__scrub.paused(), { timeout: 3000 });
  // Capture the domain only after Pause has taken effect. Frames can still be
  // published between the earlier running-state probe and this boundary.
  const reloadWhileScrubbed = await player.evaluate(() => ({
    hi: window.__scrub.range()[1],
    viewportHi: window.__scrub.view().viewport.hi,
    hadUnavailableHistory: window.__scrub.view().hasUnavailableHistory,
    lastId: Math.max(-1, ...window.__scrub.events().map((event) => event.id)),
  }));
  await player.evaluate((hi) => window.__scrub.seek(hi - 2), reloadWhileScrubbed.hi);
  await player.waitForFunction(
    (hi) => window.__scrub.frame() === hi - 2,
    reloadWhileScrubbed.hi,
    { timeout: 3000 }
  );
  const selectedBeforeReload = reloadWhileScrubbed.hi - 2;
  await page.evaluate(() =>
    window.__sandbox.setSource(`${window.__sandbox.getSource()}\n// reload while scrubbed marker`)
  );
  await player.waitForFunction(
    (lastId) =>
      window.__scrub
        .events()
        .some((event) => event.id > lastId && event.kind === "reload-ok"),
    reloadWhileScrubbed.lastId,
    { timeout: 5000 }
  );
  const scrubbedReloadMarker = await player.evaluate(
    (lastId) =>
      window.__scrub
        .events()
        .find((event) => event.id > lastId && event.kind === "reload-ok"),
    reloadWhileScrubbed.lastId
  );
  const reloadTransportIsVisible = await page.evaluate(() => {
    const chrono = document.querySelector(".mp-chrono");
    const step = document.getElementById("mp-step");
    return (
      getComputedStyle(chrono).display === "flex" &&
      getComputedStyle(step).visibility !== "hidden" &&
      !step.disabled
    );
  });
  check("reload boundary keeps the visible Step/Resume transport", reloadTransportIsVisible);
  const safeReloadView = await player.evaluate(() => window.__scrub.view());
  check(
    "paused plain-data reload keeps its selected frame and complete future",
    safeReloadView.selectedFrame === selectedBeforeReload &&
      safeReloadView.recorded.lo < selectedBeforeReload &&
      safeReloadView.recorded.hi === reloadWhileScrubbed.hi &&
      safeReloadView.viewport.hi === reloadWhileScrubbed.viewportHi &&
      safeReloadView.hasUnavailableHistory === reloadWhileScrubbed.hadUnavailableHistory,
    JSON.stringify(safeReloadView)
  );
  await page.locator("#mp-step").click();
  await sleep(500);
  const postReloadStep = await player.evaluate(() => ({
    paused: window.__scrub.paused(),
    frame: window.__scrub.frame(),
    range: window.__scrub.range(),
    view: window.__scrub.view(),
  }));
  check(
    "stepping after a safe reload branches without shrinking the visual total",
      postReloadStep.range.length === 2 &&
      postReloadStep.range[1] === selectedBeforeReload + 1 &&
      postReloadStep.view.viewport.hi === reloadWhileScrubbed.viewportHi &&
      postReloadStep.view.hasUnavailableHistory &&
      postReloadStep.view.unavailableAfterStartUnit < 1,
    JSON.stringify({ postReloadStep, scrubConsole: scrubConsole.slice(-8) })
  );
  // Host geometry: the recorded (cyan) band's left edge is recordedStartUnit —
  // 0 means the preserved history still reaches back to the timeline floor
  // (the same quantity the in-frame bar carried in scrub-played's x attr).
  const preservedRailStartsAtHistoryFloor = await page.evaluate(
    () => parseFloat(document.getElementById("mp-recorded").style.left) === 0
  );
  check(
    "preserved history keeps its cyan rail before the reload boundary",
    preservedRailStartsAtHistoryFloor
  );
  check(
    "reload while scrubbed marks the selected frame without branching",
    scrubbedReloadMarker.frame === selectedBeforeReload,
    JSON.stringify({ reloadWhileScrubbed, scrubbedReloadMarker })
  );

  await page.close();
}

// --- 10. Closure-bearing reloads retain UI continuity at a safe boundary. ---
{
  const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
  const b64u = Buffer.from(CLOSURE_HISTORY).toString("base64url");
  await page.goto(`${BASE}/sandbox.html#src=${b64u}`);
  await page.waitForFunction(
    () => window.__sandbox && window.__sandbox.status().state === "live",
    { timeout: 30000 }
  );
  const player = playerFrame(page);
  await player.waitForFunction(
    () => window.__scrub?.range().length === 2 && window.__scrub.range()[1] >= 30,
    { timeout: 10000 }
  );
  await player.evaluate(() => window.__scrub.togglePause());
  await player.waitForFunction(() => window.__scrub.paused(), { timeout: 3000 });
  await player.evaluate(() => window.__scrub.setPreview({ enabled: true, seconds: 2 }));
  const reloadTarget = await player.evaluate(() => {
    const [lo, hi] = window.__scrub.range();
    return Math.round(lo + (hi - lo) * 0.4);
  });
  await player.evaluate((frame) => window.__scrub.seek(frame), reloadTarget);
  await player.waitForFunction(
    (frame) => window.__scrub.frame() === frame,
    reloadTarget,
    { timeout: 3000 }
  );
  const beforeReload = await player.evaluate(() => ({
    frame: window.__scrub.frame(),
    view: window.__scrub.view(),
    lastId: Math.max(-1, ...window.__scrub.events().map((event) => event.id)),
  }));

  await page.evaluate(() =>
    window.__sandbox.setSource(`${window.__sandbox.getSource()}\n// closure history boundary`)
  );
  await player.waitForFunction(
    (lastId) =>
      window.__scrub.events().some(
        (event) => event.id > lastId && event.kind === "reload-ok"
      ),
    beforeReload.lastId,
    { timeout: 5000 }
  );
  // Let the host chrono bar's rAF paint catch up to the post-reload view
  // before sampling its stripes/label (the seam is already settled).
  await page
    .waitForFunction(
      () => parseFloat(document.getElementById("mp-unavailable").style.width) > 0,
      null,
      { timeout: 3000 }
    )
    .catch(() => {});
  const afterReloadSeam = await player.evaluate(() => ({
    frame: window.__scrub.frame(),
    range: window.__scrub.range(),
    view: window.__scrub.view(),
    reloadFrame: window.__scrub.events().findLast((event) => event.kind === "reload-ok").frame,
  }));
  const afterReloadChrono = await page.evaluate(() => ({
    stripeWidth: parseFloat(document.getElementById("mp-unavailable").style.width),
    stripeAfterWidth: parseFloat(
      document.getElementById("mp-unavailable-after").style.width
    ),
    playheadVisible: getComputedStyle(document.getElementById("mp-playhead")).display,
    previewVisible: getComputedStyle(document.getElementById("mp-preview-handle")).display,
    playheadValueText: document.getElementById("mp-playhead").getAttribute("aria-valuetext"),
    label: document.getElementById("mp-frame").textContent,
  }));
  const afterReload = { ...afterReloadSeam, ...afterReloadChrono };
  check(
    "closure reload keeps the paused frame and frozen viewport",
    afterReload.frame === beforeReload.frame &&
      afterReload.view.selectedFrame === beforeReload.view.selectedFrame &&
      afterReload.view.viewport.lo === beforeReload.view.viewport.lo &&
      afterReload.view.viewport.hi === beforeReload.view.viewport.hi,
    JSON.stringify({ beforeReload, afterReload })
  );
  check(
    "closure reload seeds a one-frame seekable generation at the boundary",
    afterReload.range[0] === beforeReload.frame &&
      afterReload.range[1] === beforeReload.frame &&
      afterReload.reloadFrame === beforeReload.frame,
    JSON.stringify(afterReload)
  );
  check(
    "unavailable history stays striped without cluttering the frame counter",
    afterReload.view.hasUnavailableHistory &&
      afterReload.stripeWidth > 0 &&
      afterReload.stripeAfterWidth > 0 &&
      afterReload.playheadVisible === "block" &&
      afterReload.previewVisible === "block" &&
      afterReload.playheadValueText.includes(
        `recorded frames ${beforeReload.frame} to ${beforeReload.frame}`
      ) &&
      afterReload.playheadValueText.includes("striped history") &&
      afterReload.playheadValueText.includes("unavailable") &&
      afterReload.label.includes(String(beforeReload.frame)) &&
      !afterReload.label.includes("reload boundary"),
    JSON.stringify(afterReload)
  );

  await player.evaluate((frame) => window.__scrub.seek(frame), beforeReload.view.viewport.lo);
  await sleep(150);
  const refusedOldSeek = await player.evaluate(() => window.__scrub.frame());
  check(
    "striped pre-reload frames are not seekable",
    refusedOldSeek === beforeReload.frame,
    `${beforeReload.view.viewport.lo} -> ${refusedOldSeek}`
  );
  await page.close();
}

// --- 11. The editor language-intelligence wasm analyzes source in-browser. -----
// Commits 7-8 wire this into the CodeMirror editor (diagnostics/hover); here we
// just smoke-test the bundle loads and `functor_lang_analyze` reports errors on
// a bad source and none on a clean one.
{
  const page = await browser.newPage({ viewport: { width: 800, height: 600 } });
  await page.goto(BASE); // any same-origin page; we only need /pkg/ reachable
  const result = await page.evaluate(async () => {
    let mod;
    try {
      mod = await import("/pkg/functor_lang_wasm.js");
    } catch {
      return null; // fails below — the pkg is guaranteed present (startup check)
    }
    await mod.default(); // init the wasm
    // A type error: adding a string to a float.
    const bad = JSON.parse(mod.functor_lang_analyze('let bad = 1.0 + "x"\n'));
    // A clean program using prelude names.
    const clean = JSON.parse(
      mod.functor_lang_analyze(
        "let draw = (model, tts: float) =>\n" +
          "  Frame.create(Camera3D.lookAt(Vec3.make(0.0, 0.0, -6.0), Vec3.make(0.0, 0.0, 0.0)), Scene.cube())\n"
      )
    );
    return { bad, clean };
  });
  if (!result) {
    check("language wasm analyzes source in-browser", false, "the pkg failed to import/init");
    await page.close();
  } else {
  const d = result.bad.diagnostics;
  const sane =
    Array.isArray(d) &&
    d.length >= 1 &&
    Number.isInteger(d[0].from) &&
    Number.isInteger(d[0].to) &&
    d[0].from < d[0].to;
  check(
    "language wasm analyzes source in-browser (error on bad, none on clean)",
    sane && result.clean.diagnostics.length === 0,
    `bad=${JSON.stringify(d)} clean=${result.clean.diagnostics.length}`
  );
  await page.close();
  }
}

// --- 11. Live diagnostics: the linter underlines a type error, clears on fix. --
// A valid MVU program (loads & runs live — type diagnostics are advisory in the
// dev loop) with ONE unused function whose body is a type error the checker
// flags. The `.cm-lintRange-error` underline must appear, then clear when the
// bad def is removed — all while the push loop keeps the status pill live.
{
  const CLEAN = GREEN;
  const TYPE_ERROR = `${GREEN}let oops = (x: float) => x + "type error"\n`;

  const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
  await page.goto(`${BASE}/sandbox.html`);
  await page.waitForFunction(
    () => window.__sandbox && window.__sandbox.status().state === "live",
    { timeout: 30000 }
  );

  // Await the analysis wasm's readiness. The pkg is guaranteed present (the
  // startup check), so not-ready here means it failed to load/init — a failure,
  // never a skip.
  const langAvailable = await page.evaluate(
    () => window.__lang && window.__lang.ready
  );
  if (!langAvailable) {
    check("live diagnostics: language analysis is ready", false, "__lang not ready");
    await page.close();
  } else {
    const lintCount = () => page.locator(".cm-lintRange-error").count();
    // Poll for the count to reach a predicate (covers the 300ms lint delay).
    const waitLint = async (pred, timeout = 6000) => {
      const t0 = Date.now();
      for (;;) {
        if (pred(await lintCount())) return true;
        if (Date.now() - t0 > timeout) return false;
        await sleep(150);
      }
    };

    await page.evaluate((s) => window.__sandbox.setSource(s), TYPE_ERROR);
    const gotError = await waitLint((n) => n > 0);
    check("type error draws a lint underline", gotError, `count=${await lintCount()}`);
    // Await the hot-swap RESULT before reading liveness: the debounced push
    // (busy → live) can be mid-flight right when the underline appears, so a
    // bare status() read would intermittently catch the transient "reloading".
    // TYPE_ERROR still loads and runs (type diagnostics are advisory), so the
    // push reports "model preserved".
    await page.waitForFunction(
      () => window.__sandbox.status().message.includes("model preserved"),
      { timeout: 8000 }
    );
    check(
      "diagnostics keep the sandbox live",
      (await page.evaluate(() => window.__sandbox.status().state)) === "live"
    );

    await page.evaluate((s) => window.__sandbox.setSource(s), CLEAN);
    const cleared = await waitLint((n) => n === 0);
    check("fixing the type error clears the underline", cleared, `count=${await lintCount()}`);
    // Same as above: wait for the fix's push to round-trip before asserting live.
    await page.waitForFunction(
      () => window.__sandbox.status().message.includes("model preserved"),
      { timeout: 8000 }
    );
    check(
      "sandbox returns/stays live after the fix",
      (await page.evaluate(() => window.__sandbox.status().state)) === "live"
    );

    await page.close();
  }
}

// --- 12. Hover types + inlay hints + codelens (commit 8). ---------------------
// The intel program loads fresh via #src=; once the analysis pkg is ready the
// editor grows inline `: float` inlays, a signature codelens above each def,
// and a hover tooltip — all while the push loop keeps the status pill live.
{
  const b64u = Buffer.from(INTEL_SRC).toString("base64url");
  const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
  await page.goto(`${BASE}/sandbox.html#src=${b64u}`);
  await page.waitForFunction(
    () => window.__sandbox && window.__sandbox.status().state === "live",
    { timeout: 30000 }
  );

  const langAvailable = await page.evaluate(() => window.__lang && window.__lang.ready);
  if (!langAvailable) {
    check("hover/inlay/codelens: language analysis is ready", false, "__lang not ready");
    await page.close();
  } else {
    // The signature lens appears once per top-level def; count them in the source.
    const topDefs = INTEL_SRC.split("\n").filter((l) => l.startsWith("let ")).length;

    // Inlays and lenses lag the doc by the lint debounce (they read the cache
    // the lint pass fills), so poll rather than sampling once.
    const poll = async (fn, pred, timeout = 8000) => {
      const t0 = Date.now();
      for (;;) {
        const v = await fn();
        if (pred(v)) return v;
        if (Date.now() - t0 > timeout) return v;
        await sleep(150);
      }
    };

    const inlays = await poll(() => page.locator(".cm-inlay").count(), (n) => n > 0);
    check("inlay hints decorate unannotated params", inlays > 0, `count=${inlays}`);

    const lenses = await poll(() => page.locator(".cm-lens").count(), (n) => n >= topDefs);
    check(
      "codelens shows a signature above every top-level def",
      lenses >= topDefs,
      `lenses=${lenses}, defs=${topDefs}`
    );

    // Hover a REAL code token (skip the lens/inlay widget text) and rest the
    // mouse over it — a jiggle would keep resetting the hover timer.
    const coord = await page.evaluate(() => {
      const content = document.querySelector(".cm-content");
      const walker = document.createTreeWalker(content, NodeFilter.SHOW_TEXT);
      let node;
      while ((node = walker.nextNode())) {
        if (node.parentElement.closest(".cm-lens, .cm-inlay")) continue; // widget text
        const idx = node.textContent.indexOf("speed");
        if (idx >= 0) {
          const range = document.createRange();
          range.setStart(node, idx);
          range.setEnd(node, idx + 5);
          const r = range.getBoundingClientRect();
          return { x: r.x + r.width / 2, y: r.y + r.height / 2 };
        }
      }
      return null;
    });
    check("found a hoverable token in the editor", !!coord, JSON.stringify(coord));
    if (coord) {
      await page.mouse.move(coord.x - 40, coord.y);
      await sleep(100);
      await page.mouse.move(coord.x, coord.y);
      const tip = await poll(
        async () => {
          const el = page.locator(".cm-tooltip-hover");
          return (await el.count()) ? (await el.first().textContent()) || "" : "";
        },
        (t) => t.includes(":")
      );
      check("hover shows a type tooltip", tip.includes(":"), `tooltip=${JSON.stringify(tip)}`);
    }

    check(
      "language intelligence keeps the sandbox live",
      (await page.evaluate(() => window.__sandbox.status().state)) === "live"
    );

    await page.close();
  }
}

// --- 12b. Status bar: Problems + Output. ---------------------------------------
// The bottom strip's Problems tab mirrors the lint pass (count + clickable
// rows that jump the editor), and the Output panel receives runtime console
// traces (`Debug.log`, forwarded from the player iframe) plus reload results.
{
  const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
  await page.goto(`${BASE}/sandbox.html`);
  await page.waitForFunction(
    () => window.__sandbox && window.__sandbox.status().state === "live",
    { timeout: 30000 }
  );

  const problemsTab = page.locator('.statusbar-tab[data-tab="problems"]');
  const outputTab = page.locator('.statusbar-tab[data-tab="output"]');
  const tabText = async (tab) => ((await tab.textContent()) || "").trim();
  const waitFor = async (fn, pred, timeout = 8000) => {
    const t0 = Date.now();
    for (;;) {
      const v = await fn();
      if (pred(v)) return v;
      if (Date.now() - t0 > timeout) return v;
      await sleep(150);
    }
  };

  // A type error fills the Problems tab and panel.
  const BAD = `${GREEN}let oops = (x: float) => x + "status bar"\n`;
  await page.evaluate((s) => window.__sandbox.setSource(s), BAD);
  const flagged = await waitFor(() => tabText(problemsTab), (t) => t.includes("1 problem"));
  check("problems tab counts the type error", flagged.includes("1 problem"), flagged);

  await problemsTab.click();
  const row = page.locator(".problem-row");
  const rowText = await waitFor(
    async () => ((await row.count()) ? await row.first().textContent() : ""),
    (t) => t.includes("game.fun")
  );
  check(
    "problems panel lists the diagnostic with its location",
    rowText.includes("float") && rowText.includes("game.fun"),
    rowText
  );

  // Clicking the row jumps + focuses the editor.
  await row.first().click();
  const focused = await page.evaluate(() =>
    document.activeElement ? document.activeElement.classList.contains("cm-content") : false
  );
  check("clicking a problem focuses the editor", focused);

  // Fixing the error empties the tab back out.
  await page.evaluate((s) => window.__sandbox.setSource(s), GREEN);
  const cleared = await waitFor(() => tabText(problemsTab), (t) => t.includes("0 problems"));
  check("fixing the error resets the problems tab", cleared.includes("0 problems"), cleared);

  // A top-level Debug.log fires on the hot-swap and lands in Output (the
  // player forwards its console), alongside the reload-result lines.
  await page.evaluate(
    (s) => window.__sandbox.setSource(s),
    `${GREEN}let boot = Debug.log("status-probe", 42.0)\n`
  );
  await outputTab.click();
  const outputLines = await waitFor(
    () => page.locator(".output-line").allTextContents(),
    (lines) => lines.some((l) => l.includes("status-probe"))
  );
  check(
    "Debug.log reaches the Output panel",
    outputLines.some((l) => l.includes("status-probe")),
    JSON.stringify(outputLines.slice(-4))
  );
  check(
    "reload results reach the Output panel",
    outputLines.some((l) => l.includes("model preserved")),
    JSON.stringify(outputLines.slice(-4))
  );
  // Runtime lines carry a `[Frame N | HH:MM:SS]` preamble (the game was
  // already running when the hot-swap re-evaluated the Debug.log).
  const probeLine = outputLines.find((l) => l.includes("status-probe")) || "";
  check(
    "output lines carry a [Frame N | time] preamble",
    /^\[Frame \d+ \| \d{2}:\d{2}:\d{2}\]/.test(probeLine),
    probeLine
  );

  await page.close();
}

// --- 12c. Live values while paused (the inspector overlay). --------------------
// Pausing via the player's scrubber relays the trace to the page; the editor
// grows cyan `= value` live inlays next to binders AND variable reads, the
// executions tab lists the frame's entry-point runs (tick + the synthesized
// draw), and any edit clears the overlay instantly (hash gate).
{
  const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
  await page.goto(`${BASE}/sandbox.html`);
  await page.waitForFunction(
    () => window.__sandbox && window.__sandbox.status().state === "live",
    { timeout: 30000 }
  );
  await page.waitForFunction(() => window.__lang && window.__lang.ready, { timeout: 15000 });

  const waitFor = async (fn, pred, timeout = 10000) => {
    const t0 = Date.now();
    for (;;) {
      const v = await fn();
      if (pred(v)) return v;
      if (Date.now() - t0 > timeout) return v;
      await sleep(150);
    }
  };

  // Let a few frames run, then pause via the real scrubber button.
  await sleep(800);
  await page.evaluate(() => document.getElementById("mp-pause")?.click());

  // Live inlays appear (the relay → setLiveTrace → hash gate → overlay path).
  const liveCount = await waitFor(() => page.locator(".cm-live-value").count(), (n) => n > 0);
  check("pausing shows live-value inlays in the editor", liveCount > 0, `count=${liveCount}`);
  const liveTexts = await page.locator(".cm-live-value").allTextContents();
  check(
    "live inlays carry `= value` previews",
    liveTexts.every((t) => t.startsWith("= ")) && liveTexts.length > 0,
    JSON.stringify(liveTexts.slice(0, 4))
  );
  // The hero's dot-grid loop sites (×120) sweep numerically — a multi-hit
  // numeric site renders its RANGE, not the last sample: `= 0…11 (×120)`.
  check(
    "numeric loop sites render min…max ranges",
    liveTexts.some((t) => /^= -?[\d.]+…-?[\d.]+ \(×\d+\)$/.test(t)),
    JSON.stringify(liveTexts.filter((t) => t.includes("×")).slice(0, 4))
  );

  // Position invariant: every hint's name span slices to exactly its name.
  // hero.fun's comments contain multibyte characters (em dashes) BEFORE the
  // bindings, so this fails loudly if the trace's UTF-8 byte offsets ever
  // reach the editor unconverted.
  const misplaced = await page.evaluate(() => {
    const doc = window.__sandbox.source();
    return window.__lang
      .liveHints()
      .filter((h) => doc.slice(h.nameStart, h.nameEnd) !== h.name)
      .map((h) => `${h.name}@${h.nameStart}=${JSON.stringify(doc.slice(h.nameStart, h.nameEnd))}`);
  });
  check("live hints sit exactly on their names (byte→UTF-16)", misplaced.length === 0, JSON.stringify(misplaced));

  // The executions picker lists the frame's runs, draw included.
  const execTab = page.locator('.statusbar-tab[data-tab="executions"]');
  const tabText = ((await execTab.textContent()) || "").trim();
  check("executions tab counts the paused frame's runs", /⏸ \d+ executions/.test(tabText), tabText);
  await execTab.click();
  const rows = await waitFor(
    () => page.locator(".exec-row").allTextContents(),
    (rs) => rs.some((r) => r.startsWith("draw"))
  );
  check(
    "executions list includes tick and the synthesized draw",
    rows.some((r) => r.startsWith("tick")) && rows.some((r) => r.startsWith("draw")),
    JSON.stringify(rows)
  );

  // Resuming play clears the overlay (the runtime's unpaused stub bumps the
  // trace generation; stale inlays over a running game would be lies).
  await page.evaluate(() => document.getElementById("mp-pause")?.click());
  const resumed = await waitFor(() => page.locator(".cm-live-value").count(), (n) => n === 0, 6000);
  check("resuming clears the live overlay", resumed === 0, `count=${resumed}`);

  // Pause again: the overlay returns, then an edit clears it instantly —
  // stale values must never drift over moved text (hash gate).
  await page.evaluate(() => document.getElementById("mp-pause")?.click());
  await waitFor(() => page.locator(".cm-live-value").count(), (n) => n > 0);
  await page.evaluate((s) => window.__sandbox.setSource(s), `${GREEN}// paused edit\n`);
  const cleared = await waitFor(() => page.locator(".cm-live-value").count(), (n) => n === 0, 4000);
  check("editing clears the live overlay (hash gate)", cleared === 0, `count=${cleared}`);

  await page.close();
}

// --- 12d. The execution-recency gutter. ----------------------------------------
// A parity-conditional program makes every gutter state deterministic: the
// even/odd arms alternate per frame, and a never-true branch stays dark.
// Pausing shows green (ran this frame) vs cyan (ran a frame before); scrubbing
// BACK one frame swaps the arms' colors and turns pink on (ran after).
{
  // A frame-counter threshold: the EARLY arm runs on frames n<60, the LATE
  // arm after; `never` requires hp < 0 — unreachable (statically runnable →
  // dark). Unique arm texts so line lookup can't collide with init.
  const PARITY = `let init = { n: 0.0, hp: 1.0 }
let tick = (model, dt: float, tts: float) =>
  match model.hp < 0.0 with
  | true => { n: model.n, hp: 0.0 }
  | false =>
    match model.n < 60.0 with
    | true => { n: model.n + 1.0, hp: 1.0 }
    | false => { n: model.n + 1.0, hp: 2.0 }
let draw = (model, tts: float) =>
  Frame.create(
    Camera3D.lookAt(0.0, 0.0, -6.0, 0.0, 0.0, 0.0),
    Scene.sphere() |> Scene.emissive(Color.rgb(0.1, 1.0, 0.2)))
`;
  const b64u = Buffer.from(PARITY).toString("base64url");
  const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
  await page.goto(`${BASE}/sandbox.html#src=${b64u}`);
  await page.waitForFunction(
    () => window.__sandbox && window.__sandbox.status().state === "live",
    { timeout: 30000 }
  );
  await page.waitForFunction(() => window.__lang && window.__lang.ready, { timeout: 15000 });

  const waitFor = async (fn, pred, timeout = 10000) => {
    const t0 = Date.now();
    for (;;) {
      const v = await fn();
      if (pred(v)) return v;
      if (Date.now() - t0 > timeout) return v;
      await sleep(150);
    }
  };
  const lineOf = (needle) => PARITY.slice(0, PARITY.indexOf(needle)).split("\n").length;
  const earlyLine = lineOf("{ n: model.n + 1.0, hp: 1.0 }"); // frames n<60
  const lateLine = lineOf("{ n: model.n + 1.0, hp: 2.0 }"); // frames n>=60
  const neverLine = lineOf("{ n: model.n, hp: 0.0 }");

  // Run past the threshold, then pause: the late arm is CURRENT (green),
  // the early arm history (cyan), the unreachable arm dark.
  const player = playerFrame(page);
  await player.waitForFunction(
    () => window.__scrub && window.__scrub.range().length === 2 && window.__scrub.range()[1] > 80,
    { timeout: 30000 }
  );
  await page.evaluate(() => document.getElementById("mp-pause")?.click());
  const cov = await waitFor(
    () => page.evaluate(() => window.__lang.coverage()),
    (c) => c[lateLine] === "now"
  );
  check("current arm is green", cov[lateLine] === "now", JSON.stringify(cov));
  check("pre-threshold arm is cyan (ran before)", cov[earlyLine] === "before", JSON.stringify(cov));
  check("never-taken branch is dark", cov[neverLine] === "dark", JSON.stringify(cov));
  // Gutter markers are real DOM (the viewport shows them all here).
  const domStates = await page.evaluate(() =>
    [...document.querySelectorAll(".cm-cov")].map((el) => el.className)
  );
  check(
    "gutter renders now/before/dark markers",
    ["now", "before", "dark"].every((s) => domStates.some((c) => c.includes(`cm-cov-${s}`))),
    JSON.stringify(domStates.slice(0, 6))
  );

  // Scrub back BEFORE the threshold (frame 10): the early arm becomes this
  // frame's (green — its coverage comes from the ring, the scrubbed-frame
  // path) and the late arm ran only in frames AFTER the paused one → pink.
  await player.evaluate(() => window.__scrub.seek(10));
  const scrubbed = await waitFor(
    () => page.evaluate(() => window.__lang.coverage()),
    (c) => c[earlyLine] === "now"
  );
  check(
    "scrubbed back: the early arm is green from the ring",
    scrubbed[earlyLine] === "now",
    JSON.stringify(scrubbed)
  );
  check(
    "scrubbed back: the post-threshold arm is pink (ran after)",
    scrubbed[lateLine] === "after",
    JSON.stringify(scrubbed)
  );

  await page.close();
}

// --- 13. Scope-aware autocomplete in the editor (commit 8b). ------------------
// The completion source is backed by the wasm's scope-aware `complete`, driven
// through the __sandbox.triggerComplete seam (insert text + set cursor + open
// the popup). That seam is guarded to NOT push, so the status pill stays live
// throughout.
{
  const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
  await page.goto(`${BASE}/sandbox.html`);
  await page.waitForFunction(
    () => window.__sandbox && window.__sandbox.status().state === "live",
    { timeout: 30000 }
  );

  const langAvailable = await page.evaluate(() => window.__lang && window.__lang.ready);
  if (!langAvailable) {
    check("autocomplete: language analysis is ready", false, "__lang not ready");
    await page.close();
  } else {
    // NO cache priming: the lint heartbeat's analyze pass primes the
    // completion cache on its own — dot-completion on a mid-edit (broken)
    // buffer must answer from the last clean analyze, same as real typing.

    // Open the popup and wait until its option labels satisfy `pred`; retries
    // the trigger — under load a popup open can be swallowed by a lagging
    // transaction (a lint pass landing mid-open), so a single-shot wait flakes.
    const openCompletion = async (source, cursor, pred) => {
      for (let attempt = 0; attempt < 4; attempt++) {
        await page.evaluate(
          ({ s, c }) => window.__sandbox.triggerComplete(s, c),
          { s: source, c: cursor }
        );
        const t0 = Date.now();
        while (Date.now() - t0 < 2500) {
          const labels = await page.evaluate(() =>
            [...document.querySelectorAll(".cm-tooltip-autocomplete .cm-completionLabel")].map(
              (el) => el.textContent
            )
          );
          if (pred(labels)) return labels;
          await sleep(150);
        }
      }
      return [];
    };

    // A) Member popup: cursor right after `Scene.` (empty partial) surfaces many
    // members. `triggerComplete` is guarded (no push), so status stays live.
    const memberCursor = GREEN.indexOf("Scene.") + "Scene.".length;
    const opts = await openCompletion(
      GREEN,
      memberCursor,
      (labels) => labels.length > 3 && labels.includes("sphere")
    );
    check(
      "Scene. opens the completion popup with >3 members",
      opts.length > 3,
      `options=${JSON.stringify(opts.slice(0, 8))}`
    );
    check(
      "completion offers a known Scene member (sphere)",
      opts.includes("sphere"),
      JSON.stringify(opts.slice(0, 8))
    );

    // B) Applying a completion inserts its label: a typo'd member `spher` offers
    // the sole `sphere`; accepting it fixes the program (still valid → the push
    // keeps the loop live), and the label is now in the doc.
    const GREEN_TYPO = GREEN.replace("Scene.sphere()", "Scene.spher()");
    const typoCursor = GREEN_TYPO.indexOf("Scene.spher") + "Scene.spher".length;
    const typoOpts = await openCompletion(
      GREEN_TYPO,
      typoCursor,
      (labels) => labels[0] === "sphere"
    );
    // Accept via the editor's own apply path (deterministic — no key focus).
    const accepted = await page.evaluate(() => window.__sandbox.acceptCompletion());
    await sleep(150);
    const afterAccept = await page.evaluate(() => window.__sandbox.getSource());
    check(
      "applying a completion inserts its label",
      afterAccept.includes("Scene.sphere()"),
      `accepted=${accepted}, popup=${JSON.stringify(typoOpts)}, line=${JSON.stringify(
        afterAccept.split("\n").find((l) => l.includes("Scene.")) || afterAccept.slice(0, 60)
      )}`
    );
    // The accept pushed the fixed (valid) program. Wait for the push RESULT
    // (not just state === "live": the pill is already live before the debounced
    // push fires, so that would pass early and the final live check below could
    // catch the transient "reloading").
    await page.waitForFunction(
      () => window.__sandbox.status().message.includes("model preserved"),
      { timeout: 8000 }
    );

    // C) Top-level partial `le` → the `let` keyword (guarded — no push).
    const topOpts = await openCompletion("le", 2, (labels) => labels.includes("let"));
    check(
      "top-level `le` offers the `let` keyword",
      topOpts.includes("let"),
      JSON.stringify(topOpts.slice(0, 8))
    );

    check(
      "autocomplete keeps the sandbox live",
      (await page.evaluate(() => window.__sandbox.status().state)) === "live"
    );

    await page.close();
  }
}

await browser.close();
server.kill();
console.log(failures === 0 ? "\nALL CHECKS PASSED" : `\n${failures} CHECK(S) FAILED`);
process.exit(failures === 0 ? 0 : 1);
