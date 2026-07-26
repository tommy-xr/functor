// The landing hero's status dot: the one piece of chrome on the card, pinned to
// its top-right corner. Green when the last edit went live, amber while the
// player is busy, red on a broken edit (the old program keeps running); the
// full message is its tooltip.
//
// It is the hero's entire React surface — the card itself is an iframe and a
// CodeMirror instance, neither of which React renders. The island exists so the
// dot's state lives in the same `Store` the seam reads, rather than in a DOM
// attribute the imperative code has to keep in sync by hand.

import { useSyncExternalStore } from "react";
import type { Store } from "../store.js";

/** The dot's three states — also its `data-state` attribute value. */
export type HeroState = "busy" | "live" | "error";

/** The dot's state and the detail behind its tooltip. */
export interface HeroStatus {
  state: HeroState;
  message: string;
}

export const HeroStatusDot = ({ store }: { store: Store<HeroStatus> }) => {
  const { state, message } = useSyncExternalStore(store.subscribe, store.getSnapshot);
  return <div className="hero-status" data-state={state} title={message || state} />;
};
