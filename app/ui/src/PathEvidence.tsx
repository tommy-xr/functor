import { useState, type KeyboardEvent } from "react";
import { createPortal } from "react-dom";
import type { EvidenceState, Task } from "./generated/protocol";
import { momentPhrase, pathsNote } from "./paths-model";
import { evidenceUrl } from "./socket";

type Shown = Extract<EvidenceState, { kind: "shown" }>;

const alt = (side: "before" | "after", from: string | null) =>
  `Predicted path ${side} the change${from ? `, from ${from}` : ""}`;

/**
 * The review card's paths (D38): one picture, the old path faint over the
 * new, both the game's own renders; a click opens them side by side.
 * Nothing unless they differ.
 */
export function PathCard({ task }: { task: Task }) {
  const [open, setOpen] = useState(false);
  const state = task.evidence?.state;
  if (state?.kind === "taking") {
    return (
      <p className="chat-card-note" role="status" data-testid="paths-taking">
        Predicting the path before and after…
      </p>
    );
  }
  if (state?.kind !== "shown") return null;
  const from = momentPhrase(task);
  return (
    <figure className="paths" data-testid="paths-card">
      <button
        type="button"
        className="paths-overlay"
        aria-haspopup="dialog"
        aria-label="Predicted path before and after the change: open larger"
        onMouseDown={(e) => e.preventDefault()}
        onClick={() => setOpen(true)}
      >
        <img src={evidenceUrl(state.after)} alt={alt("after", from)} />
        <img className="paths-faint" src={evidenceUrl(state.before)} alt={alt("before", from)} />
      </button>
      <figcaption className="paths-caption">
        <span className="paths-legend">
          <span className="paths-key">
            <span className="paths-swatch paths-swatch-faint" aria-hidden="true" />
            Faint: before
          </span>
          <span className="paths-key">
            <span className="paths-swatch" aria-hidden="true" />
            Bright: after
          </span>
        </span>
        <span>
          Predicted from {from}. The game's two renders, overlaid.
        </span>
      </figcaption>
      {open && <PathDialog task={task} state={state} onClose={() => setOpen(false)} />}
    </figure>
  );
}

/** The task page's paths: the pair side by side, else why there is none. */
export function PathSection({ task }: { task: Task }) {
  const [open, setOpen] = useState(false);
  const state = task.evidence?.state;
  if (state?.kind !== "shown") {
    const note = pathsNote(task);
    return note ? (
      <p className="hint" data-testid="paths-note">
        {note}
      </p>
    ) : null;
  }
  return (
    <div className="paths-section" data-testid="paths-section">
      <button
        type="button"
        className="paths-pair-button"
        aria-haspopup="dialog"
        aria-label="Predicted path before and after the change: open larger"
        onClick={() => setOpen(true)}
      >
        <Pair task={task} state={state} />
      </button>
      <p className="hint">Predicted from {momentPhrase(task)}, under the old code and the new.</p>
      {open && <PathDialog task={task} state={state} onClose={() => setOpen(false)} />}
    </div>
  );
}

function Pair({ task, state }: { task: Task; state: Shown }) {
  const from = momentPhrase(task);
  return (
    <span className="paths-pair">
      {(["before", "after"] as const).map((side) => (
        <span key={side} className="paths-side">
          <img src={evidenceUrl(state[side])} alt={alt(side, from)} />
          <span className="paths-label">{side === "before" ? "Before the change" : "After the change"}</span>
        </span>
      ))}
    </span>
  );
}

/** The pair, large; Escape, Close or the backdrop closes it, and focus goes back. */
function PathDialog({ task, state, onClose }: { task: Task; state: Shown; onClose: () => void }) {
  // Read while rendering, before Close takes the focus.
  const [before] = useState(() => document.activeElement as HTMLElement | null);
  const close = () => {
    onClose();
    before?.focus();
  };
  const onKeyDown = (e: KeyboardEvent) => {
    if (e.key === "Escape") {
      e.stopPropagation();
      close();
    }
  };
  return createPortal(
    <div className="modal-backdrop" onClick={(e) => e.target === e.currentTarget && close()}>
      <div
        className="modal paths-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby="paths-title"
        onKeyDown={onKeyDown}
        data-testid="paths-dialog"
      >
        <header className="modal-head">
          <h2 id="paths-title" className="pane-title">
            Predicted path
          </h2>
        </header>
        <div className="paths-dialog-body">
          <Pair task={task} state={state} />
        </div>
        <footer className="modal-foot paths-dialog-foot">
          <p className="hint">
            From {momentPhrase(task)}, under the old code and the new. Each is the game's own render.
          </p>
          <button type="button" className="button" autoFocus onClick={close}>
            Close
          </button>
        </footer>
      </div>
    </div>,
    document.body,
  );
}
