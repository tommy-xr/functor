// The compact, shared opt-in control for the sandbox and IDE headers. Vim is
// described as keybindings (rather than a full editor/runtime mode), and the
// pressed state mirrors the persisted preference while its chunk loads.

import { useSyncExternalStore } from "react";
import { editorKeybindingsButtonPresentation } from "../editor-keybindings.js";
import type { EditorKeybindingsController } from "../editor-keybindings.js";

export const EditorKeybindingsButton = ({
  controller,
}: {
  controller: EditorKeybindingsController;
}) => {
  const state = useSyncExternalStore(
    controller.state.subscribe,
    controller.state.getSnapshot
  );
  const presentation = editorKeybindingsButtonPresentation(state);

  return (
    <button
      id="editor-keybindings"
      className="editor-keybindings-toggle"
      type="button"
      aria-pressed={presentation.enabled}
      aria-busy={state.loading}
      title={presentation.title}
      onClick={() =>
        void controller.setMode(presentation.enabled ? "standard" : "vim")
      }
    >
      {presentation.text}
    </button>
  );
};
