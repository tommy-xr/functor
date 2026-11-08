// Record XR input to a fixture (`e2e/xr-fixture.mjs` format).
//
//   node e2e/xr-record.mjs <scenarios.mjs> <out.json> [options]
//   npm run record:vr-lab -- /tmp/throws.xr.json            # a worn Quest session
//   npm run record:vr-lab -- out.json --scenario grab-throw  # a scripted one, on desktop
//
// Attached (the default): reads a running runtime's input log over its debug
// server — the headset's under `functor run vr` (which forwards port 8123),
// or a desktop one's (`--port`). It first taps the scenarios module's
// `resetKey` (jump to the station and reset it) and the fixture starts at
// that step, so a replay from a fresh launch starts from the same state.
// Stops on Ctrl-C or after `--seconds`.
//
// `--scenario <name>`: launches the desktop runtime under `--emulate-xr`,
// plays that scripted scenario, and records it with its marks.
//
// Options:
//   --port <n>       debug-server port (default 8123; a scenario run's 8198)
//   --seconds <s>    stop an attached recording after this long
//   --key <k>        the reset key to tap (default: the module's `resetKey`)
import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { pathToFileURL } from "node:url";
import { encode, readLog, recordLive } from "./xr-fixture.mjs";
import { launch, play } from "./xr-scenario.mjs";

const [scenariosPath, out, ...rest] = process.argv.slice(2);
if (!out) {
  console.error("usage: node e2e/xr-record.mjs <scenarios.mjs> <out.json> [--scenario name] [--port n] [--seconds s] [--key k]");
  process.exit(2);
}
const opt = (flag, fallback) => {
  const i = rest.indexOf(flag);
  return i >= 0 ? rest[i + 1] : fallback;
};
const mod = await import(pathToFileURL(resolve(scenariosPath)).href);
const scenarioName = opt("--scenario");

function write(fixture) {
  const json = JSON.stringify(fixture);
  mkdirSync(dirname(resolve(out)), { recursive: true });
  writeFileSync(out, json + "\n");
  const seconds = (fixture.steps.length * fixture.dt).toFixed(2);
  console.log(`wrote ${out}: ${fixture.steps.length} steps (${seconds} s), ${(json.length / 1024).toFixed(1)} KB`);
}

if (scenarioName) {
  const scenario = mod.scenarios[scenarioName];
  if (!scenario) throw new Error(`no scenario "${scenarioName}"; have: ${Object.keys(mod.scenarios).join(", ")}`);
  const rt = await launch(mod.project, Number(opt("--port", "8198")), ["--headless", "--emulate-xr"]);
  try {
    const from = (await rt.get("/state")).frame + 1;
    const marks = {};
    await play(rt, scenario, async ({ mark, state }) => {
      if (mark) marks[mark] = state.frame;
    });
    const log = await readLog(rt, from);
    write(encode(log.steps, { marks, source: { runtime: "desktop --emulate-xr", scenario: scenarioName } }));
  } finally {
    await rt.kill();
  }
} else {
  const base = `http://127.0.0.1:${opt("--port", "8123")}`;
  const call = async (path, init) => {
    const res = await fetch(`${base}${path}`, init);
    if (!res.ok) throw new Error(`${init?.method ?? "GET"} ${path} → ${res.status}: ${await res.text()}`);
    return res;
  };
  const rt = {
    get: async (path) => (await call(path)).json(),
    post: (path, body) => call(path, { method: "POST", body: JSON.stringify(body) }),
  };
  const seconds = opt("--seconds");
  const stopped = new Promise((done) => {
    process.once("SIGINT", done);
    if (seconds) setTimeout(done, Number(seconds) * 1000);
  });
  const resetKey = opt("--key", mod.resetKey);
  console.log(`recording ${base} (reset key ${resetKey}); Ctrl-C to stop`);
  const steps = await recordLive(rt, { resetKey, stopped });
  const { build } = await rt.get("/state");
  write(encode(steps, { source: { runtime: build ? `quest ${build}` : "desktop", recorded: new Date().toISOString() } }));
  process.exit(0);
}
