// A one-line advisory above the editor, used by both editor pages for the one
// thing a share link cannot promise: assets it does not carry (see share.ts).
//
// Non-blocking on purpose — the project is already loaded and running behind
// it. It is a strip, not a dialog: it never covers the editor, and it dismisses.

import { useSyncExternalStore } from "react";
import type { Store } from "../store.js";

/** The advisory's text; empty means there is nothing to say. */
export interface BannerState {
  text: string;
}

export const ShareBanner = ({
  store,
  onDismiss,
}: {
  store: Store<BannerState>;
  onDismiss: () => void;
}) => {
  const { text } = useSyncExternalStore(store.subscribe, store.getSnapshot);
  if (!text) return null;
  return (
    <div className="share-banner" role="status">
      <span className="share-banner-icon" aria-hidden="true">
        ⚠
      </span>
      <span className="share-banner-text">{text}</span>
      <button
        type="button"
        className="share-banner-close"
        aria-label="Dismiss this notice"
        onClick={onDismiss}
      >
        ×
      </button>
    </div>
  );
};
