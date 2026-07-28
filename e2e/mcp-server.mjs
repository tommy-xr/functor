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
//   4. pause, then step twice — asserting the frame advances and that `step`
//      only returns once the queued steps have landed (pending_steps == 0).
//   5. send_input: counter's model reacts to a UI click (its `update` handles
//      Inc), so a `ui_event` on slot 0 must increment `count` after a step.
//      A `key` event is also injected to exercise that shape end to end.
//   6. the filesystem-less authoring journey: launch_game with the whole
//      project INLINE (no directory anywhere), edit it live with
//      reload_source (model preserved), then save_project — asserting the
//      saved source is the EDITED source, i.e. the wire's truth rather than
//      whatever the launch wrote.
//   7. init_game scaffolds a starter project on disk and it boots.
//   8. stop_game, then a clean shutdown of the server itself.
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
import { fileURLToPath } from "node:url";

const ROOT = fileURLToPath(new URL("..", import.meta.url));
const BIN = process.env.FUNCTOR_BIN ?? `${ROOT}target/debug/functor`;

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
  "resume",
  "rewind",
  "reload_source",
  "reload_project",
  "api_reference",
  "language_guide",
  "validate_automation_code",
  "run_automation_code",
  "init_game",
  "save_project",
];

/** A line-delimited JSON-RPC client over a child's stdio. */
class Rpc {
  constructor(proc) {
    this.proc = proc;
    this.pending = new Map();
    this.nextId = 1;
    this.buffer = "";
    this.stderr = "";
    proc.stdout.setEncoding("utf8");
    proc.stdout.on("data", (chunk) => this.#onData(chunk));
    proc.stderr.on("data", (chunk) => (this.stderr += chunk));
  }

  #onData(chunk) {
    this.buffer += chunk;
    let index;
    while ((index = this.buffer.indexOf("\n")) >= 0) {
      const line = this.buffer.slice(0, index).trim();
      this.buffer = this.buffer.slice(index + 1);
      if (!line) continue;
      const message = JSON.parse(line);
      const waiter = this.pending.get(message.id);
      if (waiter) {
        this.pending.delete(message.id);
        clearTimeout(waiter.timer);
        message.error ? waiter.reject(new Error(JSON.stringify(message.error))) : waiter.resolve(message.result);
      }
    }
  }

  notify(method, params) {
    this.proc.stdin.write(`${JSON.stringify({ jsonrpc: "2.0", method, params })}\n`);
  }

  request(method, params) {
    const id = this.nextId++;
    this.proc.stdin.write(`${JSON.stringify({ jsonrpc: "2.0", id, method, params })}\n`);
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id);
        reject(new Error(`${method} timed out\nstderr:\n${this.stderr}`));
      }, 60000);
      this.pending.set(id, { resolve, reject, timer });
    });
  }

  /** Call a tool and return its single text content block, parsed if it is JSON. */
  async call(name, args = {}) {
    const result = await this.request("tools/call", { name, arguments: args });
    const text = result.content.map((block) => block.text ?? "").join("");
    if (result.isError) throw new Error(`${name} failed: ${text}`);
    try {
      return JSON.parse(text);
    } catch {
      return text;
    }
  }
}

const failures = [];
const check = (ok, what) => {
  console.log(`  ${ok ? "✓" : "✗"} ${what}`);
  if (!ok) failures.push(what);
};

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

  console.log("\n▸ restricted automation source validates without a session");
  const automationSource = `automation("round trip")
    .pause()
    .pressKey("2")
    .step({ frames: 3, dts: 0.02 })
    .expectModel("count", 0)
    .capture("proof")`;
  const validated = await rpc.call("validate_automation_code", { code: automationSource });
  check(validated.valid === true && validated.plan?.steps?.length === 5, "validation returns the normalized serializable plan");
  check(typeof validated.canonical_code === "string" && /automation\("round trip"\)/.test(validated.canonical_code), "validation returns deterministic canonical SDK source");
  check(validated.budget?.used?.total_frames === 4, "validation reports expanded frame-budget use (pressKey + step)");
  const roundTripped = await rpc.call("validate_automation_code", { code: validated.canonical_code });
  check(
    JSON.stringify(roundTripped.plan) === JSON.stringify(validated.plan),
    "canonical source parses back to the identical plan",
  );

  const rejectedAutomation = await rpc.call("validate_automation_code", {
    code: `automation().pause().then(() => process.exit())`,
  });
  check(
    rejectedAutomation.valid === false &&
      rejectedAutomation.errors?.[0]?.line === 1 &&
      /unknown automation method|restricted/.test(rejectedAutomation.errors?.[0]?.message ?? ""),
    "callbacks, globals, and unknown calls are rejected with a source diagnostic",
  );

  console.log("\n▸ launching a game headlessly");
  const launched = await rpc.call("launch_game", { dir: "examples/counter", mode: "headless" });
  session = launched.session;
  check(/^s\d+$/.test(session), `launch_game returned a session id (${session})`);
  check(launched.discovery?.service === "functor debug runtime", "the child answered the debug-runtime discovery endpoint");
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

  console.log("\n▸ an automation plan is fully validated before its first action");
  await rpc.call("pause", { session });
  const beforeRejectedRun = await rpc.call("get_state", { session });
  let rejectedRun = null;
  try {
    // The prefix is a VALID mutating action. The invalid suffix must prevent
    // the prefix from being sent, proving whole-plan validation comes first.
    await rpc.call("run_automation_code", {
      session,
      code: `automation("must stay pure").uiClick(0).unknown()`,
    });
  } catch (error) {
    rejectedRun = error.message;
  }
  const afterRejectedRun = await rpc.call("get_state", { session });
  check(rejectedRun !== null && /unknown automation method/.test(rejectedRun), "run rejects an invalid suffix");
  check(
    afterRejectedRun.model.count === beforeRejectedRun.model.count &&
      afterRejectedRun.tts === beforeRejectedRun.tts,
    "the valid action prefix had no model or clock side effect",
  );

  console.log("\n▸ one automation call replaces raw pause/input/step/assert choreography");
  const automated = await rpc.call("run_automation_code", {
    session,
    code: `automation("counter proof")
      .pause()
      .uiClick(0)
      .step()
      .expectModel("count", 1)
      .expectModelClose("count", 1.0001, 0.001)
      .mouseMove(400, 300)
      .mouseWheel(-1)
      .keyDown("w")
      .step()
      .inspect("while held")
      .keyUp("w")`,
  });
  check(automated.ok === true && automated.steps_executed === 11, "the complete normalized plan executed");
  check(automated.assertions?.[0]?.passed === true, "expectModel passed against structured model data");
  check(
    automated.assertions?.[1]?.passed === true &&
      automated.assertions?.[1]?.abs_tolerance === 0.001,
    "expectModelClose passed a bounded numeric assertion",
  );
  check(
    automated.observations?.[0]?.state?.input?.held_keys?.includes("W"),
    "inspect observed typed held input between deterministic steps",
  );
  check(
    automated.final_state?.input?.mouse?.x === 400 &&
      automated.final_state?.input?.mouse?.y === 300,
    "mouseMove and mouseWheel used integer debug-protocol payloads successfully",
  );
  check(automated.final_state?.model?.count === 1, "the automation returned fresh final state");

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
    Camera.lookAt(Vec3.make(0.0, 1.0, -4.0), Vec3.make(0.0, 0.0, 0.0)),
    Scene.cube(),
  )
`;
  const gateGame = await rpc.call("launch_game", {
    files: [["game.fun", EDGE_COUNTER]],
    mode: "headless",
  });
  const gatedSession = gateGame.session;
  let longFinished = false;
  const longRun = rpc.call("run_automation_code", {
    session: gatedSession,
    code: `automation("hold gate")
      .pause()
      .keyDown("space")
      .step({ frames: 2000, dts: 0.016 })
      .keyUp("space")
      .inspect("released")`,
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
  check(observedHeld, "a read-only state call observed Space held inside the long automation");

  const otherSession = await rpc.call("run_automation_code", {
    session,
    code: `automation("independent session").step()`,
  });
  check(
    otherSession.ok === true && !longFinished,
    "a mutating call on another session completed without waiting for the held gate",
  );

  const queuedInput = rpc.call("send_input", {
    session: gatedSession,
    command: { type: "key", key: "space", down: true },
  });
  const queuedWaited = await Promise.race([
    queuedInput.then(() => false),
    new Promise((resolve) => setTimeout(() => resolve(true), 30)),
  ]);
  check(queuedWaited, "the overlapping lower-level input call waited behind the session gate");

  const [heldResult] = await Promise.all([longRun, queuedInput]);
  await rpc.call("step", { session: gatedSession });
  const afterQueuedInput = await rpc.call("get_state", { session: gatedSession });
  await rpc.call("send_input", {
    session: gatedSession,
    command: { type: "key", key: "space", down: false },
  });
  const afterRelease = await rpc.call("get_state", { session: gatedSession });
  check(
    heldResult.final_state?.model?.count === 1 &&
      afterQueuedInput.model?.count === 2 &&
      afterQueuedInput.input?.held_keys?.includes("Space") &&
      afterRelease.input?.held_keys?.length === 0,
    "serialized automation/input lifecycles produced two rising edges and a released final state",
  );
  await rpc.call("stop_game", { session: gatedSession });

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
    Camera.lookAt(Vec3.make(0.0, 1.5, -4.0), Vec3.make(0.0, 0.0, 0.0)),
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
  check(scaffolded.discovery?.service === "functor debug runtime", "the scaffolded project boots");
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
