// The vr-lab grip gallery: every grip spec (`Grip.specs`) placed by its
// intent and fitted, as a contact sheet per item and a metrics table. The
// editing loop for grips: edit a spec (its intent or `nudge`), rerun, look.
//
//   npm run grips:vr-lab -- [out-dir] [--write]   (default: $TMPDIR/vr-lab-grips)
//
// `--write` also regenerates `examples/vr-lab/fitted.fun`: the poses the game
// takes (`Grip.adopted`) and the guns' placements (`Grip.placementOf`);
// expects in `grip.fun` fail until it is current.
// `GRIPS_PROJECT` runs another copy of the project (a before/after).
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

const args = process.argv.slice(2);
const write = args.includes("--write");
const out = resolve(args.find((a) => !a.startsWith("--")) ?? join(tmpdir(), "vr-lab-grips"));
const project = resolve(process.env.GRIPS_PROJECT ?? `${ROOT}examples/vr-lab`);
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
const rt = await launch(project, port, ["--entry", "gallery", "--hidden"]);
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
  if (write) {
    const model = (await rt.get("/state")).model;
    writeFileSync(join(project, "fitted.fun"), fittedModule(rows.filter((r) => !r.extra), model.tables, model.grasps));
    console.log(`wrote ${join(project, "fitted.fun")}`);
  }

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

// A spec's name as a binding: "shotgun fore-end" → shotgunForeEnd.
function ident(name) {
  const words = name.replace(/[^a-z0-9]+/gi, " ").trim().split(" ");
  return words.map((w, i) => (i === 0 ? w.toLowerCase() : w[0].toUpperCase() + w.slice(1).toLowerCase())).join("");
}

// `fitted.fun`: each spec's adopted pose, then each gun grip's placement,
// one binding a line (the e2e reads the pistol grip's pose and the placements).
function fittedModule(rows, tables, grasps) {
  // `places` decimals, trailing zeros dropped (never an exponent); negative as `0.0 - x`.
  const at = (places) => (x) => {
    const t = x.toFixed(places).replace(/0+$/, "").replace(/\.$/, ".0").replace(/^-0\.0$/, "0.0");
    return t.startsWith("-") ? `0.0 - ${t.slice(1)}` : t;
  };
  const n = at(6);
  // Placements to 15: a quaternion must stay unit (`Quat.angle` near 0 goes
  // as its dot's square root).
  const m = at(15);
  const placed = rows.filter((r) => r.placement?.$ctor === "Option.Some").map((r) => [r, r.placement.args[0]]);
  const pose = ({ position: p, orientation: q }) =>
    `{ position: { x: ${m(p.x)}, y: ${m(p.y)}, z: ${m(p.z)} }, orientation: { x: ${m(q.x)}, y: ${m(q.y)}, z: ${m(q.z)}, w: ${m(q.w)} } }`;
  const bend = (b) => `{ curl: ${n(b.curl)}, tip: ${n(b.tip)} }`;
  const fingers = (p) =>
    `{ thumb: { curl: ${n(p.thumb.curl)}, tip: ${n(p.thumb.tip)}, opposition: ${n(p.thumb.opposition)} }, ` +
    `index: ${bend(p.index)}, middle: ${bend(p.middle)}, ring: ${bend(p.ring)}, little: ${bend(p.little)} }`;
  // A `{name, cells}` as a list of (pose, palm push), a cell a line.
  const cells = (suffix) => ({ name, cells: cs }) =>
    [`let ${ident(name)}${suffix}: List<(Hand.FingerPose, float)> = [`, ...cs.map((c, i) => `  (${fingers(c.pose)}, ${n(c.push)})${i < cs.length - 1 ? "," : ""}`), "]"];
  return [
    "// Fitted: the finger pose the game takes for each held item and site.",
    "// GENERATED by `npm run grips:vr-lab -- --write` from `Grip.specs` (each",
    "// spec's fit at its placement, `Grip.adopted`; the right hand's, which the",
    "// left's mirrors), the guns' placements, each support site's table",
    "// (`Grip.tableOf`) and each free prop's placements (`Grip.graspCells`).",
    "// Don't edit: expects in `grip.fun` keep it current.",
    "",
    ...rows.map((r) => `let ${ident(r.name)}: Hand.FingerPose = ${fingers(r.adopted)}`),
    "",
    "/// Every pose, by its spec's name.",
    `let all: List<(string, Hand.FingerPose)> = [${rows.map((r) => `("${r.name}", ${ident(r.name)})`).join(", ")}]`,
    "",
    "// Each gun grip's placement: the hand's root in the gun's layout frame (`Gun.placement`, …).",
    ...placed.map(([r, p]) => `let ${ident(r.name)}Placement: Input.pose = ${pose(p)}`),
    "",
    "// Each support site's table: its grid's cells, slides outer (`Grip.Slider`).",
    ...tables.flatMap(cells("Table")),
    "",
    "// Each free prop's placements, its spec's first (`Grip.Grasps`).",
    ...grasps.flatMap(cells("Grasps")),
    "",
  ].join("\n");
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
