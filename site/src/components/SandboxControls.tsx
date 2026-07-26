// The sandbox's header controls: the scene picker, reset, the external-runtime
// panel, and the live status pill. The page keeps owning the load logic — this
// island renders the two small stores it publishes and calls back on intent.

import { useSyncExternalStore } from "react";
import { RuntimeTargetPanel } from "./RuntimeTargetPanel.js";
import type { RuntimeTargetCore } from "../runtime-target-core.js";
import type { Store } from "../store.js";

/** One row of the scene picker: the repo examples plus a possible snippet. */
export interface PickerOption {
  value: string;
  label: string;
}

export interface PickerState {
  options: PickerOption[];
  selected: string;
}

/** The preview pill: its `data-state`, its label, and its tooltip detail. */
export interface PillState {
  state: "busy" | "live" | "error";
  text: string;
  detail: string;
}

export interface SandboxControlsProps {
  picker: Store<PickerState>;
  pill: Store<PillState>;
  runtimeTarget: RuntimeTargetCore;
  onSelect: (value: string) => void;
  onReset: () => void;
}

export const SandboxControls = ({
  picker,
  pill,
  runtimeTarget,
  onSelect,
  onReset,
}: SandboxControlsProps) => {
  const { options, selected } = useSyncExternalStore(picker.subscribe, picker.getSnapshot);
  const status = useSyncExternalStore(pill.subscribe, pill.getSnapshot);

  return (
    <>
      <label className="picker-label" htmlFor="example-picker">
        scene
      </label>
      <select
        id="example-picker"
        value={selected}
        onChange={(event) => onSelect(event.target.value)}
      >
        {options.map((option) => (
          <option value={option.value} key={option.value}>
            {option.label}
          </option>
        ))}
      </select>
      <button id="reset" type="button" title="Reload the example (resets the model)" onClick={onReset}>
        ↺ reset
      </button>
      <div id="runtime-target" className="runtime-target-host">
        <RuntimeTargetPanel core={runtimeTarget} />
      </div>
      <span id="status" className="status-pill" data-state={status.state} title={status.detail}>
        {status.text}
      </span>
    </>
  );
};
