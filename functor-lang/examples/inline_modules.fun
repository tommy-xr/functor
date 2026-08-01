// Inline `module` declarations: named namespaces inside one file.
type World = { tick: float }

module Server {
  type Cmd =
    | Spawn(id: float)
    | Despawn(id: float)

  // The enclosing file's `World` is visible bare from inside the module.
  let step = (w: World, c: Cmd): World =>
    match c with
    | Spawn(id) => { tick: w.tick + id }
    | Despawn(_) => { tick: w.tick - 1.0 }
}

module Client {
  // `Spawn` again: constructor uniqueness is per MODULE, so this is fine.
  type Cmd = | Spawn(id: float)

  let describe = (w: World): string => $"tick {w.tick}"
}

let start: World = { tick: 0.0 }

let main = () => Client.describe(Server.step(start, Server.Spawn(4.0)))
