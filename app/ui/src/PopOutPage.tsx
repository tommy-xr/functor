import { useEffect } from "react";
import { Canvas } from "./Canvas";
import { tabTitles, viewKey, windowPanes } from "./layout-model";
import { useAppSocket } from "./socket";
import { TransportContext } from "./TransportBar";

/**
 * A pane in a window of its own (`?pane=N`): only that pane of tabs, over
 * the same socket and core state as the main window. Docked, it closes.
 */
export function PopOutPage({ pane }: { pane: number }) {
  const { state, conn, send, transcripts, fetchTranscript, notice, diffs, fetchDiff, files, fetchFile, transport, marks } = useAppSocket();
  const online = conn === "open" && state !== null;
  const layout = state?.active ? state.layouts[state.active] : null;
  const mine = windowPanes(layout).find((p) => p.id === pane) ?? null;
  const shown = mine?.tabs[mine.active];
  const title = shown ? (tabTitles(mine!.tabs, state?.tasks ?? []).get(viewKey(shown)) ?? "") : "";
  useEffect(() => {
    document.title = title ? `${title} — Functor` : "Functor";
  }, [title]);
  // Docked from either window: a window a browser page opened closes itself
  // (the macOS window closes it from the main window's list).
  const gone = state !== null && mine === null;
  useEffect(() => {
    if (gone) window.close();
  }, [gone]);
  return (
    <TransportContext.Provider value={{ transport, marks }}>
      <div className="app app-window">
        <Canvas
          window={pane}
          state={state}
          online={online}
          send={send}
          diffs={diffs}
          fetchDiff={fetchDiff}
          notice={notice}
          target={null}
          files={files}
          fetchFile={fetchFile}
          currentTask={null}
          onNewTask={() => send({ type: "select_task", task: null, if_unset: false })}
          transcripts={transcripts}
          fetchTranscript={fetchTranscript}
          onContinue={(task) => send({ type: "select_task", task, if_unset: false })}
        />
      </div>
    </TransportContext.Provider>
  );
}
