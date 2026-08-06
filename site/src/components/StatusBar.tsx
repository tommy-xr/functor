// The VSCode-style bottom status bar: a slim strip with three toggleable
// panels — Problems (live type diagnostics), Output (runtime console traces +
// reload results), and Executions (the paused inspector's entry-point picker).
//
// The rows come from `status-bar-store.ts`; this is only the view, rendered by
// both editor pages. It emits the same markup the imperative `status-bar.ts`
// built (deleted once the IDE converted), so styles.css and the e2e selectors
// are untouched — including keeping all three lists MOUNTED and toggled with
// `display`, exactly as before.

import { useEffect, useLayoutEffect, useRef, useState, useSyncExternalStore } from "react";
import type { ReactNode } from "react";
import { outputPreamble } from "../status-bar-store.js";
import type { StatusBarStore } from "../status-bar-store.js";
import { EditorKeybindingsButton } from "./EditorKeybindingsButton.js";
import type { EditorKeybindingsController } from "../editor-keybindings.js";

/** Which panel a tab opens; also the tab's `data-tab`. */
type TabName = "problems" | "output" | "executions" | "wire";

// The Vim adapter renders its --MODE-- readout, pending keys, and `:`/`/`
// command input into this segment (imperatively — React never writes children
// here). Mode names color the segment via data-vim-mode. Its own component so
// mode keystrokes re-render one empty span, not the panel lists.
const VimSegment = ({ controller }: { controller: EditorKeybindingsController }) => {
  const { vimMode } = useSyncExternalStore(
    controller.state.subscribe,
    controller.state.getSnapshot
  );
  const host = useRef<HTMLSpanElement>(null);
  useEffect(() => {
    controller.setStatusbarHost(host.current);
    return () => controller.setStatusbarHost(null);
  }, [controller]);
  return (
    <span
      className="statusbar-vim"
      data-vim-mode={vimMode ?? undefined}
      hidden={vimMode === null}
    >
      {/* Our own label — plain NORMAL, no --hyphens--. The adapter's own
          hyphenated mode button inside the host is hidden in CSS. */}
      <span className="statusbar-vim-mode">{vimMode?.toUpperCase()}</span>
      <span ref={host} className="statusbar-vim-host" />
    </span>
  );
};

const problemsLabel = (count: number, errors: number): string =>
  count === 0
    ? "✓ 0 problems"
    : `${errors > 0 ? "✖" : "⚠"} ${count} problem${count === 1 ? "" : "s"}`;

export const StatusBar = ({
  store,
  editorKeybindings,
}: {
  store: StatusBarStore;
  editorKeybindings: EditorKeybindingsController;
}) => {
  const { problems, output, executions, wirePresent } = useSyncExternalStore(
    store.subscribe,
    store.getSnapshot
  );
  const [open, setOpen] = useState<TabName | null>(null);
  const outputList = useRef<HTMLDivElement>(null);
  // The traffic monitor's two nodes (`store.wire`), mounted exactly like the
  // view strip below: their owner is imperative and repaints per frame.
  const wireHost = useRef<HTMLDivElement>(null);
  const wireBadgeHost = useRef<HTMLSpanElement>(null);
  // The view-scoped strip (`store.viewStrip`): a node an imperative surface
  // fills — today the pane grid's NETWORK legend and pane readouts. React
  // mounts it and never renders into it, so its owner can repaint per frame
  // without touching this component. See status-bar-store's note.
  const viewHost = useRef<HTMLDivElement>(null);
  // Stick to the bottom only if the user is already there (don't yank the
  // scroll out from under them mid-read). Measured HERE, during the render
  // that carries new lines: the DOM still holds the previous ones, so this is
  // the same pre-append reading the imperative bar took just before appending.
  const stick = useRef(true);
  const rendered = useRef(output);
  if (rendered.current !== output) {
    rendered.current = output;
    const list = outputList.current;
    if (list) stick.current = list.scrollTop + list.clientHeight >= list.scrollHeight - 4;
  }

  // The tab goes loud (red ✖) only for errors — a warnings-only file keeps the
  // calm glyph.
  const errors = problems.filter((item) => (item.severity || "error") === "error").length;

  useLayoutEffect(() => {
    const list = outputList.current;
    if (list && open === "output" && stick.current) list.scrollTop = list.scrollHeight;
    // Only new lines: an `open` change is the other effect's job.
  }, [output]);

  // Lines appended while the panel was hidden couldn't stick to the bottom (a
  // display:none subtree measures 0) — on open, land on the newest, not the
  // oldest, exactly as the imperative bar's tab handler did.
  useLayoutEffect(() => {
    const list = outputList.current;
    if (list && open === "output") list.scrollTop = list.scrollHeight;
  }, [open]);

  // `replaceChildren`, not `appendChild`: React does not manage these nodes, so
  // a swapped store would otherwise leave the previous ones mounted beside them.
  useLayoutEffect(() => {
    viewHost.current?.replaceChildren(store.viewStrip);
  }, [store]);

  useLayoutEffect(() => {
    wireHost.current?.replaceChildren(store.wire.panel);
    wireBadgeHost.current?.replaceChildren(store.wire.badge);
  }, [store, wirePresent]);

  // The monitor polls this rather than being pushed to — see `WireSlot`. An
  // example that drops its authority takes the tab with it, so an open wire
  // panel closes rather than lingering over a session that no longer has one.
  useEffect(() => {
    store.wire.open = open === "wire" && wirePresent;
    if (open === "wire" && !wirePresent) setOpen(null);
  }, [store, open, wirePresent]);

  const panel = (name: TabName) => ({
    className: `statusbar-list ${name}-list`,
    style: open === name ? undefined : { display: "none" },
  });

  const tab = (name: TabName, label: ReactNode, extra = "") => (
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
        <div {...panel("output")} ref={outputList}>
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
        {/* The traffic monitor, filled by wire-tab.ts. Mounted only while the
            session has an authority, so its rows can never outlive their
            session's links. */}
        {wirePresent && <div {...panel("wire")} ref={wireHost} />}
      </div>
      <div className="statusbar-strip">
        {tab("problems", problemsLabel(problems.length, errors), errors > 0 ? " has-problems" : "")}
        {tab("output", "output")}
        {tab("executions", executions.length ? `⏸ ${executions.length} executions` : "executions")}
        {wirePresent &&
          tab(
            "wire",
            <>
              wire
              {/* The live packet count. Hidden while the panel is open — the
                  rows are the count then, and a number ticking beside them is
                  noise.

                  `aria-hidden` deliberately: it changes ten times a second, and
                  a control whose ACCESSIBLE NAME churns at that rate is worse
                  than one without the number. The tab keeps the stable name
                  "wire"; the traffic itself is real text once the panel is
                  open — the rows and the footer's "showing N of M". */}
              <span
                className="wire-badge"
                ref={wireBadgeHost}
                hidden={open === "wire"}
                aria-hidden="true"
              />
            </>
          )}
        <div className="statusbar-view-host" ref={viewHost} />
        <EditorKeybindingsButton controller={editorKeybindings} />
        <VimSegment controller={editorKeybindings} />
      </div>
    </>
  );
};
