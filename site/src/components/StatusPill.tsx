// The live-preview pill, shared by the sandbox and the IDE headers: one span
// whose `data-state` drives the colour and whose tooltip carries the detail
// (the runtime's reload note, or a parse error). Both pages publish the same
// three-field store, so the element exists once rather than twice.

import { useSyncExternalStore } from "react";
import type { Store } from "../store.js";

/** The preview pill: its `data-state`, its label, and its tooltip detail. */
export interface PillState {
  state: "busy" | "live" | "error";
  text: string;
  detail: string;
}

export const StatusPill = ({ store }: { store: Store<PillState> }) => {
  const { state, text, detail } = useSyncExternalStore(store.subscribe, store.getSnapshot);
  return (
    <span id="status" className="status-pill" data-state={state} title={detail}>
      {text}
    </span>
  );
};
