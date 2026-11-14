import type { Harness, Scope } from "./generated/protocol";
import { Icon } from "./Icon";
import { useMenu } from "./menu";
import { CODEX_NOTE, SCOPE_WORDS, SCOPES, scopesFor } from "./scope-model";

/**
 * Beside the agent picker: how much of the user's own setup the next
 * request reads (D39), and the project's default for its new tasks.
 */
export function ScopeSettings({
  scope,
  projectDefault,
  harness,
  online,
  disabled,
  onPick,
  onDefault,
}: {
  /** What the next request reads. */
  scope: Scope;
  projectDefault: Scope;
  harness: Harness;
  online: boolean;
  disabled: boolean;
  onPick: (scope: Scope) => void;
  onDefault: (scope: Scope) => void;
}) {
  const { open, setOpen, ref, onKeyDown } = useMenu();
  const allowed = scopesFor(harness);
  return (
    <div className="review-menu scope-menu" ref={ref} onKeyDown={onKeyDown}>
      <button
        type="button"
        className="review-toggle"
        aria-haspopup="dialog"
        aria-expanded={open}
        disabled={disabled}
        title={`How much of your own setup the agent reads: ${SCOPE_WORDS[scope].name}`}
        onClick={() => setOpen((o) => !o)}
      >
        {SCOPE_WORDS[scope].short}
      </button>
      {open && (
        <div className="review-panel scope-panel" role="dialog" aria-label="Instruction scope">
          <fieldset className="scope-levels">
            <legend>What this request reads</legend>
            {SCOPES.map((s) => (
              <label key={s} className={`scope-level${allowed.includes(s) ? "" : " scope-off"}`}>
                <input
                  type="radio"
                  name="scope"
                  value={s}
                  checked={scope === s}
                  disabled={!online || !allowed.includes(s)}
                  onChange={() => onPick(s)}
                />
                <span>
                  <span className="scope-name">{SCOPE_WORDS[s].name}</span>
                  <span className="review-help">{SCOPE_WORDS[s].help}</span>
                </span>
              </label>
            ))}
          </fieldset>
          {harness === "codex" && <p className="review-help scope-note">{CODEX_NOTE}</p>}
          <label className="review-field">
            <span>New tasks in this project</span>
            <span className="switcher model-picker">
              <select
                aria-label="Project default"
                value={projectDefault}
                disabled={!online}
                onChange={(e) => onDefault(e.target.value as Scope)}
              >
                {SCOPES.map((s) => (
                  <option key={s} value={s}>
                    {SCOPE_WORDS[s].name}
                  </option>
                ))}
              </select>
              <Icon name="chevron" />
            </span>
          </label>
          <p className="review-help">What the agent may do does not change: at every level it may edit only inside the open folder.</p>
        </div>
      )}
    </div>
  );
}
