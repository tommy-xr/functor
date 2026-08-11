// The share-link codec's contract: every real project round-trips losslessly
// and small enough to fit in a URL, and NOTHING an attacker can put in a
// fragment gets through as a project (or throws).
//
// The size half is a regression guard with teeth: a share link has to survive
// being pasted into a chat client, so the encoded fragment for every example in
// the repo is asserted under a cap. A payload-format change that stops
// compressing — or starts carrying something it shouldn't — fails here.
//
//   node --test site/src/share-link.test.ts
import { test } from "node:test";
import assert from "node:assert/strict";
import { existsSync, readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";
import { encodeShare, decodeShare, assetLocators } from "./share-link.ts";
import type { ShareProject } from "./share-link.ts";

const EXAMPLES = new URL("../../examples", import.meta.url).pathname;
/** A share link must stay pasteable; the biggest example sets the bar. */
const MAX_ENCODED_CHARS = 24_000;

const roundTrip = async (project: ShareProject) => {
  const hash = await encodeShare(project);
  const back = await decodeShare(hash);
  return { hash, back };
};

test("round-trips a multi-file project with entries config and options", async () => {
  const project: ShareProject = {
    files: [
      { path: "game.fun", source: "let init = { t: 0.0 }\n" },
      { path: "protocol.fun", source: "let hello = \"hi\"\n" },
    ],
    entry: "game.fun",
    config: {
      entries: {
        client: { file: "game.fun", module: "Client" },
        server: "protocol.fun",
      },
    },
    options: { mouseCapture: false },
  };
  const { hash, back } = await roundTrip(project);
  assert.match(hash, /^#code=[A-Za-z0-9_-]+$/);
  assert.deepEqual(back, {
    // `entry` is dropped on the wire when it is the default, so it comes back absent.
    files: project.files,
    config: project.config,
    options: { mouseCapture: false },
  });
});

test("carries a non-default entry and a cursor policy", async () => {
  const { back } = await roundTrip({
    files: [{ path: "main.fun", source: "let init = 0\n" }],
    entry: "main.fun",
    options: { cursor: "visible" },
  });
  assert.equal(back?.entry, "main.fun");
  assert.deepEqual(back?.options, { cursor: "visible" });
});

// --- the real projects -------------------------------------------------------

interface ExampleConfig {
  entry?: string;
  entries?: Record<string, unknown>;
  cursor?: string;
  mouseCapture?: boolean;
}

/** Read an `examples/<name>/` directory the way a Share button would. */
const readExample = (name: string): ShareProject => {
  const dir = join(EXAMPLES, name);
  const files = readdirSync(dir)
    .filter((f) => f.endsWith(".fun"))
    .sort()
    .map((f) => ({ path: f, source: readFileSync(join(dir, f), "utf8") }));
  const config: ExampleConfig = JSON.parse(readFileSync(join(dir, "functor.json"), "utf8"));
  const project: ShareProject = { files };
  // `game.fun` is the codec's default and never rides on the wire, so a project
  // built for comparison must not claim it either.
  if (config.entry && config.entry !== "game.fun") project.entry = config.entry;
  if (config.entries) project.config = { entries: config.entries as never };
  const options: ShareProject["options"] = {};
  if (config.cursor === "visible") options.cursor = "visible";
  if (config.mouseCapture === false) options.mouseCapture = false;
  if (Object.keys(options).length > 0) project.options = options;
  return project;
};

// Every example that is a PROJECT — `examples/replay` is a bare scene fixture
// with no functor.json, so it has nothing to share.
const exampleNames = readdirSync(EXAMPLES, { withFileTypes: true })
  .filter((e) => e.isDirectory() && existsSync(join(EXAMPLES, e.name, "functor.json")))
  .map((e) => e.name)
  .sort();

test("round-trips every example project, and each stays pasteable", async (t) => {
  assert.ok(exampleNames.length > 10, "expected the examples/ tree to be populated");
  for (const name of exampleNames) {
    const project = readExample(name);
    const raw = project.files.reduce((n, f) => n + f.source.length, 0);
    const { hash, back } = await roundTrip(project);
    const encoded = hash.length - "#code=".length;
    // One line per project: the size table a size regression shows up in.
    t.diagnostic(
      `${name.padEnd(20)} ${String(project.files.length).padStart(2)} files ` +
        `${String(raw).padStart(7)} raw ${String(encoded).padStart(6)} encoded`
    );
    assert.deepEqual(back, project, `${name} did not round-trip losslessly`);
    assert.ok(
      encoded <= MAX_ENCODED_CHARS,
      `${name} encodes to ${encoded} chars, over the ${MAX_ENCODED_CHARS} cap`
    );
  }
});

// --- the locators a link cannot carry ----------------------------------------

test("finds the relative Asset locators, and only those", async () => {
  const files = [
    {
      path: "game.fun",
      source: `let ship = Asset.model("ship.glb")
let shot = Asset.sound("sfx/shot.ogg")
let cdn = Asset.model("https://cdn.example/x.glb")
let proto = Asset.texture("//cdn.example/y.png")
let spaced = Asset.texture(
  "grid.png")
// Not asset coercion: a texture FILE and a clip NAME keep their strings.
let tex = Texture.file("nope.png")
let clip = Anim.clip("walk", tts)
`,
    },
    // The same locator twice is one asset; a sibling's locators count too.
    { path: "lib.fun", source: `let again = Asset.model("ship.glb")\nlet hit = Asset.sound("hit.ogg")\n` },
  ];
  assert.deepEqual(assetLocators(files), [
    "grid.png",
    "hit.ogg",
    "sfx/shot.ogg",
    "ship.glb",
  ]);
});

test("a project with no local assets has nothing to warn about", () => {
  assert.deepEqual(assetLocators([{ path: "game.fun", source: "let init = 0\n" }]), []);
});

// --- the legacy docs fragment ------------------------------------------------

// Exactly what site/src/docs.ts writes into a "▶ try it" href.
const docsToBase64Url = (s: string): string =>
  btoa(String.fromCharCode(...new TextEncoder().encode(s)))
    .replace(/\+/g, "-")
    .replace(/\//g, "_")
    .replace(/=+$/, "");

test("decodes the docs' legacy #src= fragment as a single game.fun", async () => {
  const source = "let init = 0\nlet draw = (model, tts: float) => Frame.create()\n// ünïcode ✦\n";
  const project = await decodeShare(`#src=${docsToBase64Url(source)}`);
  assert.deepEqual(project, { files: [{ path: "game.fun", source }] });
});

test("finds its param anywhere in a multi-param hash", async () => {
  // The sandbox writes `#clients=2&src=…` (site/src/sandbox.tsx), so a share
  // param is not necessarily the first thing after the `#`.
  const source = "let init = 0\n";
  const legacy = await decodeShare(`#clients=2&src=${docsToBase64Url(source)}`);
  assert.deepEqual(legacy, { files: [{ path: "game.fun", source }] });

  const code = (await encodeShare({ files: [{ path: "game.fun", source }] })).slice("#".length);
  assert.deepEqual(await decodeShare(`#clients=3&${code}`), {
    files: [{ path: "game.fun", source }],
  });
});

// --- hostile input -----------------------------------------------------------

test("rejects everything that is not a valid fragment", async () => {
  const hostile = [
    "",
    "#",
    "#code=",
    "#src=",
    "#nope=abc",
    "#code=!!!not base64!!!",
    "#code=" + "AAAA", // valid base64url, not a deflate stream
    "#src=%%%",
  ];
  for (const hash of hostile) {
    assert.equal(await decodeShare(hash), null, `expected ${JSON.stringify(hash)} to be rejected`);
  }
});

/** Encode an arbitrary envelope, bypassing `encodeShare`'s well-formed input. */
const rawCode = async (envelope: unknown): Promise<string> => {
  const json = new TextEncoder().encode(JSON.stringify(envelope));
  const stream = new Blob([json]).stream().pipeThrough(new CompressionStream("deflate-raw"));
  const bytes = new Uint8Array(await new Response(stream).arrayBuffer());
  let binary = "";
  for (const b of bytes) binary += String.fromCharCode(b);
  return `#code=${btoa(binary).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "")}`;
};

test("rejects a wrong or missing version", async () => {
  const files = { "game.fun": "let init = 0\n" };
  assert.equal(await decodeShare(await rawCode({ v: 2, f: files })), null);
  assert.equal(await decodeShare(await rawCode({ f: files })), null);
  assert.equal(await decodeShare(await rawCode({ v: "1", f: files })), null);
  // …and accepts the one version it knows, so the cases above fail on `v` alone.
  assert.ok(await decodeShare(await rawCode({ v: 1, f: files })));
});

test("rejects file names outside the flat module space", async () => {
  const bad = [
    "../x.fun",
    "a/b.fun",
    "/etc/passwd",
    "..%2Fx.fun",
    "game.js",
    "game.fun.bak",
    "1game.fun",
    "",
    ".fun",
  ];
  for (const path of bad) {
    const hash = await rawCode({ v: 1, f: { [path]: "let init = 0\n" } });
    assert.equal(await decodeShare(hash), null, `expected ${JSON.stringify(path)} to be rejected`);
  }
});

test("rejects a malformed envelope shape", async () => {
  const bad: unknown[] = [
    { v: 1 }, // no files
    { v: 1, f: {} }, // empty module space
    { v: 1, f: [] }, // files as an array
    { v: 1, f: { "game.fun": 7 } }, // source not a string
    { v: 1, f: { "game.fun": "x" }, e: "missing.fun" }, // entry not in the project
    { v: 1, f: { "game.fun": "x" }, e: 3 },
    { v: 1, f: { "game.fun": "x" }, c: 7 },
    { v: 1, f: { "game.fun": "x" }, c: { entries: { client: "missing.fun" } } },
    { v: 1, f: { "game.fun": "x" }, c: { entries: { "../c": "game.fun" } } },
    // a role declares at most one of module / prefix
    { v: 1, f: { "game.fun": "x" }, c: { entries: { c: { file: "game.fun", module: "C", prefix: "c" } } } },
    { v: 1, f: { "game.fun": "x" }, c: { entries: { c: { file: "game.fun", module: "not an ident" } } } },
    // inherited object properties are not project files
    { v: 1, f: { "game.fun": "x" }, e: "__proto__" },
    { v: 1, f: { "game.fun": "x" }, e: "constructor" },
    { v: 1, f: { "game.fun": "x" }, c: { entries: { s: "toString" } } },
    { v: 1, f: { "game.fun": "x" }, c: { entries: { s: { file: "constructor" } } } },
    // nothing to boot: no roles, and no (default) entry file
    { v: 1, f: { "main.fun": "x" } },
    // a module space that could not exist on a case-insensitive filesystem
    { v: 1, f: { "game.fun": "x", "Game.fun": "y" } },
    { v: 1, f: { "game.fun": "x" }, o: { cursor: "hidden" } },
    { v: 1, f: { "game.fun": "x" }, o: { mouseCapture: true } },
    { v: 1, f: { "game.fun": "x" }, o: "nope" },
    "just a string",
    42,
    null,
    [1, 2, 3],
  ];
  for (const envelope of bad) {
    const hash = await rawCode(envelope);
    assert.equal(
      await decodeShare(hash),
      null,
      `expected ${JSON.stringify(envelope)} to be rejected`
    );
  }
});

test("rejects an oversize payload without inflating all of it", async () => {
  // Highly compressible: ~4MB of source in a tiny fragment — the deflate-bomb
  // shape. It must come back null, not as a 4MB project.
  const files: Record<string, string> = { "game.fun": "let init = 0\n" };
  for (let i = 0; i < 8; i++) files[`m${i}.fun`] = "a".repeat(512 * 1024);
  const hash = await rawCode({ v: 1, f: files });
  assert.ok(hash.length < 20_000, "the bomb should be small on the wire");
  assert.equal(await decodeShare(hash), null);
});

test("rejects too many files", async () => {
  const files: Record<string, string> = { "game.fun": "let init = 0\n" };
  for (let i = 0; i < 200; i++) files[`m${i}.fun`] = "let x = 0\n";
  assert.equal(await decodeShare(await rawCode({ v: 1, f: files })), null);
});

test("refuses to mint a link that would not decode", async () => {
  const source = "let init = 0\n";
  const unshareable: ShareProject[] = [
    { files: [{ path: "game.fun", source }, { path: "game.fun", source }] }, // dupe
    { files: [{ path: "../x.fun", source }] },
    { files: [] },
    { files: [{ path: "main.fun", source }] }, // no boot target
    { files: [{ path: "game.fun", source }], config: { entries: { c: "missing.fun" } } },
  ];
  for (const project of unshareable) {
    await assert.rejects(
      () => encodeShare(project),
      /share-link/,
      `expected ${JSON.stringify(project)} to be refused`
    );
  }
});
