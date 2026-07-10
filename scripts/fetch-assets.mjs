// Fetch the sample assets referenced by the examples (glTF models, skybox
// faces). None are checked into the repo (*.glb / example *.jpg are
// gitignored). Source: BabylonJS/Assets, which is credited in the README.
// Existing files are left alone, so this is safe to re-run.
//
// Usage: npm run fetch:assets

import { writeFile, stat } from "node:fs/promises";
import path from "node:path";

const MESHES_BASE = "https://raw.githubusercontent.com/BabylonJS/Assets/master/meshes";
const SKYBOX_BASE =
  "https://raw.githubusercontent.com/BabylonJS/Assets/master/skyboxes/TropicalSunnyDay";

// Which assets each sample needs (a sample loads them relative to its own dir).
const TARGETS = [
  {
    dir: "examples/hello",
    baseUrl: MESHES_BASE,
    assets: ["shark.glb", "ExplodingBarrel.glb", "fish.glb", "Xbot.glb"],
  },
  {
    dir: "examples/lighting",
    baseUrl: MESHES_BASE,
    assets: ["shark.glb"],
  },
  {
    dir: "examples/animation",
    baseUrl: MESHES_BASE,
    assets: ["Xbot.glb"],
  },
  {
    dir: "examples/crossfade",
    baseUrl: MESHES_BASE,
    assets: ["Xbot.glb"],
  },
  {
    dir: "examples/atmosphere",
    baseUrl: SKYBOX_BASE,
    assets: [
      "TropicalSunnyDay_px.jpg",
      "TropicalSunnyDay_nx.jpg",
      "TropicalSunnyDay_py.jpg",
      "TropicalSunnyDay_ny.jpg",
      "TropicalSunnyDay_pz.jpg",
      "TropicalSunnyDay_nz.jpg",
    ],
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
for (const { dir, baseUrl, assets } of TARGETS) {
  for (const name of assets) {
    const dest = path.join(dir, name);
    if (await exists(dest)) {
      console.log(`ok       ${dest}`);
      continue;
    }

    const url = `${baseUrl}/${name}`;
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
