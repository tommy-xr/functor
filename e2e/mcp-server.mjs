// `functor mcp` E2E: prove the MCP server really drives a game.
//
// This is the agent-verifiability proof for the MCP surface (docs/mcp.md): it
// speaks raw JSON-RPC 2.0 over the server's stdio — no MCP client library — so
// what is asserted is exactly what a coding agent's client would see.
//
//   1. initialize / notifications/initialized / tools/list, asserting the whole
//      documented tool surface is advertised.
//   2. api_reference, called before any session exists (it is session-free):
//      a known item's signature, a module listing, and the zero-match case —
//      and language_guide, its language-side twin: the table of contents, a
//      section by name, and an unknown section.
//   3. launch_game on examples/counter in HEADLESS mode (no GL, so this runs
//      anywhere CI does), and assert /state's structured `model` arrives.
//   4. run_game_code_unsafe executes ordinary JavaScript against its injected
//      SDK, including callbacks/loops/stepUntil, and returns an SDK-call trace.
//   5. pause, then step twice — asserting the frame advances and that `step`
//      only returns once the queued steps have landed (pending_steps == 0).
//   6. send_input: counter's model reacts to a UI click (its `update` handles
//      Inc), so a `ui_event` on slot 0 must increment `count` after a step.
//      A `key` event is also injected to exercise that shape end to end.
//   7. the filesystem-less authoring journey: launch_game with the whole
//      project INLINE (no directory anywhere), edit it live with
//      reload_source (model preserved), then save_project — asserting the
//      saved source is the EDITED source, i.e. the wire's truth rather than
//      whatever the launch wrote.
//   8. init_game scaffolds a starter project on disk and it boots.
//   9. stop_game, then a clean shutdown of the server itself.
//
// Run manually (needs the CLI built; the wasm bundle is not required):
//
//   cargo build -p functor-cli --no-default-features
//   node e2e/mcp-server.mjs
//
// Set FUNCTOR_BIN when the build uses a shared CARGO_TARGET_DIR.
import { spawn } from "node:child_process";
import { existsSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

// The raw JSON-RPC client is shared with e2e/mcp-step-all.mjs.
import { BIN, check, failures, Rpc, ROOT } from "./mcp-rpc.mjs";

/** Every tool the server must advertise (docs/mcp.md). */
const EXPECTED_TOOLS = [
  "launch_game",
  "connect_game",
  "list_sessions",
  "stop_game",
  "get_state",
  "get_scene",
  "get_trace",
  "capture_frame",
  "send_input",
  "pause",
  "step",
  "step_all",
  "resume",
  "rewind",
  "reload_source",
  "reload_project",
  "api_reference",
  "language_guide",
  "run_game_code_unsafe",
  "init_game",
  "save_project",
];

const proc = spawn(BIN, ["mcp"], { cwd: ROOT, stdio: ["pipe", "pipe", "pipe"] });
const rpc = new Rpc(proc);
let session;

try {
  console.log("▸ the MCP handshake");
  const initialized = await rpc.request("initialize", {
    protocolVersion: "2025-06-18",
    capabilities: {},
    clientInfo: { name: "functor-e2e", version: "0" },
  });
  rpc.notify("notifications/initialized", {});
  check(typeof initialized.protocolVersion === "string", `the server negotiated ${initialized.protocolVersion}`);
  check(initialized.capabilities?.tools !== undefined, "the server advertises the tools capability");

  const { tools } = await rpc.request("tools/list", {});
  const names = tools.map((tool) => tool.name).sort();
  const missing = EXPECTED_TOOLS.filter((name) => !names.includes(name));
  check(missing.length === 0, `tools/list advertises all ${EXPECTED_TOOLS.length} tools${missing.length ? ` (missing ${missing})` : ""}`);
  check(
    tools.every((tool) => (tool.description ?? "").length > 20),
    "every tool carries a real description (the agent-facing docs)",
  );

  // Deliberately before any session exists: api_reference is session-free.
  console.log("\n▸ the API reference, with no session");
  const cube = await rpc.call("api_reference", { query: "Scene.cube" });
  check(/Scene\.cube/.test(cube) && /let cube : \(\) => t/.test(cube), "api_reference returns Scene.cube's signature");

  const effect = await rpc.call("api_reference", { module: "Effect" });
  const effectItems = effect.split("\n").filter((line) => /^Effect\./.test(line));
  check(effectItems.length > 1, `browsing a module lists its items (${effectItems.length} from Effect)`);

  // Scene has more items than a search's result cap: a module listing is whole.
  const scene = await rpc.call("api_reference", { module: "Scene" });
  const sceneItems = scene.split("\n").filter((line) => /^Scene\./.test(line));
  check(sceneItems.length > 20 && !/matches shown/.test(scene), `a module listing is not truncated (${sceneItems.length} from Scene)`);

  let missed = null;
  try {
    await rpc.call("api_reference", { query: "zzzznotathing" });
  } catch (error) {
    missed = error.message;
  }
  check(missed !== null && /Scene/.test(missed) && /Effect/.test(missed), "a query matching nothing names the available modules");

  // The language surface is session-free too: an agent reads it BEFORE it has
  // written, let alone launched, anything.
  console.log("\n▸ the language guide, with no session");
  const contents = await rpc.call("language_guide");
  const sectionsBlock = contents.split("## Sections\n")[1] ?? "";
  const sectionNames = sectionsBlock.split("\n").map((line) => line.trim()).filter(Boolean);
  check(sectionNames.length > 3, `the table of contents lists ${sectionNames.length} sections`);
  check(/Assignment is `:=`/.test(contents), "the contents lead with the quick facts (`:=`, not `<-`)");
  check(/thread-LAST/.test(contents), "the thread-last pipeline rule is in the quick facts");

  const syntax = await rpc.call("language_guide", { section: "syntax-subset" });
  check(/^## Syntax subset/.test(syntax) && syntax.length > 2000, `a named section returns its full text (${syntax.length} chars)`);
  check(/let draw = \(model, tts\)/.test(await rpc.call("language_guide", { section: "game contract" })), "a section is addressable by a fragment of its name");

  // A section stops at its first subsection, so a parent must say where it
  // continues rather than reading like the end of the topic.
  const modules = await rpc.call("language_guide", { section: "modules-multi-file-projects" });
  check(/Continues in: interface-files/.test(modules), "a parent section points at its subsections");

  let badSection = null;
  try {
    await rpc.call("language_guide", { section: "monad-transformers" });
  } catch (error) {
    badSection = error.message;
  }
  check(badSection !== null && /syntax-subset/.test(badSection), "an unknown section names the sections that exist");

  console.log("\n▸ launching a game headlessly");
  const launched = await rpc.call("launch_game", { dir: "examples/counter", mode: "headless" });
  session = launched.session;
  check(/^s\d+$/.test(session), `launch_game returned a session id (${session})`);
  check(
    typeof launched.protocol_version === "number",
    `the child answered discovery, reporting protocol v${launched.protocol_version}`,
  );
  check(launched.discovery === undefined, "the ~2 KB endpoint index is not repeated on every launch (pass discovery: true for it)");
  check(launched.owned === true, "the session is owned (this server spawned it)");

  const listed = await rpc.call("list_sessions");
  check(listed.sessions.length === 1 && listed.sessions[0].alive === true, "list_sessions reports it alive");

  console.log("\n▸ the structured model view (protocol v4)");
  const state = await rpc.call("get_state", { session });
  check(
    state.model !== null && typeof state.model === "object",
    "get_state carries model as a structured object",
  );
  check(state.model?.count === 0, `the counter starts at 0 (model.count = ${state.model?.count})`);

  console.log("\n▸ submitted code is parsed before its first action");
  await rpc.call("pause", { session });
  const beforeRejectedRun = await rpc.call("get_state", { session });
  let rejectedRun = null;
  try {
    await rpc.call("run_game_code_unsafe", {
      session,
      code: `async (game) => {
        await game.uiClick(0);
        const syntax error = true;
      }`,
    });
  } catch (error) {
    rejectedRun = error.message;
  }
  const afterRejectedRun = await rpc.call("get_state", { session });
  check(rejectedRun !== null && /SyntaxError/.test(rejectedRun), "a syntax error is reported");
  check(
    afterRejectedRun.model.count === beforeRejectedRun.model.count &&
      afterRejectedRun.tts === beforeRejectedRun.tts,
    "a syntactically invalid function had no model or clock side effect",
  );
  const pathologicalThrowStarted = Date.now();
  let pathologicalThrow = null;
  try {
    await rpc.call("run_game_code_unsafe", {
      session,
      code: `async () => { throw Object.create(null); }`,
      timeout_ms: 5000,
    });
  } catch (error) {
    pathologicalThrow = error.message;
  }
  check(
    pathologicalThrow !== null &&
      /unprintable thrown value/.test(pathologicalThrow) &&
      Date.now() - pathologicalThrowStarted < 4000,
    "an uncoercible thrown value is reported without waiting for the run timeout",
  );

  console.log("\n▸ one unsafe SDK call replaces raw choreography");
  const automated = await rpc.call("run_game_code_unsafe", {
    session,
    code: `async (game) => {
      await game.pause();
      await game.uiClick(0);
      const settled = await game.stepUntil(
        (state) => state.model.count === 1,
        { maxFrames: 3, dts: 0.016, description: "counter increment" },
      );
      await game.mouseMove(400, 300);
      await game.mouseWheel(-1);
      await game.keyDown("w");
      await game.step();
      const whileHeld = await game.state();
      await game.keyUp("w");
      await game.keyDown("1");
      const digitHeld = await game.isKeyDown("1");
      await game.keyUp("1");
      console.log("counter proof", settled.model.count);
      process.stdout.write("stdout proof");
      return {
        count: settled.model.count,
        heldDuringStep: whileHeld.input.held_keys.includes("W"),
        digitHeld,
      };
    }`,
  });
  check(automated.ok === true && automated.unsafe?.rce_equivalent === true, "the Node child completed and labels the RCE-equivalent trust model");
  check(
    automated.return_value?.count === 1 &&
      automated.return_value?.heldDuringStep === true &&
      automated.return_value?.digitHeld === true,
    "ordinary callbacks, digit-key observation, local values, and structured returns work",
  );
  check(
    automated.trace.some((entry) => entry.method === "uiClick") &&
      automated.trace.some((entry) => entry.method === "state") &&
      automated.trace.every((entry) => typeof entry.elapsed_ms === "number"),
    "the result carries a structured trace of the SDK calls that actually ran",
  );
  check(
    automated.logs?.some((entry) => entry.text === "counter proof 1") &&
      automated.logs?.some(
        (entry) => entry.level === "stdout" && entry.text === "stdout proof",
      ),
    "console and ordinary stdout output are captured without corrupting the child RPC protocol",
  );
  check(
    automated.final_state?.input?.mouse?.x === 400 &&
      automated.final_state?.input?.mouse?.y === 300,
    "the injected SDK used the integer mouse protocol successfully",
  );

  console.log("\n▸ thrown code restores only input that code touched");
  await rpc.call("send_input", {
    session,
    command: { type: "key", key: "w", down: true },
  });
  await rpc.call("send_input", {
    session,
    command: { type: "key", key: "1", down: true },
  });
  let assertionOnlyFailure = null;
  try {
    await rpc.call("run_game_code_unsafe", {
      session,
      code: `async () => { throw new Error("unrelated failure"); }`,
    });
  } catch (error) {
    assertionOnlyFailure = error.message;
  }
  const afterAssertionOnly = await rpc.call("get_state", { session });
  check(
    /unrelated failure/.test(assertionOnlyFailure ?? "") &&
      !/code-touched key/.test(assertionOnlyFailure ?? "") &&
      afterAssertionOnly.input?.held_keys?.includes("W"),
    "code that injected no input did not release pre-existing held input",
  );

  let baselineRestoreFailure = null;
  try {
    await rpc.call("run_game_code_unsafe", {
      session,
      code: `async (game) => {
        await game.keyUp("w");
        await game.keyUp("1");
        throw new Error("restore baseline");
      }`,
    });
  } catch (error) {
    baselineRestoreFailure = error.message;
  }
  const afterBaselineRestore = await rpc.call("get_state", { session });
  check(
    /code-touched key and mouse-button levels were restored/.test(baselineRestoreFailure ?? "") &&
      afterBaselineRestore.input?.held_keys?.includes("W") &&
      afterBaselineRestore.input?.held_keys?.includes("Num1"),
    "cleanup restored pre-existing held letter and digit keys after failed code released them",
  );

  let ownedInputFailure = null;
  try {
    await rpc.call("run_game_code_unsafe", {
      session,
      code: `async (game) => {
        await game.keyDown("space");
        throw new Error("owned cleanup");
      }`,
    });
  } catch (error) {
    ownedInputFailure = error.message;
  }
  const afterOwnedInputFailure = await rpc.call("get_state", { session });
  check(
    /code-touched key and mouse-button levels were restored/.test(ownedInputFailure ?? "") &&
      afterOwnedInputFailure.input?.held_keys?.includes("W") &&
      !afterOwnedInputFailure.input?.held_keys?.includes("Space"),
    "cleanup released the failed code's Space edge without releasing pre-existing W",
  );

  const caughtInvalidKey = await rpc.call("run_game_code_unsafe", {
    session,
    code: `async (game) => {
      try {
        await game.keyDown("Shift");
      } catch (error) {
        return { caught: String(error).includes("unknown key") };
      }
      return { caught: false };
    }`,
  });
  check(
    caughtInvalidKey.return_value?.caught === true,
    "ordinary code can catch a rejected SDK call without quarantining the session",
  );
  await rpc.call("send_input", {
    session,
    command: { type: "key", key: "w", down: false },
  });
  await rpc.call("send_input", {
    session,
    command: { type: "key", key: "1", down: false },
  });

  console.log("\n▸ timeout and MCP cancellation terminate code and restore input");
  let timeoutFailure = null;
  try {
    await rpc.call("run_game_code_unsafe", {
      session,
      timeout_ms: 250,
      code: `async (game) => {
        await game.keyDown("space");
        await new Promise(() => {});
      }`,
    });
  } catch (error) {
    timeoutFailure = error.message;
  }
  const afterTimeout = await rpc.call("get_state", { session });
  check(
    /exceeded its 250ms wall-clock timeout/.test(timeoutFailure ?? "") &&
      /code-touched key and mouse-button levels were restored/.test(timeoutFailure ?? ""),
    "a hung function is killed at its requested wall-clock timeout and reports cleanup",
  );
  check(
    !afterTimeout.input?.held_keys?.includes("Space"),
    "timeout cleanup released the key held by submitted code",
  );

  const cancellableRun = rpc.beginRequest("tools/call", {
    name: "run_game_code_unsafe",
    arguments: {
      session,
      code: `async (game) => {
        await game.keyDown("space");
        await new Promise(() => {});
      }`,
    },
  });
  let cancellationHeld = false;
  for (let attempt = 0; attempt < 200 && !cancellationHeld; attempt += 1) {
    const during = await rpc.call("get_state", { session });
    cancellationHeld = during.input?.held_keys?.includes("Space") === true;
    if (!cancellationHeld) await new Promise((resolve) => setTimeout(resolve, 5));
  }
  check(cancellationHeld, "the cancellable function began and held Space");
  rpc.cancelRequest(cancellableRun.id, "abort submitted SDK code");
  await cancellableRun.result;

  let cancellationReleased = false;
  for (let attempt = 0; attempt < 200 && !cancellationReleased; attempt += 1) {
    const afterCancel = await rpc.call("get_state", { session });
    cancellationReleased =
      afterCancel.pending_steps === 0 &&
      !afterCancel.input?.held_keys?.includes("Space");
    if (!cancellationReleased) await new Promise((resolve) => setTimeout(resolve, 5));
  }
  check(
    cancellationReleased,
    "an MCP cancellation killed the child and released code-touched input",
  );

  const beforeOversizedStep = await rpc.call("get_state", { session });
  let oversizedStep = null;
  try {
    await rpc.call("step", { session, frames: 10001 });
  } catch (error) {
    oversizedStep = error.message;
  }
  const afterOversizedStep = await rpc.call("get_state", { session });
  check(
    oversizedStep !== null &&
      /between 1 and 10000/.test(oversizedStep) &&
      afterOversizedStep.frame === beforeOversizedStep.frame,
    "the lower-level step cap rejects 10,001 frames before changing the session",
  );

  console.log("\n▸ a landed code step does not own later direct-client clock work");
  const postStepFailure = rpc.call("run_game_code_unsafe", {
    session,
    code: `async (game) => {
      await game.step();
      await game.keyDown("q");
      await game.waitForState(
        (state) => state.input.held_keys.includes("Num9"),
        { timeoutMs: 5000, intervalMs: 5, description: "the direct-client trigger" },
      );
      throw new Error("after-landed-step");
    }`,
  }).then(
    () => null,
    (error) => error.message,
  );
  let landedStepSignalled = false;
  for (let attempt = 0; attempt < 400 && !landedStepSignalled; attempt += 1) {
    const during = await rpc.call("get_state", { session });
    landedStepSignalled = during.input?.held_keys?.includes("Q") === true;
    if (!landedStepSignalled) await new Promise((resolve) => setTimeout(resolve, 5));
  }
  check(landedStepSignalled, "submitted code signalled after its own step had landed");
  const directAdvance = await fetch(`${launched.url}/time`, {
    method: "POST",
    body: JSON.stringify({ type: "advance", dts: 0.016, frames: 10000 }),
  });
  check(directAdvance.ok, "a direct debug client queued later clock work");
  const directTrigger = await fetch(`${launched.url}/input`, {
    method: "POST",
    body: JSON.stringify({ type: "key", key: "9", down: true }),
  });
  check(directTrigger.ok, "the direct client released submitted code's wait");
  const postStepError = await postStepFailure;
  const afterPostStepFailure = await rpc.call("get_state", { session });
  check(
    /after-landed-step/.test(postStepError ?? "") &&
      afterPostStepFailure.pending_steps > 0 &&
      !afterPostStepFailure.input?.held_keys?.includes("Q") &&
      afterPostStepFailure.input?.held_keys?.includes("Num9"),
    "later code failure restored its input without cancelling the direct client's queue",
  );
  const directCancel = await fetch(`${launched.url}/time`, {
    method: "POST",
    body: JSON.stringify({ type: "cancel" }),
  });
  check(directCancel.ok, "the direct-client test queue was explicitly cleaned up");
  const directTriggerRelease = await fetch(`${launched.url}/input`, {
    method: "POST",
    body: JSON.stringify({ type: "key", key: "9", down: false }),
  });
  check(directTriggerRelease.ok, "the direct-client trigger input was explicitly released");

  console.log("\n▸ mutating calls serialize per session, while other sessions stay independent");
  const EDGE_COUNTER = `type Model = { count: float, held: bool }

let init: Model = { count: 0.0, held: false }

let input = (m: Model, key: Key.t, isDown: bool): Model =>
  if key == Key.Space then
    if isDown then
      if m.held then m else { m with count: m.count + 1.0, held: true }
    else { m with held: false }
  else m

let tick = (m: Model, dt, tts) => m

let draw = (m: Model, tts) =>
  Frame.create(
    Camera3D.lookAt(Vec3.make(0.0, 1.0, -4.0), Vec3.make(0.0, 0.0, 0.0)),
    Scene.cube(),
  )
`;
  const gateGame = await rpc.call("launch_game", {
    files: [["game.fun", EDGE_COUNTER]],
    mode: "headless",
  });
  const gatedSession = gateGame.session;
  const gateAlias = (await rpc.call("connect_game", { url: gateGame.url })).session;
  let longFinished = false;
  const longRun = rpc.call("run_game_code_unsafe", {
    session: gatedSession,
    code: `async (game) => {
      await game.pause();
      await game.keyDown("space");
      await game.stepFrames(2000, 0.016);
      await game.keyUp("space");
      return { released: !(await game.isKeyDown("space")) };
    }`,
  });
  longRun.then(
    () => { longFinished = true; },
    () => { longFinished = true; },
  );

  let observedHeld = false;
  for (let attempt = 0; attempt < 200 && !observedHeld; attempt += 1) {
    const during = await rpc.call("get_state", { session: gatedSession });
    observedHeld = during.input?.held_keys?.includes("Space") === true;
    if (!observedHeld) await new Promise((resolve) => setTimeout(resolve, 5));
  }
  check(observedHeld, "a read-only state call observed Space held inside the long code run");

  const otherSession = await rpc.call("run_game_code_unsafe", {
    session,
    code: `async (game) => {
      await game.step();
      return "independent";
    }`,
  });
  check(
    otherSession.ok === true && !longFinished,
    "a mutating call on another session completed without waiting for the held gate",
  );

  const queuedInput = rpc.beginRequest("tools/call", {
    name: "send_input",
    arguments: {
      session: gateAlias,
      command: { type: "key", key: "space", down: true },
    },
  });
  const queuedWaited = await Promise.race([
    queuedInput.result.then(() => false),
    new Promise((resolve) => setTimeout(() => resolve(true), 30)),
  ]);
  check(queuedWaited, "an exact-URL alias shares the active session gate");
  rpc.cancelRequest(queuedInput.id, "prove a cancelled queued mutation never runs");
  await queuedInput.result;

  // Mark the attached alias closing while the long operation still owns the
  // shared gate. A new target on that id must reject immediately, and stop
  // itself must drain the active operation before detaching.
  const aliasStop = rpc.call("stop_game", { session: gateAlias });
  await new Promise((resolve) => setTimeout(resolve, 30));
  let afterStopStarted = null;
  try {
    await rpc.call("send_input", {
      session: gateAlias,
      command: { type: "key", key: "space", down: true },
    });
  } catch (error) {
    afterStopStarted = error.message;
  }
  check(
    afterStopStarted !== null && /stopping/.test(afterStopStarted),
    "a new mutation on an attached session is rejected after stop marks it closing",
  );

  const [heldResult, stoppedAlias] = await Promise.all([longRun, aliasStop]);
  check(/detached/.test(stoppedAlias), "stop drained the gate before detaching the attached alias");
  await new Promise((resolve) => setTimeout(resolve, 30));
  const afterCancelledInput = await rpc.call("get_state", { session: gatedSession });
  check(
    heldResult.final_state?.model?.count === 1 &&
      afterCancelledInput.model?.count === 1 &&
      afterCancelledInput.input?.held_keys?.length === 0,
    "the cancelled queued input never landed after the gate became available",
  );

  await rpc.call("send_input", {
    session: gatedSession,
    command: { type: "key", key: "space", down: false },
  });
  await rpc.call("send_input", {
    session: gatedSession,
    command: { type: "key", key: "space", down: true },
  });
  const afterQueuedInput = await rpc.call("step", { session: gatedSession });
  await rpc.call("send_input", {
    session: gatedSession,
    command: { type: "key", key: "space", down: false },
  });
  const afterRelease = await rpc.call("get_state", { session: gatedSession });
  check(
    afterQueuedInput.model?.count === 2 &&
      afterQueuedInput.input?.held_keys?.includes("Space") &&
      afterRelease.input?.held_keys?.length === 0,
    "stopping the attached alias left the original session valid and independently mutable",
  );

  console.log("\n▸ stopping an active attached id clears its queue and held input");
  const abortAlias = (await rpc.call("connect_game", { url: gateGame.url })).session;
  const attachedRun = rpc.call("run_game_code_unsafe", {
    session: abortAlias,
    code: `async (game) => {
      await game.pause();
      await game.keyDown("space");
      await game.stepFrames(10000, 0.016);
      await game.keyUp("space");
    }`,
  }).then(
    (value) => ({ value }),
    (error) => ({ error: error.message }),
  );
  let attachedHeld = false;
  for (let attempt = 0; attempt < 200 && !attachedHeld; attempt += 1) {
    const during = await rpc.call("get_state", { session: gatedSession });
    attachedHeld = during.input?.held_keys?.includes("Space") === true;
    if (!attachedHeld) await new Promise((resolve) => setTimeout(resolve, 5));
  }
  check(attachedHeld, "the attached-id code held input while its batch was active");
  const attachedStop = await rpc.call("stop_game", { session: abortAlias });
  const attachedOutcome = await attachedRun;
  const afterAttachedStop = await rpc.call("get_state", { session: gatedSession });
  check(
    /queued steps were cancelled/.test(attachedOutcome.error ?? "") &&
      /code-touched key and mouse-button levels were restored/.test(attachedOutcome.error ?? ""),
    "the interrupted code reported both clock and input cleanup",
  );
  check(/detached/.test(attachedStop), "the attached id detached after cleanup");
  check(
    afterAttachedStop.pending_steps === 0 &&
      afterAttachedStop.input?.held_keys?.length === 0,
    "the still-live runtime has no residual queue or held input after detach",
  );

  console.log("\n▸ a connect queued during owned stop cannot survive as a dead session");
  const stopRaceRun = rpc.call("run_game_code_unsafe", {
    session: gatedSession,
    code: `async (game) => {
      await game.pause();
      await game.keyDown("space");
      await game.stepFrames(10000, 0.016);
      await game.keyUp("space");
    }`,
  }).then(
    (value) => ({ value }),
    (error) => ({ error: error.message }),
  );
  let stopRaceHeld = false;
  for (let attempt = 0; attempt < 200 && !stopRaceHeld; attempt += 1) {
    const during = await rpc.call("get_state", { session: gatedSession });
    stopRaceHeld = during.input?.held_keys?.includes("Space") === true;
    if (!stopRaceHeld) await new Promise((resolve) => setTimeout(resolve, 5));
  }
  check(stopRaceHeld, "the owned runtime gate is held before the racing connect starts");
  const racingConnect = rpc.call("connect_game", { url: gateGame.url }).then(
    (value) => ({ value }),
    (error) => ({ error: error.message }),
  );
  const connectWaited = await Promise.race([
    racingConnect.then(() => false),
    new Promise((resolve) => setTimeout(() => resolve(true), 30)),
  ]);
  check(connectWaited, "connect reserves and waits on the existing exact-URL gate before discovery");
  const stopStartedAt = Date.now();
  const ownerStop = rpc.call("stop_game", { session: gatedSession });
  const [runOutcome, connectOutcome, ownerStopped] = await Promise.all([
    stopRaceRun,
    racingConnect,
    ownerStop,
  ]);
  check(
    /safe boundary|stopping/.test(runOutcome.error ?? "") &&
      Date.now() - stopStartedAt < 10000,
    "owned stop interrupts a progressing 10,000-frame operation at a safe polling boundary",
  );
  check(
    /stopping/.test(connectOutcome.error ?? ""),
    "owned stop closes the queued connect lifecycle before it can insert",
  );
  check(/killed/.test(ownerStopped), "owned stop completed child cleanup");
  const afterOwnedStop = await rpc.call("list_sessions");
  check(
    !afterOwnedStop.sessions.some((known) => known.url === gateGame.url),
    "owner, aliases, and the rejected connect leave no dead session record",
  );

  console.log("\n▸ pause + step is a deterministic clock");
  await rpc.call("pause", { session });
  const paused = await rpc.call("get_state", { session });
  const first = await rpc.call("step", { session });
  const second = await rpc.call("step", { session });
  check(first.frame > paused.frame, `the first step advanced the frame (${paused.frame} → ${first.frame})`);
  check(second.frame > first.frame, `the second step advanced it again (→ ${second.frame})`);
  check(second.pending_steps === 0, "step returns only once its queued steps have landed");
  check(
    Math.abs(second.tts - first.tts - 0.016) < 1e-4,
    `each step advanced time by its dts (tts ${first.tts.toFixed(3)} → ${second.tts.toFixed(3)})`,
  );
  let invalidDts = null;
  try {
    await rpc.call("step", { session, dts: -0.016 });
  } catch (error) {
    invalidDts = error.message;
  }
  const afterInvalidDts = await rpc.call("get_state", { session });
  check(
    /finite positive/.test(invalidDts ?? "") &&
      afterInvalidDts.tts === second.tts,
    "step rejects a negative dts before simulation time can move backwards",
  );

  console.log("\n▸ injected input reaches the game");
  // counter's `update` handles Inc, delivered by clicking the button — slot 0,
  // the first interactive widget in its `ui` tree.
  const before = (await rpc.call("get_state", { session })).model.count;
  await rpc.call("send_input", { session, command: { type: "ui_event", slot: 0, kind: "Clicked" } });
  const clicked = await rpc.call("step", { session });
  check(clicked.model.count === before + 1, `a ui_event click incremented the model (${before} → ${clicked.model.count})`);

  // A key event exercises the keyboard shape end to end. counter has no `input`
  // hook, so the assertion is that the runtime accepted and sampled it — not a
  // model change it cannot make.
  await rpc.call("send_input", { session, command: { type: "key", key: "w", down: true } });
  const held = await rpc.call("step", { session });
  check(held.input.held_keys.includes("W"), `an injected key is held level state (held_keys = ${JSON.stringify(held.input.held_keys)})`);

  console.log("\n▸ errors are teaching errors, not silence");
  let capture = null;
  try {
    await rpc.call("capture_frame", { session });
  } catch (error) {
    capture = error.message;
  }
  check(capture !== null && /headless/.test(capture), "capture_frame on a headless session explains why there are no pixels");

  let unknown = null;
  try {
    await rpc.call("get_state", { session: "s99" });
  } catch (error) {
    unknown = error.message;
  }
  check(unknown !== null && unknown.includes(session), "an unknown session id names the sessions that do exist");

  console.log("\n▸ authoring a game with no filesystem");
  // The whole project inline — an entry plus a sibling module, neither of
  // which exists anywhere on disk.
  const ENTRY = `type Model = { n: float }

let init = { n: 0.0 }

let tick = (m: Model, dt, tts) => { m with n: m.n + Step.amount }

let draw = (m: Model, tts) =>
  Frame.create(
    Camera3D.lookAt(Vec3.make(0.0, 1.5, -4.0), Vec3.make(0.0, 0.0, 0.0)),
    Scene.cube() |> Scene.rotateY(Angle.degrees(m.n)),
  )
`;
  const EDITED = ENTRY.replace("m.n + Step.amount", "m.n + Step.amount * 10.0");

  const inline = await rpc.call("launch_game", {
    files: [
      ["game.fun", ENTRY],
      ["step.fun", "let amount = 1.0\n"],
    ],
    mode: "headless",
  });
  const authored = inline.session;
  check(/^s\d+$/.test(authored), `an inline project launched (${authored}), from ${inline.dir}`);

  await rpc.call("pause", { session: authored });
  const ticked = await rpc.call("step", { session: authored });
  check(ticked.model.n >= 1, `the inline game ran its own tick (n = ${ticked.model.n})`);

  // A live edit of the ENTRY, with the pushed sibling still linked.
  await rpc.call("reload_source", { session: authored, source: EDITED });
  const beforeEdit = (await rpc.call("get_state", { session: authored })).model.n;
  const afterEdit = (await rpc.call("step", { session: authored })).model.n;
  check(afterEdit - beforeEdit === 10, `the edit took effect, model preserved (${beforeEdit} → ${afterEdit})`);

  const saveDir = mkdtempSync(join(tmpdir(), "functor-mcp-e2e-save-"));
  const saved = await rpc.call("save_project", { session: authored, dir: saveDir });
  check(saved.files.includes("game.fun") && saved.files.includes("step.fun"), `save_project wrote ${saved.files}`);
  const savedEntry = readFileSync(join(saveDir, "game.fun"), "utf8");
  check(savedEntry === EDITED, "the SAVED source is the EDITED source (the runtime's truth, not the launch files)");
  check(readFileSync(join(saveDir, "step.fun"), "utf8").trim() === "let amount = 1.0", "the pushed sibling was saved too");
  check(JSON.parse(readFileSync(join(saveDir, "functor.json"), "utf8")).entry === "game.fun", "a functor.json was synthesized for the saved project");

  // A second save into the same directory is refused without an explicit
  // overwrite: a matching entry name is no evidence of the same project.
  let occupied = null;
  try {
    await rpc.call("save_project", { session: authored, dir: saveDir });
  } catch (error) {
    occupied = error.message;
  }
  check(occupied !== null && /already holds a project/.test(occupied), "save_project refuses an occupied directory");
  await rpc.call("save_project", { session: authored, dir: saveDir, overwrite: true });
  check(readFileSync(join(saveDir, "game.fun"), "utf8") === EDITED, "overwrite: true re-saves it");

  await rpc.call("stop_game", { session: authored });
  check(!existsSync(inline.dir), "the scratch project directory is removed with its session");

  let bothSources = null;
  try {
    await rpc.call("launch_game", { dir: "examples/counter", files: [["game.fun", "let init = 1"]] });
  } catch (error) {
    bothSources = error.message;
  }
  check(bothSources !== null && /dir OR files/.test(bothSources), "launch_game refuses dir and files together");

  console.log("\n▸ init_game scaffolds a project that boots");
  const scaffoldRoot = mkdtempSync(join(tmpdir(), "functor-mcp-e2e-init-"));
  const scaffold = join(scaffoldRoot, "game");
  const scaffoldResult = await rpc.call("init_game", { dir: scaffold });
  check(scaffoldResult.files.includes("game.fun"), `init_game wrote ${scaffoldResult.files} into ${scaffoldResult.dir}`);

  const scaffolded = await rpc.call("launch_game", { dir: scaffold, mode: "headless" });
  check(typeof scaffolded.protocol_version === "number", "the scaffolded project boots");
  await rpc.call("stop_game", { session: scaffolded.session });

  let reinit = null;
  try {
    await rpc.call("init_game", { dir: scaffold });
  } catch (error) {
    reinit = error.message;
  }
  check(reinit !== null && /game\.fun/.test(reinit), "init_game refuses to overwrite an existing project");
  rmSync(saveDir, { recursive: true, force: true });
  rmSync(scaffoldRoot, { recursive: true, force: true });

  console.log("\n▸ shutdown");
  const stopped = await rpc.call("stop_game", { session });
  session = null;
  check(/killed/.test(stopped), "stop_game killed the launched runtime");
  const empty = await rpc.call("list_sessions");
  check(empty.sessions.length === 0, "the registry is empty again");
} finally {
  if (session) {
    try {
      await rpc.call("stop_game", { session });
    } catch {
      // best effort
    }
  }
  proc.stdin.end();
  const exit = await new Promise((resolve) => {
    proc.on("exit", resolve);
    setTimeout(() => resolve("timeout"), 5000);
  });
  check(exit === 0, `the server exited cleanly on stdin close (${exit})`);
}

if (failures.length) {
  console.error(`\n✗ mcp-server failed: ${failures.join(", ")}`);
  if (rpc.stderr) console.error(`\nserver stderr:\n${rpc.stderr}`);
  process.exit(1);
}
console.log("\n✓ mcp-server passed");
