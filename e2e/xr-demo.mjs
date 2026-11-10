// Record a scripted XR scenario as a demo video, from a spectator's view.
//
//   node e2e/xr-demo.mjs <scenarios.mjs> <scenario> <out.gif> [options]
//   npm run demo:vr-lab -- grab-throw /tmp/grab-throw.gif --mp4 /tmp/grab-throw.mp4
//   npm run demo:vr-lab -- /tmp/throws.xr.json /tmp/throws.gif   # a recorded fixture
//
// Plays the named scenario (`e2e/xr-scenario.mjs` format), or a fixture file
// (`e2e/xr-fixture.mjs`, filmed from the module's `defaultSpectator`), one fixed step at a
// time in a hidden window, points the shell's debug camera at the scenario's
// `spectator` view (`POST /camera` — it never enters the game, so the run is
// the same one the e2e checks), captures a frame per step, and assembles a
// looping GIF (and an MP4) with ffmpeg. Deterministic: the same scenario
// renders the same frames.
//
// Options:
//   --mp4 <path>     also write an H.264 MP4
//   --still <path>   also write one PNG: the step marked `release`, else the last
//   --every <n>      keep every nth step (default 2: 30 fps of 60 Hz steps)
//   --size <WxH>     framebuffer size (default 960x600)
//   --view rig       film the player's own view instead of the spectator's
//   --port <n>       debug-server port (default 8197)
import { execFileSync } from "node:child_process";
import { copyFileSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { pathToFileURL } from "node:url";
import { launch, play } from "./xr-scenario.mjs";

const [scenariosPath, name, out, ...rest] = process.argv.slice(2);
if (!out) {
  console.error("usage: node e2e/xr-demo.mjs <scenarios.mjs> <scenario> <out.gif> [--mp4 p] [--still p] [--every n] [--size WxH] [--view rig]");
  process.exit(2);
}
const opt = (flag, fallback) => {
  const i = rest.indexOf(flag);
  return i >= 0 ? rest[i + 1] : fallback;
};
const every = Number(opt("--every", "2"));
const size = opt("--size", "960x600");
const port = Number(opt("--port", "8197"));
const mp4 = opt("--mp4");
const still = opt("--still");
const view = opt("--view", "spectator");

const { project, scenarios, defaultSpectator } = await import(pathToFileURL(resolve(scenariosPath)).href);
const scenario = name.endsWith(".json")
  ? { fixture: resolve(name), spectator: defaultSpectator }
  : scenarios[name];
if (!scenario) {
  console.error(`no scenario "${name}"; have: ${Object.keys(scenarios).join(", ")}`);
  process.exit(2);
}

const frames = mkdtempSync(join(tmpdir(), "xr-demo-"));
const rt = await launch(project, port, ["--hidden", "--capture-size", size]);
let count = 0;
let releaseFrame;
let lastFrame;
try {
  if (view !== "rig" && scenario.spectator) await rt.post("/camera", { type: "set", ...scenario.spectator });
  await play(rt, scenario, async ({ step, mark }) => {
    if (step % every !== 0 && mark !== "release") return;
    lastFrame = join(frames, `f${String(count++).padStart(4, "0")}.png`);
    writeFileSync(lastFrame, await rt.capture());
    if (mark === "release") releaseFrame = lastFrame;
  });
} finally {
  await rt.kill();
}
const stillFrame = releaseFrame ?? lastFrame;

const fps = String(60 / every);
mkdirSync(dirname(resolve(out)), { recursive: true });
execFileSync(
  "ffmpeg",
  [
    "-y", "-loglevel", "error", "-framerate", fps, "-i", join(frames, "f%04d.png"),
    "-vf", "split[s0][s1];[s0]palettegen=stats_mode=diff[p];[s1][p]paletteuse=dither=bayer:bayer_scale=3",
    "-loop", "0", out,
  ],
  { stdio: "inherit" },
);
console.log(`wrote ${out} (${count} frames)`);
if (mp4) {
  mkdirSync(dirname(resolve(mp4)), { recursive: true });
  execFileSync(
    "ffmpeg",
    [
      "-y", "-loglevel", "error", "-framerate", fps, "-i", join(frames, "f%04d.png"),
      "-c:v", "libx264", "-pix_fmt", "yuv420p", "-vf", "scale=trunc(iw/2)*2:trunc(ih/2)*2", mp4,
    ],
    { stdio: "inherit" },
  );
  console.log(`wrote ${mp4}`);
}
if (still && stillFrame) {
  copyFileSync(stillFrame, still);
  console.log(`wrote ${still}`);
}
rmSync(frames, { recursive: true, force: true });
