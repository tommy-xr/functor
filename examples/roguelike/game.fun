// gallery: Neon Depths — a deterministic, turn-based roguelike with fog-of-war and pure enemy AI.
// gallery-controls: Arrow keys or WASD move/attack · Space waits · R restarts

type Phase =
  | Exploring
  | Victory
  | Defeat

type EnemyMode =
  | Dormant
  | Hunting
  | Stunned

type PickupKind =
  | DataShard
  | Medkit

type GameEvent =
  | EnteredFloor
  | BumpedWall
  | Moved
  | Waited
  | PickedUp(name: string)
  | HitEnemy(id: float, hp: float)
  | KilledEnemy(id: float)

type Player = {
  x: float,
  y: float,
  hp: float,
  shards: float,
}

type Enemy = {
  id: float,
  x: float,
  y: float,
  hp: float,
  mode: EnemyMode,
}

type Pickup = {
  id: float,
  x: float,
  y: float,
  kind: PickupKind,
}

type Room = {
  x: float,
  y: float,
  width: float,
  height: float,
  label: string,
}

type Floor = {
  width: float,
  height: float,
  walls: Map<string, bool>,
  rooms: List<Room>,
  seedLabel: string,
}

type Model = {
  phase: Phase,
  floor: Floor,
  player: Player,
  enemies: List<Enemy>,
  pickups: List<Pickup>,
  explored: Map<string, bool>,
  visible: Map<string, bool>,
  seed: Random.Seed,
  turn: float,
  score: float,
  kills: float,
  enemyDamage: float,
  lastEvent: GameEvent,
}

type EnemyDecision = {
  enemy: Enemy,
  damage: float,
  seed: Random.Seed,
}

type EnemyTurn = {
  enemies: List<Enemy>,
  damage: float,
  seed: Random.Seed,
}

type EnemyFold = {
  enemies: List<Enemy>,
  damage: float,
  seed: Random.Seed,
  occupied: Map<string, bool>,
}

let gridWidth = 21.0
let gridHeight = 15.0
let visionRadius = 4.0
let fixedSeed = 77.0

let cellKey = (x: float, y: float): string =>
  $"{Text.fixed(x, 0.0)},{Text.fixed(y, 0.0)}"

let inBounds = (x: float, y: float): bool =>
  x >= 0.0 && x < gridWidth && y >= 0.0 && y < gridHeight

let isWallLayout = (x: float, y: float): bool =>
  x == 0.0 || y == 0.0 || x == gridWidth - 1.0 || y == gridHeight - 1.0
    || (x == 7.0 && y >= 1.0 && y <= 10.0 && y != 4.0)
    || (x == 13.0 && y >= 4.0 && y <= 13.0 && y != 10.0)
    || (y == 7.0 && x >= 7.0 && x <= 17.0 && x != 10.0 && x != 13.0)
    || (x == 16.0 && y >= 1.0 && y <= 5.0 && y != 3.0)

let makeWalls = (): Map<string, bool> =>
  List.range(gridWidth * gridHeight)
    |> List.filter((index) =>
      let x = Math.mod(index, gridWidth) in
      let y = Math.floor(index / gridWidth) in
      isWallLayout(x, y))
    |> List.map((index) =>
      let x = Math.mod(index, gridWidth) in
      let y = Math.floor(index / gridWidth) in
      (cellKey(x, y), true))
    |> Map.fromList

let makeRooms = (): List<Room> => [
  { x: 1.0, y: 8.0, width: 6.0, height: 6.0, label: "ENTRY VAULT" },
  { x: 8.0, y: 8.0, width: 5.0, height: 6.0, label: "RELAY HALL" },
  { x: 14.0, y: 1.0, width: 6.0, height: 6.0, label: "CORE CHAMBER" },
]

let makeFloor = (): Floor => {
  width: gridWidth,
  height: gridHeight,
  walls: makeWalls(),
  rooms: makeRooms(),
  seedLabel: "NEON-77",
}

let makePlayer = (): Player =>
  { x: 1.0, y: 12.0, hp: 5.0, shards: 0.0 }

let makeEnemies = (): List<Enemy> => [
  { id: 1.0, x: 6.0, y: 12.0, hp: 2.0, mode: Dormant },
  { id: 2.0, x: 18.0, y: 3.0, hp: 3.0, mode: Dormant },
]

let makePickups = (): List<Pickup> => [
  { id: 1.0, x: 3.0, y: 12.0, kind: DataShard },
  { id: 2.0, x: 10.0, y: 4.0, kind: Medkit },
]

let wallAt = (floor: Floor, x: float, y: float): bool =>
  not inBounds(x, y) || Map.member(cellKey(x, y), floor.walls)

let manhattan = (ax: float, ay: float, bx: float, by: float): float =>
  Math.abs(ax - bx) + Math.abs(ay - by)

let makeVisible = (player: Player): Map<string, bool> =>
  List.range(gridWidth * gridHeight)
    |> List.filter((index) =>
      let x = Math.mod(index, gridWidth) in
      let y = Math.floor(index / gridWidth) in
      manhattan(x, y, player.x, player.y) <= visionRadius)
    |> List.map((index) =>
      let x = Math.mod(index, gridWidth) in
      let y = Math.floor(index / gridWidth) in
      (cellKey(x, y), true))
    |> Map.fromList

let mergeMaps = (
  original: Map<string, bool>,
  additions: Map<string, bool>
): Map<string, bool> =>
  additions
    |> Map.toList
    |> List.fold(
      (combined, entry) =>
        let (key, value) = entry in
        Map.insert(key, value, combined),
      original)

let enemyAt = (
  x: float,
  y: float,
  enemies: List<Enemy>
): Option.t<Enemy> =>
  enemies |> List.find((enemy) => enemy.x == x && enemy.y == y)

let pickupAt = (
  x: float,
  y: float,
  pickups: List<Pickup>
): Option.t<Pickup> =>
  pickups |> List.find((pickup) => pickup.x == x && pickup.y == y)

let pickupName = (kind: PickupKind): string =>
  match kind with
  | DataShard => "DATA SHARD"
  | Medkit => "MEDKIT"

let enemyOccupancy = (enemies: List<Enemy>): Map<string, bool> =>
  enemies
    |> List.map((enemy) => (cellKey(enemy.x, enemy.y), true))
    |> Map.fromList

let applyPickup = (player: Player, pickup: Pickup): Player =>
  match pickup.kind with
  | DataShard => { player with shards: player.shards + 1.0 }
  | Medkit => { player with hp: Math.min(5.0, player.hp + 2.0) }

let decideEnemy = (
  floor: Floor,
  player: Player,
  occupied: Map<string, bool>,
  seed: Random.Seed,
  enemy: Enemy
): EnemyDecision =>
  let (roll, nextSeed) = Random.range(0.0, 4.0, seed) in
  if enemy.mode == Stunned then
    {
      enemy: { enemy with mode: Hunting },
      damage: 0.0,
      seed: nextSeed
    }
  else
    let distance = manhattan(enemy.x, enemy.y, player.x, player.y) in
    if distance <= 1.0 then
      {
        enemy: { enemy with mode: Hunting },
        damage: 1.0,
        seed: nextSeed
      }
    else if distance <= 6.0 then
      let dx = player.x - enemy.x in
      let dy = player.y - enemy.y in
      let stepX =
        if Math.abs(dx) >= Math.abs(dy) then Math.sign(dx) else 0.0 in
      let stepY =
        if stepX == 0.0 then Math.sign(dy) else 0.0 in
      let targetX = enemy.x + stepX in
      let targetY = enemy.y + stepY in
      let moved =
        if wallAt(floor, targetX, targetY)
          || Map.member(cellKey(targetX, targetY), occupied)
        then { enemy with mode: Hunting }
        else { enemy with x: targetX, y: targetY, mode: Hunting } in
      {
        enemy: moved,
        damage:
          if manhattan(moved.x, moved.y, player.x, player.y) <= 1.0
          then 1.0
          else 0.0,
        seed: nextSeed
      }
    else
      {
        enemy: {
          enemy with
            mode: Dormant,
            x:
              if Math.floor(roll) == 0.0
                && not wallAt(floor, enemy.x + 1.0, enemy.y)
                && not Map.member(cellKey(enemy.x + 1.0, enemy.y), occupied)
              then enemy.x + 1.0
              else enemy.x
        },
        damage: 0.0,
        seed: nextSeed
      }

let advanceEnemies = (
  floor: Floor,
  player: Player,
  seed: Random.Seed,
  enemies: List<Enemy>
): EnemyTurn =>
  let folded =
    enemies
      |> List.fold(
        (state, enemy) =>
          let decision =
            decideEnemy(floor, player, state.occupied, state.seed, enemy) in
          {
            enemies: [decision.enemy, ..state.enemies],
            damage: state.damage + decision.damage,
            seed: decision.seed,
            occupied:
              Map.insert(
                cellKey(decision.enemy.x, decision.enemy.y),
                true,
                state.occupied)
          },
        {
          enemies: [],
          damage: 0.0,
          seed: seed,
          occupied: enemyOccupancy(enemies)
        }) in
  {
    enemies: List.reverse(folded.enemies),
    damage: folded.damage,
    seed: folded.seed
  }

let finishTurn = (model: Model): Model =>
  let enemyTurn =
    advanceEnemies(model.floor, model.player, model.seed, model.enemies) in
  let hurtPlayer = {
    model.player with
      hp: Math.max(0.0, model.player.hp - enemyTurn.damage)
  } in
  let visible = makeVisible(hurtPlayer) in
  let phase =
    if hurtPlayer.hp <= 0.0 then Defeat
    else if List.isEmpty(enemyTurn.enemies) then Victory
    else Exploring in
  {
    model with
      phase: phase,
      player: hurtPlayer,
      enemies: enemyTurn.enemies,
      explored: mergeMaps(model.explored, visible),
      visible: visible,
      seed: enemyTurn.seed,
      enemyDamage: enemyTurn.damage
  }

let takeTurn = (model: Model, dx: float, dy: float): Model =>
  if model.phase != Exploring then model
  else
    let targetX = model.player.x + dx in
    let targetY = model.player.y + dy in
    if dx == 0.0 && dy == 0.0 then
      finishTurn({
        model with
          turn: model.turn + 1.0,
          enemyDamage: 0.0,
          lastEvent: Waited
      })
    else if wallAt(model.floor, targetX, targetY) then
      {
        model with
          enemyDamage: 0.0,
          lastEvent: BumpedWall
      }
    else
      match enemyAt(targetX, targetY, model.enemies) with
      | Option.Some(target) =>
        let remainingHp = target.hp - 1.0 in
        let survivors =
          if remainingHp <= 0.0 then
            model.enemies
              |> List.filter((enemy) => enemy.id != target.id)
          else
            model.enemies
              |> List.map((enemy) =>
                if enemy.id == target.id
                then { enemy with hp: remainingHp, mode: Stunned }
                else enemy) in
        finishTurn({
          model with
            enemies: survivors,
            turn: model.turn + 1.0,
            score: model.score + (if remainingHp <= 0.0 then 100.0 else 0.0),
            kills: model.kills + (if remainingHp <= 0.0 then 1.0 else 0.0),
            enemyDamage: 0.0,
            lastEvent:
              if remainingHp <= 0.0
              then KilledEnemy(target.id)
              else HitEnemy(target.id, remainingHp)
        })
      | Option.None =>
        let movedPlayer = { model.player with x: targetX, y: targetY } in
        match pickupAt(targetX, targetY, model.pickups) with
        | Option.Some(pickup) =>
          finishTurn({
            model with
              player: applyPickup(movedPlayer, pickup),
              pickups:
                model.pickups
                  |> List.filter((item) => item.id != pickup.id),
              turn: model.turn + 1.0,
              score:
                model.score
                  + (if pickup.kind == DataShard then 25.0 else 0.0),
              enemyDamage: 0.0,
              lastEvent: PickedUp(pickupName(pickup.kind))
          })
        | Option.None =>
          finishTurn({
            model with
              player: movedPlayer,
              turn: model.turn + 1.0,
              enemyDamage: 0.0,
              lastEvent: Moved
          })

let freshGame = (): Model =>
  let floor = makeFloor() in
  let player = makePlayer() in
  let visible = makeVisible(player) in
  {
    phase: Exploring,
    floor: floor,
    player: player,
    enemies: makeEnemies(),
    pickups: makePickups(),
    explored: visible,
    visible: visible,
    seed: Random.seed(fixedSeed),
    turn: 0.0,
    score: 0.0,
    kills: 0.0,
    enemyDamage: 0.0,
    lastEvent: EnteredFloor,
  }

let init: Model = freshGame()

let pressed = (key: Key.t, snapshot: Input.snapshot): bool =>
  snapshot.pressedKeys |> List.any((candidate) => candidate == key)

let sampledInput = (model: Model, snapshot: Input.snapshot): Model =>
  if pressed(Key.R, snapshot) then freshGame()
  else if pressed(Key.Left, snapshot) || pressed(Key.A, snapshot)
  then takeTurn(model, -1.0, 0.0)
  else if pressed(Key.Right, snapshot) || pressed(Key.D, snapshot)
  then takeTurn(model, 1.0, 0.0)
  else if pressed(Key.Up, snapshot) || pressed(Key.W, snapshot)
  then takeTurn(model, 0.0, -1.0)
  else if pressed(Key.Down, snapshot) || pressed(Key.S, snapshot)
  then takeTurn(model, 0.0, 1.0)
  else if pressed(Key.Space, snapshot) then takeTurn(model, 0.0, 0.0)
  else model

let tick = (model: Model, dt: float, tts: float): Model => model

let worldWidth = 32.0
let worldHeight = 24.0
let tileSize = 0.82
let boardX = -13.6
let boardY = 6.1
let camera2d = Camera2D.create(worldWidth, worldHeight)

let ink = Color.rgb(0.012, 0.016, 0.045)
let panel = Color.rgb(0.025, 0.04, 0.095)
let floorDim = Color.rgb(0.055, 0.085, 0.14)
let floorLit = Color.rgb(0.08, 0.16, 0.22)
let wallDim = Color.rgb(0.09, 0.13, 0.22)
let wallLit = Color.rgb(0.14, 0.42, 0.55)
let cyan = Color.rgb(0.15, 0.94, 1.0)
let pink = Color.rgb(1.0, 0.18, 0.6)
let lime = Color.rgb(0.4, 1.0, 0.48)
let orange = Color.rgb(1.0, 0.48, 0.16)
let white = Color.rgb(0.9, 0.98, 1.0)
let muted = Color.rgb(0.35, 0.5, 0.65)

let cellWorldX = (x: float): float => boardX + x * tileSize
let cellWorldY = (y: float): float => boardY - y * tileSize

let drawTile = (model: Model, index: float): Sprite.t =>
  let x = Math.mod(index, gridWidth) in
  let y = Math.floor(index / gridWidth) in
  let key = cellKey(x, y) in
  let visible = Map.member(key, model.visible) in
  let explored = Map.member(key, model.explored) in
  let wall = Map.member(key, model.floor.walls) in
  let tile =
    if not explored then Sprite.square(ink, tileSize - 0.05)
    else if wall then
      Sprite.group([
        Sprite.square(if visible then wallLit else wallDim, tileSize - 0.05),
        Sprite.square(if visible then cyan else muted, tileSize - 0.28)
          |> Sprite.fade(if visible then 0.24 else 0.08),
      ])
    else
      Sprite.group([
        Sprite.square(if visible then floorLit else floorDim, tileSize - 0.05),
        Sprite.square(cyan, 0.05)
          |> Sprite.fade(if visible then 0.32 else 0.08),
      ]) in
  tile |> Sprite.move(cellWorldX(x), cellWorldY(y))

let enemyVisible = (model: Model, enemy: Enemy): bool =>
  Map.member(cellKey(enemy.x, enemy.y), model.visible)

let drawEnemy = (enemy: Enemy): Sprite.t =>
  let color =
    match enemy.mode with
    | Dormant => orange
    | Hunting => pink
    | Stunned => white in
  Sprite.group([
    Sprite.circle(color, 0.34) |> Sprite.fade(0.22),
    Sprite.square(color, 0.48) |> Sprite.rotate(Angle.degrees(45.0)),
    Sprite.square(ink, 0.12),
  ])
    |> Sprite.move(cellWorldX(enemy.x), cellWorldY(enemy.y))

let pickupVisible = (model: Model, pickup: Pickup): bool =>
  Map.member(cellKey(pickup.x, pickup.y), model.visible)

let drawPickup = (pickup: Pickup): Sprite.t =>
  let color =
    match pickup.kind with
    | DataShard => lime
    | Medkit => cyan in
  Sprite.group([
    Sprite.circle(color, 0.3) |> Sprite.fade(0.16),
    Sprite.square(color, 0.24) |> Sprite.rotate(Angle.degrees(45.0)),
    Sprite.square(white, 0.07),
  ])
    |> Sprite.move(cellWorldX(pickup.x), cellWorldY(pickup.y))

let drawPlayer = (player: Player): Sprite.t =>
  Sprite.group([
    Sprite.circle(cyan, 0.38) |> Sprite.fade(0.2),
    Sprite.square(cyan, 0.5) |> Sprite.rotate(Angle.degrees(45.0)),
    Sprite.square(white, 0.2) |> Sprite.rotate(Angle.degrees(45.0)),
  ])
    |> Sprite.move(cellWorldX(player.x), cellWorldY(player.y))

let leftText = (
  color: Color.t,
  size: float,
  x: float,
  y: float,
  text: string
): Sprite.t =>
  let width = Sprite.measure(size, text).width in
  Sprite.text(color, size, text) |> Sprite.move(x + width / 2.0, y)

let eventText = (event: GameEvent): string =>
  match event with
  | EnteredFloor => "SIGNAL LOCKED. PURGE HOSTILES."
  | BumpedWall => "WALL // NO TURN SPENT"
  | Moved => "FOOTSTEP IN THE STATIC"
  | Waited => "YOU WAIT. THEY MOVE."
  | PickedUp(name) => $"ACQUIRED: {name}"
  | HitEnemy(id, hp) =>
    $"STRUCK HOSTILE {Text.fixed(id, 0.0)} // HP {Text.fixed(hp, 0.0)}"
  | KilledEnemy(id) => $"HOSTILE {Text.fixed(id, 0.0)} PURGED"

let phaseText = (phase: Phase): string =>
  match phase with
  | Exploring => "EXPLORE"
  | Victory => "SECTOR CLEAR // R TO RESTART"
  | Defeat => "SIGNAL LOST // R TO RESTART"

let draw = (model: Model, tts: float): Frame.t =>
  let tiles = List.range(gridWidth * gridHeight) |> List.map((i) => drawTile(model, i)) in
  let enemies =
    model.enemies
      |> List.filter((enemy) => enemyVisible(model, enemy))
      |> List.map(drawEnemy) in
  let pickups =
    model.pickups
      |> List.filter((pickup) => pickupVisible(model, pickup))
      |> List.map(drawPickup) in
  let hpPips =
    List.range(model.player.hp)
      |> List.map((i) =>
        Sprite.rectangle(pink, 0.56, 0.22)
          |> Sprite.move(7.3 + i * 0.68, 3.65)) in
  let eventColor = if model.enemyDamage > 0.0 then pink else cyan in
  let overlay =
    if model.phase == Exploring then Sprite.blank()
    else
      Sprite.group([
        Sprite.rectangle(panel, 13.0, 2.2),
        Sprite.rectangle(if model.phase == Victory then lime else pink, 12.5, 0.08)
          |> Sprite.moveY(1.02),
        Sprite.text(white, 0.56, phaseText(model.phase)),
      ])
        |> Sprite.move(-4.9, -0.2) in
  Frame.create2D(
    camera2d,
    Sprite.group([
      Sprite.rectangle(ink, worldWidth, worldHeight),
      Sprite.rectangle(panel, 18.4, 14.1) |> Sprite.move(-5.4, 0.35),
      Sprite.group(tiles),
      Sprite.group(pickups),
      Sprite.group(enemies),
      drawPlayer(model.player),
      Sprite.rectangle(panel, 10.2, 14.1) |> Sprite.move(10.1, 0.35),
      leftText(cyan, 0.72, 5.65, 6.6, "NEON DEPTHS"),
      leftText(muted, 0.36, 5.7, 5.8, $"SEED // {model.floor.seedLabel}"),
      leftText(white, 0.42, 5.7, 4.7, "INTEGRITY"),
      Sprite.group(hpPips),
      leftText(white, 0.42, 5.7, 2.75, $"SHARDS   {Text.fixed(model.player.shards, 0.0)}"),
      leftText(white, 0.42, 5.7, 1.95, $"SCORE    {Text.fixed(model.score, 0.0)}"),
      leftText(white, 0.42, 5.7, 1.15, $"TURN     {Text.fixed(model.turn, 0.0)}"),
      leftText(white, 0.42, 5.7, 0.35, $"HOSTILES {Text.fixed(List.length(model.enemies), 0.0)}"),
      Sprite.rectangle(floorLit, 8.9, 1.35) |> Sprite.move(10.05, -1.3),
      leftText(eventColor, 0.32, 5.8, -1.05, eventText(model.lastEvent)),
      leftText(muted, 0.31, 5.8, -2.6, "ARROWS / WASD  MOVE + ATTACK"),
      leftText(muted, 0.31, 5.8, -3.2, "SPACE          WAIT"),
      leftText(muted, 0.31, 5.8, -3.8, "R              RESTART"),
      leftText(cyan, 0.3, 5.8, -5.3, "TURN-LOCKED // TICK-FREE"),
      leftText(muted, 0.29, 5.8, -5.85, phaseText(model.phase)),
      overlay,
    ]))

expect Map.member(cellKey(0.0, 12.0), makeWalls())
expect (
  let model = freshGame() in
  let bumped = takeTurn(model, -1.0, 0.0) in
  bumped.player.x == model.player.x && bumped.turn == 0.0
)
expect (
  let model = freshGame() in
  let moved = takeTurn(model, 1.0, 0.0) in
  moved.player.x == 2.0
    && moved.turn == 1.0
    && (moved.enemies |> List.find((enemy) => enemy.id == 1.0)
          |> Option.map((enemy) => enemy.mode)
          |> Option.defaultValue(Dormant)) == Hunting
)
expect (
  let model = freshGame() in
  let afterMove = takeTurn(model, 1.0, 0.0) in
  let afterPickup = takeTurn(afterMove, 1.0, 0.0) in
  afterPickup.player.shards == 1.0
    && afterPickup.score == 25.0
    && List.length(afterPickup.pickups) == 1.0
)
expect (
  let floor = makeFloor() in
  let player = { makePlayer() with x: 5.0, y: 5.0 } in
  let converging = [
    { id: 1.0, x: 3.0, y: 4.0, hp: 2.0, mode: Hunting },
    { id: 2.0, x: 4.0, y: 3.0, hp: 2.0, mode: Hunting },
  ] in
  let advanced =
    advanceEnemies(floor, player, Random.seed(fixedSeed), converging) in
  let first = advanced.enemies |> List.find((enemy) => enemy.id == 1.0) in
  let second = advanced.enemies |> List.find((enemy) => enemy.id == 2.0) in
  first
    |> Option.map((a) =>
      second
        |> Option.map((b) => a.x != b.x || a.y != b.y)
        |> Option.defaultValue(false))
    |> Option.defaultValue(false)
)
