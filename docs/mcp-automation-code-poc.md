# Restricted SDK source over MCP — architecture PoC

## Question

Can Functor give an MCP client n8n-like “submit SDK code” ergonomics without
turning `functor mcp` into a general remote-code-execution endpoint—and does
that remove friction observed in the July 2026 five-game jam?

This PoC answers the architectural part with one intentionally narrow vertical
slice. It is not proposed as a production-ready language.

## One plan, two front ends

Both front ends use the same fluent vocabulary and serializable
`AutomationPlan`:

```text
MCP client source ──restricted Rust parser──┐
                                           ├── AutomationPlan ── debug runtime
standalone TypeScript ── AutomationBuilder ─┘
```

The existing `tools/functor-sdk` exports `automation()`, `AutomationBuilder`,
`canonicalAutomationCode()`, and `runAutomation()`. Standalone programs remain
ordinary trusted TypeScript and may use their own variables, loops, and
callbacks *around* a builder:

```ts
import {
  FunctorRunner,
  automation,
  runAutomation,
} from "@functor/sdk";

await using runner = await FunctorRunner.launch(/* … */);

const proof = automation("stress setting")
  .pause()
  .pressKey("3")
  .step({ frames: 120 })
  .expectModelClose("enemyTarget", 144, 0)
  .inspect("settled")
  .capture("stress");

console.log(proof.toPlan());
console.log(proof.toCode()); // directly submit this restricted source to MCP
const result = await runAutomation(runner, proof);
```

The MCP front end accepts only the expression returned by `toCode()`. Its custom
Rust lexer/parser never executes source. `validate_automation_code` returns the
normalized plan and deterministically regenerated `canonical_code`; a
round-trip test reparses that code and requires plan equality.

## Security boundary

The accepted grammar is one expression:

```text
automation(optional-literal-name).allowlistedMethod(literal-arguments)…
```

Supported literals are JSON-shaped plus JavaScript single-quoted strings and
identifier object keys. The parser rejects everything outside that grammar,
including:

- imports, declarations, variables, and arbitrary calls;
- functions, callbacks, arrows, loops, and branching;
- classes, `new`, async/await, and promises;
- eval/Function/require and process/global/browser objects;
- fetch, timers, dynamic member access, and computed properties;
- duplicate object keys and non-finite numbers.

Limits are 16 KiB of source, 64 logical steps, eight levels of literal nesting,
10,000 total requested frames, and four captures. Strings used as names, labels,
keys, and model paths have narrower byte limits. Mouse and UI values are also
bounded to their runtime wire domains. Ordinary runtime text and individual raw
capture responses are capped at 8 MiB; an automation result has a 4 MiB
serialized text cap and a 16 MiB aggregate raw-capture cap. Response readers
reject an oversized `Content-Length` before allocation and enforce the same cap
as chunks arrive. Automation error text is centrally truncated to 4 MiB, and
the final base64 MCP image content has its own 24 MiB aggregate cap checked
before encoding begins.

The run tool completes this entire parse and validation before looking up the
session. The MCP E2E submits a valid mutating prefix followed by an invalid
method and proves the paused game’s model/time do not change.

After validation, execution delegates to the existing HTTP-client operations in
`functor mcp`: pin clock, inject typed input, issue waited clock advances, read
structured state, and capture PNG bytes. It does not introduce a second runtime
control path.

## Jam-friction result

Issue #565 found one 5/5 workflow gap: every participant fell back from the
hosted surface to repository-only raw debug-runtime launch/input/time/state/
capture recipes. Issue #541 records the recurring mistakes: input spellings,
fixed-time versus stepped time, `pending_steps` polling, and capture workflow
selection.

This PoC materially improves three parts of that evidence:

1. **Raw choreography becomes one intent-bearing call.** `pause → input → waited
   step → structured model assertion → inspect` is one plan. An agent no longer
   needs `/time` payloads or `pending_steps` polling rules; the existing `step`
   implementation remains the authority.
2. **Common input is typed vocabulary, not wire JSON.** `pressKey("3")`,
   `mouseMove(600, 200)`, and `uiClick(0)` replace hand-authored tagged request
   bodies. `pressKey` specifically supplies the down → deterministic step → up
   lifecycle that edge-triggered jam actions needed.
3. **Evidence is part of the plan.** `expectModel`, labeled `inspect`, and
   `capture` keep behavior proof next to the actions that produced it.
   Structured model assertions avoid parsing Debug text, while captures return
   ordinary MCP image blocks.

### Real-entry trial

The finished PoC was exercised through the actual MCP stdio tools against two
jam entries rather than only the synthetic counter E2E:

- The fixed photo vignette (`ebfa750`) used `pause`, two integer `mouseMove`
  events, one `step`, and tolerant assertions for `yawOffset = -0.6` and
  `pitchOffset = 0.3`. Validation returned a seven-step plan, canonical source
  revalidated to the identical plan, and execution passed both assertions.
- Swarm Survival used `pressKey("3")`, held `d` across ten frames, released it,
  stepped once so `sampledInput` observed the release, and asserted the result.
  Validation returned a ten-step plan using twelve frames; execution raised the
  target from 96 to 144 enemies, moved the player from `0` to `0.8320000395`,
  and finished with `moveX = 0` and no held keys.

The photo proof replaces roughly five raw pause/input/step/state calls with
`validate_automation_code` plus `run_automation_code`; the fuller swarm proof
replaces roughly ten with those same two calls. A caller that does not need the
separate parse-only checkpoint can use only the run call. The trial also found
and fixed a real protocol mismatch in the first implementation: mouse
coordinates and wheel deltas must lower to signed 32-bit integers, not JSON
floating-point values.

Two fresh agent trials then used only the canonical builder vocabulary for
their gameplay proofs—no fallback raw `/input`, `/time`, `/state`, or
`pending_steps` choreography:

- Marble Golf completed a 23-step proof with 12 model assertions. Its submitted
  source canonicalized and revalidated to the identical plan before execution.
- Tower Defense completed a 20-step proof with 10 model assertions, likewise
  round-tripping through canonical source before execution.

Those trials confirmed the one-call architecture, while exposing the next
bounded vocabulary gaps: relational/changed/collection-length assertions,
`stepUntil`/polling, atomic launch-paused startup, and a headless viewport
configuration seam.

It does **not** improve engine/API blockers discovered by the entries:
camera-space picking/orientation, quaternion transforms and camera up, compound
or rotated colliders, procedural collision terrain, 3D lines, efficient keyed
grouping/range queries, richer lightweight UI, opaque host values, instancing,
or phase telemetry. It also does not choose hidden versus headless, launch a
game implicitly, expose `frame_stats`, or make capture available without a GL
context.

## Deliberate PoC limitations

- The parser is a bespoke small grammar, not a complete TypeScript AST parser.
  That is the safety property for the PoC, but its lexer/parser needs fuzzing
  and a threat-model review before accepting untrusted internet clients.
- Rust and TypeScript currently duplicate the plan schema and validators.
  Tests pin each side and canonical round trips, but production should generate
  both from one versioned schema and add cross-language fixtures.
- Execution is not transactional. Static parse/plan-budget rejection has no
  side effects; after execution starts, a runtime, assertion, or runtime-output
  cap failure can leave earlier steps applied.
- `expectModel` provides exact JSON equality and `expectModelClose` provides
  finite numeric absolute tolerance at static literal paths. There are no
  other comparisons, predicates, polling-until, branching, variables, loops,
  dataflow, or rollback.
- `pressKey` owns one fixed 16ms step. More input macros need explicit timing
  semantics rather than silently growing an ad hoc scripting language.
- Result payloads still include full observations/final state and captures are
  held in memory within explicit byte caps. Production may also want selective
  model reads to spend those budgets more deliberately.
- A per-runtime async gate makes mutating tool calls and whole automation plans
  non-interleaving. Calls that overlap on one exact normalized base URL wait
  for the active operation to finish, but waiter order is unspecified; use an
  explicit sequencer when relative ordering itself matters. Queued cancellation
  prevents a mutation; after gate acquisition the operation runs to its
  boundary. Connect reserves the same lifecycle before discovery, and owned
  stop closes pending connects and completes cleanup even when its response is
  cancelled. Normalization only strips trailing `/`, so `localhost` and
  `127.0.0.1` aliases are not unified. This is process-local coordination, not
  a distributed lock against a separate HTTP client driving the same attached
  runtime directly.
- Rust and TypeScript do not yet pin the canonical spelling of negative zero
  across languages; cross-language fixtures should cover that edge case.
- There is no per-session capability policy beyond the existing MCP session:
  a valid plan can perform any operation in this allowlist.

## Recommendation

**Go for a second, bounded iteration; no-go for production exposure yet.**

The PoC proves the core shape: the useful jam loop becomes significantly
shorter, the source round-trips to inspectable data, the same builder works
standalone, no JavaScript evaluator is needed, and the two representative jam
trials above work as static plans. The next slice should measure this against
fresh agent transcripts rather than reconstructed workflows, then decide
whether static plans are enough.

Before production, make the plan schema single-source/versioned, fuzz the
parser, add a capability policy, and choose whether conditionals should be
explicit bounded plan nodes or remain in trusted standalone TypeScript. Do not
add arbitrary callbacks or a JavaScript VM to the MCP dialect.
