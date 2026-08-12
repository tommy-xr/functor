// The sandbox's header controls: the scene picker, reset, the external-runtime
// panel, and the live status pill. The page keeps owning the load logic — this
// island renders the two small stores it publishes and calls back on intent.

import { useSyncExternalStore } from "react";
import { RuntimeTargetPanel } from "./RuntimeTargetPanel.js";
import { ShareButton } from "./ShareButton.js";
import type { ShareState } from "./ShareButton.js";
import { StatusPill } from "./StatusPill.js";
import type { PillState } from "./StatusPill.js";
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

/** The multiplayer prototype's CLIENTS control (mp-panes.ts). */
export interface ClientsState {
  count: number;
  /** Shown only for multiplayer-capable samples, or while panes are forced. */
  visible: boolean;
}

export interface SandboxControlsProps {
  picker: Store<PickerState>;
  pill: Store<PillState>;
  clients: Store<ClientsState>;
  share: Store<ShareState>;
  runtimeTarget: RuntimeTargetCore;
  onSelect: (value: string) => void;
  onReset: () => void;
  onClients: (count: number) => void;
  onShare: () => void;
}

export const SandboxControls = ({
  picker,
  pill,
  clients,
  share,
  runtimeTarget,
  onSelect,
  onReset,
  onClients,
  onShare,
}: SandboxControlsProps) => {
  const { options, selected } = useSyncExternalStore(picker.subscribe, picker.getSnapshot);
  const clientsState = useSyncExternalStore(clients.subscribe, clients.getSnapshot);

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
      {clientsState.visible && (
        <>
          <label className="picker-label" htmlFor="client-count">
            clients
          </label>
          <select
            id="client-count"
            title="Run N clients of this program side by side (multiplayer prototype)"
            value={String(clientsState.count)}
            onChange={(event) => onClients(Number(event.target.value))}
          >
            <option value="1">1</option>
            <option value="2">2</option>
            <option value="3">3</option>
          </select>
        </>
      )}
      <button id="reset" type="button" title="Reload the example (resets the model)" onClick={onReset}>
        ↺ reset
      </button>
      <ShareButton store={share} onShare={onShare} />
      <div id="runtime-target" className="runtime-target-host">
        <RuntimeTargetPanel core={runtimeTarget} />
      </div>
      <StatusPill store={pill} />
    </>
  );
};
