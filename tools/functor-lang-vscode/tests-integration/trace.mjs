// The canned wire-contract trace the E2E harness delivers to the extension.
//
// The LSP hash-gates every live-value hint: it only shows values for a file
// whose CURRENT text hashes (sha256) to the trace's recorded `sources[].hash`.
// So the trace is built from the on-disk game.fun text at launch time — same
// bytes VS Code opens into the buffer, same bytes the LSP sees over didOpen.
//
// Shape mirrors docs/visual-debugger-implementation.md's "wire contract" and the
// LSP's inspector::TraceDoc. We attach ONE binding — the `model` parameter of
// `update` (a real recorder capture site) — with a sentinel value so the inlay
// hint it produces ("= 42") is unambiguous in the editor DOM (type hints read
// ": Type", never "= …").
import { createHash } from "node:crypto";

// The binder whose live value we assert on, and the sentinel value we assert.
export const BINDING_NAME = "model";
export const CANNED_VALUE = "42";
// What the resulting inlay hint's text is (LSP renders live hints as "= value").
export const EXPECTED_HINT = `= ${CANNED_VALUE}`;

// Build the trace doc for a given game.fun source text. Places the binding at
// the FIRST occurrence of `model` (the `update` parameter, near the top of the
// file so its inlay hint is inside the default viewport).
export function buildTrace(source) {
  const start = source.indexOf(BINDING_NAME);
  if (start < 0) {
    throw new Error(`binding '${BINDING_NAME}' not found in game.fun`);
  }
  const hash = createHash("sha256").update(Buffer.from(source, "utf8")).digest("hex");
  return {
    frame: 1,
    tts: 1.0,
    paused: true,
    sources: [{ file: "game.fun", hash }],
    invocations: [
      {
        entry: "update",
        index: 0,
        count: 1,
        provenance: "subscription: Tick",
        ghost: false,
        result: "{ ticks = 42.0, lastTime = 0.0 }",
        truncated: false,
        bindings: [
          {
            name: BINDING_NAME,
            file: "game.fun",
            start,
            end: start + BINDING_NAME.length,
            value: CANNED_VALUE,
            count: 1,
          },
        ],
      },
    ],
  };
}
