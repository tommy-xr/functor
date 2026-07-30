// Sandbox sibling for the canonical examples/animation/game.fun.
// The repo sample's fetched Xbot.glb stays gitignored; this equivalent typed
// manifest points at the CORS-friendly BabylonJS CDN copy instead.

let xbot = Asset.model("https://cdn.jsdelivr.net/gh/BabylonJS/Assets@master/meshes/Xbot.glb")

type Clip = { name: string, duration: float }

type XbotClips = {
  idle: Clip,
  run: Clip,
  walk: Clip,
}

let xbotClips: XbotClips = {
  idle: { name: "idle", duration: 2.5 },
  run: { name: "run", duration: 0.7 },
  walk: { name: "walk", duration: 0.9667 },
}

type XbotJoints = {
  mixamorig_Head: string,
}

let xbotJoints: XbotJoints = {
  mixamorig_Head: "mixamorig:Head",
}
