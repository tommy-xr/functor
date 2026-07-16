// A VSCode-style bottom status bar for the sandbox and IDE pages: a slim
// strip with two toggleable panels — Problems (live type diagnostics from the
// language analysis) and Output (runtime console traces + reload results).
// Plain DOM, no framework. The host page owns placement (an empty container)
// and feeds the returned handle:
//
//   const bar = createStatusBar({ host });
//   bar.setProblems([{ severity, message, loc, jump }]);   // whole list, each pass
//   bar.appendOutput(level, text);                          // one line at a time
//
// Problems entries carry their own `jump` (the host closes over its editor),
// so the component stays editor-agnostic.

const MAX_OUTPUT_LINES = 500;

export const createStatusBar = ({ host }) => {
  host.className = "statusbar";

  const panel = document.createElement("div");
  panel.className = "statusbar-panel";
  panel.hidden = true;

  const problemsList = document.createElement("div");
  problemsList.className = "statusbar-list problems-list";
  const outputList = document.createElement("div");
  outputList.className = "statusbar-list output-list";

  const strip = document.createElement("div");
  strip.className = "statusbar-strip";

  const tabs = {};
  let open = null; // "problems" | "output" | null

  const show = (name) => {
    open = name;
    panel.hidden = open === null;
    problemsList.style.display = open === "problems" ? "" : "none";
    outputList.style.display = open === "output" ? "" : "none";
    for (const [tabName, button] of Object.entries(tabs)) {
      button.classList.toggle("active", tabName === open);
    }
    // Lines appended while the panel was hidden couldn't stick to the bottom
    // (a display:none subtree measures 0) — land on the newest, not the oldest.
    if (open === "output") outputList.scrollTop = outputList.scrollHeight;
  };

  const makeTab = (name, label) => {
    const button = document.createElement("button");
    button.type = "button";
    button.className = "statusbar-tab";
    button.dataset.tab = name;
    button.textContent = label;
    button.addEventListener("click", () => show(open === name ? null : name));
    tabs[name] = button;
    strip.appendChild(button);
    return button;
  };

  const problemsTab = makeTab("problems", "✓ 0 problems");
  makeTab("output", "output");

  panel.appendChild(problemsList);
  panel.appendChild(outputList);
  host.appendChild(panel);
  host.appendChild(strip);

  const setProblems = (items) => {
    // The tab goes loud (red ✖) only for errors — a warnings-only file keeps
    // the calm glyph.
    const errors = items.filter((item) => (item.severity || "error") === "error").length;
    problemsTab.textContent =
      items.length === 0
        ? "✓ 0 problems"
        : `${errors > 0 ? "✖" : "⚠"} ${items.length} problem${items.length === 1 ? "" : "s"}`;
    problemsTab.classList.toggle("has-problems", errors > 0);
    problemsList.textContent = "";
    if (items.length === 0) {
      const empty = document.createElement("div");
      empty.className = "statusbar-empty";
      empty.textContent = "No problems detected.";
      problemsList.appendChild(empty);
      return;
    }
    for (const item of items) {
      const row = document.createElement("button");
      row.type = "button";
      row.className = "problem-row";
      const icon = document.createElement("span");
      const severity = item.severity || "error";
      icon.className = `problem-icon severity-${severity}`;
      icon.textContent = severity === "error" ? "✖" : "⚠";
      const message = document.createElement("span");
      message.className = "problem-message";
      message.textContent = item.message;
      const loc = document.createElement("span");
      loc.className = "problem-loc";
      loc.textContent = item.loc;
      row.append(icon, message, loc);
      if (item.jump) row.addEventListener("click", item.jump);
      problemsList.appendChild(row);
    }
  };
  setProblems([]);

  // Appends are rAF-coalesced: a per-frame `Debug.log` in tick/draw arrives
  // ~60/sec, and appending each line individually would force a layout read
  // (scrollHeight) per message while the panel is open. One DOM flush per
  // frame keeps the panel usable under a logging loop.
  let pending = [];
  let flushScheduled = false;

  const flushOutput = () => {
    flushScheduled = false;
    if (pending.length === 0) return;
    // Stick to the bottom only if the user is already there (don't yank the
    // scroll out from under them mid-read).
    const atBottom = outputList.scrollTop + outputList.clientHeight >= outputList.scrollHeight - 4;
    // A burst larger than the cap only ever shows its tail — skip the rest.
    const lines = pending.slice(-MAX_OUTPUT_LINES);
    pending = [];
    for (const { level, text } of lines) {
      const line = document.createElement("div");
      line.className = `output-line output-${level}`;
      line.textContent = text;
      outputList.appendChild(line);
    }
    while (outputList.childElementCount > MAX_OUTPUT_LINES) {
      outputList.firstElementChild.remove();
    }
    if (atBottom) outputList.scrollTop = outputList.scrollHeight;
  };

  const appendOutput = (level, text) => {
    pending.push({ level, text });
    if (!flushScheduled) {
      flushScheduled = true;
      requestAnimationFrame(flushOutput);
    }
  };

  return { setProblems, appendOutput };
};
