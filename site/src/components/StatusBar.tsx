// The VSCode-style bottom status bar: a slim strip with three toggleable
// panels — Problems (live type diagnostics), Output (runtime console traces +
// reload results), and Executions (the paused inspector's entry-point picker).
//
// The rows come from `status-bar-store.ts`; this is only the view. It renders
// the same markup the imperative `status-bar.ts` builds (still the IDE's), so
// styles.css and the e2e selectors are untouched — including keeping all three
// lists MOUNTED and toggled with `display`, exactly as before.

import { useEffect, useLayoutEffect, useRef, useState, useSyncExternalStore } from "react";
import { outputPreamble } from "../status-bar-store.js";
import type { StatusBarStore } from "../status-bar-store.js";

/** Which panel a tab opens; also the tab's `data-tab`. */
type TabName = "problems" | "output" | "executions";

const problemsLabel = (count: number, errors: number): string =>
  count === 0
    ? "✓ 0 problems"
    : `${errors > 0 ? "✖" : "⚠"} ${count} problem${count === 1 ? "" : "s"}`;

export const StatusBar = ({ store }: { store: StatusBarStore }) => {
  const { problems, output, executions } = useSyncExternalStore(store.subscribe, store.getSnapshot);
  const [open, setOpen] = useState<TabName | null>(null);
  const outputList = useRef<HTMLDivElement>(null);
  // Stick to the bottom only while the user is already there (don't yank the
  // scroll out from under them mid-read). Tracked from scroll events rather
  // than measured mid-update: by the time a layout effect runs, the new lines
  // are already in the DOM and the pre-append position is gone.
  const stick = useRef(true);

  // The tab goes loud (red ✖) only for errors — a warnings-only file keeps the
  // calm glyph.
  const errors = problems.filter((item) => (item.severity || "error") === "error").length;

  useLayoutEffect(() => {
    const list = outputList.current;
    // Lines appended while the panel was hidden couldn't stick to the bottom
    // (a display:none subtree measures 0) — land on the newest, not the oldest.
    if (list && open === "output" && stick.current) list.scrollTop = list.scrollHeight;
  }, [output, open]);

  // Opening the panel is the one moment a hidden list must be re-pinned even
  // if the user had scrolled away inside it earlier.
  useEffect(() => {
    const list = outputList.current;
    if (list && open === "output") {
      stick.current = true;
      list.scrollTop = list.scrollHeight;
    }
  }, [open]);

  const panel = (name: TabName) => ({
    className: `statusbar-list ${name}-list`,
    style: open === name ? undefined : { display: "none" },
  });

  const tab = (name: TabName, label: string, extra = "") => (
    <button
      type="button"
      className={`statusbar-tab${open === name ? " active" : ""}${extra}`}
      data-tab={name}
      onClick={() => setOpen(open === name ? null : name)}
    >
      {label}
    </button>
  );

  return (
    <>
      <div className="statusbar-panel" hidden={open === null}>
        <div {...panel("problems")}>
          {problems.length === 0 ? (
            <div className="statusbar-empty">No problems detected.</div>
          ) : (
            problems.map((item, index) => {
              const severity = item.severity || "error";
              return (
                <button type="button" className="problem-row" key={index} onClick={item.jump}>
                  <span className={`problem-icon severity-${severity}`}>
                    {severity === "error" ? "✖" : "⚠"}
                  </span>
                  <span className="problem-message">{item.message}</span>
                  <span className="problem-loc">{item.loc}</span>
                </button>
              );
            })
          )}
        </div>
        <div
          {...panel("output")}
          ref={outputList}
          onScroll={(event) => {
            const list = event.currentTarget;
            stick.current = list.scrollTop + list.clientHeight >= list.scrollHeight - 4;
          }}
        >
          {output.map((line) => (
            <div className={`output-line output-${line.level}`} key={line.id}>
              <span className="output-preamble">{outputPreamble(line.frame, line.time)}</span>
              {` ${line.text}`}
            </div>
          ))}
        </div>
        <div {...panel("executions")}>
          {executions.length === 0 ? (
            <div className="statusbar-empty">Pause the game to inspect the frame's executions.</div>
          ) : (
            executions.map((item, index) => (
              <button
                type="button"
                className={`exec-row${item.selected ? " selected" : ""}`}
                key={index}
                onClick={item.onPick}
              >
                {item.label}
              </button>
            ))
          )}
        </div>
      </div>
      <div className="statusbar-strip">
        {tab("problems", problemsLabel(problems.length, errors), errors > 0 ? " has-problems" : "")}
        {tab("output", "output")}
        {tab("executions", executions.length ? `⏸ ${executions.length} executions` : "executions")}
      </div>
    </>
  );
};
