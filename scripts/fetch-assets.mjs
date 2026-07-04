// Fetch the sample glTF assets referenced by the examples. None are checked
// into the repo (*.glb is gitignored). Source: BabylonJS/Assets, which is
// credited in the README. Existing files are left alone, so this is safe to
// re-run.
//
// Usage: npm run fetch:assets

import { writeFile, stat } from "node:fs/promises";
import path from "node:path";

const BASE_URL = "https://raw.githubusercontent.com/BabylonJS/Assets/master/meshes";

// Which assets each sample needs (a sample loads them relative to its own dir).
const TARGETS = [
  {
    dir: "examples/hello",
    assets: ["shark.glb", "ExplodingBarrel.glb", "fish.glb", "Xbot.glb"],
  },
  {
    dir: "examples/lighting",
    assets: ["shark.glb"],
  },
  {
    dir: "examples/mle-hello-gltf",
    assets: ["shark.glb", "ExplodingBarrel.glb", "fish.glb", "Xbot.glb"],
  },
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
for (const { dir, assets } of TARGETS) {
  for (const name of assets) {
    const dest = path.join(dir, name);
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
}

process.exit(failures === 0 ? 0 : 1);
