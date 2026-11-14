// Panes in windows of their own (D20). The macOS window opens and closes
// real windows to match the layout's list and reports one the user closed;
// a browser opens a window from the user's click (popup blockers allow no
// other) and watches it close. A window closed either way docks its tabs.

import { useEffect, useRef } from "react";
import type { Frame } from "./generated/protocol";
import type { WantedWindow } from "./layout-model";
import { popOutUrl, type Send } from "./socket";

type Hooks = {
  functorTitlebar?: boolean;
  ipc?: { postMessage(text: string): void };
  functorWindowClosed?: (pane: number) => void;
  functorWindowFrame?: (pane: number, frame: Frame) => void;
};
const hooks = window as Hooks;

/** In the macOS window, which manages the windows itself. */
const NATIVE = hooks.functorTitlebar === true;

/** Moves and resizes settle this long before the layout keeps the frame. */
const FRAME_SETTLE_MS = 400;
const CLOSED_POLL_MS = 500;

/** Browser windows this page opened, by pane. */
const opened = new Map<number, Window>();

/** Shows pane `pane`'s window: brought to the front, or opened again in a browser. Call from a click. */
export function showWindow(pane: number) {
  if (NATIVE) {
    hooks.ipc?.postMessage(JSON.stringify({ window: { show: pane } }));
    return;
  }
  const w = window.open(popOutUrl(pane), `functor-pane-${pane}`, "popup,width=900,height=640");
  if (w) {
    opened.set(pane, w);
    w.focus();
  }
}

/** Pops pane `pane` out. A browser's window opens now, within the click; the macOS one follows the layout. */
export function popOut(pane: number, send: Send) {
  if (!NATIVE) showWindow(pane);
  send({ type: "layout", op: { type: "pop_out", pane } });
}

/** Keeps the windows in step with the active project's `wanted` ones. The main window's page only. */
export function useWindows(wanted: WantedWindow[], online: boolean, send: Send) {
  const live = useRef(send);
  live.current = send;
  const list = JSON.stringify(wanted);

  // The macOS window: the list to keep open, and what it reports back.
  useEffect(() => {
    if (NATIVE && online) hooks.ipc?.postMessage(JSON.stringify({ windows: JSON.parse(list) }));
  }, [list, online]);
  useEffect(() => {
    if (!NATIVE) return;
    const timers = new Map<number, ReturnType<typeof setTimeout>>();
    hooks.functorWindowClosed = (pane) => live.current({ type: "layout", op: { type: "dock", window: pane } });
    hooks.functorWindowFrame = (pane, frame) => {
      clearTimeout(timers.get(pane));
      timers.set(pane, setTimeout(() => live.current({ type: "layout", op: { type: "set_frame", window: pane, frame } }), FRAME_SETTLE_MS));
    };
    return () => {
      timers.forEach(clearTimeout);
      delete hooks.functorWindowClosed;
      delete hooks.functorWindowFrame;
    };
  }, []);

  // A browser: a window the user closed docks; one docked from here closes.
  useEffect(() => {
    if (NATIVE) return;
    const panes = new Set((JSON.parse(list) as WantedWindow[]).map((w) => w.pane));
    for (const [pane, w] of opened) {
      if (!panes.has(pane)) {
        opened.delete(pane);
        w.close();
      }
    }
    const poll = setInterval(() => {
      for (const [pane, w] of opened) {
        if (!w.closed) continue;
        opened.delete(pane);
        live.current({ type: "layout", op: { type: "dock", window: pane } });
      }
    }, CLOSED_POLL_MS);
    return () => clearInterval(poll);
  }, [list]);
}

/** Whether this page opened pane `pane`'s browser window and it is still open (always, in the macOS window). */
export function windowOpen(pane: number): boolean {
  return NATIVE || opened.get(pane)?.closed === false;
}
