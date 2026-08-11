// The IDE's header controls: the zip download, the share link, the preview
// restart, the external-runtime panel, and the live status pill. The page keeps owning the
// project and preview logic — this island renders the pill's store and calls
// back on intent, exactly as the sandbox's controls do.

import { RuntimeTargetPanel } from "./RuntimeTargetPanel.js";
import { ShareButton } from "./ShareButton.js";
import type { ShareState } from "./ShareButton.js";
import { StatusPill } from "./StatusPill.js";
import type { PillState } from "./StatusPill.js";
import type { RuntimeTargetCore } from "../runtime-target-core.js";
import type { Store } from "../store.js";

export interface IdeControlsProps {
  pill: Store<PillState>;
  share: Store<ShareState>;
  runtimeTarget: RuntimeTargetCore;
  onDownload: () => void;
  onRestart: () => void;
  onShare: () => void;
}

export const IdeControls = ({
  pill,
  share,
  runtimeTarget,
  onDownload,
  onRestart,
  onShare,
}: IdeControlsProps) => (
  <>
    <button id="download" type="button" title="Download the project as a .zip" onClick={onDownload}>
      ↓ project.zip
    </button>
    <ShareButton store={share} onShare={onShare} />
    <button
      id="restart"
      type="button"
      title="Reload the preview with a fresh model"
      onClick={onRestart}
    >
      ↻ restart
    </button>
    <div id="runtime-target" className="runtime-target-host">
      <RuntimeTargetPanel core={runtimeTarget} />
    </div>
    <StatusPill store={pill} />
  </>
);
