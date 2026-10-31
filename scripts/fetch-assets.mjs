// Fetch the sample assets referenced by the examples (glTF models, skybox
// faces). None are checked into the repo (*.glb / example *.jpg are
// gitignored). Sources: BabylonJS/Assets, Kenney, and Poly Haven (credited in
// the README / per-example ASSETS.md). Existing files are left alone, so this
// is safe to re-run.
//
// Usage: npm run fetch:assets

import { writeFile, stat, mkdtemp, rm } from "node:fs/promises";
import { execFileSync } from "node:child_process";
import os from "node:os";
import path from "node:path";
import { fetchPolyHavenGlb } from "./polyhaven-gltf.mjs";

const MESHES_BASE = "https://raw.githubusercontent.com/BabylonJS/Assets/master/meshes";
const SKYBOX_BASE =
  "https://raw.githubusercontent.com/BabylonJS/Assets/master/skyboxes/TropicalSunnyDay";
// Poly Haven serves direct per-file CDN URLs (no archive to unpack), so its
// textures drop straight into the same fetch as everything else. All CC0.
const POLYHAVEN_BASE = "https://dl.polyhaven.org/file/ph-assets/Textures/jpg/1k";

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
    dir: "examples/crowd",
    baseUrl: MESHES_BASE,
    assets: ["Xbot.glb"],
  },
  {
    dir: "examples/fullbody",
    baseUrl: MESHES_BASE,
    assets: ["Xbot.glb"],
  },
  {
    // Terrain detail maps, one per `Terrain.layered` band. Poly Haven nests
    // each texture under its own id, so these carry a full path rather than a
    // bare filename.
    dir: "examples/terrain",
    baseUrl: POLYHAVEN_BASE,
    assets: [
      { from: "forest_ground_04/forest_ground_04_diff_1k.jpg", to: "detail-low.jpg" },
      { from: "aerial_grass_rock/aerial_grass_rock_diff_1k.jpg", to: "detail-high.jpg" },
      { from: "rock_face/rock_face_diff_1k.jpg", to: "detail-rock.jpg" },
      { from: "snow_02/snow_02_diff_1k.jpg", to: "detail-snow.jpg" },
    ],
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

// Poly Haven CC0 models, repacked to a single .glb and simplified to a Quest
// budget (≤ ~5k triangles each). See `polyhaven-gltf.mjs` for the options.
const FIR_BRANCHES = { id: "fir_sapling_medium", dropMaterials: ["fir_sapling_medium_twigs"] };
const POLYHAVEN_MODELS = [
  {
    dir: "examples/night-terrain",
    models: [
      // The firs' trunks and branches only: their needles are dense geometry
      // that simplification erases, so foliage comes from baked impostor cards.
      { to: "fir-a.glb", ...FIR_BRANCHES, node: 0, simplify: 0.45 },
      { to: "fir-b.glb", ...FIR_BRANCHES, node: 1, simplify: 0.85 },
      { to: "fir-c.glb", ...FIR_BRANCHES, node: 2, simplify: 0.85 },
      { to: "stump.glb", id: "tree_stump_01", simplify: 0.1 },
      { to: "log.glb", id: "dead_tree_trunk", simplify: 0.045 },
      // Four single boulders picked out of Poly Haven's two mossy rock sets.
      { to: "rock-a.glb", id: "rock_moss_set_01", node: 0, simplify: 0.4 },
      { to: "rock-b.glb", id: "rock_moss_set_01", node: 3, simplify: 0.45 },
      { to: "rock-c.glb", id: "rock_moss_set_01", node: 4, simplify: 0.28 },
      { to: "rock-d.glb", id: "rock_moss_set_02", node: 5, simplify: 0.6 },
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
  for (const asset of assets) {
    // A bare string is fetched and saved under the same name; `{ from, to }`
    // renames, for hosts that nest a file under a per-asset path.
    const from = typeof asset === "string" ? asset : asset.from;
    const to = typeof asset === "string" ? asset : asset.to;
    const dest = path.join(dir, to);
    if (await exists(dest)) {
      console.log(`ok       ${dest}`);
      continue;
    }

    const url = `${baseUrl}/${from}`;
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

for (const { dir, models } of POLYHAVEN_MODELS) {
  for (const { to, ...spec } of models) {
    const dest = path.join(dir, to);
    if (await exists(dest)) {
      console.log(`ok       ${dest}`);
      continue;
    }
    process.stdout.write(`fetching ${dest} (Poly Haven ${spec.id}) ... `);
    try {
      const triangles = await fetchPolyHavenGlb({ ...spec, out: dest });
      console.log(`${triangles} triangles`);
    } catch (error) {
      console.log(`FAILED (${error.message})`);
      failures++;
    }
  }
}

process.exit(failures === 0 ? 0 : 1);
