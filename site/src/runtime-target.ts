// The imperative DOM view of the external-runtime link, mounted by the IDE.
// All protocol and state-machine behaviour lives in `runtime-target-core.ts`;
// this module owns only the markup and the snapshot→DOM writes. (The sandbox
// renders the same core through `components/RuntimeTargetPanel.tsx`; this file
// goes away when the IDE converts too.)

import { createRuntimeTargetCore, storedEndpoint } from "./runtime-target-core.js";
import type {
  ProjectAssetInput,
  ProjectFileInput,
  RuntimeTargetCore,
  RuntimeTargetSnapshot,
} from "./runtime-target-core.js";
import type { ConsoleLevel } from "./protocol.js";

export interface RuntimeTargetOptions {
  /** Nullable because the caller passes `document.getElementById(…)`; the
   *  guard below is what turns that into the teaching error. */
  host: HTMLElement | null;
  getProject: () => ProjectFileInput[];
  getAssets?: (() => ProjectAssetInput[] | Promise<ProjectAssetInput[]>) | null;
  onOutput?: (level: ConsoleLevel, message: string) => void;
}

export function createRuntimeTarget({
  host,
  getProject,
  getAssets = null,
  onOutput = () => {},
}: RuntimeTargetOptions): RuntimeTargetCore {
  if (!host) throw new Error("runtime target host is required");

  host.classList.add("runtime-target-host");
  host.innerHTML = `
    <details class="runtime-target">
      <summary data-runtime-summary>
        <span class="runtime-target-dot" aria-hidden="true"></span>
        <span>device</span>
        <span class="runtime-target-summary-state" data-runtime-summary-state>off</span>
      </summary>
      <div class="runtime-target-panel">
        <div class="runtime-target-heading">
          <span>EXTERNAL RUNTIME</span>
          <span class="runtime-target-kind">QUEST / DESKTOP</span>
          <button data-runtime-close type="button" aria-label="Close external runtime panel">×</button>
        </div>
        <label class="runtime-target-label">
          endpoint
          <input
            data-runtime-endpoint
            type="url"
            inputmode="url"
            spellcheck="false"
            autocomplete="off"
            aria-label="Functor runtime endpoint"
          />
        </label>
        <p class="runtime-target-hint">
          Quest over USB:
          <code>adb forward tcp:8123 tcp:8123</code>
        </p>
        <div class="runtime-target-actions">
          <button data-runtime-push type="button">push + go live</button>
          <button data-runtime-capture type="button" disabled>capture</button>
        </div>
        <p class="runtime-target-status" data-runtime-status data-state="off" aria-live="polite">
          Ready to start a fresh model, then preserve it across edits.
        </p>
        <div class="runtime-target-telemetry" data-runtime-telemetry hidden>
          <div><span>FRAME</span><strong data-runtime-frame>—</strong></div>
          <div><span>TIME</span><strong data-runtime-time>—</strong></div>
          <div><span>VIEWS</span><strong data-runtime-views>—</strong></div>
        </div>
        <pre class="runtime-target-model" data-runtime-model hidden></pre>
        <figure class="runtime-target-capture" data-runtime-capture-frame hidden>
          <img data-runtime-image alt="Captured frame from the linked Functor runtime" />
          <figcaption>RAW RUNTIME CAPTURE</figcaption>
        </figure>
      </div>
    </details>
  `;

  // Every query below targets the markup this function just wrote, so each
  // element is present by construction.
  const details = host.querySelector<HTMLDetailsElement>(".runtime-target")!;
  const summary = host.querySelector<HTMLElement>("[data-runtime-summary]")!;
  const summaryState = host.querySelector<HTMLElement>("[data-runtime-summary-state]")!;
  const endpointInput = host.querySelector<HTMLInputElement>("[data-runtime-endpoint]")!;
  const pushButton = host.querySelector<HTMLButtonElement>("[data-runtime-push]")!;
  const captureButton = host.querySelector<HTMLButtonElement>("[data-runtime-capture]")!;
  const closeButton = host.querySelector<HTMLButtonElement>("[data-runtime-close]")!;
  const status = host.querySelector<HTMLElement>("[data-runtime-status]")!;
  const telemetry = host.querySelector<HTMLElement>("[data-runtime-telemetry]")!;
  const frameValue = host.querySelector<HTMLElement>("[data-runtime-frame]")!;
  const timeValue = host.querySelector<HTMLElement>("[data-runtime-time]")!;
  const viewsValue = host.querySelector<HTMLElement>("[data-runtime-views]")!;
  const modelValue = host.querySelector<HTMLElement>("[data-runtime-model]")!;
  const captureFrame = host.querySelector<HTMLElement>("[data-runtime-capture-frame]")!;
  const captureImage = host.querySelector<HTMLImageElement>("[data-runtime-image]")!;

  endpointInput.value = storedEndpoint();

  const core = createRuntimeTargetCore({ getProject, getAssets, onOutput });

  // The endpoint field is the one input the core does not own: it is written
  // by the user (and, in the e2e, assigned directly and given a bare `change`),
  // so its value is mirrored INTO the core on both events rather than rendered
  // back out of it.
  const render = (snapshot: RuntimeTargetSnapshot): void => {
    details.dataset.state = snapshot.state;
    summary.dataset.state = snapshot.state;
    summaryState.textContent = snapshot.summaryText;
    status.dataset.state = snapshot.state;
    status.textContent = snapshot.message;
    pushButton.disabled = snapshot.pushDisabled;
    captureButton.disabled = snapshot.captureDisabled;
    telemetry.hidden = snapshot.telemetry === null;
    if (snapshot.telemetry) {
      frameValue.textContent = snapshot.telemetry.frame;
      timeValue.textContent = snapshot.telemetry.time;
      viewsValue.textContent = snapshot.telemetry.views;
    }
    modelValue.textContent = snapshot.model;
    modelValue.hidden = snapshot.model.length === 0;
    captureFrame.hidden = snapshot.captureUrl === null;
    if (snapshot.captureUrl) {
      captureImage.src = snapshot.captureUrl;
    } else {
      captureImage.removeAttribute("src");
    }
  };
  core.subscribe(() => render(core.getSnapshot()));
  render(core.getSnapshot());

  endpointInput.addEventListener("input", () => core.setEndpoint(endpointInput.value));
  endpointInput.addEventListener("change", () => {
    core.setEndpoint(endpointInput.value);
    core.disconnect();
  });
  pushButton.addEventListener("click", core.push);
  captureButton.addEventListener("click", core.capture);
  closeButton.addEventListener("click", () => {
    details.open = false;
    summary.focus();
  });

  return core;
}
