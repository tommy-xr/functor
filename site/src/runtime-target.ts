// A browser-native client + compact UI for linking either editor surface to a
// running Functor debug runtime. The in-page wasm preview stays independent:
// the first explicit push starts a fresh model on the external runtime, then
// later edits hot-reload there with the model preserved.
//
// The wire types below mirror the runtime's HTTP debug protocol
// (runtime/functor-runtime-common/src/debug_protocol.rs). As with the
// postMessage types in protocol.ts, response BODIES are asserted rather than
// validated: the pre-migration code read these fields straight off
// `response.json()`, and the few defensive runtime checks it did make (the
// discovery handshake, `Number.isFinite(state.frame)`) are kept as they were.

import type { ConsoleLevel, ProjectFile } from "./protocol.js";

export const DEFAULT_RUNTIME_ENDPOINT = "http://127.0.0.1:8123";

const SERVICE = "functor debug runtime";
const MIN_PROTOCOL_VERSION = 2;
const REQUEST_TIMEOUT_MS = 10_000;
const ASSET_TIMEOUT_MS = 40_000;
// Quest's raw 3360×1760 stereo PNG can take >10s to encode and transfer. The
// runtime itself waits up to 30s for a captured frame; leave transfer margin.
const CAPTURE_TIMEOUT_MS = 40_000;
const LIVE_PUSH_DEBOUNCE_MS = 300;
const STATE_POLL_MS = 1_000;
const STORAGE_KEY = "functor-runtime-endpoint-v1";

/** The `GET /` handshake body every runtime target serves. */
export interface RuntimeDiscovery {
  service: string;
  protocol_version: number;
  endpoints: Record<string, string>;
}

/** A pixel rectangle in the runtime's output surface. */
export interface RuntimeViewport {
  width: number;
  height: number;
}

/** One rendered view: desktop reports a single `main`, stereo XR one per eye. */
export interface RuntimeView {
  name: string;
  viewport: RuntimeViewport;
}

/**
 * The `GET /state` snapshot. `input` (held keys, mouse, optional XR) is a
 * runtime-owned payload this client never reads, so it stays `unknown` here.
 */
export interface RuntimeState {
  frame: number;
  tts: number;
  pending_steps: number;
  viewport: RuntimeViewport;
  views: RuntimeView[];
  model: string;
  input: unknown;
}

/** One project file as a caller may hand it over: a pair, or an IDE file. */
export type ProjectFileInput = [string, string] | ProjectFile;

/** Asset bytes as a caller may hand them over, alongside their path. */
export type ProjectAssetBytes = Uint8Array | ArrayBuffer | ArrayBufferView | Blob;
export type ProjectAssetInput =
  | [string, ProjectAssetBytes]
  | { path: string; bytes: ProjectAssetBytes };

export class RuntimeHttpError extends Error {
  readonly method: string;
  readonly url: string;
  readonly status: number;
  readonly body: string;

  constructor(method: string, url: string, status: number, body: string) {
    super(`${method} ${url} failed with status ${status}: ${body}`);
    this.name = "RuntimeHttpError";
    this.method = method;
    this.url = url;
    this.status = status;
    this.body = body;
  }
}

// The TypeScript SDK is Node-oriented (`capture()` returns a Buffer). This
// fetch client intentionally keeps the browser response as a Blob while using
// the same canonical protocol routes and JSON bodies.
export class BrowserRuntimeClient {
  readonly endpoint: string;
  private readonly fetchImpl: typeof fetch;
  private readonly timeoutMs: number;

  constructor(
    endpoint: unknown,
    {
      fetchImpl = fetch,
      timeoutMs = REQUEST_TIMEOUT_MS,
    }: { fetchImpl?: typeof fetch; timeoutMs?: number } = {}
  ) {
    this.endpoint = normalizeEndpoint(endpoint);
    this.fetchImpl = fetchImpl;
    this.timeoutMs = timeoutMs;
  }

  async discover(): Promise<RuntimeDiscovery> {
    const discovery = await this.#json<RuntimeDiscovery>("GET", "/");
    if (
      discovery?.service !== SERVICE ||
      !Number.isInteger(discovery?.protocol_version) ||
      discovery.protocol_version < MIN_PROTOCOL_VERSION
    ) {
      throw new Error(
        `not a compatible Functor runtime (expected ${SERVICE} protocol ${MIN_PROTOCOL_VERSION}+)`
      );
    }
    return discovery;
  }

  state(): Promise<RuntimeState> {
    return this.#json<RuntimeState>("GET", "/state");
  }

  loadProject(files: [string, string][]): Promise<string> {
    return this.#text("POST", "/load-project", files);
  }

  reloadProject(files: [string, string][]): Promise<string> {
    return this.#text("POST", "/reload-project", files);
  }

  reloadAsset(path: string, bytes: Uint8Array): Promise<string> {
    const pathBytes = new TextEncoder().encode(path);
    if (pathBytes.byteLength > 0xffff_ffff) throw new Error("asset path is too long");
    const body = new Uint8Array(4 + pathBytes.byteLength + bytes.byteLength);
    new DataView(body.buffer).setUint32(0, pathBytes.byteLength, false);
    body.set(pathBytes, 4);
    body.set(bytes, 4 + pathBytes.byteLength);
    return this.#request(
      "POST",
      "/reload-asset",
      body,
      ASSET_TIMEOUT_MS,
      "application/octet-stream"
    ).then((response) => response.text());
  }

  syncAssets(paths: string[]): Promise<string> {
    return this.#text("POST", "/sync-assets", paths);
  }

  capture(): Promise<Blob> {
    return this.#request("POST", "/capture", undefined, CAPTURE_TIMEOUT_MS).then((response) =>
      response.blob()
    );
  }

  async #json<T>(method: string, path: string, body?: unknown): Promise<T> {
    return (await this.#request(method, path, body)).json() as Promise<T>;
  }

  async #text(method: string, path: string, body?: unknown): Promise<string> {
    return (await this.#request(method, path, body)).text();
  }

  async #request(
    method: string,
    path: string,
    body?: unknown,
    timeoutMs: number = this.timeoutMs,
    contentType: string | null = null
  ): Promise<Response> {
    const controller = new AbortController();
    const timeout = setTimeout(() => controller.abort(), timeoutMs);
    const url = `${this.endpoint}${path}`;
    try {
      // Invoke an injected/global fetch as a plain function. Calling it as
      // `this.fetchImpl(...)` supplies the client as the Web API receiver,
      // which some browsers reject as an illegal invocation before networking.
      const fetchRequest = this.fetchImpl;
      // `exactOptionalPropertyTypes` rejects an explicit `undefined` body even
      // though fetch treats it exactly like an absent one, so the init is
      // asserted rather than restructured — the request is unchanged.
      const response = await fetchRequest(url, {
        method,
        headers:
          body === undefined
            ? undefined
            : { "Content-Type": contentType ?? "application/json" },
        // An explicit `contentType` is only ever passed with binary bytes (the
        // asset envelope); every other body goes out as JSON text. The types
        // cannot express that correlation, hence the assertion.
        body:
          body === undefined
            ? undefined
            : contentType === null
              ? JSON.stringify(body)
              : (body as BodyInit),
        signal: controller.signal,
      } as RequestInit);
      if (!response.ok) {
        throw new RuntimeHttpError(method, url, response.status, await response.text());
      }
      return response;
    } finally {
      clearTimeout(timeout);
    }
  }
}

export function normalizeEndpoint(raw: unknown): string {
  let value = String(raw ?? "").trim();
  if (!/^[a-z][a-z0-9+.-]*:\/\//i.test(value)) value = `http://${value}`;
  const url = new URL(value);
  if (url.protocol !== "http:" && url.protocol !== "https:") {
    throw new Error("runtime endpoint must use http:// or https://");
  }
  if (url.username || url.password) throw new Error("runtime endpoint cannot include credentials");
  url.search = "";
  url.hash = "";
  url.pathname = url.pathname.replace(/\/+$/, "");
  return url.toString().replace(/\/$/, "");
}

const isLoopbackHost = (host: string) =>
  host === "localhost" || host === "127.0.0.1" || host === "::1" || host === "[::1]";

const storedEndpoint = () => {
  try {
    return localStorage.getItem(STORAGE_KEY) || DEFAULT_RUNTIME_ENDPOINT;
  } catch {
    return DEFAULT_RUNTIME_ENDPOINT;
  }
};

const rememberEndpoint = (endpoint: string) => {
  try {
    localStorage.setItem(STORAGE_KEY, endpoint);
  } catch {
    /* endpoint persistence is a convenience */
  }
};

// The producers are still JavaScript, so the shape checks in both normalizers
// below stay exactly as they were: the input types are the contract, not a
// guarantee, and the assertions keep the checks meaningful to the checker.
const projectPairs = (files: ProjectFileInput[]): [string, string][] => {
  if (!Array.isArray(files) || files.length === 0) {
    throw new Error("the editor has no project files to push");
  }
  return files.map((file, index) => {
    const pair = (Array.isArray(file) ? file : [file?.path, file?.source]) as [string, string];
    if (
      pair.length !== 2 ||
      typeof pair[0] !== "string" ||
      typeof pair[1] !== "string" ||
      pair[0].length === 0
    ) {
      throw new Error(`project file ${index + 1} is not a [path, source] pair`);
    }
    return pair;
  });
};

const projectAssets = async (
  files: ProjectAssetInput[] | null
): Promise<[string, Uint8Array][]> => {
  if (!Array.isArray(files)) throw new Error("project assets must be an array");
  return Promise.all(
    files.map(async (file, index): Promise<[string, Uint8Array]> => {
      const pair = (Array.isArray(file) ? file : [file?.path, file?.bytes]) as [
        string,
        ProjectAssetBytes,
      ];
      if (pair.length !== 2 || typeof pair[0] !== "string" || pair[0].length === 0) {
        throw new Error(`project asset ${index + 1} is not a [path, bytes] pair`);
      }
      let bytes: Uint8Array;
      if (pair[1] instanceof Uint8Array) {
        bytes = pair[1];
      } else if (pair[1] instanceof ArrayBuffer) {
        bytes = new Uint8Array(pair[1]);
      } else if (ArrayBuffer.isView(pair[1])) {
        bytes = new Uint8Array(pair[1].buffer, pair[1].byteOffset, pair[1].byteLength);
      } else if (typeof Blob !== "undefined" && pair[1] instanceof Blob) {
        bytes = new Uint8Array(await pair[1].arrayBuffer());
      } else {
        throw new Error(`project asset ${index + 1} has unsupported bytes`);
      }
      return [pair[0], bytes];
    })
  );
};

const errorMessage = (error: unknown, endpoint: string, action: string): string => {
  if (error instanceof RuntimeHttpError) {
    const detail = error.body.trim();
    if (error.status === 503 && action === "capture") {
      return "capture unavailable — wake the headset and wait for XR rendering";
    }
    return detail || `${action} failed with HTTP ${error.status}`;
  }
  // Read `name` off whatever was thrown, exactly as before: an aborted fetch
  // rejects with a DOMException, but an injected fetch may throw anything.
  if ((error as { name?: unknown } | null | undefined)?.name === "AbortError") {
    return `${action} timed out at ${endpoint}`;
  }
  if (error instanceof TypeError) {
    const detail = error.message && error.message !== "Failed to fetch" ? ` (${error.message})` : "";
    return `cannot reach ${endpoint}${detail} — check the runtime, adb port forward, and browser local-network permission`;
  }
  return error instanceof Error ? error.message : String(error);
};

export interface RuntimeTargetOptions {
  host: HTMLElement;
  getProject: () => ProjectFileInput[];
  getAssets?: (() => ProjectAssetInput[] | Promise<ProjectAssetInput[]>) | null;
  onOutput?: (level: ConsoleLevel, message: string) => void;
}

// Mount the same device-link control in the multi-file IDE and the single-file
// sandbox. `getProject` is evaluated at push time, so queued edits always send
// the latest complete source set.
export function createRuntimeTarget({
  host,
  getProject,
  getAssets = null,
  onOutput = () => {},
}: RuntimeTargetOptions) {
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
  const localEditorOrigin = isLoopbackHost(window.location.hostname);

  let client: BrowserRuntimeClient | null = null;
  let connected = false;
  let live = false;
  let projectRevision = 0;
  let syncedRevision = -1;
  let freshRequiredRevision = 0;
  let freshSatisfiedRevision = -1;
  // `clearTimeout` does not take null, so every clear below passes
  // `?? undefined`; the stored handles are unchanged by the migration.
  let pushTimer: ReturnType<typeof setTimeout> | null = null;
  let pollTimer: ReturnType<typeof setTimeout> | null = null;
  let pollingGeneration: number | null = null;
  let pushing = false;
  let pushQueued = false;
  let capturing = false;
  let captureQueued = false;
  let generation = 0;
  let captureUrl: string | null = null;
  let syncError: string | null = null;
  let transportError: string | null = null;
  let lastSyncedAssetSource: ProjectAssetInput[] | null = null;
  let lastSyncedAssets: [string, Uint8Array][] = [];
  let assetSyncPending = false;

  const renderStatus = (state: string, summaryText: string, message: string) => {
    details.dataset.state = state;
    summary.dataset.state = state;
    summaryState.textContent = summaryText;
    status.dataset.state = state;
    status.textContent = message;
  };

  const renderState = (state: RuntimeState) => {
    if (!state || !Number.isFinite(state.frame)) return;
    const views = Array.isArray(state.views) ? state.views.map((view) => view.name).join(" + ") : "—";
    frameValue.textContent = String(state.frame);
    timeValue.textContent = Number.isFinite(state.tts) ? `${state.tts.toFixed(2)}s` : "—";
    viewsValue.textContent = views || "—";
    telemetry.hidden = false;
    const model = typeof state.model === "string" ? state.model.trim() : "";
    modelValue.textContent = model;
    modelValue.hidden = model.length === 0;
  };

  const currentEndpoint = () => normalizeEndpoint(endpointInput.value);
  const projectIsDirty = () =>
    syncedRevision < projectRevision ||
    freshSatisfiedRevision < freshRequiredRevision ||
    assetSyncPending;

  const disconnect = () => {
    generation += 1;
    clearTimeout(pushTimer ?? undefined);
    clearTimeout(pollTimer ?? undefined);
    client = null;
    connected = false;
    live = false;
    syncedRevision = -1;
    freshRequiredRevision = projectRevision;
    freshSatisfiedRevision = -1;
    pushQueued = false;
    captureQueued = false;
    syncError = null;
    transportError = null;
    lastSyncedAssetSource = null;
    lastSyncedAssets = [];
    assetSyncPending = false;
    pushButton.disabled = !localEditorOrigin;
    captureButton.disabled = true;
    telemetry.hidden = true;
    modelValue.hidden = true;
    captureFrame.hidden = true;
    if (captureUrl) {
      URL.revokeObjectURL(captureUrl);
      captureUrl = null;
      captureImage.removeAttribute("src");
    }
    renderStatus("off", "off", "Endpoint changed. Push to start a fresh model.");
  };

  const pollState = async () => {
    clearTimeout(pollTimer ?? undefined);
    pollTimer = null;
    if (!connected || !client || capturing || pushing) return;
    const ownGeneration = generation;
    // Pushes and captures request an immediate refresh, but they must not start
    // independent polling chains when a state request is already in flight.
    if (pollingGeneration === ownGeneration) return;
    pollingGeneration = ownGeneration;
    try {
      const state = await client.state();
      if (ownGeneration !== generation) return;
      if (capturing || pushing) return;
      renderState(state);
      if (transportError) {
        transportError = null;
        if (syncError) {
          renderStatus("error", "sync error", syncError);
        } else if (projectIsDirty()) {
          renderStatus("busy", "syncing", "Connection restored. Retrying the latest project…");
          queuePush({ immediate: true });
        } else {
          renderStatus("live", "live", "Connection restored. Project is synchronized.");
        }
      }
    } catch (error) {
      if (ownGeneration !== generation) return;
      if (capturing || pushing) return;
      // `client` was non-null on entry; only `disconnect()` clears it, and that
      // bumps the generation the guard above already returned on.
      transportError = errorMessage(error, client!.endpoint, "state poll");
      renderStatus(
        "error",
        syncError ? "sync error" : "retrying",
        syncError ? `${syncError} Runtime unreachable: ${transportError}` : transportError
      );
    } finally {
      if (pollingGeneration === ownGeneration) pollingGeneration = null;
      if (ownGeneration === generation && connected && !capturing && !pushing) {
        clearTimeout(pollTimer ?? undefined);
        pollTimer = setTimeout(pollState, STATE_POLL_MS);
      }
    }
  };

  const ensureConnection = async (): Promise<BrowserRuntimeClient | null> => {
    const endpoint = currentEndpoint();
    if (!client || client.endpoint !== endpoint) {
      client = new BrowserRuntimeClient(endpoint);
      connected = false;
    }
    const runtime = client;
    if (!connected) {
      await runtime.discover();
      // Endpoint changes disconnect the old client while discovery is in
      // flight. Do not let its late response mark the replacement as linked.
      if (client !== runtime) return null;
      connected = true;
      rememberEndpoint(endpoint);
      captureButton.disabled = false;
    }
    return runtime;
  };

  const pushOnce = async () => {
    const ownGeneration = generation;
    // Snapshot the authored project and its fresh-model requirement before the
    // first await. A reset that arrives during discovery or asset transfer gets
    // a later revision and therefore remains fresh in the queued push.
    const requestRevision = projectRevision;
    const files = projectPairs(getProject());
    const rawAssetsSnapshot = getAssets ? getAssets() : null;
    const fresh = !live || freshSatisfiedRevision < freshRequiredRevision;
    const sourceNeedsSync = syncedRevision < requestRevision || fresh;
    let endpoint = endpointInput.value;
    let assetCount = 0;
    let resolvedAssetsSource: ProjectAssetInput[] | null = null;
    let assets: [string, Uint8Array][] = [];
    let sourceResult: string | null = null;
    let runtime: BrowserRuntimeClient | null = null;
    let assetsMutated = false;
    let sourceAccepted = false;
    const priorAssetsKnown = lastSyncedAssetSource !== null;
    const priorAssets = lastSyncedAssets;
    syncError = null;
    renderStatus("busy", "syncing", connected ? "Sending the latest project…" : "Finding the runtime…");
    pushButton.disabled = true;
    try {
      runtime = await ensureConnection();
      if (ownGeneration !== generation || !runtime) return;
      endpoint = runtime.endpoint;
      if (getAssets) {
        resolvedAssetsSource = await rawAssetsSnapshot;
        assets = await projectAssets(resolvedAssetsSource);
        assetCount = assets.length;
        if (resolvedAssetsSource !== lastSyncedAssetSource) {
          assetSyncPending = true;
          for (const [path, bytes] of assets) {
            await runtime.reloadAsset(path, bytes);
            assetsMutated = true;
          }
          if (ownGeneration !== generation) return;
        }
      }
      if (sourceNeedsSync) {
        sourceResult = fresh
          ? await runtime.loadProject(files)
          : await runtime.reloadProject(files);
        if (ownGeneration !== generation) return;
        live = true;
        syncedRevision = Math.max(syncedRevision, requestRevision);
        if (fresh) {
          freshSatisfiedRevision = Math.max(freshSatisfiedRevision, requestRevision);
        }
        sourceAccepted = true;
      }
      // Finalize deletions only after the new source is accepted. If source
      // loading fails, the last-good program keeps every asset it still needs.
      if (getAssets && resolvedAssetsSource !== lastSyncedAssetSource) {
        await runtime.syncAssets(assets.map(([path]) => path));
        if (ownGeneration !== generation) return;
        lastSyncedAssetSource = resolvedAssetsSource;
        lastSyncedAssets = assets;
        assetSyncPending = false;
      }
      if (ownGeneration !== generation) return;
      syncError = null;
      transportError = null;
      renderStatus(
        "live",
        "live",
        fresh && sourceResult
          ? `Fresh model loaded from ${files.length} source file${files.length === 1 ? "" : "s"}${
              getAssets ? ` + ${assetCount} asset${assetCount === 1 ? "" : "s"}` : ""
            }.`
          : sourceResult ?? `Synchronized ${assetCount} project asset${assetCount === 1 ? "" : "s"}.`
      );
    } catch (error) {
      if (ownGeneration !== generation) return;
      let message = errorMessage(error, endpoint, "push");
      // Asset reloads are eager. If the runtime definitively rejects the new
      // source, restore the last browser-synchronized bytes + manifest so the
      // last-good program cannot keep an overwritten or partially uploaded set.
      if (
        error instanceof RuntimeHttpError &&
        (error.status === 400 || error.status === 413) &&
        runtime &&
        assetsMutated &&
        !sourceAccepted &&
        priorAssetsKnown
      ) {
        try {
          for (const [path, bytes] of priorAssets) await runtime.reloadAsset(path, bytes);
          await runtime.syncAssets(priorAssets.map(([path]) => path));
          message = `${message} Last-good assets were restored.`;
        } catch (rollbackError) {
          message = `${message} Asset rollback also failed: ${errorMessage(
            rollbackError,
            endpoint,
            "asset rollback"
          )}`;
        }
      }
      if (ownGeneration !== generation) return;
      if (error instanceof RuntimeHttpError) {
        syncError = message;
        transportError = null;
        renderStatus("error", "sync error", message);
      } else {
        transportError = message;
        renderStatus("error", connected ? "retrying" : "offline", message);
      }
      onOutput("error", `[device] ${message}`);
    } finally {
      if (ownGeneration === generation) pushButton.disabled = false;
    }
  };

  const flushPush = async () => {
    if (capturing) {
      pushQueued = true;
      return;
    }
    if (pushing) {
      pushQueued = true;
      return;
    }
    pushing = true;
    clearTimeout(pollTimer ?? undefined);
    do {
      pushQueued = false;
      await pushOnce();
    } while (pushQueued);
    pushing = false;
    if (captureQueued) {
      captureQueued = false;
      capture();
    } else if (connected) {
      pollState();
    }
  };

  const queuePush = ({ immediate = false }: { immediate?: boolean } = {}) => {
    clearTimeout(pushTimer ?? undefined);
    // The initial load snapshots source before its first await. Edits that land
    // while that load is in flight must queue even though `live` is still false.
    if (pushing) {
      pushQueued = true;
      return;
    }
    if (!live && !immediate) return;
    if (capturing) {
      pushQueued = true;
      return;
    }
    if (immediate) {
      flushPush();
    } else {
      pushTimer = setTimeout(flushPush, LIVE_PUSH_DEBOUNCE_MS);
    }
  };

  const projectChanged = ({ fresh = false }: { fresh?: boolean } = {}) => {
    projectRevision += 1;
    if (fresh) freshRequiredRevision = projectRevision;
    syncError = null;
    queuePush();
  };

  const capture = async () => {
    if (capturing) return;
    if (pushing) {
      captureQueued = true;
      captureButton.disabled = true;
      return;
    }
    const ownGeneration = generation;
    let endpoint = endpointInput.value;
    capturing = true;
    clearTimeout(pollTimer ?? undefined);
    captureButton.disabled = true;
    renderStatus("busy", "capture", "Waiting for the next rendered frame…");
    try {
      const runtime = await ensureConnection();
      if (ownGeneration !== generation || !runtime) return;
      endpoint = runtime.endpoint;
      const blob = await runtime.capture();
      if (ownGeneration !== generation) return;
      if (captureUrl) URL.revokeObjectURL(captureUrl);
      captureUrl = URL.createObjectURL(blob);
      captureImage.src = captureUrl;
      captureFrame.hidden = false;
      transportError = null;
      if (syncError) {
        renderStatus(
          "error",
          "sync error",
          "Captured the last-good framebuffer; the latest editor source is not synchronized."
        );
      } else if (projectIsDirty()) {
        renderStatus(
          "busy",
          "syncing",
          "Captured the last-good framebuffer. Retrying the latest project…"
        );
        queuePush({ immediate: true });
      } else {
        renderStatus("live", live ? "live" : "linked", "Captured the runtime framebuffer.");
      }
    } catch (error) {
      if (ownGeneration !== generation) return;
      const message = errorMessage(error, endpoint, "capture");
      if (!(error instanceof RuntimeHttpError)) transportError = message;
      renderStatus("error", live ? "capture error" : "offline", message);
      onOutput("error", `[device] ${message}`);
    } finally {
      capturing = false;
      if (ownGeneration === generation) captureButton.disabled = !connected;
      if (pushQueued) flushPush();
      if (ownGeneration === generation && connected) pollState();
    }
  };

  const restart = () => {
    if (!live) return;
    projectRevision += 1;
    freshRequiredRevision = projectRevision;
    syncError = null;
    queuePush({ immediate: true });
  };

  endpointInput.addEventListener("change", disconnect);
  pushButton.addEventListener("click", () => queuePush({ immediate: true }));
  captureButton.addEventListener("click", capture);
  closeButton.addEventListener("click", () => {
    details.open = false;
    summary.focus();
  });

  if (!localEditorOrigin) {
    pushButton.disabled = true;
    renderStatus(
      "off",
      "local only",
      "For safety, Functor runtimes accept editor links only from a localhost-served IDE."
    );
  }

  const destroy = () => {
    disconnect();
    if (captureUrl) URL.revokeObjectURL(captureUrl);
  };
  window.addEventListener("pagehide", destroy, { once: true });

  return {
    projectChanged,
    restart,
    capture,
    destroy,
    state: () => ({
      endpoint: endpointInput.value,
      connected,
      live,
      fresh: !live || freshSatisfiedRevision < freshRequiredRevision,
      status: status.dataset.state,
      message: status.textContent,
    }),
  };
}
