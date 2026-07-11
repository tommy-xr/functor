// Fetch the sample assets referenced by the examples (glTF models, skybox
// faces). None are checked into the repo (*.glb / example *.jpg are
// gitignored). Sources: BabylonJS/Assets and Kenney (both credited in the
// README / per-example ASSETS.md). Existing files are left alone, so this
// is safe to re-run.
//
// Usage: npm run fetch:assets

import { writeFile, stat, mkdtemp, rm } from "node:fs/promises";
import { execFileSync } from "node:child_process";
import os from "node:os";
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

// Zip-packed assets (Kenney packs): the zip is downloaded once to a temp
// dir and single members are extracted with the system `unzip` (present on
// macOS and the Linux CI images).
const ZIP_TARGETS = [
  {
    dir: "examples/asteroids",
    zipUrl:
      "https://kenney.nl/media/pages/assets/space-kit/20874c75ac-1677698978/kenney_space-kit.zip",
    members: [{ from: "Models/GLTF format/craft_racer.glb", to: "ship.glb" }],
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

for (const { dir, zipUrl, members } of ZIP_TARGETS) {
  const missing = [];
  for (const m of members) {
    const dest = path.join(dir, m.to);
    if (await exists(dest)) {
      console.log(`ok       ${dest}`);
    } else {
      missing.push(m);
    }
  }
  if (missing.length === 0) continue;

  const tmp = await mkdtemp(path.join(os.tmpdir(), "functor-assets-"));
  const zipPath = path.join(tmp, "pack.zip");
  try {
    process.stdout.write(`fetching ${zipUrl} ... `);
    const response = await fetch(zipUrl);
    if (!response.ok) {
      throw new Error(`HTTP ${response.status} from ${zipUrl}`);
    }
    const bytes = Buffer.from(await response.arrayBuffer());
    await writeFile(zipPath, bytes);
    console.log(`${(bytes.length / 1024 / 1024).toFixed(1)}MB`);
    for (const m of missing) {
      const dest = path.join(dir, m.to);
      try {
        const out = execFileSync("unzip", ["-p", zipPath, m.from], {
          maxBuffer: 256 * 1024 * 1024,
        });
        await writeFile(dest, out);
        console.log(`ok       ${dest} (from ${m.from})`);
      } catch (error) {
        console.log(`FAILED   ${dest} (member ${m.from}: ${error.message})`);
        failures++;
      }
    }
  } catch (error) {
    console.log(`FAILED (${error.message})`);
    failures++;
  } finally {
    await rm(tmp, { recursive: true, force: true });
  }
}

process.exit(failures === 0 ? 0 : 1);
