// A demo-capture overlay: caption bar, keystroke badges, and click ripples that
// the site/demos scripts drive so a recorded GIF explains itself — you can see
// WHAT is being typed, WHY the scene changes, and WHERE a click lands. It is
// injected into the page at capture time only; it is never part of the shipped
// site.
//
//   import { installOverlay } from "./lib/overlay.mjs";
//   const ov = await installOverlay(page);
//   await ov.caption("Edit the colour →");
//   await ov.key("9"); await ov.key("5");
//   await ov.click(x, y);

// The browser-side payload (injected via addScriptTag). Defines window.__ov.
const OVERLAY_JS = `
(() => {
  if (window.__ov) return;
  const root = document.createElement("div");
  root.id = "demo-overlay";
  root.innerHTML = \`
    <style>
      #demo-overlay { position: fixed; inset: 0; pointer-events: none; z-index: 99999;
        font-family: "JetBrains Mono", ui-monospace, monospace; }
      #do-caption { position: absolute; left: 50%; top: 26px; transform: translateX(-50%) translateY(-8px);
        max-width: 82%; text-align: center; background: rgba(15,12,29,0.94); color: #e9e6f2;
        border: 1px solid #2b2542; border-radius: 999px;
        padding: 11px 26px; font-size: 15px; letter-spacing: 0.01em; opacity: 0;
        transition: opacity 0.3s ease, transform 0.3s ease;
        box-shadow: 0 12px 40px -12px rgba(0,0,0,0.85); }
      #do-caption.show { opacity: 1; transform: translateX(-50%) translateY(0); }
      #do-caption b { color: #41d8e6; font-weight: 600; }
      #do-keys { position: absolute; left: 50%; top: 80px; transform: translateX(-50%);
        display: flex; gap: 6px; }
      .do-key { min-width: 16px; text-align: center; background: #1e1833; color: #41d8e6;
        border: 1px solid #41d8e6; border-radius: 9px; padding: 6px 11px; font-size: 16px;
        font-weight: 600; box-shadow: 0 3px 12px -3px rgba(0,0,0,0.7);
        transition: opacity 0.4s ease, transform 0.4s ease; }
      .do-key.fade { opacity: 0; transform: translateY(-8px); }
      .do-click { position: absolute; width: 36px; height: 36px; margin: -18px 0 0 -18px;
        border: 2px solid #e858b8; border-radius: 50%; animation: do-ripple 0.65s ease-out forwards; }
      @keyframes do-ripple { from { transform: scale(0.35); opacity: 0.95; }
        to { transform: scale(1.7); opacity: 0; } }
    </style>
    <div id="do-caption"></div>
    <div id="do-keys"></div>\`;
  document.body.appendChild(root);
  const cap = root.querySelector("#do-caption");
  const keys = root.querySelector("#do-keys");
  window.__ov = {
    caption(html) { cap.innerHTML = html || ""; cap.classList.toggle("show", !!html); },
    key(text) {
      const el = document.createElement("span");
      el.className = "do-key"; el.textContent = text;
      keys.appendChild(el);
      setTimeout(() => el.classList.add("fade"), 650);
      setTimeout(() => el.remove(), 1100);
    },
    click(x, y) {
      const r = document.createElement("div");
      r.className = "do-click"; r.style.left = x + "px"; r.style.top = y + "px";
      root.appendChild(r);
      setTimeout(() => r.remove(), 700);
    },
  };
})();
`;

export async function installOverlay(page) {
  await page.addScriptTag({ content: OVERLAY_JS });
  return {
    caption: (html) => page.evaluate((h) => window.__ov.caption(h), html),
    clearCaption: () => page.evaluate(() => window.__ov.caption("")),
    key: (text) => page.evaluate((t) => window.__ov.key(t), text),
    click: (x, y) => page.evaluate(([cx, cy]) => window.__ov.click(cx, cy), [x, y]),
  };
}
