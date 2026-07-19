// Live `expect` test status — the pure half of the gutter wiring (the
// inspector.js pattern: decision logic here, node-tested; extension.js does
// the vscode.Range/setDecorations plumbing).
//
// The LSP pushes `functor/tests/status`:
//   { generation, files: { "<uri>": [ { line, state, detail } ] } }
// Every uri present is AUTHORITATIVE for that file (an empty list clears
// it); absent uris keep their previous rows. States: running | pass | fail
// | error | unrunnable; unknown states are dropped (a newer server must not
// break an older extension).

const STATUS = "functor/tests/status";
const EXPECT_STATES = ["running", "pass", "fail", "error", "unrunnable"];

// Merge one status push into `byUri` (a Map of uri → rows). Returns the
// list of uris whose rows changed (the files to repaint), or null for a
// malformed push.
function mergeStatuses(byUri, params) {
  if (!params || typeof params.files !== "object" || params.files === null) {
    return null;
  }
  const touched = [];
  for (const [uri, rows] of Object.entries(params.files)) {
    if (!Array.isArray(rows)) continue;
    const kept = rows.filter(
      (row) =>
        row &&
        typeof row.line === "number" &&
        EXPECT_STATES.includes(row.state)
    );
    byUri.set(uri, kept);
    touched.push(uri);
  }
  return touched;
}

// Per-state line lists for one uri's rows — what each decoration type gets.
function groupByState(rows) {
  const groups = { running: [], pass: [], fail: [], error: [], unrunnable: [] };
  for (const row of rows || []) groups[row.state].push(row.line);
  return groups;
}

// The Problems-panel entries for one uri's rows: failing/erroring expects
// only, with the decomposed/actual detail as the message.
function problemRows(rows) {
  return (rows || [])
    .filter((row) => row.state === "fail" || row.state === "error")
    .map((row) => ({
      line: row.line,
      message:
        row.state === "fail"
          ? `expect failed${row.detail ? `: ${row.detail}` : ""}`
          : `expect errored${row.detail ? `: ${row.detail}` : ""}`,
    }));
}

// Hover text for the gutter line (running/unrunnable/pass want context too).
function hoverText(row) {
  switch (row.state) {
    case "running":
      return "expect: re-running…";
    case "pass":
      return "expect: pass";
    case "unrunnable":
      return `expect: not runnable in the editor${row.detail ? ` — ${row.detail}` : ""}`;
    default:
      return `expect: ${row.state}${row.detail ? ` — ${row.detail}` : ""}`;
  }
}

module.exports = {
  STATUS,
  EXPECT_STATES,
  mergeStatuses,
  groupByState,
  problemRows,
  hoverText,
};
