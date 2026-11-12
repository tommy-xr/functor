import { test } from "node:test";
import assert from "node:assert/strict";
import {
  applyItem,
  atBottom,
  clip,
  connectionLines,
  initialTask,
  outcome,
  phraseText,
  stepAvailable,
  stepPhrase,
  walkTo,
  tokens,
  toolKind,
  usageBefore,
  type Snapshot,
} from "./chat-model.ts";
import type { Item, Task } from "./generated/protocol";

const phrase = (name: string, input: unknown = {}) => phraseText(stepPhrase(name, input as never));

test("game tools lose their server prefix and read as short phrases", () => {
  assert.equal(phrase("mcp__mis__get_state", { session: "s1" }), "Read the game state");
  assert.equal(phrase("mcp__mis__capture_frame"), "Captured a frame");
  assert.equal(phrase("mcp__mis__get_scene"), "Read the scene");
  assert.equal(phrase("mcp__mis__send_input"), "Sent input to the game");
  assert.equal(phrase("mcp__mis__pause"), "Paused the game");
  assert.equal(phrase("mcp__mis__resume"), "Resumed the game");
  assert.equal(phrase("mcp__mis__reload_source"), "Reloaded the game");
  assert.equal(phrase("mcp__mis__reload_project"), "Reloaded the project");
  assert.equal(phrase("mcp__mis__app_context"), "Checked what you are looking at");
  assert.equal(phrase("mcp__mis__step", { frames: 120 }), "Stepped 120 frames");
  assert.equal(phrase("mcp__mis__step", {}), "Stepped 1 frame");
  // Without the prefix too, as Codex may name them.
  assert.equal(phrase("get_state"), "Read the game state");
});

test("built-in tools name their file, command or pattern", () => {
  assert.deepEqual(stepPhrase("Read", { file_path: "/w/examples/counter/game.fun" }), {
    text: "Read",
    code: "game.fun",
  });
  assert.equal(phrase("Edit", { file_path: "/w/ladder.fun", old_string: "a", new_string: "b" }), "Edited ladder.fun");
  assert.equal(phrase("MultiEdit", { file_path: "C:\\w\\ladder.fun" }), "Edited ladder.fun");
  assert.equal(phrase("Write", { file_path: "/w/new.fun" }), "Wrote new.fun");
  assert.equal(phrase("Bash", { command: "cargo test -p functor-app\necho done" }), "Ran cargo test -p functor-app");
  assert.equal(phrase("Grep", { pattern: "fn tick" }), "Searched for fn tick");
  assert.equal(phrase("Glob", { pattern: "**/*.fun" }), "Listed **/*.fun");
  assert.equal(phrase("ToolSearch", { query: "select:mcp__mis__get_state" }), "Looked up tools");
  assert.equal(phrase("Bash", { command: "x".repeat(100) }).length, "Ran ".length + 60);
});

test("inputs without the expected fields still read, and unknown tools fall back to their bare name", () => {
  assert.equal(phrase("Read", {}), "Read a file");
  assert.equal(phrase("Edit", { file_path: 3 }), "Edited a file");
  assert.equal(phrase("Bash", "ls"), "Ran a command");
  assert.equal(phrase("Grep", null), "Searched the code");
  assert.equal(phrase("mcp__mis__brand_new_tool"), "brand_new_tool");
  assert.equal(phrase("mcp__other_server__lookup"), "lookup");
  assert.equal(phrase("WebFetch", { url: "https://x" }), "WebFetch");
  assert.equal(phrase("constructor"), "constructor");
});

test("edits are change cards, captures capture cards, the rest steps", () => {
  assert.equal(toolKind("Edit"), "change");
  assert.equal(toolKind("Write"), "change");
  assert.equal(toolKind("MultiEdit"), "change");
  assert.equal(toolKind("mcp__mis__capture_frame"), "capture");
  assert.equal(toolKind("mcp__mis__get_state"), "step");
  assert.equal(toolKind("Read"), "step");
});

const user = (text: string): Item => ({ kind: "user", text });
const agent = (text: string, partial = true): Item => ({ kind: "agent", text, partial });

test("items replace by index or append at the end", () => {
  const items = [user("hi"), agent("He")];
  const grown = applyItem(items, 1, agent("Hello"));
  assert.deepEqual(grown, [user("hi"), agent("Hello")]);
  // Untouched items keep their identity, so their rows do not re-render.
  assert.equal((grown as Item[])[0], items[0]);
  assert.deepEqual(items, [user("hi"), agent("He")], "not mutated");
  assert.deepEqual(applyItem(items, 2, { kind: "stopped" }), [user("hi"), agent("He"), { kind: "stopped" }]);
});

test("an item for a transcript not fetched is dropped, and one past a gap asks for a refetch", () => {
  assert.equal(applyItem(undefined, 3, user("x")), undefined);
  assert.equal(applyItem([user("hi")], 3, user("x")), "refetch");
});

test("outcomes: done, failed with the summary's first line, stopped", () => {
  assert.deepEqual(outcome({ kind: "finished", ok: true, summary: "All good" }), { tone: "ok", label: "Done" });
  assert.deepEqual(outcome({ kind: "finished", ok: false, summary: "\nspawning claude\nmore" }), {
    tone: "danger",
    label: "Failed: spawning claude",
  });
  assert.deepEqual(outcome({ kind: "finished", ok: false, summary: null }), { tone: "danger", label: "Failed" });
  assert.deepEqual(outcome({ kind: "stopped" }), { tone: "neutral", label: "Stopped" });
  assert.equal(outcome(user("hi")), null);
});

test("usage folds into the outcome after it", () => {
  const items: Item[] = [user("hi"), { kind: "usage", input: 24100, output: 410 }, { kind: "stopped" }];
  assert.deepEqual(usageBefore(items, 2), { input: 24100, output: 410 });
  assert.equal(usageBefore(items, 1), null);
  assert.equal(usageBefore(items, 0), null);
  assert.equal(tokens(410), "410");
  assert.equal(tokens(24100), "24.1k");
  assert.equal(tokens(2000), "2k");
  assert.equal(tokens(1_250_000), "1.3M");
});

test("long text is clipped by lines and characters", () => {
  const lines = Array.from({ length: 40 }, (_, i) => `line ${i}`).join("\n");
  const cut = clip(lines);
  assert.equal(cut.more, true);
  assert.equal(cut.shown.split("\n").length, 12);
  assert.ok(cut.shown.endsWith("…"));
  assert.equal(clip("x".repeat(5000)).shown.length, 801);
  assert.deepEqual(clip("short"), { shown: "short", more: false });
});

const task = (id: number, project: string, kind: Task["status"]["kind"]): Task =>
  ({
    id,
    project,
    title: `t${id}`,
    status: kind === "failed" ? { kind, summary: null } : { kind },
    harness: "claude",
    model: null,
    session_id: null,
    run: null,
  }) as Task;

test("the dock opens on the newest open task of the project", () => {
  const tasks = [
    task(1, "a", "waiting_on_you"),
    task(2, "a", "in_progress"),
    task(3, "a", "failed"),
    task(4, "b", "waiting_on_you"),
  ];
  assert.equal(initialTask(tasks, "a"), 2);
  assert.equal(initialTask(tasks, "b"), 4);
  assert.equal(initialTask([task(3, "a", "failed")], "a"), null);
  assert.equal(initialTask(tasks, null), null);
});

const snap = (over: Partial<Snapshot>): Snapshot => ({ conn: "open", active: "a", session: null, ...over });
const running = { kind: "running", debug_url: "http://127.0.0.1:1", frame: 3, surface: null } as const;

test("connection lines: the socket dropping and coming back", () => {
  assert.deepEqual(connectionLines(snap({}), snap({ conn: "reconnecting" })), ["Connection lost, reconnecting…"]);
  assert.deepEqual(connectionLines(snap({ conn: "reconnecting" }), snap({})), ["Reconnected"]);
  // The first connection is not news.
  assert.deepEqual(connectionLines(snap({ conn: "connecting", active: undefined }), snap({})), []);
});

test("connection lines: the active project's session starting, stopping and failing", () => {
  assert.deepEqual(connectionLines(snap({ session: { kind: "starting" } }), snap({ session: running })), [
    "Game session started",
  ]);
  assert.deepEqual(connectionLines(snap({ session: running }), snap({ session: { kind: "stopped" } })), [
    "Game session stopped",
  ]);
  assert.deepEqual(
    connectionLines(snap({ session: running }), snap({ session: { kind: "exited", reason: "exit 1\nmore" } })),
    ["Game session exited: exit 1"],
  );
  assert.deepEqual(
    connectionLines(snap({ session: { kind: "starting" } }), snap({ session: { kind: "failed", reason: "no gpu" } })),
    ["Game session failed: no gpu"],
  );
  // Frames ticking by, the first state, and a project switch say nothing.
  assert.deepEqual(connectionLines(snap({ session: running }), snap({ session: { ...running, frame: 9 } })), []);
  assert.deepEqual(connectionLines(snap({ active: undefined }), snap({ session: running })), []);
  assert.deepEqual(connectionLines(snap({ session: running }), snap({ active: "b" })), []);
  assert.deepEqual(connectionLines(snap({}), snap({ session: { kind: "starting" } })), []);
});

test("following the bottom: within a few pixels counts", () => {
  assert.equal(atBottom({ scrollTop: 500, scrollHeight: 1000, clientHeight: 500 }), true);
  assert.equal(atBottom({ scrollTop: 480, scrollHeight: 1000, clientHeight: 500 }), true);
  assert.equal(atBottom({ scrollTop: 400, scrollHeight: 1000, clientHeight: 500 }), false);
  assert.equal(atBottom({ scrollTop: 0, scrollHeight: 300, clientHeight: 500 }), true);
});

test("the view tools read as what they showed", () => {
  const file = { kind: "file", path: "src/ladder.fun", lines: [4, 5] };
  assert.equal(phrase("mcp__mis__show_view", { view: file }), "Showed ladder.fun, lines 4–5");
  assert.deepEqual(stepPhrase("mcp__mis__show_view", { view: file }), { text: "Showed", code: "ladder.fun", after: ", lines 4–5" });
  assert.equal(phrase("mcp__mis__show_view", { view: { ...file, lines: [7, 7] } }), "Showed ladder.fun, line 7");
  assert.equal(phrase("mcp__mis__show_view", { view: { kind: "file", path: "a.fun", lines: null } }), "Showed a.fun");
  assert.equal(phrase("mcp__mis__show_view", { view: { kind: "game" } }), "Showed the game");
  assert.equal(phrase("mcp__mis__show_view", { view: { kind: "changes", path: null } }), "Showed the changes");
  assert.equal(phrase("mcp__mis__show_view", { view: { kind: "changes", path: "game.fun" } }), "Showed game.fun in the changes");
  assert.equal(phrase("mcp__mis__show_view", {}), "Showed a view");
  assert.equal(phrase("mcp__mis__present", { steps: [] }), "Presented a walkthrough");
});

test("a walkthrough step is unavailable once its file or change is gone", () => {
  const file = { kind: "file", path: "game.fun", lines: null } as const;
  assert.equal(stepAvailable(file, ["game.fun"], false), true);
  assert.equal(stepAvailable(file, ["other.fun"], true), false);
  assert.equal(stepAvailable({ kind: "changes", path: null }, [], true), true);
  assert.equal(stepAvailable({ kind: "changes", path: "game.fun" }, ["game.fun"], false), false);
  assert.equal(stepAvailable({ kind: "game" }, [], false), true);
});

test("prev and next skip unavailable steps and stop at the ends", () => {
  const ok = [true, false, true, true];
  assert.equal(walkTo(ok, null, 1), 0);
  assert.equal(walkTo(ok, null, -1), null);
  assert.equal(walkTo(ok, 0, 1), 2);
  assert.equal(walkTo(ok, 2, -1), 0);
  assert.equal(walkTo(ok, 3, 1), null);
  assert.equal(walkTo([false, true], null, 1), 1);
});
