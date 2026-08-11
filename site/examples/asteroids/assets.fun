// Sandbox sibling for the canonical examples/asteroids/assets.fun.
// The repo sample's ship.glb is unzipped out of the Kenney "Space Kit" pack by
// `npm run fetch:assets` and stays gitignored, so there is nothing for the site
// build to copy. This equivalent typed manifest points the model locator at a
// CORS-friendly CDN copy of the SAME file instead (jsDelivr, pinned to a commit;
// verified byte-identical to the pack's `Models/GLTF format/craft_racer.glb`),
// and keeps the checked-in sounds as local locators — the site build copies
// those verbatim. Kenney's assets are CC0, so mirroring is unrestricted.

// Models.
let ship = Asset.model("https://cdn.jsdelivr.net/gh/lampe-games/godot-open-rts@0f3a059686c9d258e215e48c0a23cf795ef4b696/assets/models/kenney-spacekit/craft_racer.glb")

// Sounds.
let explosion = Asset.sound("explosion.ogg")
let laser = Asset.sound("laser.ogg")
let ship_explosion = Asset.sound("ship-explosion.ogg")
let thrust_loop = Asset.sound("thrust-loop.ogg")
