// The vr-lab grip gallery: every grip spec (`Grip.specs`) placed by its
// intent and fitted, as a contact sheet per item and a metrics table. The
// editing loop for grips: edit a spec (its intent or `nudge`), rerun, look.
//
//   npm run grips:vr-lab -- [out-dir]        (default: $TMPDIR/vr-lab-grips)
//
// Runs the project's `gallery` entry (`examples/vr-lab/gallery.fun`) in a
// hidden window. Its first step fits every item; `/state`'s model holds the
// metrics. The clock picks the item shown, `POST /camera` the view: front,
// side (the palm's), top, and the eye's (above and behind a held hand). Each
// sheet is the four views, the right hand's capsules white where free, green
// touching, red inside; the intent's targets yellow.
import { execFileSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { launch } from "./xr-scenario.mjs";
import { ROOT, sleep } from "./mcp-rpc.mjs";

const out = resolve(process.argv[2] ?? join(tmpdir(), "vr-lab-grips"));
const port = Number(process.env.GRIPS_PORT ?? 8791);
mkdirSync(out, { recursive: true });

// The hand's root sits at (0, 1, 0) facing -Z (`Gallery.root`).
const at = [0, 1, -0.03];
const views = [
  ["front", [0, 1.02, -0.38]],
  ["side", [-0.38, 1.02, -0.03]],
  ["top", [0, 1.38, -0.031]],
  ["eye", [-0.15, 1.3, 0.32]],
];

// A 2x2 sheet of the four views, titled.
const SHEET = `
import sys
from PIL import Image, ImageDraw
out, title, *paths = sys.argv[1:]
ims = [Image.open(p).convert("RGB") for p in paths]
w, h = ims[0].size
sheet = Image.new("RGB", (2 * w, 2 * h + 28), (16, 16, 20))
d = ImageDraw.Draw(sheet)
d.text((8, 7), title, fill=(235, 235, 235))
for i, (im, name) in enumerate(zip(ims, ["front", "side", "top", "eye"])):
    x, y = (i % 2) * w, 28 + (i // 2) * h
    sheet.paste(im, (x, y))
    d.text((x + 6, y + 4), name, fill=(255, 220, 90))
sheet.save(out)
`;

const started = Date.now();
const rt = await launch(`${ROOT}examples/vr-lab`, port, ["--entry", "gallery", "--hidden"]);
try {
  // The first step fits every item.
  let rows = [];
  for (const deadline = Date.now() + 60000; rows.length === 0; await sleep(100)) {
    rows = (await rt.get("/state")).model?.rows ?? [];
    if (Date.now() > deadline) throw new Error("the gallery never fitted its items");
  }
  writeFileSync(join(out, "grips.json"), JSON.stringify(rows, null, 2));
  writeFileSync(join(out, "grips.md"), table(rows));
  console.log(table(rows));

  const sheets = [];
  for (const [i, row] of rows.entries()) {
    await rt.post("/time", { type: "set", tts: i + 0.5 });
    const shots = [];
    for (const [name, eye] of views) {
      await rt.post("/camera", { type: "set", eye, target: at, fov: 40 });
      // The first frame after a move can still be the old view.
      await rt.capture();
      const path = join(out, `${slug(row.name)}-${name}.png`);
      writeFileSync(path, await rt.capture());
      shots.push(path);
    }
    const sheet = join(out, `${slug(row.name)}.png`);
    execFileSync("python3", ["-c", SHEET, sheet, `${row.name} (${row.intent})`, ...shots]);
    sheets.push(sheet);
  }
  console.log(`\n${sheets.length} sheets in ${out} (${((Date.now() - started) / 1000).toFixed(1)} s)`);
} finally {
  await rt.kill();
}

function slug(name) {
  return name.replace(/[^a-z0-9]+/gi, "-").replace(/^-|-$/g, "").toLowerCase();
}

// mm, one decimal.
function mm(x) {
  return (Math.round(x * 10000) / 10).toString();
}

function table(rows) {
  const digits = (m) => m.digits.map(mm).join("/");
  const pose = (f) =>
    `${f.thumb.curl.toFixed(2)},${f.thumb.tip.toFixed(2)} o${f.thumb.opposition} / ` +
    ["index", "middle", "ring", "little"].map((k) => `${f[k].curl.toFixed(2)},${f[k].tip.toFixed(2)}`).join(" ");
  const lines = [
    "| item | intent | preset gaps T/I/M/R/L | preset palm | fitted gaps | thenar | palm | contacts | strain | trigger | pinch | tip err | buried | fitted pose |",
    "|---|---|---|---|---|---|---|---|---|---|---|---|---|---|",
  ];
  for (const r of rows) {
    const c = r.compared;
    const buried = ["T", "I", "M", "R", "L"].filter((_, i) => c.fitted.buried[i]).join(" ");
    lines.push(
      `| ${r.name} | ${r.intent} | ${digits(c.preset)} | ${mm(c.preset.palm)} | ${digits(c.fit)} | ${mm(c.fit.thenar)} | ${mm(c.fit.palm)} | ` +
        `${c.fit.contacts} | ${c.fit.strain.toFixed(2)} | ${r.intent === "pistol" ? mm(r.checks.trigger) : ""} | ` +
        `${r.intent === "pinch" ? mm(r.checks.pinch) : ""} | ${c.error.map(mm).join("/")} | ${buried} | ${pose(c.fitted.pose)} |`,
    );
  }
  return lines.join("\n") + "\n";
}
