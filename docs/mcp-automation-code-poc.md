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
  .expectModel("enemyTarget", 144)
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
bounded to their runtime wire domains.

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
- Execution is not transactional. Parse/budget rejection has no side effects;
  a runtime failure after execution starts can leave earlier steps applied.
- Assertions are exact JSON equality at static literal paths. There are no
  comparisons, predicates, polling-until, branching, variables, loops,
  dataflow, or rollback.
- `pressKey` owns one fixed 16ms step. More input macros need explicit timing
  semantics rather than silently growing an ad hoc scripting language.
- Result payloads include full observations/final state and captures are held
  in memory. Production needs output-size accounting and possibly selective
  model reads.
- There is no per-session capability policy beyond the existing MCP session:
  a valid plan can perform any operation in this allowlist.

## Recommendation

**Go for a second, bounded iteration; no-go for production exposure yet.**

The PoC proves the core shape: the useful jam loop becomes significantly
shorter, the source round-trips to inspectable data, the same builder works
standalone, and no JavaScript evaluator is needed. The next slice should use
real agent transcripts on two representative entries (photo-mode mouse look
and a key-driven stress/control case), measure tool-call/error reduction, and
decide whether static plans are enough.

Before production, make the plan schema single-source/versioned, fuzz the
parser, add output/capture byte budgets and a capability policy, and choose
whether conditionals should be explicit bounded plan nodes or remain in trusted
standalone TypeScript. Do not add arbitrary callbacks or a JavaScript VM to the
MCP dialect.
