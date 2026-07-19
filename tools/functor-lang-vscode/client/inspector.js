// Pure decision logic for the visual-debugger (paused-scene inspector) glue in
// extension.js. Everything that *decides* anything lives here so it can be
// exercised headlessly with `node --test` (see inspector.test.js) — no VS Code
// host, no LSP server. extension.js keeps only the thin wiring: command
// registration, the actual client.sendNotification calls, the status bar item,
// and the webview hookup.
//
// The two custom LSP notifications (server contract, see the LSP crate):
//   functor/inspector/attach  { port: N }    attach the poller to GET :N/trace
//   functor/inspector/attach  { port: null } detach
//   functor/inspector/trace   <wire JSON>    push a trace doc (wasm relay path)
// Attach persists server-side until detach, so one attach per run session is
// enough — no re-attach machinery here.

const ATTACH = "functor/inspector/attach";
const TRACE = "functor/inspector/trace";

// The port the runtime's debug server listens on by default (`--debug-port`).
const DEFAULT_PORT = 8077;

// Command ids (kept in sync with package.json's contributes.commands and the
// registrations in extension.js). Prefixed `functor-lang.` to match the
// existing `functor.openLivePreview` convention.
const ATTACH_COMMAND = "functor.inspector.attach";
const DETACH_COMMAND = "functor.inspector.detach";

// --- webview relay --------------------------------------------------------
// The preview webview forwards `functor-inspector-trace` window messages (from
// the game iframe — the future wasm push path, PR2b) to the extension. Turn
// such a message into the LSP notification to send, or null for anything else
// (unrelated messages must be ignored). The message shape is
//   { type: "functor-inspector-trace", trace: <wire-contract JSON> }
function relayTrace(msg) {
  if (!msg || msg.type !== "functor-inspector-trace") return null;
  return { notification: TRACE, params: msg.trace };
}

// --- recency-gutter coverage ----------------------------------------------
// The LSP pushes `functor/inspector/coverage` with per-line recency states
// (`{ uri, lines: [{line, state}] }`). Group into per-state line lists for
// the extension to hand to its four decoration types; unknown states are
// dropped (a newer server must not break an older extension). Pure — the
// tested half; extension.js does the vscode.Range/setDecorations wiring.
const COVERAGE = "functor/inspector/coverage";
const COVERAGE_STATES = ["now", "before", "after", "dark"];

function groupCoverage(params) {
  const groups = { now: [], before: [], after: [], dark: [] };
  if (!params || typeof params.uri !== "string" || !Array.isArray(params.lines)) {
    return null;
  }
  for (const entry of params.lines) {
    if (!entry || typeof entry.line !== "number") continue;
    if (COVERAGE_STATES.includes(entry.state)) groups[entry.state].push(entry.line);
  }
  return { uri: params.uri, groups };
}

// --- attach / detach notifications ---------------------------------------
function attachNotification(port) {
  return { notification: ATTACH, params: { port } };
}

function detachNotification() {
  return { notification: ATTACH, params: { port: null } };
}

// --- port validation / persistence ---------------------------------------
// Parse a user-entered port string. Returns { port } on success or { error }
// with a human message (the error doubles as VS Code's validateInput return).
function parsePort(input) {
  const trimmed = String(input == null ? "" : input).trim();
  if (!/^\d+$/.test(trimmed)) return { error: "Enter a port number (1–65535)." };
  const port = Number(trimmed);
  if (port < 1 || port > 65535) return { error: "Port must be between 1 and 65535." };
  return { port };
}

// --- attach state + status bar derivation --------------------------------
// State is { attached, port }: `port` is the last-used port (remembered across
// attach/detach and, via the host's globalState, across sessions).
function initialState(lastPort) {
  const parsed = parsePort(lastPort);
  return { attached: false, port: parsed.error ? DEFAULT_PORT : parsed.port };
}

function reduce(state, action) {
  switch (action && action.type) {
    case "attach":
      return { attached: true, port: action.port };
    case "detach":
      return { attached: false, port: state.port };
    default:
      return state;
  }
}

// The value to pre-fill the attach prompt with (the remembered port).
function promptDefault(state) {
  return String(state.port);
}

// Status bar item derivation: text, tooltip, and the command a click runs —
// attach when detached, detach when attached (so clicking toggles sensibly).
function statusBar(state) {
  if (state.attached) {
    return {
      text: `$(debug) inspector :${state.port}`,
      tooltip: `Functor Lang inspector attached to :${state.port} — click to detach`,
      command: DETACH_COMMAND,
    };
  }
  return {
    text: "$(debug-disconnect) inspector",
    tooltip: "Functor Lang inspector detached — click to attach",
    command: ATTACH_COMMAND,
  };
}

module.exports = {
  ATTACH,
  TRACE,
  DEFAULT_PORT,
  ATTACH_COMMAND,
  DETACH_COMMAND,
  relayTrace,
  groupCoverage,
  COVERAGE,
  COVERAGE_STATES,
  attachNotification,
  detachNotification,
  parsePort,
  initialState,
  reduce,
  promptDefault,
  statusBar,
};
