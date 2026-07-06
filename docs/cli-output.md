# CLI output — the event stream

The `functor` CLI speaks **two languages from one source of truth**. Command logic
(`build`/`run`/`develop`/`push`/`init`) never formats user-facing strings itself — it
**emits typed [`Event`](../cli/src/output.rs)s**, and a single renderer, selected once at
startup, turns that stream into either human-readable text or newline-delimited JSON
(ndjson). This is design principle #2 (LLM-native): anything a human sees, an agent can get
as structured data.

```
command logic ──emit(Event)──▶ ┌─ PlainRenderer  (human text; color on a TTY)
                               └─ JsonRenderer   (ndjson: one {type,…} object per line)
```

If you find yourself formatting the same fact in two places, the design is wrong: add or
extend an `Event` and let the renderers present it.

This is **PR-1 of a 3-PR pass**. See "What PR-1 does *not* do" at the bottom.

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
| stdout **not a TTY**, or `CI` set   | `PlainRenderer` — **plain text** (no ANSI) | off   |
| interactive TTY (default)           | `PlainRenderer` — human text              | on    |

Color is enabled only when **stdout is a TTY** and `NO_COLOR` is unset and `--no-color` is
not passed and `CI` is unset. `--json` never colors. This is enforced globally via
`colored::control::set_override`, so a dumb terminal / pipe / `NO_COLOR` never leaks ANSI.

**Confirmed defaults:** non-TTY (piped/redirected) → **plain text**, *not* ndjson. `--json`
is the explicit opt-in to ndjson. TTY detection uses `std::io::IsTerminal` (std, no dep).

Flags are **global** (accepted before the subcommand): `--json`, `--quiet`, `--no-color`.

## The `Event` schema (stable API)

Serialized with serde as `{"type": "<snake_case>", …fields}`. Optional fields are omitted
when absent (`skip_serializing_if`). Every line of `--json` output is one of these objects.

### Implemented in PR-1 (CLI-side)

| `type`             | Fields                                                        | Emitted when |
| ------------------ | ------------------------------------------------------------ | ------------ |
| `command_started`  | `command` (string), `project` (string?), `env` (string?)     | a command begins |
| `command_finished` | `ok` (bool), `duration_ms` (number)                          | a command ends |
| `mle_loaded`       | `entry` (string), `sibling_count` (number)                   | `build` typecheck passes |
| `diagnostic`       | `severity` (`"error"`\|`"warning"`), `file` (string?), `line` (number?), `col` (number?), `message` (string) | an MLE check / load error |
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
{"type":"diagnostic","severity":"error","file":"scratch/broken/game.mle","line":3,"col":5,"message":"..."}
{"type":"error","message":"1 type error(s) in the scratch/broken/game.mle project"}
{"type":"command_finished","ok":false,"duration_ms":4}
```

### Reserved for PR-2 (runtime-side — not emitted yet)

These come from the **in-process desktop runtime** (frame loop / hot-reload / asset load),
which today still `println!`s directly. Routing them requires a runtime event-sink refactor
(below) and lands in PR-2. Documented here so the schema is planned, not surprising:

| `type`           | Fields                                                                 |
| ---------------- | --------------------------------------------------------------------- |
| `runtime_ready`  | —                                                                     |
| `frame_stats`    | `tick_us`, `draw_us`, `frame_us`, `budget_pct`, `over_n_frames`       |
| `capture_written`| `path` (string)                                                       |
| `hot_reload`     | `ok` (bool), `message` (string)                                       |
| `asset_error`    | `path` (string), `message` (string)                                   |
| `reload`         | — (wasm page reload)                                                  |

## Runtime-output routing — deferred to PR-2 (and why)

Post-#243 `functor run native` drives the desktop runtime **in the CLI's process**, and that
runtime currently `println!`s its own lines (`[mle] avg over 300 frames…`, capture
confirmations, asset-load errors, hot-reload notices) straight to stdout, **bypassing the
renderer**. Under `--json` these raw lines would corrupt the ndjson stream.

Fixing this is **explicitly out of scope for PR-1** because the clean fix is a runtime change,
not a CLI change: give the runtime a structured event sink (a callback/channel the host passes
in) so it *emits* `FrameStats`/`CaptureWritten`/`HotReload`/`AssetError` instead of printing,
and the CLI renders them through the same stream. That keeps the functional-core / imperative-
shell boundary honest and makes the runtime observable to the SDK too — but it touches
`runtime/` crates and both producers, so it is its own reviewable PR.

Until then: `run`/`develop native` still print runtime lines raw on stdout. `build`, `push`,
`run wasm` (dev server), and all CLI-side status are fully routed.

## Phasing

- **PR-1 (this):** the `Event` enum + `Renderer` trait + selection; `JsonRenderer` (ndjson)
  and a plain `PlainRenderer` (no animation); retire the raw debug prints; fix the `--help`
  wording; global `--json`/`--quiet`/`--no-color`.
- **PR-2:** the ink-style human renderer (spinners, a live region above scrollback, grouped
  styled sections) **and** the runtime event-sink routing above.
- **PR-3:** rich MLE diagnostics (source line + caret), actionable error hints, polish.
</content>
</invoke>
