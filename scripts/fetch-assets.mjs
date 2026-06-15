// Fetch the sample glTF assets referenced by examples/hello. None are checked
// into the repo (*.glb is gitignored there). Source: BabylonJS/Assets, which
// is credited in the README. Existing files are left alone, so this is safe
// to re-run.
//
// Usage: npm run fetch:assets

import { writeFile, stat } from "node:fs/promises";
import path from "node:path";

const BASE_URL = "https://raw.githubusercontent.com/BabylonJS/Assets/master/meshes";
const TARGET_DIR = "examples/hello";

const ASSETS = [
  "shark.glb",
  "ExplodingBarrel.glb",
  "fish.glb",
  "Xbot.glb",
];

async function exists(filePath) {
  try {
    await stat(filePath);
    return true;
  } catch {
    return false;
  }
}

let failures = 0;
for (const name of ASSETS) {
  const dest = path.join(TARGET_DIR, name);
  if (await exists(dest)) {
    console.log(`ok       ${dest}`);
    continue;
  }

  const url = `${BASE_URL}/${name}`;
  process.stdout.write(`fetching ${dest} ... `);
  try {
    const response = await fetch(url);
    if (!response.ok) {
      throw new Error(`HTTP ${response.status} from ${url}`);
    }
    const bytes = Buffer.from(await response.arrayBuffer());
    await writeFile(dest, bytes);
    console.log(`${(bytes.length / 1024 / 1024).toFixed(1)}MB`);
  } catch (error) {
    console.log(`FAILED (${error.message})`);
    failures++;
  }
}

process.exit(failures === 0 ? 0 : 1);
