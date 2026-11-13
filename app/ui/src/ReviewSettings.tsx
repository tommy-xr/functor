import { useEffect, useState } from "react";
import type { AppState, ReviewSetting } from "./generated/protocol";
import { useMenu } from "./menu";
import { modelHint, reviewSetting } from "./review-model";

/**
 * Beside the agent picker: how the active project's changes are reviewed
 * (D31) — on or off, and the review's model and instructions. Each change
 * is saved as it is made (text fields when they lose focus).
 */
export function ReviewSettings({
  state,
  online,
  onChange,
}: {
  state: AppState | null;
  online: boolean;
  onChange: (setting: ReviewSetting) => void;
}) {
  const { open, setOpen, ref, onKeyDown } = useMenu();
  const setting = reviewSetting(state);
  const [model, setModel] = useState(setting.model ?? "");
  const [instructions, setInstructions] = useState(setting.instructions ?? "");
  // The fields follow the saved setting each time the panel opens.
  useEffect(() => {
    setModel(setting.model ?? "");
    setInstructions(setting.instructions ?? "");
  }, [open, setting.model, setting.instructions]);
  // Reviews run on Claude: no agent is saved, so a stored one never blocks a save.
  const save = (change: Partial<ReviewSetting>) => onChange({ ...setting, harness: null, ...change });
  const text = (value: string) => value.trim() || null;
  const editable = online && setting.on;
  return (
    <div className="review-menu" ref={ref} onKeyDown={onKeyDown}>
      <button
        type="button"
        className="review-toggle"
        aria-haspopup="dialog"
        aria-expanded={open}
        disabled={!state?.active}
        title="How each change is reviewed before you decide"
        onClick={() => setOpen((o) => !o)}
      >
        Review {setting.on ? "on" : "off"}
      </button>
      {open && (
        <div className="review-panel" role="dialog" aria-label="Review settings">
          <label className="review-on">
            <input type="checkbox" checked={setting.on} disabled={!online} onChange={(e) => save({ on: e.target.checked })} />
            Prepare a separate review
          </label>
          <p className="review-help">
            When a change waits for you, a read-only agent writes its summary, evidence and things to note. Off, the working agent's
            own report is shown.
          </p>
          <p className="review-help">Agent: Claude, for every task. Codex reviews cannot be held read-only yet.</p>
          <label className="review-field">
            <span>Model</span>
            <input
              className="text-input"
              value={model}
              placeholder={modelHint()}
              disabled={!editable}
              spellCheck={false}
              onChange={(e) => setModel(e.target.value)}
              onBlur={() => text(model) !== setting.model && save({ model: text(model) })}
            />
          </label>
          <label className="review-field">
            <span>Instructions</span>
            <textarea
              className="text-input"
              rows={3}
              value={instructions}
              placeholder="Optional: added to the review's instructions, e.g. a skill to follow"
              disabled={!editable}
              onChange={(e) => setInstructions(e.target.value)}
              onBlur={() => text(instructions) !== setting.instructions && save({ instructions: text(instructions) })}
            />
          </label>
        </div>
      )}
    </div>
  );
}
