# CLI output — the event stream

The `functor` CLI speaks **two languages from one source of truth**. Command logic
(`build`/`run`/`develop`/`push`/`init`) never formats user-facing strings itself — it
**emits typed [`Event`](../cli/src/output.rs)s**, and a single renderer, selected once at
startup, turns that stream into either human-readable text or newline-delimited JSON
(ndjson). This is design principle #2 (LLM-native): anything a human sees, an agent can get
as structured data.

```
command logic ──emit(Event)──▶ ┌─ LiveRenderer   (ink-style live region; interactive color TTY)
                               ├─ PlainRenderer  (human text; --quiet / non-TTY / CI / NO_COLOR)
                               └─ JsonRenderer   (ndjson: one {type,…} object per line)
```

If you find yourself formatting the same fact in two places, the design is wrong: add or
extend an `Event` and let the renderers present it.

This is a multi-PR pass. See **Phasing** at the bottom for what each PR adds.

## Where the stream goes

The event stream (both renderers) writes to **stdout**, one fd, one stream. An agent drives
`functor build --json` and parses stdout line by line. The process **exit code** still
signals success/failure (0 / 1); the stream is advisory.

`inspect` is the **one exception**: it is a *data* command, not a status command. Its report
is the payload and it already has its own dual mode (`--format text|json`), so it writes its
report to stdout directly and does **not** emit lifecycle events (which would pollute the
report JSON). `inspect` errors go to stderr. Everything else flows through the event stream.

## Renderer selection

Selected once at startup from flags + environment:

| Condition (first match wins)        | Renderer / mode                          | Color |
| ----------------------------------- | ---------------------------------------- | ----- |
| `--json`                            | `JsonRenderer` — ndjson                   | never |
| `--quiet`                           | `PlainRenderer`, minimal (errors + final status only) | per rule below |
| color allowed (interactive TTY, no `NO_COLOR`/`--no-color`/`CI`) | `LiveRenderer` — ink-style live region (PR-2b) | on |
| otherwise (non-TTY / `CI` / `NO_COLOR` / `--no-color`) | `PlainRenderer` — **plain text** (no ANSI) | off |

Color is enabled only when **stdout is a TTY** and `NO_COLOR` is unset and `--no-color` is
not passed and `CI` is unset. `--json` never colors. This is enforced globally via
`colored::control::set_override`, so a dumb terminal / pipe / `NO_COLOR` never leaks ANSI.

The **`LiveRenderer`** (PR-2b, `cli/src/output/live.rs`) is the ink-style human path: an
in-flight phase spinner that resolves to a stable `✓` line in scrollback, and — for
`run native` — a sticky multi-line telemetry panel (fps / tick / draw / frame / a budget bar
/ last hot-reload) pinned above scrollback, fed by `frame_stats`/`hot_reload`. It activates
**only** on a color-allowed TTY and never under `--quiet`, so every machine-facing path
(`--json`, `--quiet`, non-TTY, `CI`, `NO_COLOR`, `--no-color`) keeps the plain/json renderer
byte-for-byte — no spinner or control char reaches a piped/CI/agent stream. It reuses
`PlainRenderer::lines` for every committed line (no formatting is duplicated), animates via
indicatif's cheap `enable_steady_tick` (never touching the game loop's hot path), and wipes
the live region + restores the cursor on Ctrl-C (a `tokio::signal::ctrl_c` handler) and on
panic (a chained panic hook).

**Confirmed defaults:** non-TTY (piped/redirected) → **plain text**, *not* ndjson. `--json`
is the explicit opt-in to ndjson. TTY detection uses `std::io::IsTerminal` (std, no dep).

Flags are **global** (accepted before or after the subcommand): `--json`, `--quiet`,
`--no-color`, `--ascii`, `-v/--verbose`. `--json` takes precedence: under `--json`, `--quiet` is a
no-op (the full event stream is emitted — machine consumers filter it themselves).

**ASCII glyph fallback (PR-3).** The human renderers use unicode status glyphs (`▸ ✓ ✗ ◈ ↻`) and a
braille spinner / block budget bar. On a terminal that can't render them, they degrade to ASCII
(`-> [ok] [x] :: ~`, a `|/-\` spinner, a `#`/`-` bar) so output is never mojibake. This is decided
**once in `init`, alongside the color decision, from a single source** (`output::ascii()`, read by
both the plain and live renderers), when any of: `--ascii` is passed, `TERM=dumb`, or the locale
(`LC_ALL` / `LC_CTYPE` / `LANG`, first set wins) is set but not UTF-8. An **unset** locale is treated
as UTF-8, so the default is unchanged. `--json` is pure structured data (no glyphs) and is
unaffected. ASCII mode is orthogonal to color: a color TTY under `TERM=dumb`/`--ascii` keeps its
colored live region but swaps in the ASCII glyphs.

## Logging & verbosity (PR-4a)

Free-form logs are first-class events, not raw prints. Any `log::{debug,info,warn,error}!` call —
in the CLI **or** the in-process runtime (or a runtime dependency) — is turned into an
[`Event::Log`](#the-log-event) and travels the **same region-aware renderer** as everything else.
This is the crux: under the live renderer a log line is committed with `MultiProgress::println`, so
it lands in scrollback **above** the sticky telemetry panel and the panel redraws intact — a raw
`println!` into that region would corrupt it. Under `--json` a log is one more ndjson object; under
`PlainRenderer` it's a plain `[level] message` line.

**How it's wired.** The CLI installs one process-wide `log::Log` in `output::init` (a small adapter
in `cli/src/output.rs`) that maps each `log::Record` onto `Event::Log { level, message }` and calls
`output::emit`. The runtime crates depend only on the `log` **facade** (never on `cli`), so a plain
`log::debug!("…")` in `runtime/` renders cleanly with zero coupling. Structured, schema'd facts
(`frame_stats`, `capture_written`, `hot_reload`, `asset_error`, `runtime_ready`) still go through the
typed `functor_runtime_common::events` sink — the `log` facade is only for **free-form** diagnostics.

**Scoped to Functor's crates.** The logger filters on `record.target()` (which defaults to the
module path) to Functor's own crates — `functor*` and `mle`. Transitive deps (notify, mio, tokio,
hyper, glow, egui, gltf, …) all use `log` too, so without this a `-v` run would drown in their
debug/trace and any dep `warn!` would surface in normal output. Scoping keeps `-v` meaning "*Functor's*
debug logs."

**Level & verbosity.** `log`'s global `max_level` gates cheaply (a suppressed `log::debug!` on a hot
path is nearly free — just a level compare), decided once in `init`:

| Condition (first match wins) | Level shown |
| ---------------------------- | ----------- |
| `RUST_LOG=<level>` set (a bare `error`/`warn`/`info`/`debug`/`trace`) | that level |
| `-v` / `--verbose`           | `debug` and up (debug/info/warn/error) |
| otherwise (default)          | `warn` and up (warn/error only) — the CLI stays quiet |

So the CLI is **quiet by default** (warnings + errors); `-v` (or `RUST_LOG=debug`) opens the
debug/info firehose. `--quiet` independently suppresses any log below `warn` in the plain renderer,
so `-v --quiet` still shows only warn/error. Under `--json` the level gate still applies (a default
`--json` run carries warn/error logs; `-v --json` carries debug/info too) — always as valid ndjson.

## The `Event` schema (stable API)

Serialized with serde as `{"type": "<snake_case>", …fields}`. Optional fields are omitted
when absent (`skip_serializing_if`). Every line of `--json` output is one of these objects.

### Implemented in PR-1 (CLI-side)

| `type`             | Fields                                                        | Emitted when |
| ------------------ | ------------------------------------------------------------ | ------------ |
| `command_started`  | `command` (string), `project` (string?), `env` (string?)     | a command begins |
| `command_finished` | `ok` (bool), `duration_ms` (number)                          | a command ends |
| `mle_loaded`       | `entry` (string), `sibling_count` (number)                   | `build` typecheck passes |
| `diagnostic`       | `severity` (`"error"`\|`"warning"`), `file` (string?), `line` (number?), `col` (number?), `message` (string), `source_line` (string?) | an MLE check / load error |
| `server_listening` | `url` (string)                                               | the wasm dev server binds |
| `info`             | `message` (string)                                           | neutral status (e.g. hot-reload hint, a push ack) |
| `warning`          | `message` (string)                                           | non-fatal issue (e.g. ignored wasm runner args) |
| `error`            | `message` (string), `hint` (string?)                         | a fatal error (before exit 1) |

Example (`functor -d examples/primitives build --json`):

```json
{"type":"command_started","command":"build","project":"examples/primitives"}
{"type":"mle_loaded","entry":"game.mle","sibling_count":0}
{"type":"command_finished","ok":true,"duration_ms":6}
```

A build with a type error:

```json
{"type":"command_started","command":"build","project":"scratch/broken"}
{"type":"diagnostic","severity":"error","file":"scratch/broken/game.mle","line":4,"col":11,"message":"`+` needs Float operands, got String","source_line":"  model + \"not a number\""}
{"type":"error","message":"1 type error(s) in the scratch/broken/game.mle project"}
{"type":"command_finished","ok":false,"duration_ms":4}
```

### Rich diagnostics — the source line + caret (PR-3)

`diagnostic` carries an **optional `source_line`**: the raw offending line of source
(newline-stripped, **no caret baked in**). The CLI fills it for check errors straight from the
in-memory source, and for load/parse errors by re-reading the file (missing file / out-of-range
line → the field is simply omitted, `skip_serializing_if`). The `--json` schema stays structured
and stable: `source_line` is one more optional string field, and machine consumers still read
`line`/`col` and render their own pointer.

The **human** renderer turns those fields into a rustc-style block — the location header, then the
source line in a numbered gutter, then a caret under `col`:

```
error: game.mle:4:11: `+` needs Float operands, got String
  |
4 |   model + "not a number"
  |           ^
```

The caret indent copies the source line's own leading run (tabs kept as tabs), so it stays aligned
regardless of tab width. `col` is 1-based, counting characters from the start of the line.

### Actionable error hints (PR-3)

The `error` event's optional `hint` is populated for the common, recognizable CLI failures
(`functor.json not found` → *point `-d` at an MLE project directory*; a project missing
`"language": "mle"`; a missing `entry` file). Hints are **targeted** — most errors have no useful
generic advice and carry none. Both renderers show the hint (human: a `hint:` line under the error;
`--json`: the `hint` field).

### Implemented in PR-2a (runtime-side — routed through the event sink)

These come from the **in-process desktop runtime** (frame loop / hot-reload / asset load), which
used to `println!` directly. PR-2a routes them through the runtime event sink (below), so under
`--json` they are ndjson like everything else. Optional fields are omitted when absent.

| `type`            | Fields                                                                           | Emitted when |
| ----------------- | -------------------------------------------------------------------------------- | ------------ |
| `runtime_ready`   | —                                                                                | the runtime loaded and is about to render |
| `frame_stats`     | `tick_us`, `draw_us`, `frame_us`?, `budget_pct`?, `over_n_frames`                | every `over_n_frames` frames (300) |
| `capture_written` | `path` (string)                                                                  | a `--capture-frame` PNG was written |
| `hot_reload`      | `ok` (bool), `message` (string)                                                  | a hot-reload settled (`ok:false` = rejected edit; old program kept) |
| `asset_error`     | `path` (string?), `message` (string)                                             | an asset failed to load (fallback served) |
| `reload`          | —                                                                                | reserved for the wasm dev-server page reload (not emitted natively yet) |

`frame_stats` folds the runtime's per-frame `tick`/`physics`/`draw` cost into three numbers:
`tick_us` and `draw_us` are the tick and draw averages; `frame_us` is the **total** (tick +
physics + draw), and `budget_pct` is that total against a 60 fps (16.666 ms) budget. All are
rounded to one decimal. `over_n_frames` is the averaging window (300). This is a per-window,
**not** per-frame, emission — see the cadence note below.

Example tail of `functor -d examples/lighting run native --json` (frame stats + capture):

```json
{"type":"runtime_ready"}
{"type":"frame_stats","tick_us":16.4,"draw_us":163.8,"frame_us":180.3,"budget_pct":1.1,"over_n_frames":300}
{"type":"capture_written","path":"/tmp/frame.png"}
```

### The `log` event (PR-4a)

The one free-form event — any `log::{debug,info,warn,error}!` in the CLI or the in-process runtime,
funneled through the region-aware renderer (see [Logging & verbosity](#logging--verbosity-pr-4a)).

| `type` | Fields                                                                    | Emitted when |
| ------ | ------------------------------------------------------------------------- | ------------ |
| `log`  | `level` (`"debug"`\|`"info"`\|`"warn"`\|`"error"`), `message` (string)     | a `log!` call passes the active level |

```json
{"type":"log","level":"debug","message":"loaded asset 'grid-neon.png' (24601 bytes)"}
```

## Runtime-output routing — the event sink (PR-2a)

Post-#243 `functor run native` drives the desktop runtime **in the CLI's process**. That runtime
used to `println!` its own lines (`[mle] avg over 300 frames…`, capture confirmations, asset-load
errors, hot-reload notices) straight to stdout, **bypassing the renderer** — corrupting the ndjson
stream under `--json`. PR-2a routes them through a structured sink.

**The key constraint is dependency direction: `cli` depends on the runtime crates; the runtime
crates must never depend on `cli`.** So the event type and the sink live in the runtime, and the
CLI installs an adapter:

- `functor_runtime_common::events` defines a small `RuntimeEvent` enum (`Ready`, `FrameStats`,
  `CaptureWritten`, `HotReload`, `AssetError`) and a process-wide sink — a
  `OnceLock<Box<dyn Fn(RuntimeEvent) + Send + Sync>>` with `set_sink` / `emit`. It lives in the
  *common* crate (not `-desktop`) because asset-load errors are emitted from the shared asset
  pipeline, which both shells use.
- The desktop runtime (`mle_game.rs`, `run.rs`) calls `events::emit(RuntimeEvent::…)` at each site
  that used to print. The frame loop's hot path (per-frame `tick`/`draw`) does **not** emit.
- The CLI installs the sink once, right before it calls `functor_runtime_desktop::run(…)`:
  `events::set_sink(Box::new(|ev| output::emit(ev.into())))`. `impl From<RuntimeEvent> for
  output::Event` (in `cli/src/output.rs`) is the whole mapping — the one place a runtime fact
  becomes a CLI event.

This keeps the functional-core / imperative-shell boundary honest and makes the runtime observable
to the SDK too, while `cli → runtime` stays the only dependency edge.

**Non-blocking / cadence.** `emit` is a cheap `OnceLock` load plus a call to the installed `Fn`;
no lock is taken in the runtime. The renderer locks stdout, but nothing emits on the per-frame hot
path: `frame_stats` fires once per **300-frame window** (the pre-existing averaging cadence, ~5 s
at 60 fps — one stdout write every 300 frames, identical to the old `println!`), and the rest are
one-shot (ready, capture) or event-driven (hot-reload, asset error). So frame time and hot-reload
latency are unaffected.

**No sink installed** (wasm, tests, a bare runtime): `emit` drops routine notices and sends
`AssetError` to stderr, so a caller that never opted in is never corrupted and asset failures stay
visible where they were.

**Everything else stays off stdout.** A few flag-gated runtime notices have no natural event —
`--headless` mode, the `--debug-port` "listening on…" line, `--replay` "loaded N frames" — so they
go to **stderr**, not the event stream. That keeps stdout pure ndjson under `--json` even for the
`--debug-port` / `--headless` combos an SDK/automation consumer uses. Free-form runtime status that
*doesn't* map to a structured event (e.g. Xreal connect/calibration status, or an asset-load debug
line) now goes through the **`log` facade** (PR-4a) instead of a raw `println!`, so it too renders
region-aware and never corrupts `--json`. (The two genuinely-interactive TTY notices — the
cursor-release hint and the F1-recenter ack — stay plain stdout `println!`; they only fire on a
keypress in a focused window, never on a piped/`--json`/captured run.)

## Phasing

- **PR-1:** the `Event` enum + `Renderer` trait + selection; `JsonRenderer` (ndjson) and a plain
  `PlainRenderer` (no animation); retire the raw debug prints; fix the `--help` wording; global
  `--json`/`--quiet`/`--no-color`.
- **PR-2a (this):** route the in-process runtime output (frame stats, capture, hot-reload, asset
  errors, ready) through the event sink above, so `run/develop native --json` is clean ndjson.
  `PlainRenderer` prints the new events as plain lines; `JsonRenderer` serializes them. No TUI.
- **PR-2b (this):** the ink-style `LiveRenderer` — a phase spinner that resolves to a stable
  `✓` line, and a sticky `run native` telemetry panel that consumes `frame_stats`/`hot_reload`,
  above scrollback. TTY-only; the machine paths are untouched. (A full-screen `--dashboard` is
  explicitly out of scope.)
- **PR-3 (closes the UX pass):** rich MLE diagnostics (the `source_line` field + a human caret
  block), actionable error `hint`s for common failures, and an ASCII glyph fallback for dumb /
  non-UTF-8 terminals (`--ascii`).
- **PR-4a (this):** region-aware logging — the `log` event + a `log::Log` facade the CLI installs,
  so any `log::{debug,info,warn,error}!` (CLI or in-process runtime) renders through the same
  region-aware path; `-v/--verbose` + `RUST_LOG` set the level (quiet warn/error default). Converts
  the runtime's informational `println!`s (asset-load debug, Xreal status). PR-4b adds the MLE
  `Debug.log` builtin through the same path.
</content>
</invoke>
