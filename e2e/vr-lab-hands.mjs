// vr-lab hands E2E: the capsule glove (`examples/vr-lab/hand.fun`) and the
// skinned glove (`skin.fun`, the default) driven by injected XR samples,
// headlessly.
//
// Plays scenarios in `e2e/vr-lab-scenarios.mjs` against `examples/vr-lab`
// (`--headless`, pinned clock, one sample per fixed step) and reads the
// glove from `/scene` (children 4/5 the left and right hands, 6/7 their
// ghosts): two parts (the palm and thumb, then the fingers), each one
// instanced draw of its capsules' caps and one of their cylinders, under the
// hand's `Scene.atPose`.
//
//   1. Both hands turned palms down: every cap and cylinder sits, turned and
//      sized, where forward kinematics puts it for the injected grips (the
//      skeleton below, computed independently of the game; the fingers at
//      their drawn 1/64 curl levels), within 0.1 mm, open, squeezed, the
//      trigger pulled and let out.
//   2. The squeeze curls the middle, ring and little fingers and not the
//      index; the trigger curls the index and nothing else.
//   3. A held pistol's index follows its trigger (0, half, full, out); its
//      other fingers stay wrapped.
//   4. The ghost: past 2 cm the same glove, translucent (0.4), at the tracked
//      grip; within it, none.
//   (1-4 switch to the capsule glove first, with G.)
//   5. The skinned glove, open, a fist and the trigger alone: each of its 20
//      skinned joints placed (`Anim.place`, read from `/scene`) where FK puts
//      it in the world, within 1 mm, each turned within its correction's
//      bound of FK's joint (12.5° the wrist, 6° the rest). G switches it to
//      the capsules and back, in `/state` and `/scene`.
//   6. A held pistol's skinned index follows its trigger, placed as FK puts
//      the preset; the skinned ghost is the drawn hand's, translucent.
//   7. The pistol's hooked index (its tip bent further than its knuckle):
//      every capsule (placed, turned, sized) and every skinned joint (placed,
//      turned within its correction's bound) where FK puts the preset's
//      curl and tip, at each pull.
//
//   8. A support hand on the shotgun's fore-end, home and pumped back: drawn
//      at its fitted placement (`/state`'s `shown`), every capsule where FK
//      puts its fitted fingers; it rides the fore-end back its travel.
//   9. The ball taken at two of its fitted placements: each its own
//      placement (`grasp`) and its own fitted pose, the capsules at FK.
//
//   cargo build -p functor-cli --no-default-features
//   node e2e/vr-lab-hands.mjs
import { readFileSync } from "node:fs";
import { ROOT, check, failures } from "./mcp-rpc.mjs";
import { add, compose, len, qmul, qrot, record, rigToWorld, session, sub } from "./xr-scenario.mjs";
import { BALL, armory, grabTable, project, scenarios } from "./vr-lab-scenarios.mjs";

const PORT = 8300;
const run = (name, body) => session(name, project, PORT, body);
// A scenario with the capsule glove (G pressed before it starts).
const capsules = (s) => ({ ...s, steps: [{ frames: 1, tap: "G" }, ...s.steps] });

// --- the skeleton, as `hand.fun` states it (right hand, wrist frame) ---
const deg = (d) => (d * Math.PI) / 180;
const axisAngle = ([x, y, z], a) => [x * Math.sin(a / 2), y * Math.sin(a / 2), z * Math.sin(a / 2), Math.cos(a / 2)];
const flex = (a) => axisAngle([1, 0, 0], -a);
const yawQ = (a) => axisAngle([0, 1, 0], a);
const FINGERS = [
  { base: [-0.019, 0, -0.022], splay: 0.12, bones: [0.066, 0.04, 0.023, 0.02], radii: [0.0095, 0.0085, 0.0075] },
  { base: [-0.004, 0, -0.024], splay: 0, bones: [0.064, 0.045, 0.027, 0.021], radii: [0.0098, 0.0088, 0.0078] },
  { base: [0.007, 0, -0.023], splay: -0.12, bones: [0.058, 0.042, 0.026, 0.02], radii: [0.0092, 0.0082, 0.0073] },
  { base: [0.016, 0, -0.021], splay: -0.24, bones: [0.053, 0.033, 0.019, 0.018], radii: [0.0082, 0.0072, 0.0065] },
];
const THUMB = { base: [-0.022, -0.01, -0.02], bones: [0.046, 0.032, 0.029], rest: [0.7, 0.3, 1.0], opposed: [0.3, 1.3, 1.6], flex: [0.2, 0.9, 1.2] };
const FIST = [deg(80), deg(100), deg(65)];
const WRIST = { position: [0.031, -0.015, 0.062], orientation: qmul(axisAngle([1, 0, 0], 0.2), axisAngle([0, 0, 1], -Math.PI / 2)) };
// A finger pose: the thumb's curl (its CMC and MCP), tip (its IP) and
// opposition; each finger's curl (its MCP) and tip (its PIP and DIP).
// `uniform`: one curl a finger, the tip with it.
const uniform = (c) => ({ curl: c, tip: c });
// The empty hand's (`Hand.empty`).
const POWER = { middle: 0.75, ring: 0.72, little: 0.6 };
const mix = (a, b, t) => a + (b - a) * Math.min(1, Math.max(0, t));
const empty = (trigger, squeeze) => ({
  thumb: { curl: 0.5, tip: 0.5, opposition: 0.25 },
  fingers: [mix(0.15, 0.55, trigger), mix(0.15, POWER.middle, squeeze), mix(0.15, POWER.ring, squeeze), mix(0.15, POWER.little, squeeze)].map(uniform),
});
// The pistol grip's: its fitted pose (generated into `fitted.fun`), the
// trigger pulling the index in (`Hand.pullTrigger`: 0.30 a curl, 0.35 a tip).
const fitted = (() => {
  const line = readFileSync(`${ROOT}examples/vr-lab/fitted.fun`, "utf8").split("\n").find((l) => l.startsWith("let pistolGrip:"));
  const v = [...line.matchAll(/(curl|tip|opposition): (0\.0 - )?([0-9.]+)/g)].map((m) => (m[2] ? -1 : 1) * Number(m[3]));
  return { thumb: { curl: v[0], tip: v[1], opposition: v[2] }, fingers: [0, 1, 2, 3].map((k) => ({ curl: v[3 + 2 * k], tip: v[4 + 2 * k] })) };
})();
const onTrigger = (t) => {
  const k = Math.min(1, Math.max(0, t));
  const [index, ...rest] = fitted.fingers;
  return { thumb: fitted.thumb, fingers: [{ curl: index.curl + k * 0.3, tip: index.tip + k * 0.35 }, ...rest] };
};

// Each bone `[length, flexion at its base]` from `base`: the joints, then the tip.
const chain = (base, bones) => {
  const out = [];
  let { position: p, orientation: q } = base;
  for (const [length, angle] of bones) {
    q = qmul(q, flex(angle));
    out.push({ position: p, orientation: q });
    p = add(p, qrot(q, [0, 0, -length]));
  }
  return [...out, { position: p, orientation: q }];
};
// The 26 joints in the grip's frame (OpenXR order), the left mirrored.
function joints(side, pose) {
  const t = Math.min(1, Math.max(-1, pose.thumb.opposition));
  const [ty, tp, tr] = THUMB.rest.map((r, i) => r + (THUMB.opposed[i] - r) * t);
  const thumbQ = qmul(yawQ(ty), qmul(flex(tp), axisAngle([0, 0, 1], tr)));
  const inWrist = [
    { position: add(FINGERS[1].base, [0, 0, -FINGERS[1].bones[0] / 2]), orientation: [0, 0, 0, 1] },
    { position: [0, 0, 0], orientation: [0, 0, 0, 1] },
    ...chain({ position: THUMB.base, orientation: thumbQ }, THUMB.bones.map((b, i) => [b, (i < 2 ? pose.thumb.curl : pose.thumb.tip) * THUMB.flex[i]])),
    ...FINGERS.flatMap((f, k) => {
      const { curl, tip } = pose.fingers[k];
      return chain({ position: f.base, orientation: yawQ(f.splay) }, f.bones.map((b, i) => [b, i === 0 ? 0 : (i === 1 ? curl : tip) * FIST[i - 1]]));
    }),
  ];
  const right = inWrist.map((j) => compose(WRIST, j));
  return side === "right" ? right : right.map(({ position: [x, y, z], orientation: [qx, qy, qz, qw] }) => ({ position: [-x, y, z], orientation: [qx, -qy, -qz, qw] }));
}
// The capsules, `[joint, radius, length]` (`Hand.capsules`).
const CAPSULES = [
  [2, 0.014, THUMB.bones[0]], [3, 0.0105, THUMB.bones[1]], [4, 0.0095, THUMB.bones[2] - 0.0095],
  ...FINGERS.flatMap((f, k) => {
    const j = 6 + 5 * k;
    const [rp, ri, rd] = f.radii;
    return [[j, 0.014, f.bones[0]], [j + 1, rp, f.bones[1]], [j + 2, ri, f.bones[2]], [j + 3, rd, f.bones[3] - rd]];
  }),
];
// Each capsule's two ends in the grip's frame.
const capsuleEnds = (side, pose) => {
  const js = joints(side, pose);
  return CAPSULES.map(([j, , length]) => [js[j].position, add(js[j].position, qrot(js[j].orientation, [0, 0, -length]))]);
};
// The glove's two parts, in draw order: the palm and thumb (the thumb's and
// the metacarpals' capsules), then the fingers' phalanges. Its cylinders are
// theirs in that order; its caps each chain's first start (the
// metacarpals'), then each capsule's end in that order.
const STARTS = [2, 6, 11, 16, 21];
const PALM = CAPSULES.flatMap(([j], i) => (j < 6 || STARTS.includes(j) ? [i] : []));
const ORDER = [...PALM, ...CAPSULES.flatMap((_, i) => (PALM.includes(i) ? [] : [i]))];
const CAPS = [...PALM.filter((i) => STARTS.includes(CAPSULES[i][0])).map((i) => ["start", i]), ...ORDER.map((i) => ["end", i])];
const caps = (ends) => CAPS.map(([end, i]) => ends[i][end === "start" ? 0 : 1]);
const capOf = (end, i) => CAPS.findIndex(([e, j]) => e === end && j === i);
// The fingers are drawn bent to the nearest 1/64, curl and tip
// (`Hand.levels`).
const level = (c) => Math.round(Math.min(1, Math.max(0, c)) * 64) / 64;
const drawnPose = (pose) => ({ ...pose, fingers: pose.fingers.map(({ curl, tip }) => ({ curl: level(curl), tip: level(tip) })) });

// --- reading the glove from `/scene` ---
const IDENTITY = JSON.stringify([[1, 0, 0, 0], [0, 1, 0, 0], [0, 0, 1, 0], [0, 0, 0, 1]]);
// Every node with `key` under `node`, depth first (`xforms`: the transforms above each, outermost first).
function find(node, key, xforms = [], out = []) {
  if (!node || typeof node !== "object") return out;
  const here = Array.isArray(node.xform) && JSON.stringify(node.xform) !== IDENTITY ? [...xforms, node.xform] : xforms;
  if (node.obj?.[key] !== undefined) out.push({ value: node.obj[key], xforms: here });
  for (const child of Object.values(node)) {
    if (Array.isArray(child)) child.forEach((c) => find(c, key, here, out));
    else if (child && typeof child === "object") find(child, key, here, out);
  }
  return out;
}
// A column-major 4×4 applied to a point (`w` 1) or a direction (`w` 0).
const apply = (m, [x, y, z], w = 1) => [0, 1, 2].map((r) => m[0][r] * x + m[1][r] * y + m[2][r] * z + m[3][r] * w);
const unit = (v) => v.map((c) => c / len(v));
// A hand's glove: its cap and cylinder instances, each part's in draw order
// (positions and the cylinders' axes in the world).
function glove(node) {
  const draws = find(node, "Instanced");
  if (draws.length === 0) return undefined;
  const world = (p, xforms, w = 1) => xforms.reduceRight((q, m) => apply(m, q, w), p);
  const of = (geometry) =>
    draws.filter((d) => JSON.stringify(d.value.template).includes(`"${geometry}"`)).flatMap((d) => d.value.instances.map((i) => ({ i, xforms: d.xforms })));
  const [spheres, cylinders] = [of("Sphere"), of("Cylinder")];
  return {
    spheres: spheres.map(({ i, xforms }) => ({ at: world(i.position, xforms), radius: i.scale[0] })),
    cylinders: cylinders.map(({ i, xforms }) => ({
      at: world(i.position, xforms),
      axis: unit(world(qrot(i.rotation, [0, 1, 0]), xforms, 0)),
      size: i.scale,
    })),
    raw: [spheres.map(({ i }) => i), cylinders.map(({ i }) => i)],
  };
}
// Where FK puts the glove for a hand at grip `grip` (rig-local), in the world.
function expected(side, grip, pose, rig) {
  const toWorld = (p) => rigToWorld(add(grip.position, qrot(grip.orientation, p)), rig.feet, rig.yaw);
  const ends = capsuleEnds(side, drawnPose(pose));
  return {
    spheres: caps(ends).map((p, k) => ({ at: toWorld(p), radius: CAPSULES[CAPS[k][1]][1] })),
    cylinders: ORDER.map((i) => {
      const [a, b] = ends[i];
      return { at: toWorld(a.map((v, k) => (v + b[k]) / 2)), axis: unit(sub(toWorld(b), toWorld(a))), size: [2 * CAPSULES[i][1], CAPSULES[i][2], 2 * CAPSULES[i][1]] };
    }),
  };
}
// The farthest any cap or cylinder is from where it should be: its place, its
// axis's direction (as the distance between unit axes), and its radius and length.
const worst = (got, want) =>
  Math.max(
    ...got.spheres.map((c, i) => Math.max(len(sub(c.at, want.spheres[i].at)), Math.abs(c.radius - want.spheres[i].radius))),
    ...got.cylinders.map((c, i) => Math.max(len(sub(c.at, want.cylinders[i].at)), len(sub(c.axis, want.cylinders[i].axis)),
      ...c.size.map((s, k) => Math.abs(s - want.cylinders[i].size[k])))),
  );
// The fingertips' caps (the distal capsules' ends), and the palm joint (half
// way between the middle metacarpal's caps).
const TIP = { thumb: capOf("end", 2), index: capOf("end", 6), middle: capOf("end", 10), ring: capOf("end", 14), little: capOf("end", 18) };
const palmOf = (g) => g.spheres[capOf("start", 7)].at.map((v, i) => (v + g.spheres[capOf("end", 7)].at[i]) / 2);
const reach = (g, finger) => len(sub(g.spheres[TIP[finger]].at, palmOf(g)));
const mm = (d) => `${(d * 1000).toFixed(3)} mm`;

await run("glove at FK, squeezed and pulled", async (rt) => {
  const { scenes, at } = await record(rt, capsules(scenarios["hand-curl"]), { scenes: true });
  const kids = (mark) => scenes[mark].scene.obj.Group;
  const gripOf = (mark, side) => ({ position: at(mark)[side].at, orientation: at(mark)[side].orientation });
  const inputs = { open: [0, 0], squeezed: [0, 1], pulled: [1, 0], released: [0, 0] };
  for (const [mark, [trigger, squeeze]] of Object.entries(inputs)) {
    for (const [side, child] of [["left", 4], ["right", 5]]) {
      const g = glove(kids(mark)[child]);
      const want = expected(side, gripOf(mark, side), empty(trigger, squeeze), grabTable);
      const off = g ? worst(g, want) : Infinity;
      check(g && g.spheres.length === 24 && g.cylinders.length === 19 && off < 1e-4,
        `${mark}: the ${side} glove's 19 capsules where FK puts them, turned and sized (worst ${mm(off)})`);
    }
  }
  for (const [side, child] of [["left", 4], ["right", 5]]) {
    const [open, squeezed, pulled] = ["open", "squeezed", "pulled"].map((m) => glove(kids(m)[child]));
    const closer = (g, f) => reach(open, f) - reach(g, f);
    const fmtCm = (d) => `${(d * 100).toFixed(1)} cm`;
    check(["middle", "ring", "little"].every((f) => closer(squeezed, f) > 0.02) && Math.abs(closer(squeezed, "index")) < 1e-6,
      `${side}: the squeeze closes the middle, ring and little fingers (${["middle", "ring", "little"].map((f) => fmtCm(closer(squeezed, f))).join(", ")} nearer the palm), not the index`);
    check(closer(pulled, "index") > 0.02 && ["middle", "ring", "little", "thumb"].every((f) => Math.abs(closer(pulled, f)) < 1e-6),
      `${side}: the trigger closes the index (${fmtCm(closer(pulled, "index"))}) and nothing else`);
  }
  // The left glove is the right's mirror, both palms down: the thumbs inward.
  const [l, r] = [glove(kids("open")[4]), glove(kids("open")[5])];
  const across = (g) => sub(g.spheres[TIP.thumb].at, palmOf(g));
  const rightDir = rigToWorld([1, 0, 0], [0, 0, 0], grabTable.yaw);
  const dotX = (v) => v.reduce((s, c, i) => s + c * rightDir[i], 0);
  check(dotX(across(r)) < 0 && dotX(across(l)) > 0, "palms down, each thumb lies toward the other hand");
});

await run("a held pistol's index follows its trigger", async (rt) => {
  const { scenes, marks } = await record(rt, capsules(scenarios["pistol-trigger"]), { scenes: true });
  const g = (mark) => glove(scenes[mark].scene.obj.Group[5]);
  const reaches = ["trigger0", "trigger50", "trigger100", "triggerOut"].map((m) => reach(g(m), "index"));
  check(marks.trigger0.model.props.some((p) => p.kind.$ctor === "Props.Pistol" && p.held.$ctor === "Option.Some"), "the pistol in the right hand");
  check(reaches[1] < reaches[0] - 0.005 && reaches[2] < reaches[1] - 0.005 && Math.abs(reaches[3] - reaches[0]) < 1e-6,
    `the index closes with the trigger and opens with it (${reaches.map((d) => (d * 100).toFixed(2)).join(" → ")} cm from the palm)`);
  const others = ["middle", "ring", "little", "thumb"].every((f) => Math.abs(reach(g("trigger0"), f) - reach(g("trigger100"), f)) < 1e-6);
  check(others, "the other fingers stay wrapped");
  // Wrapped round the grip, the index on the trigger (its fitted pose).
  const local = (mark) => g(mark).raw[0].map((i) => i.position);
  const want = (t) => caps(capsuleEnds("right", drawnPose(onTrigger(t)))).map(([x, y, z]) => [-x, y, -z]);
  const off = Math.max(...[["trigger0", 0], ["trigger50", 0.5], ["trigger100", 1]].flatMap(([m, t]) => local(m).map((p, i) => len(sub(p, want(t)[i])))));
  check(off < 1e-4, `posed as the pistol grip's preset at each pull, in the hand's frame (worst ${mm(off)})`);
});

await run("the ghost glove", async (rt) => {
  const { scenes } = await record(rt, capsules(scenarios["hand-view"]), { scenes: true });
  const kids = (mark) => scenes[mark].scene.obj.Group;
  const ghost = kids("blockedAccepted")[7];
  const [opacity] = find(ghost, "Opacity");
  const drawn = glove(kids("blockedAccepted")[5]);
  const shadow = glove(ghost);
  check(opacity && Math.abs(opacity.value[0] - 0.4) < 1e-6, `blocked at the wall: a ghost at 0.4 opacity (${opacity?.value[0]})`);
  check(shadow && JSON.stringify(shadow.raw) === JSON.stringify(drawn.raw), "the same glove as the drawn hand, curled the same");
  const apart = shadow && len(sub(shadow.spheres[0].at, drawn.spheres[0].at));
  check(apart > 0.02, `drawn ${apart && (apart * 100).toFixed(1)} cm from it`);
  check(glove(kids("nearWall")[7]) === undefined && glove(kids("nearWall")[5]) !== undefined, "within 2 cm: no ghost");
});

// --- the skinned glove (`skin.fun`) ---
// Its skinned joints (indices into the 26: the wrist, the thumb's three
// bones, each finger's four), by their WebXR names.
const NAMES = ["palm", "wrist", "thumb-metacarpal", "thumb-phalanx-proximal", "thumb-phalanx-distal", "thumb-tip",
  ...["index", "middle", "ring", "pinky"].flatMap((f) => ["metacarpal", "phalanx-proximal", "phalanx-intermediate", "phalanx-distal", "tip"].map((b) => `${f}-finger-${b}`))];
const SKINNED = [1, 2, 3, 4, 6, 7, 8, 9, 11, 12, 13, 14, 16, 17, 18, 19, 21, 22, 23, 24];
const between = (a, b) => (Math.acos(Math.min(1, Math.max(-1, a.reduce((s, c, i) => s + c * b[i], 0)))) * 180) / Math.PI;
// A hand's skinned glove: its model's file and each placed joint's place and
// -Z/+Y axes in the world.
function skin(node) {
  const [model] = find(node, "Model");
  if (!model) return undefined;
  const world = (p, w = 1) => model.xforms.reduceRight((q, m) => apply(m, q, w), p);
  const joints = Object.fromEntries(model.value.animation.Place.joints.map((j) => [j.joint, {
    at: world(j.position),
    z: unit(world(qrot(j.rotation, [0, 0, -1]), 0)),
    y: unit(world(qrot(j.rotation, [0, 1, 0]), 0)),
  }]));
  return { file: model.value.handle.File, joints };
}
// Where FK puts each skinned joint for a hand at grip `grip` (rig-local),
// in the world: its place and -Z/+Y axes.
function skinExpected(side, grip, pose, rig) {
  const toWorld = (p) => rigToWorld(add(grip.position, qrot(grip.orientation, p)), rig.feet, rig.yaw);
  const dir = (q, v) => unit(sub(toWorld(qrot(q, v)), toWorld([0, 0, 0])));
  const js = joints(side, drawnPose(pose));
  return Object.fromEntries(SKINNED.map((j) => {
    const { position, orientation } = js[j];
    return [NAMES[j], { at: toWorld(position), z: dir(orientation, [0, 0, -1]), y: dir(orientation, [0, 1, 0]) }];
  }));
}
// The farthest a skinned joint is from FK's, and the largest turn (of -Z or
// +Y) past its correction's bound (the wrist's 12.5°, the rest 6°).
function skinOff(got, want) {
  let far = 0;
  let over = -Infinity;
  for (const j of SKINNED) {
    const [g, w] = [got.joints[NAMES[j]], want[NAMES[j]]];
    if (!g) return { far: Infinity, over: Infinity, count: Object.keys(got.joints).length };
    far = Math.max(far, len(sub(g.at, w.at)));
    over = Math.max(over, Math.max(between(g.z, w.z), between(g.y, w.y)) - (j === 1 ? 12.5 : 6));
  }
  return { far, over, count: Object.keys(got.joints).length };
}

await run("skinned glove at FK: open, a fist, the trigger; G switches it", async (rt) => {
  const { scenes, at, marks } = await record(rt, scenarios["skin-curl"], { scenes: true });
  const kids = (mark) => scenes[mark].scene.obj.Group;
  const gripOf = (mark, side) => ({ position: at(mark)[side].at, orientation: at(mark)[side].orientation });
  const inputs = { open: [0, 0], fist: [1, 1], trigger: [1, 0] };
  check(marks.open.model.glove.$ctor === "Skinned", "the skinned glove by default (`/state` glove: Skinned)");
  for (const [mark, [trigger, squeeze]] of Object.entries(inputs)) {
    for (const [side, child] of [["left", 4], ["right", 5]]) {
      const g = skin(kids(mark)[child]);
      const off = g ? skinOff(g, skinExpected(side, gripOf(mark, side), empty(trigger, squeeze), grabTable)) : { far: Infinity, over: Infinity, count: 0 };
      check(g && g.file.endsWith(`generic-hand/${side}.glb`) && off.count === 20 && off.far < 0.001 && off.over <= 0,
        `${mark}: the ${side} skinned glove's 20 joints where FK puts them (worst ${mm(off.far)}), each turned within its correction's bound (${(-off.over).toFixed(2)}° inside)`);
    }
  }
  // The fist closes every finger on the palm; the trigger alone, the index.
  const tipToPalm = (mark, side, tip) => {
    const js = skin(kids(mark)[side === "left" ? 4 : 5]).joints;
    return len(sub(js[`${tip}-finger-phalanx-distal`].at, js["middle-finger-metacarpal"].at));
  };
  for (const side of ["left", "right"]) {
    const closer = (mark, tip) => tipToPalm("open", side, tip) - tipToPalm(mark, side, tip);
    check(["index", "middle", "ring", "pinky"].every((f) => closer("fist", f) > 0.02), `${side}: the fist closes every finger`);
    check(closer("trigger", "index") > 0.02 && ["middle", "ring", "pinky"].every((f) => Math.abs(closer("trigger", f)) < 1e-6), `${side}: the trigger alone closes the index`);
  }
  // G: the capsules (no model), in `/state` and `/scene`; G again: skinned.
  const capsule = kids("capsule");
  check(marks.capsule.model.glove.$ctor === "Capsule" && glove(capsule[5])?.spheres.length === 24 && !skin(capsule[5]) && !skin(capsule[4]),
    "G: the capsule glove, in `/state` and `/scene`");
  check(marks.skinned.model.glove.$ctor === "Skinned" && skin(kids("skinned")[5]) && !glove(kids("skinned")[5]), "G again: the skinned glove");
});

await run("a held pistol's skinned index", async (rt) => {
  const { scenes } = await record(rt, scenarios["pistol-trigger"], { scenes: true });
  const local = (mark) => {
    const [model] = find(scenes[mark].scene.obj.Group[5], "Model");
    return Object.fromEntries(model.value.animation.Place.joints.map((j) => [j.joint, j.position]));
  };
  // In the hand's frame (the grip's turned half round +Y), as FK puts the preset.
  const want = (t) => joints("right", drawnPose(onTrigger(t))).map(({ position: [x, y, z] }) => [-x, y, -z]);
  const off = Math.max(...[["trigger0", 0], ["trigger50", 0.5], ["trigger100", 1]].flatMap(([m, t]) =>
    SKINNED.map((j) => len(sub(local(m)[NAMES[j]], want(t)[j])))));
  check(off < 1e-4, `posed as the pistol grip's preset at each pull (worst ${mm(off)})`);
  const reach = (m) => len(sub(local(m)["index-finger-phalanx-distal"], local(m)["middle-finger-metacarpal"]));
  check(reach("trigger50") < reach("trigger0") - 0.005 && Math.abs(reach("triggerOut") - reach("trigger0")) < 1e-6, "the index closes with the trigger and opens with it");
});

await run("the skinned ghost", async (rt) => {
  const { scenes } = await record(rt, scenarios["hand-view"], { scenes: true });
  const kids = scenes.blockedAccepted.scene.obj.Group;
  const [opacity] = find(kids[7], "Opacity");
  const [drawn, shadow] = [find(kids[5], "Model")[0], find(kids[7], "Model")[0]];
  check(opacity && Math.abs(opacity.value[0] - 0.4) < 1e-6 && shadow && JSON.stringify(shadow.value) === JSON.stringify(drawn.value),
    "blocked at the wall: the skinned glove's ghost, posed the same, at 0.4 opacity");
  const apart = shadow && len(sub(skin(kids[7]).joints.wrist.at, skin(kids[5]).joints.wrist.at));
  check(apart > 0.02, `drawn ${apart && (apart * 100).toFixed(1)} cm from it`);
});

// A held pistol, both gloves, in the hand's frame (the grip's turned half
// round +Y: `Hand.partOf`'s and `Skin.toModel`'s): the preset's index has a
// tip apart from its curl, and every capsule and skinned joint is where the
// two-curl FK puts it.
await run("a hooked index (tip ≠ curl): capsules and skinned at FK", async (rt) => {
  const PULLS = [["trigger0", 0], ["trigger50", 0.5], ["trigger100", 1]];
  const turned = ([x, y, z]) => [-x, y, -z];
  // Skinned (the default) first, then G: the capsules.
  const skinned = await record(rt, scenarios["pistol-trigger"], { scenes: true });
  const capsule = await record(rt, capsules(scenarios["pistol-trigger"]), { scenes: true });
  // Each cap's place and radius, each cylinder's place, axis and size.
  let off = 0;
  for (const [mark, t] of PULLS) {
    const [spheres, cylinders] = glove(capsule.scenes[mark].scene.obj.Group[5]).raw;
    if (spheres.length !== 24 || cylinders.length !== 19) off = Infinity;
    const ends = capsuleEnds("right", drawnPose(onTrigger(t)));
    const wantCaps = caps(ends).map(turned);
    off = Math.max(off, ...spheres.map((i, k) => Math.max(len(sub(i.position, wantCaps[k])), Math.abs(i.scale[0] - CAPSULES[CAPS[k][1]][1]))));
    off = Math.max(off, ...cylinders.map((i, k) => {
      const [a, b] = ends[ORDER[k]].map(turned);
      const [, radius, length] = CAPSULES[ORDER[k]];
      const size = [2 * radius, length, 2 * radius];
      return Math.max(len(sub(i.position, a.map((v, c) => (v + b[c]) / 2))), len(sub(qrot(i.rotation, [0, 1, 0]), unit(sub(b, a)))),
        ...i.scale.map((s, c) => Math.abs(s - size[c])));
    }));
  }
  check(off < 1e-4, `capsules: each of the 24 caps and 19 cylinders at FK's curl and tip, at each pull (worst ${mm(off)})`);
  // Skinned: each joint placed at FK's, its -Z and +Y within its
  // correction's bound of FK's joint.
  let far = 0;
  let over = -Infinity;
  for (const [mark, t] of PULLS) {
    const [model] = find(skinned.scenes[mark].scene.obj.Group[5], "Model");
    const got = Object.fromEntries(model.value.animation.Place.joints.map((j) => [j.joint, j]));
    const js = joints("right", drawnPose(onTrigger(t)));
    for (const j of SKINNED) {
      const g = got[NAMES[j]];
      const { position, orientation } = js[j];
      far = Math.max(far, len(sub(g.position, turned(position))));
      const axes = [[0, 0, -1], [0, 1, 0]].map((v) => between(unit(qrot(g.rotation, v)), turned(qrot(orientation, v))));
      over = Math.max(over, Math.max(...axes) - (j === 1 ? 12.5 : 6));
    }
  }
  check(far < 1e-4 && over <= 0, `skinned: each of the 20 joints at FK's curl and tip, at each pull (worst ${mm(far)}), turned within its correction's bound (${(-over).toFixed(2)}° inside)`);
});

// --- fitted holds (`grip.fun`'s tables, looked up at runtime) ---
// A hand as `/state` shows it drawn (`shown`): its grip (rig-local) and its
// fingers, in this script's shapes.
const sideOf = (s) => (s.$ctor ?? s).endsWith("Left") ? "left" : "right";
function shownOf(state, side) {
  const h = state.model.shown.find((s) => sideOf(s.side) === side);
  if (!h) return undefined;
  const { position: p, orientation: q } = h.at;
  const f = h.fingers;
  return { grip: { position: [p.x, p.y, p.z], orientation: [q.x, q.y, q.z, q.w] }, pose: { thumb: f.thumb, fingers: [f.index, f.middle, f.ring, f.little] } };
}
const somed = (o) => (o?.$ctor === "Option.Some" ? o.args[0] : undefined);

await run("a support hand fitted on the sliding fore-end", async (rt) => {
  const loop = scenarios["shotgun-loop"];
  const upTo = loop.steps.findIndex((s) => s.mark === "pumped");
  const { scenes, marks } = await record(rt, capsules({ ...loop, steps: loop.steps.slice(0, upTo + 1) }), { scenes: true });
  const [home, back] = ["onFore", "pumpedBack"].map((m) => shownOf(marks[m], "left"));
  for (const [mark, s] of [["onFore", home], ["pumpedBack", back]]) {
    const g = glove(scenes[mark].scene.obj.Group[4]);
    const off = g && s ? worst(g, expected("left", s.grip, s.pose, armory)) : Infinity;
    check(off < 1e-4, `${mark}: the support glove's capsules where FK puts its fitted fingers, at its fitted placement (worst ${mm(off)})`);
  }
  const moved = home && back ? len(sub(back.grip.position, home.grip.position)) : 0;
  check(Math.abs(moved - 0.09) < 0.005, `pumped back, the support hand rides the fore-end back ${(moved * 100).toFixed(1)} cm (its 9 cm travel)`);
  const wrapped = (s) => s && s.pose.fingers.every((f) => f.curl > 0.3);
  check(wrapped(home) && wrapped(back), "its fingers wrap the fore-end (each finger's curl over 0.3) at both");
});

// The ball's centre in a right hand's grip frame at two of its fitted
// placements (`Grip.graspSets`): its spec's, and 2 cm toward the fingers.
const BALL_IN_HAND = [[-0.044, 0, 0.0079], [-0.044, 0, -0.0121]];
const takeBall = (centre) => {
  const at = sub(BALL, centre);
  return {
    station: 1,
    steps: [
      { frames: 20, right: { to: add(at, [0, 0.12, 0]), squeeze: 0 } },
      { frames: 15, right: { to: at } },
      { frames: 6, right: { squeeze: 1 }, mark: "held" },
      { frames: 10, right: { to: add(at, [0, 0.1, 0]) }, mark: "lifted" },
    ],
  };
};
const taken = [];
for (const [i, centre] of BALL_IN_HAND.entries()) {
  await run(`a ball taken at its placement ${i}, fitted there`, async (rt) => {
    const { scenes, marks } = await record(rt, capsules(takeBall(centre)), { scenes: true });
    const ball = marks.lifted.model.props.find((p) => p.kind.$ctor === "Props.Ball");
    const s = shownOf(marks.lifted, "right");
    const g = glove(scenes.lifted.scene.obj.Group[5]);
    const off = g && s ? worst(g, expected("right", s.grip, s.pose, grabTable)) : Infinity;
    check(somed(ball?.held) && off < 1e-4, `held at placement ${somed(ball?.grasp)}: the glove's capsules where FK puts its fitted fingers (worst ${mm(off)})`);
    taken.push({ grasp: somed(ball?.grasp), pose: s?.pose });
  });
}
const [ta, tb] = taken;
check(ta?.grasp === 0 && tb?.grasp === 1, `the ball taken two ways: its spec's placement, then the one 2 cm toward the fingers (${ta?.grasp}, ${tb?.grasp})`);
check(ta && tb && JSON.stringify(ta.pose) !== JSON.stringify(tb.pose), "two different fitted poses");

if (failures.length) {
  console.error(`\n${failures.length} failure(s)`);
  process.exit(1);
}
console.log("\nvr-lab hands e2e: all checks passed");
