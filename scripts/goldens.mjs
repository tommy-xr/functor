// Loads the golden-image scenarios from every `examples/<sample>/goldens.json`
// (see docs/goldens.md). Shared by the wasm golden spec and its guard test;
// the native harness (runtime/functor-runtime-desktop/tests/golden.rs) mirrors it.
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

export const REPO_ROOT = fileURLToPath(new URL("..", import.meta.url));

const KEYS = new Set(["name", "fixedTime", "debugRender", "targets", "inputScript", "captureAtFrame"]);
const TARGETS = new Set(["native", "wasm"]);

/** Every scenario, ordered by sample directory then file order, each with
 *  `sample` set to its directory name. Throws on any malformed or duplicate entry. */
export function loadGoldenScenarios(root = REPO_ROOT) {
  // The pre-split shared manifest; a stale branch can re-add it on rebase.
  if (existsSync(join(root, "golden-scenarios.json"))) {
    throw new Error("golden-scenarios.json is retired: move its scenarios to examples/<sample>/goldens.json");
  }
  const examples = join(root, "examples");
  const samples = readdirSync(examples, { withFileTypes: true })
    .filter((d) => d.isDirectory() && existsSync(join(examples, d.name, "goldens.json")))
    .map((d) => d.name)
    .sort();
  const seen = new Map();
  const scenarios = [];
  for (const sample of samples) {
    const file = join("examples", sample, "goldens.json");
    const list = JSON.parse(readFileSync(join(root, file), "utf8"));
    if (!Array.isArray(list)) throw new Error(`${file}: expected an array of scenarios`);
    list.forEach((s, i) => {
      const at = `${file}[${i}]`;
      validate(s, at);
      if (seen.has(s.name)) throw new Error(`${at}: duplicate scenario name '${s.name}' (also in ${seen.get(s.name)})`);
      seen.set(s.name, at);
      scenarios.push({ ...s, sample });
    });
  }
  return scenarios;
}

function validate(s, at) {
  const fail = (msg) => {
    throw new Error(`${at}: ${msg}`);
  };
  if (typeof s !== "object" || s === null || Array.isArray(s)) fail("expected an object");
  for (const k of Object.keys(s)) if (!KEYS.has(k)) fail(`unknown field '${k}'`);
  if (typeof s.name !== "string" || !s.name) fail("'name' must be a non-empty string");
  if (!Array.isArray(s.targets) || !s.targets.length || !s.targets.every((t) => TARGETS.has(t))) {
    fail("'targets' must be a non-empty array of \"native\"/\"wasm\"");
  }
  if (s.debugRender != null && typeof s.debugRender !== "string") fail("'debugRender' must be a string or null");
  // Either a pinned frame time, or an input script captured at a frame.
  const timed = typeof s.fixedTime === "number" && s.inputScript === undefined && s.captureAtFrame === undefined;
  const scripted =
    s.fixedTime === undefined && typeof s.inputScript === "string" && Number.isInteger(s.captureAtFrame) && s.captureAtFrame >= 0;
  if (!timed && !scripted) fail("set 'fixedTime', or 'inputScript' with 'captureAtFrame'");
}
