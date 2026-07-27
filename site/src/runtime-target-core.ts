// The external-runtime link's CLIENT and CONTROLLER, split out of the DOM
// component so a React view (`components/RuntimeTargetPanel.tsx`, rendered by
// both the sandbox and the IDE) can render it. Every line of protocol and
// state-machine behaviour below is carried over unchanged from the imperative
// module it came from; only the writes that used to poke elements now publish a
// snapshot for a view to render.
//
// The in-page wasm preview stays independent: the first explicit push starts a
// fresh model on the external runtime, then later edits hot-reload there with
// the model preserved.
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
  /** v3 and later only — this client still links v2 runtimes, which omit it. */
  pending_steps?: number;
  viewport: RuntimeViewport;
  views: RuntimeView[];
  /** v4+: the structured JSON model view; pre-v4 runtimes send the Debug
   * text under this key. This panel only displays a dump, so it reads
   * `model_debug` and falls back to a pre-v4 string `model`. */
  model: unknown;
  /** v4 and later only — the Rust-`Debug` model dump. */
  model_debug?: string;
  input: unknown;
}

/** One project file as a caller may hand it over: a pair, or an IDE file. */
export type ProjectFileInput = [string, string] | ProjectFile;

/** Asset bytes as a caller may hand them over, alongside their path. */
export type ProjectAssetBytes = Uint8Array | ArrayBuffer | ArrayBufferView | Blob;
export type ProjectAssetInput =
  | [string, ProjectAssetBytes]
  | { path: string; bytes: ProjectAssetBytes };

// `declare` on the fields of both classes below: they are assigned by the
// constructor, and a plain declaration would EMIT a field definition, creating
// the properties (as undefined) before those assignments. That is observable
// on this subclass — it would reorder `name` behind them — so declare the
// types and let the constructor keep being the only thing that emits.
export class RuntimeHttpError extends Error {
  declare readonly method: string;
  declare readonly url: string;
  declare readonly status: number;
  declare readonly body: string;

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
  declare readonly endpoint: string;
  private declare readonly fetchImpl: typeof fetch;
  private declare readonly timeoutMs: number;

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
      const response = await fetchRequest(url, {
        method,
        headers: (body === undefined
          ? undefined
          : { "Content-Type": contentType ?? "application/json" }) as HeadersInit,
        // Two assertions, narrowed to the property that needs them: an
        // explicit `contentType` is only ever passed with binary bytes (the
        // asset envelope) while every other body goes out as JSON text, and
        // `exactOptionalPropertyTypes` rejects the explicit `undefined` that
        // fetch treats exactly like an absent body. `method` and `signal`
        // stay checked.
        body: (body === undefined
          ? undefined
          : contentType === null
            ? JSON.stringify(body)
            : body) as BodyInit,
        signal: controller.signal,
      });
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

const storedEndpoint = (): string => {
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

export interface RuntimeTargetCoreOptions {
  getProject: () => ProjectFileInput[];
  getAssets?: (() => ProjectAssetInput[] | Promise<ProjectAssetInput[]>) | null;
  onOutput?: (level: ConsoleLevel, message: string) => void;
}

/** The link's four display states; each is a `data-state` CSS selector. */
export type RuntimeLinkState = "off" | "busy" | "live" | "error";

/** Everything a view renders. One object, replaced on every change. */
export interface RuntimeTargetSnapshot {
  /** The raw endpoint text, as typed — normalized only at request time. */
  endpoint: string;
  /** `data-state` on the details/summary/status trio. */
  state: RuntimeLinkState;
  /** The collapsed summary's trailing word ("off", "live", "sync error", …). */
  summaryText: string;
  /** The panel's status sentence. */
  message: string;
  pushDisabled: boolean;
  captureDisabled: boolean;
  /** The `/state` telemetry row, or null while nothing has been polled. */
  telemetry: { frame: string; time: string; views: string } | null;
  /** The polled model dump; empty hides the block. */
  model: string;
  /** Object URL of the last capture; null hides the figure. */
  captureUrl: string | null;
}

/** The state seam both editor pages expose to their e2e harness. */
export interface RuntimeTargetState {
  endpoint: string;
  connected: boolean;
  live: boolean;
  fresh: boolean;
  status: string;
  message: string;
}

export interface RuntimeTargetCore {
  subscribe: (listener: () => void) => () => void;
  getSnapshot: () => RuntimeTargetSnapshot;
  /** The endpoint field changed (an `input` or a `change` event). */
  setEndpoint: (value: string) => void;
  /** The endpoint field was committed (its native `change`) — relink. */
  disconnect: () => void;
  /** "push + go live". */
  push: () => void;
  capture: () => void;
  projectChanged: (options?: { fresh?: boolean }) => void;
  restart: () => void;
  destroy: () => void;
  state: () => RuntimeTargetState;
}

// `getProject` is evaluated at push time, so queued edits always send the
// latest complete source set.
export function createRuntimeTargetCore({
  getProject,
  getAssets = null,
  onOutput = () => {},
}: RuntimeTargetCoreOptions): RuntimeTargetCore {
  const localEditorOrigin = isLoopbackHost(window.location.hostname);

  let snapshot: RuntimeTargetSnapshot = {
    endpoint: storedEndpoint(),
    // The markup's initial `data-state="off"` on the status line; no CSS rule
    // matches "off" on the summary, so this is the pre-migration paint.
    state: "off",
    summaryText: "off",
    message: "Ready to start a fresh model, then preserve it across edits.",
    pushDisabled: false,
    captureDisabled: true,
    telemetry: null,
    model: "",
    captureUrl: null,
  };
  const listeners = new Set<() => void>();
  const update = (patch: Partial<RuntimeTargetSnapshot>): void => {
    snapshot = { ...snapshot, ...patch };
    for (const listener of listeners) listener();
  };

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

  const renderStatus = (state: RuntimeLinkState, summaryText: string, message: string) =>
    update({ state, summaryText, message });

  const renderState = (state: RuntimeState) => {
    if (!state || !Number.isFinite(state.frame)) return;
    const views = Array.isArray(state.views) ? state.views.map((view) => view.name).join(" + ") : "—";
    const model =
      typeof state.model_debug === "string"
        ? state.model_debug.trim()
        : typeof state.model === "string"
          ? state.model.trim()
          : "";
    update({
      telemetry: {
        frame: String(state.frame),
        time: Number.isFinite(state.tts) ? `${state.tts.toFixed(2)}s` : "—",
        views: views || "—",
      },
      model,
    });
  };

  const currentEndpoint = () => normalizeEndpoint(snapshot.endpoint);
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
    if (captureUrl) {
      URL.revokeObjectURL(captureUrl);
      captureUrl = null;
    }
    // One notification: every field the disconnect resets, at once.
    update({
      pushDisabled: !localEditorOrigin,
      captureDisabled: true,
      telemetry: null,
      model: "",
      captureUrl: null,
      state: "off",
      summaryText: "off",
      message: "Endpoint changed. Push to start a fresh model.",
    });
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
      update({ captureDisabled: false });
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
    let endpoint = snapshot.endpoint;
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
    update({ pushDisabled: true });
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
      if (ownGeneration === generation) update({ pushDisabled: false });
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
      update({ captureDisabled: true });
      return;
    }
    const ownGeneration = generation;
    let endpoint = snapshot.endpoint;
    capturing = true;
    clearTimeout(pollTimer ?? undefined);
    update({ captureDisabled: true });
    renderStatus("busy", "capture", "Waiting for the next rendered frame…");
    try {
      const runtime = await ensureConnection();
      if (ownGeneration !== generation || !runtime) return;
      endpoint = runtime.endpoint;
      const blob = await runtime.capture();
      if (ownGeneration !== generation) return;
      if (captureUrl) URL.revokeObjectURL(captureUrl);
      captureUrl = URL.createObjectURL(blob);
      update({ captureUrl });
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
      if (ownGeneration === generation) update({ captureDisabled: !connected });
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

  if (!localEditorOrigin) {
    update({ pushDisabled: true });
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
    subscribe: (listener) => {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
    getSnapshot: () => snapshot,
    setEndpoint: (value) => update({ endpoint: value }),
    disconnect,
    push: () => queuePush({ immediate: true }),
    capture,
    projectChanged,
    restart,
    destroy,
    state: () => ({
      endpoint: snapshot.endpoint,
      connected,
      live,
      fresh: !live || freshSatisfiedRevision < freshRequiredRevision,
      status: snapshot.state,
      message: snapshot.message,
    }),
  };
}
