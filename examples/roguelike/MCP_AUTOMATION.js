async (game) => {
  const assert = (condition, message) => {
    if (!condition) throw new Error(`Roguelike MCP assertion failed: ${message}`);
  };
  const ctor = (value) =>
    value && typeof value === "object" && "$ctor" in value
      ? value.$ctor
      : value;

  await game.pause();
  const initial = await game.state();
  const floor = initial.model.floor;
  const player = initial.model.player;
  const enemies = initial.model.enemies;

  assert(floor.width === 21 && floor.height === 15, "floor dimensions should be discoverable");
  assert(Array.isArray(floor.rooms) && floor.rooms.length === 3, "floor rooms should be structured");
  assert(player.x === 1 && player.y === 12 && player.hp === 5, "player state should be structured");
  assert(Array.isArray(enemies) && enemies.length === 2, "enemy state should be structured");
  assert(ctor(enemies[0].mode) === "Dormant", "first enemy should begin dormant");

  await game.pressKey("left");
  const bumped = await game.state();
  assert(bumped.model.player.x === player.x, "walking into the west wall must not move");
  assert(bumped.model.turn === 0, "wall collision must not spend a turn");
  assert(ctor(bumped.model.lastEvent) === "BumpedWall", "wall event should be explicit");

  await game.pressKey("right");
  const moved = await game.state();
  const awakened = moved.model.enemies.find((enemy) => enemy.id === 1);
  assert(moved.model.player.x === 2, "open floor should permit movement");
  assert(moved.model.turn === 1, "successful movement should spend one turn");
  assert(ctor(awakened.mode) === "Hunting", "nearby enemy should enter Hunting");
  assert(awakened.x === 5, "hunting enemy should advance deterministically");

  await game.pressKey("right");
  const picked = await game.stepUntil(
    (state) => state.model.player.shards === 1 && state.model.score === 25,
    {
      maxFrames: 2,
      dts: 1 / 60,
      description: "the shard pickup transition",
    },
  );
  assert(picked.model.player.x === 3, "pickup tile should be entered");
  assert(picked.model.pickups.length === 1, "picked shard should leave the floor");
  assert(ctor(picked.model.lastEvent) === "PickedUp", "pickup event should be structured");
  assert(picked.model.enemyDamage === 1, "adjacent hunter should counterattack");

  const encounterPng = await game.capture();
  assert(
    Buffer.isBuffer(encounterPng) && encounterPng.length > 1000,
    "encounter capture should return a PNG",
  );

  let combat = picked;
  const startKills = combat.model.kills;
  for (let attack = 0; attack < 3 && combat.model.kills === startKills; attack += 1) {
    await game.pressKey("right");
    combat = await game.state();
  }
  assert(combat.model.kills === startKills + 1, "bounded attacks should defeat one enemy");
  assert(combat.model.score === 125, "pickup and kill score should total 125");
  assert(combat.model.enemies.length === 1, "defeated enemy should leave structured state");
  assert(ctor(combat.model.lastEvent) === "KilledEnemy", "kill transition should be explicit");

  const held = await game.heldKeys();
  assert(Array.isArray(held) && held.length === 0, "pressKey must not leak held input");

  const trace = await game.trace();
  const finalPng = await game.capture();
  assert(
    Buffer.isBuffer(finalPng) && finalPng.length > 1000,
    "final hidden capture should return a PNG",
  );

  const wallShape =
    Array.isArray(floor.walls)
      ? "array"
      : floor.walls && typeof floor.walls === "object"
        ? Object.keys(floor.walls).sort()
        : typeof floor.walls;

  console.log(
    "roguelike-proof",
    JSON.stringify({
      turn: combat.model.turn,
      score: combat.model.score,
      kills: combat.model.kills,
    }),
  );

  return {
    discovered: {
      phase: ctor(initial.model.phase),
      floor: {
        width: floor.width,
        height: floor.height,
        rooms: floor.rooms.length,
        wallShape,
      },
      player: { x: player.x, y: player.y, hp: player.hp },
      enemies: enemies.map((enemy) => ({
        id: enemy.id,
        x: enemy.x,
        y: enemy.y,
        hp: enemy.hp,
        mode: ctor(enemy.mode),
      })),
    },
    wallCollision: {
      x: bumped.model.player.x,
      turn: bumped.model.turn,
      event: ctor(bumped.model.lastEvent),
    },
    successfulMove: {
      x: moved.model.player.x,
      turn: moved.model.turn,
      enemyMode: ctor(awakened.mode),
      enemyX: awakened.x,
    },
    pickup: {
      shards: picked.model.player.shards,
      score: picked.model.score,
      enemyDamage: picked.model.enemyDamage,
    },
    combat: {
      turn: combat.model.turn,
      score: combat.model.score,
      kills: combat.model.kills,
      enemiesRemaining: combat.model.enemies.length,
      event: ctor(combat.model.lastEvent),
    },
    heldKeys: held,
    traceObserved: trace !== null && typeof trace === "object",
    captureBytes: {
      encounter: encounterPng.length,
      final: finalPng.length,
    },
  };
}
