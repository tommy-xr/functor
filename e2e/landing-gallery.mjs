// Site e2e: the landing page's GAMES CAROUSEL — the shelf of box art between
// the feature rows and "How does it work?". Builds the site, serves dist with
// site/serve.mjs, then drives headless Chromium through the four properties the
// carousel is supposed to have and nothing else:
//
//   1. the cards are PRERENDERED — ten of them, each one link to its sandbox
//      example — so the shelf works with the island's script removed;
//   2. the box art is poster-first: no animation is fetched until a card is
//      actually on screen, and then hovering or focusing one plays it;
//   3. swapping the animation in moves nothing (the art keeps its box), and the
//      rail scrolls sideways without the PAGE ever doing so — at desktop and at
//      420px;
//   4. under `prefers-reduced-motion: reduce` no animation is ever loaded or
//      shown, and the posters carry the shelf on their own.
//
// Run manually (needs the wasm bundle, like the other site e2e):
//
//   wasm-pack build runtime/functor-runtime-web --target=web   # once
//   node e2e/landing-gallery.mjs
import { spawn, spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { chromium } from "@playwright/test";

const PORT = Number(process.env.FUNCTOR_SITE_PORT ?? 8127);
const BASE = `http://127.0.0.1:${PORT}`;
const ROOT = fileURLToPath(new URL("..", import.meta.url));
const CARDS = 10;

let failures = 0;
const check = (name, ok, detail = "") => {
  console.log(`${ok ? "PASS" : "FAIL"}: ${name}${ok || !detail ? "" : ` — ${detail}`}`);
  if (!ok) failures += 1;
};
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const build = spawnSync("node", ["site/build.mjs"], { cwd: ROOT, stdio: "inherit" });
if (build.status !== 0) process.exit(build.status ?? 1);

// An occupied port would make serve.mjs die while the readiness probe below
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
const noAnimationLoaded = (page) =>
  page.evaluate(() =>
    [...document.querySelectorAll(".game-anim")].every((img) => !img.getAttribute("src"))
  );
const pageScrollsSideways = (page) =>
  page.evaluate(
    () => document.documentElement.scrollWidth > document.documentElement.clientWidth
  );

// --- 1-3: the shelf, at desktop width --------------------------------------
{
  const page = await browser.newPage({ viewport: { width: 1440, height: 900 } });
  await page.goto(`${BASE}/index.html`, { waitUntil: "load" });
  const cards = page.locator(".game-card");
  check(`${CARDS} cards are prerendered`, (await cards.count()) === CARDS, `${await cards.count()}`);
  check(
    "every card is one link to its sandbox example",
    (await page.locator('.game-case[href^="sandbox.html?example="]').count()) === CARDS
  );
  check("no animation is loaded before the shelf is in view", await noAnimationLoaded(page));

  const first = cards.first();
  const before = await first.locator(".game-art").boundingBox();
  await page.locator(".games-section").scrollIntoViewIfNeeded();
  await sleep(1500);
  check(
    "a card in view plays its box art",
    await first.evaluate((el) => el.classList.contains("is-playing"))
  );
  check(
    "what it plays is that card's GIF",
    await first.evaluate((el) =>
      (el.querySelector(".game-anim")?.getAttribute("src") ?? "").endsWith(".gif")
    )
  );
  const after = await first.locator(".game-art").boundingBox();
  check(
    "the swap does not resize the art",
    Math.abs(before.width - after.width) < 1 && Math.abs(before.height - after.height) < 1,
    `${before.width}x${before.height} → ${after.width}x${after.height}`
  );

  const last = cards.last();
  await last.scrollIntoViewIfNeeded();
  await last.hover();
  await sleep(400);
  check("hovering a card plays it", await last.evaluate((el) => el.classList.contains("is-playing")));
  // Nothing may be left running once the shelf is behind the reader — a card
  // hovered as it scrolls away must still stop, or it animates off screen
  // forever (it gets no further observer callback).
  await page.mouse.move(2, 2);
  await page.locator(".cta-band").scrollIntoViewIfNeeded();
  await sleep(800);
  check(
    "nothing is left playing once the shelf is off screen",
    (await page.locator(".game-card.is-playing").count()) === 0,
    `${await page.locator(".game-card.is-playing").count()} still playing`
  );
  await page.locator(".games-section").scrollIntoViewIfNeeded();
  await sleep(600);

  const rail = page.locator(".games-rail");
  check(
    "the rail scrolls sideways",
    await rail.evaluate((el) => el.scrollWidth > el.clientWidth + 200)
  );
  // The KEYBOARD must be able to operate the scroll region — which means the
  // focusable, labelled element has to be the one that actually scrolls.
  // Park the rail back at the start and WAIT for it to settle: a snap scroll is
  // animated, and keys pressed mid-animation are absorbed.
  await rail.evaluate((el) => el.scrollTo({ left: 0, behavior: "instant" }));
  await page.waitForFunction(
    () => document.querySelector(".games-rail").scrollLeft === 0,
    null,
    { timeout: 5000 }
  );
  await rail.focus();
  check(
    "the scrolling rail is the focusable, labelled region",
    await page.evaluate(
      () =>
        document.activeElement?.classList.contains("games-rail") &&
        document.activeElement?.getAttribute("role") === "region" &&
        Boolean(document.activeElement?.getAttribute("aria-label"))
    )
  );
  for (let i = 0; i < 6; i++) {
    await page.keyboard.press("ArrowRight");
    await sleep(120);
  }
  const scrolled = await rail
    .evaluate(
      (el) =>
        new Promise((resolve) => {
          const started = Date.now();
          const poll = () => {
            if (el.scrollLeft > 0 || Date.now() - started > 3000) resolve(el.scrollLeft);
            else requestAnimationFrame(poll);
          };
          poll();
        })
    );
  check("arrow keys scroll the shelf", scrolled > 0, `scrollLeft ${scrolled}`);

  const third = cards.nth(2);
  await third.locator(".game-case").focus();
  await sleep(300);
  check("focusing a card plays it", await third.evaluate((el) => el.classList.contains("is-playing")));
  check("the page itself does not scroll sideways", !(await pageScrollsSideways(page)));
  await page.close();
}

// --- 4: prefers-reduced-motion ---------------------------------------------
{
  const page = await browser.newPage({
    viewport: { width: 1440, height: 900 },
    reducedMotion: "reduce",
  });
  await page.goto(`${BASE}/index.html`, { waitUntil: "load" });
  await page.locator(".games-section").scrollIntoViewIfNeeded();
  await page.locator(".game-card").first().hover();
  await sleep(1500);
  check("reduced motion: no animation is loaded, even on hover", await noAnimationLoaded(page));
  check(
    "reduced motion: no card is playing",
    (await page.locator(".game-card.is-playing").count()) === 0
  );
  check(
    "reduced motion: the posters still carry the shelf",
    (await page.locator(".game-poster").count()) === CARDS
  );
  await page.close();
}

// --- 3 (cont.): 420px ------------------------------------------------------
{
  const page = await browser.newPage({ viewport: { width: 420, height: 900 } });
  await page.goto(`${BASE}/index.html`, { waitUntil: "load" });
  await page.locator(".games-section").scrollIntoViewIfNeeded();
  await sleep(800);
  const card = await page.locator(".game-card").first().boundingBox();
  check("mobile: a card fits the viewport", card.width <= 420, `${card.width}px`);
  check("mobile: the page does not scroll sideways", !(await pageScrollsSideways(page)));
  await page.close();
}

await browser.close();
server.kill();
console.log(failures === 0 ? "\nALL CHECKS PASSED" : `\n${failures} CHECK(S) FAILED`);
process.exit(failures === 0 ? 0 : 1);
