// The external-runtime link's panel, rendered from `runtime-target-core.ts`'s
// snapshot. The core is created once by the page and merely subscribed to here
// — a controller that re-created itself on a React remount would drop queued
// pushes and restart the `/state` poll chain.
//
// The markup mirrors `runtime-target.ts`'s (the IDE's imperative view) element
// for element, class for class, data-attribute for data-attribute.

import { useEffect, useRef, useState, useSyncExternalStore } from "react";
import type { RuntimeTargetCore } from "../runtime-target-core.js";

export const RuntimeTargetPanel = ({ core }: { core: RuntimeTargetCore }) => {
  const snapshot = useSyncExternalStore(core.subscribe, core.getSnapshot);
  const details = useRef<HTMLDetailsElement>(null);
  const summary = useRef<HTMLElement>(null);
  const endpoint = useRef<HTMLInputElement>(null);
  // The endpoint field stays UNCONTROLLED and is mirrored into the core by
  // native listeners: it is written by the user and, in the e2e harness,
  // assigned directly and given a bare `change` event — neither of which
  // React's synthetic `onChange` (an `input`-delegated listener behind a value
  // tracker) would see.
  const [initialEndpoint] = useState(() => core.getSnapshot().endpoint);

  useEffect(() => {
    const input = endpoint.current;
    if (!input) return;
    const onInput = () => core.setEndpoint(input.value);
    const onChange = () => {
      core.setEndpoint(input.value);
      core.disconnect();
    };
    input.addEventListener("input", onInput);
    input.addEventListener("change", onChange);
    return () => {
      input.removeEventListener("input", onInput);
      input.removeEventListener("change", onChange);
    };
  }, [core]);

  return (
    <details className="runtime-target" data-state={snapshot.state} ref={details}>
      <summary data-runtime-summary="" data-state={snapshot.state} ref={summary}>
        <span className="runtime-target-dot" aria-hidden="true"></span>
        <span>device</span>
        <span className="runtime-target-summary-state" data-runtime-summary-state="">
          {snapshot.summaryText}
        </span>
      </summary>
      <div className="runtime-target-panel">
        <div className="runtime-target-heading">
          <span>EXTERNAL RUNTIME</span>
          <span className="runtime-target-kind">QUEST / DESKTOP</span>
          <button
            data-runtime-close=""
            type="button"
            aria-label="Close external runtime panel"
            // Imperative on purpose: the `<details>` is uncontrolled (the user
            // toggles it natively), and the close must be visible in the DOM
            // the instant the click handler returns.
            onClick={() => {
              if (details.current) details.current.open = false;
              summary.current?.focus();
            }}
          >
            ×
          </button>
        </div>
        <label className="runtime-target-label">
          endpoint
          <input
            data-runtime-endpoint=""
            type="url"
            inputMode="url"
            spellCheck="false"
            autoComplete="off"
            aria-label="Functor runtime endpoint"
            defaultValue={initialEndpoint}
            ref={endpoint}
          />
        </label>
        <p className="runtime-target-hint">
          Quest over USB: <code>adb forward tcp:8123 tcp:8123</code>
        </p>
        <div className="runtime-target-actions">
          <button
            data-runtime-push=""
            type="button"
            disabled={snapshot.pushDisabled}
            onClick={core.push}
          >
            push + go live
          </button>
          <button
            data-runtime-capture=""
            type="button"
            disabled={snapshot.captureDisabled}
            onClick={core.capture}
          >
            capture
          </button>
        </div>
        <p
          className="runtime-target-status"
          data-runtime-status=""
          data-state={snapshot.state}
          aria-live="polite"
        >
          {snapshot.message}
        </p>
        <div
          className="runtime-target-telemetry"
          data-runtime-telemetry=""
          hidden={snapshot.telemetry === null}
        >
          <div>
            <span>FRAME</span>
            <strong data-runtime-frame="">{snapshot.telemetry?.frame ?? "—"}</strong>
          </div>
          <div>
            <span>TIME</span>
            <strong data-runtime-time="">{snapshot.telemetry?.time ?? "—"}</strong>
          </div>
          <div>
            <span>VIEWS</span>
            <strong data-runtime-views="">{snapshot.telemetry?.views ?? "—"}</strong>
          </div>
        </div>
        <pre className="runtime-target-model" data-runtime-model="" hidden={snapshot.model === ""}>
          {snapshot.model}
        </pre>
        <figure
          className="runtime-target-capture"
          data-runtime-capture-frame=""
          hidden={snapshot.captureUrl === null}
        >
          <img
            data-runtime-image=""
            alt="Captured frame from the linked Functor runtime"
            src={snapshot.captureUrl ?? undefined}
          />
          <figcaption>RAW RUNTIME CAPTURE</figcaption>
        </figure>
      </div>
    </details>
  );
};
