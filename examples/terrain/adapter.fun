// Headset options for this game, kept out of game.fun: every sibling `.fun`
// loads with the project, and the runtime picks up a sibling's `config` and
// `settings` when the entry does not define them. Desktop reports them in
// GET /state but has no headset to apply them to.

// Read once at load; changing it takes a restart. 80 Hz is also the headset
// default, so this line only makes the choice explicit — try 90.
let config = Config.create() |> Config.xr(Xr.create() |> Xr.refreshRate(80))

// Evaluated every frame; the headset re-applies foveation when it changes.
// The periphery here is ground texture, so the coarsest level costs nothing
// visible.
let settings = (model) =>
  Settings.create() |> Settings.xr(Xr.create() |> Xr.foveation(Xr.High))
