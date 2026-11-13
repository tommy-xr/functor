import { useEffect, useRef, useState } from "react";
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
        <span className="paths-legend">Faint: before the change. Bright: after.</span>
        <span>Predicted from {from}. The game's two renders, overlaid.</span>
      </figcaption>
      {open && <PathDialog task={task} state={state} onClose={() => setOpen(false)} />}
    </figure>
  );
}

/**
 * The decision page's pair, in place (D35): two fixed frames that fill when
 * the captures arrive, so nothing under them moves. Nothing unless they are
 * shown or being taken. A click opens them on the Captures tab.
 */
export function PairFrames({ task, onOpen }: { task: Task; onOpen: () => void }) {
  const state = task.evidence?.state;
  if (state?.kind !== "shown" && state?.kind !== "taking") return null;
  const from = momentPhrase(task);
  return (
    <figure className="pair-frames" data-testid="decision-pair" aria-busy={state.kind === "taking"}>
      <button type="button" className="pair-frames-button" aria-label="Before and after: open on the Captures tab" onClick={onOpen}>
        {(["before", "after"] as const).map((side) => (
          <span key={side} className="pair-side">
            <span className="pair-frame">
              {state.kind === "shown" ? <img src={evidenceUrl(state[side])} alt={alt(side, from)} /> : <span className="pair-wait">Predicting…</span>}
            </span>
            <span className="pair-label">{side === "before" ? "Before" : "After"}</span>
          </span>
        ))}
      </button>
      {/* The moment is known before the captures: the caption never changes. */}
      <figcaption className="pair-frames-caption">
        {from ? `Predicted path from ${from}, under the old code and the new.` : "Predicted path under the old code and the new."}
      </figcaption>
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

/**
 * The pair, large, in a native modal dialog: the rest of the page is inert
 * and Tab stays inside; Escape, Close or the backdrop closes it, and focus
 * goes back where it was.
 */
function PathDialog({ task, state, onClose }: { task: Task; state: Shown; onClose: () => void }) {
  const dialog = useRef<HTMLDialogElement>(null);
  const close = useRef<HTMLButtonElement>(null);
  // Read while rendering, before the dialog takes the focus.
  const [before] = useState(() => document.activeElement as HTMLElement | null);
  useEffect(() => {
    dialog.current?.showModal();
    close.current?.focus();
    return () => before?.focus();
  }, [before]);
  return (
    <dialog
      ref={dialog}
      className="modal paths-dialog"
      aria-labelledby="paths-title"
      data-testid="paths-dialog"
      onClose={onClose}
      // The backdrop is the dialog itself; its content is not.
      onClick={(e) => e.target === e.currentTarget && dialog.current?.close()}
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
        <p className="hint">From {momentPhrase(task)}, under the old code and the new. Each is the game's own render.</p>
        <button type="button" className="button" ref={close} onClick={() => dialog.current?.close()}>
          Close
        </button>
      </footer>
    </dialog>
  );
}
