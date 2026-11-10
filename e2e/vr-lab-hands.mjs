// vr-lab hands E2E: the capsule glove (`examples/vr-lab/hand.fun`) driven by
// injected XR samples, headlessly.
//
// Plays scenarios in `e2e/vr-lab-scenarios.mjs` against `examples/vr-lab`
// (`--headless`, pinned clock, one sample per fixed step) and reads the
// glove from `/scene` (children 4/5 the left and right hands, 6/7 their
// ghosts): one instanced draw of the capsules' caps (each chain's first
// start, then every end) and one of their cylinders, under the hand's
// `Scene.atPose`.
//
//   1. Both hands turned palms down: every cap and cylinder sits, turned and
//      sized, where forward kinematics puts it for the injected grips (the
//      skeleton below, computed independently of the game), within 0.1 mm,
//      open, squeezed, the trigger pulled and let out.
//   2. The squeeze curls the middle, ring and little fingers and not the
//      index; the trigger curls the index and nothing else.
//   3. A held pistol's index follows its trigger (0, half, full, out); its
//      other fingers stay wrapped.
//   4. The ghost: past 2 cm the same glove, translucent (0.4), at the tracked
//      grip; within it, none.
//
//   cargo build -p functor-cli --no-default-features
//   node e2e/vr-lab-hands.mjs
import { check, failures } from "./mcp-rpc.mjs";
import { add, compose, len, qmul, qrot, record, rigToWorld, session, sub } from "./xr-scenario.mjs";
import { grabTable, project, scenarios } from "./vr-lab-scenarios.mjs";

const PORT = 8300;
const run = (name, body) => session(name, project, PORT, body);

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
// The empty hand's finger pose (`Hand.empty`).
const POWER = { middle: 0.75, ring: 0.72, little: 0.6 };
const mix = (a, b, t) => a + (b - a) * Math.min(1, Math.max(0, t));
const empty = (trigger, squeeze) => ({
  thumb: { curl: 0.5, opposition: 0.25 },
  curls: [mix(0.15, 0.55, trigger), mix(0.15, POWER.middle, squeeze), mix(0.15, POWER.ring, squeeze), mix(0.15, POWER.little, squeeze)],
});

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
  const t = Math.min(1, Math.max(0, pose.thumb.opposition));
  const [ty, tp, tr] = THUMB.rest.map((r, i) => r + (THUMB.opposed[i] - r) * t);
  const thumbQ = qmul(yawQ(ty), qmul(flex(tp), axisAngle([0, 0, 1], tr)));
  const inWrist = [
    { position: add(FINGERS[1].base, [0, 0, -FINGERS[1].bones[0] / 2]), orientation: [0, 0, 0, 1] },
    { position: [0, 0, 0], orientation: [0, 0, 0, 1] },
    ...chain({ position: THUMB.base, orientation: thumbQ }, THUMB.bones.map((b, i) => [b, pose.thumb.curl * THUMB.flex[i]])),
    ...FINGERS.flatMap((f, k) =>
      chain({ position: f.base, orientation: yawQ(f.splay) }, f.bones.map((b, i) => [b, i === 0 ? 0 : pose.curls[k] * FIST[i - 1]]))),
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
// The glove's caps: each chain's first start (the metacarpals'), then every
// capsule's end.
const FIRSTS = CAPSULES.flatMap(([j], i) => ([2, 6, 11, 16, 21].includes(j) ? [i] : []));
const caps = (ends) => [...FIRSTS.map((i) => ends[i][0]), ...ends.map(([, b]) => b)];

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
// A hand's glove: its cap and cylinder instances (positions and the
// cylinders' axes in the world).
function glove(node) {
  const [spheres, cylinders] = find(node, "Instanced");
  if (!spheres || !cylinders) return undefined;
  const world = (p, xforms, w = 1) => xforms.reduceRight((q, m) => apply(m, q, w), p);
  return {
    spheres: spheres.value.instances.map((i) => ({ at: world(i.position, spheres.xforms), radius: i.scale[0] })),
    cylinders: cylinders.value.instances.map((i) => ({
      at: world(i.position, cylinders.xforms),
      axis: unit(world(qrot(i.rotation, [0, 1, 0]), cylinders.xforms, 0)),
      size: i.scale,
    })),
    raw: [spheres.value.instances, cylinders.value.instances],
  };
}
// Where FK puts the glove for a hand at grip `grip` (rig-local), in the world.
function expected(side, grip, pose, rig) {
  const toWorld = (p) => rigToWorld(add(grip.position, qrot(grip.orientation, p)), rig.feet, rig.yaw);
  const ends = capsuleEnds(side, pose);
  const radii = [...FIRSTS.map((i) => CAPSULES[i][1]), ...CAPSULES.map(([, r]) => r)];
  return {
    spheres: caps(ends).map((p, i) => ({ at: toWorld(p), radius: radii[i] })),
    cylinders: ends.map(([a, b], i) => ({
      at: toWorld(a.map((v, k) => (v + b[k]) / 2)),
      axis: unit(sub(toWorld(b), toWorld(a))),
      size: [2 * CAPSULES[i][1], CAPSULES[i][2], 2 * CAPSULES[i][1]],
    })),
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
// Cap indices (`caps`): five chain starts, then each capsule's end. The
// distal capsules end at the fingertips; the middle metacarpal's two caps
// straddle the palm joint.
const TIP = { thumb: 7, index: 11, middle: 15, ring: 19, little: 23 };
const palmOf = (g) => g.spheres[2].at.map((v, i) => (v + g.spheres[12].at[i]) / 2);
const reach = (g, finger) => len(sub(g.spheres[TIP[finger]].at, palmOf(g)));
const mm = (d) => `${(d * 1000).toFixed(3)} mm`;

await run("glove at FK, squeezed and pulled", async (rt) => {
  const { scenes, at } = await record(rt, scenarios["hand-curl"], { scenes: true });
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
  const { scenes, marks } = await record(rt, scenarios["pistol-trigger"], { scenes: true });
  const g = (mark) => glove(scenes[mark].scene.obj.Group[5]);
  const reaches = ["trigger0", "trigger50", "trigger100", "triggerOut"].map((m) => reach(g(m), "index"));
  check(marks.trigger0.model.props.some((p) => p.kind.$ctor === "Props.Pistol" && p.held.$ctor === "Option.Some"), "the pistol in the right hand");
  check(reaches[1] < reaches[0] - 0.005 && reaches[2] < reaches[1] - 0.005 && Math.abs(reaches[3] - reaches[0]) < 1e-6,
    `the index closes with the trigger and opens with it (${reaches.map((d) => (d * 100).toFixed(2)).join(" → ")} cm from the palm)`);
  const others = ["middle", "ring", "little", "thumb"].every((f) => Math.abs(reach(g("trigger0"), f) - reach(g("trigger100"), f)) < 1e-6);
  check(others, "the other fingers stay wrapped");
  // Wrapped round the grip, the index on the trigger (`Hand.onTrigger`).
  const pose = (t) => ({ thumb: { curl: 0.75, opposition: 1 }, curls: [mix(0.25, 0.55, t), 0.75, 0.72, 0.6] });
  const local = (mark) => g(mark).raw[0].map((i) => i.position);
  const want = (t) => caps(capsuleEnds("right", pose(t))).map(([x, y, z]) => [-x, y, -z]);
  const off = Math.max(...[["trigger0", 0], ["trigger50", 0.5], ["trigger100", 1]].flatMap(([m, t]) => local(m).map((p, i) => len(sub(p, want(t)[i])))));
  check(off < 1e-4, `posed as the pistol grip's preset at each pull, in the hand's frame (worst ${mm(off)})`);
});

await run("the ghost glove", async (rt) => {
  const { scenes } = await record(rt, scenarios["hand-view"], { scenes: true });
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

if (failures.length) {
  console.error(`\n${failures.length} failure(s)`);
  process.exit(1);
}
console.log("\nvr-lab hands e2e: all checks passed");
