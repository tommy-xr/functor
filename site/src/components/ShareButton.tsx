// The Share control, shared by the sandbox and the IDE headers: one header
// button that copies a link carrying the project in its fragment.
//
// The button is also its own confirmation — it flips to "✓ copied" for a beat
// and back. No toast, no new chrome: the control that was clicked is where the
// reader is already looking, and a share is over the moment it is copied.

import { useSyncExternalStore } from "react";
import type { Store } from "../store.js";

/** The button's label, its tone (colour), and its tooltip. */
export interface ShareState {
  label: string;
  tone: "idle" | "ok" | "error";
  detail: string;
}

export const SHARE_IDLE: ShareState = {
  label: "⧉ share",
  tone: "idle",
  detail: "Copy a link that carries this project — files, entry, roles",
};

export const ShareButton = ({
  store,
  onShare,
}: {
  store: Store<ShareState>;
  onShare: () => void;
}) => {
  const { label, tone, detail } = useSyncExternalStore(store.subscribe, store.getSnapshot);
  return (
    <button id="share" type="button" data-tone={tone} title={detail} onClick={onShare}>
      {label}
    </button>
  );
};
