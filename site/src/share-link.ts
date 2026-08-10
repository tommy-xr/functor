// The share-link codec: a whole Functor project ⇄ a URL fragment.
//
// A share link carries the project itself, not a reference to one — there is no
// server, so the fragment IS the storage. `#code=<base64url>` is a
// deflate-raw'd JSON envelope:
//
//   { v: 1,
//     e?: "main.fun",                      // entry; omitted when "game.fun"
//     f: { "game.fun": "…", … },           // the flat module space
//     c?: { entries: { … } },              // functor.json subset (roles)
//     o?: { cursor?: "visible", mouseCapture?: false } }
//
// Everything a decoder sees is attacker-controlled: the fragment is whatever
// was in the URL. So `decodeShare` NEVER throws and never trusts — bad base64,
// a truncated deflate stream, a `../x.fun` path, a deflate bomb, and a future
// `v` all come back as `null` rather than as a half-applied project.
//
// The legacy `#src=<base64url>` fragment (an uncompressed single program, what
// the docs' "▶ try it" buttons emit — site/src/docs.ts) decodes through the
// same entry point, so callers have exactly one path to handle.
import type { ProjectFile } from "./protocol.js";

/**
 * A valid project file: a bare module name + `.fun` (the project is a flat
 * module space — no path separators). This is the site's one definition of the
 * rule, shared by the IDE (which enforces it on created and localStorage-loaded
 * files) and by this codec — a share link must not be able to smuggle in a
 * `../x.fun`, which would be a zip-slip entry on download and a bad module at
 * load.
 */
export const MODULE_FILE = /^[A-Za-z][A-Za-z0-9_]*\.fun$/;

/** The default entry, omitted from the payload to keep short links short. */
const DEFAULT_ENTRY = "game.fun";

// Caps. Generous next to the real projects (the largest example encodes to a
// few KB) but far below what a browser will hand back from a URL, so a hostile
// fragment is rejected before it costs anything.
const MAX_FILES = 64;
/** Total UTF-8 bytes of source across all files. */
const MAX_SOURCE_BYTES = 512 * 1024;
/** Encoded fragment length, checked before any inflation. */
const MAX_CODE_CHARS = 256 * 1024;
/** Inflated envelope bytes — the guard against a deflate bomb. */
const MAX_JSON_BYTES = 1024 * 1024;

const ROLE_NAME = /^[A-Za-z][A-Za-z0-9_-]*$/;
const IDENT = /^[A-Za-z][A-Za-z0-9_]*$/;

/**
 * One role of a multi-entry project: an entry file, or a file plus the inline
 * `module` (preferred) or binding `prefix` (transitional) that names the role's
 * contract inside it. Mirrors `functor.json`'s `entries`.
 */
export type ShareRole = string | { file: string; module?: string; prefix?: string };

/** The `functor.json` subset a share link carries. `language` is implied. */
export interface ShareConfig {
  entries?: Record<string, ShareRole>;
}

/** Player options that live in the URL rather than in the project files. */
export interface ShareOptions {
  cursor?: "visible";
  mouseCapture?: false;
}

/** A shareable project: the module space plus how to boot it. */
export interface ShareProject {
  files: ProjectFile[];
  /** Defaults to `game.fun` when absent. */
  entry?: string;
  config?: ShareConfig;
  options?: ShareOptions;
}

// --- base64url ---------------------------------------------------------------

const toBase64Url = (bytes: Uint8Array): string => {
  // Chunked: `String.fromCharCode(...bytes)` blows the argument limit somewhere
  // north of ~100KB, which real projects reach.
  let binary = "";
  for (let i = 0; i < bytes.length; i += 0x8000) {
    binary += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  }
  return btoa(binary).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
};

const BASE64URL = /^[A-Za-z0-9_-]*$/;

const fromBase64Url = (code: string): Uint8Array<ArrayBuffer> | null => {
  if (!BASE64URL.test(code)) return null; // atob is lenient about whitespace; we are not
  try {
    const binary = atob(code.replace(/-/g, "+").replace(/_/g, "/"));
    const bytes = new Uint8Array(binary.length);
    for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
    return bytes;
  } catch {
    return null;
  }
};

// --- deflate ----------------------------------------------------------------

/**
 * Push `bytes` through a (de)compression stream and read it back whole, giving
 * up if the output passes `limit`. Returns null on a stream error — which for
 * decompression means "not a deflate-raw stream", the common hostile case.
 */
const pump = async (
  bytes: Uint8Array,
  transform: GenericTransformStream,
  limit: number
): Promise<Uint8Array<ArrayBuffer> | null> => {
  const source = new ReadableStream<Uint8Array>({
    start(controller) {
      controller.enqueue(bytes);
      controller.close();
    },
  });
  const reader = source.pipeThrough(transform).getReader();
  const chunks: Uint8Array[] = [];
  let total = 0;
  try {
    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      total += value.length;
      if (total > limit) {
        await reader.cancel();
        return null;
      }
      chunks.push(value);
    }
  } catch {
    return null;
  }
  const out = new Uint8Array(total);
  let offset = 0;
  for (const chunk of chunks) {
    out.set(chunk, offset);
    offset += chunk.length;
  }
  return out;
};

// --- encode -----------------------------------------------------------------

/** The wire envelope. Field names are short because they ride in a URL. */
interface Envelope {
  v: 1;
  e?: string;
  f: Record<string, string>;
  c?: ShareConfig;
  o?: ShareOptions;
}

/**
 * Serialize a project into the URL fragment that reproduces it — the full hash
 * including the leading `#`, so a caller can assign it straight to
 * `location.hash` or append it to a page URL.
 */
export async function encodeShare(project: ShareProject): Promise<string> {
  const f: Record<string, string> = {};
  for (const file of project.files) f[file.path] = file.source;
  const envelope: Envelope = { v: 1, f };
  if (project.entry && project.entry !== DEFAULT_ENTRY) envelope.e = project.entry;
  if (project.config && project.config.entries) envelope.c = { entries: project.config.entries };
  if (project.options && Object.keys(project.options).length > 0) envelope.o = project.options;

  const json = new TextEncoder().encode(JSON.stringify(envelope));
  const deflated = await pump(json, new CompressionStream("deflate-raw"), Infinity);
  // CompressionStream cannot fail on a valid input; the null branch is the type.
  if (!deflated) throw new Error("share-link: compression failed");
  return `#code=${toBase64Url(deflated)}`;
}

// --- decode -----------------------------------------------------------------

const isRecord = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null && !Array.isArray(value);

const validRole = (role: unknown, files: Record<string, string>): ShareRole | null => {
  if (typeof role === "string") return files[role] === undefined ? null : role;
  if (!isRecord(role)) return null;
  const { file, module, prefix } = role;
  if (typeof file !== "string" || files[file] === undefined) return null;
  if (module !== undefined && prefix !== undefined) return null; // a role declares at most one
  if (module !== undefined && (typeof module !== "string" || !IDENT.test(module))) return null;
  if (prefix !== undefined && (typeof prefix !== "string" || !IDENT.test(prefix))) return null;
  const out: { file: string; module?: string; prefix?: string } = { file };
  if (typeof module === "string") out.module = module;
  if (typeof prefix === "string") out.prefix = prefix;
  return out;
};

const validConfig = (c: unknown, files: Record<string, string>): ShareConfig | null => {
  if (!isRecord(c)) return null;
  if (c.entries === undefined) return {};
  if (!isRecord(c.entries)) return null;
  const entries: Record<string, ShareRole> = {};
  for (const [name, role] of Object.entries(c.entries)) {
    if (!ROLE_NAME.test(name)) return null;
    const valid = validRole(role, files);
    if (!valid) return null;
    entries[name] = valid;
  }
  return Object.keys(entries).length > 0 ? { entries } : {};
};

const validOptions = (o: unknown): ShareOptions | null => {
  if (!isRecord(o)) return null;
  const options: ShareOptions = {};
  if (o.cursor !== undefined) {
    if (o.cursor !== "visible") return null;
    options.cursor = "visible";
  }
  if (o.mouseCapture !== undefined) {
    if (o.mouseCapture !== false) return null;
    options.mouseCapture = false;
  }
  return options;
};

/** An envelope, validated field by field. Null means "reject the whole link". */
const validEnvelope = (parsed: unknown): ShareProject | null => {
  if (!isRecord(parsed)) return null;
  if (parsed.v !== 1) return null;
  if (!isRecord(parsed.f)) return null;

  const files: Record<string, string> = {};
  let bytes = 0;
  for (const [path, source] of Object.entries(parsed.f)) {
    if (!MODULE_FILE.test(path)) return null;
    if (typeof source !== "string") return null;
    files[path] = source;
    bytes += source.length; // a char-count lower bound on UTF-8 bytes; exact for the ASCII norm
    if (bytes > MAX_SOURCE_BYTES) return null;
  }
  const paths = Object.keys(files);
  if (paths.length === 0 || paths.length > MAX_FILES) return null;

  const project: ShareProject = { files: paths.map((path) => ({ path, source: files[path] })) };

  if (parsed.e !== undefined) {
    if (typeof parsed.e !== "string" || files[parsed.e] === undefined) return null;
    project.entry = parsed.e;
  }
  if (parsed.c !== undefined) {
    const config = validConfig(parsed.c, files);
    if (!config) return null;
    if (config.entries) project.config = config;
  }
  if (parsed.o !== undefined) {
    const options = validOptions(parsed.o);
    if (!options) return null;
    if (Object.keys(options).length > 0) project.options = options;
  }
  return project;
};

/** Pull `name`'s value out of a hash, with or without its leading `#`. */
const fragment = (hash: string, name: string): string | null => {
  const body = hash.startsWith("#") ? hash.slice(1) : hash;
  const prefix = `${name}=`;
  return body.startsWith(prefix) ? body.slice(prefix.length) : null;
};

/**
 * Decode a share fragment back into a project, or null if it is not a valid
 * one. Accepts `#code=…` (this codec) and the legacy `#src=…` of the docs'
 * "try it" links (an uncompressed single program, loaded as `game.fun`).
 */
export async function decodeShare(hash: string): Promise<ShareProject | null> {
  const legacy = fragment(hash, "src");
  if (legacy !== null) {
    if (legacy.length > MAX_CODE_CHARS) return null;
    const bytes = fromBase64Url(legacy);
    if (!bytes || bytes.length === 0) return null;
    return { files: [{ path: DEFAULT_ENTRY, source: new TextDecoder().decode(bytes) }] };
  }

  const code = fragment(hash, "code");
  if (code === null || code.length === 0 || code.length > MAX_CODE_CHARS) return null;
  const deflated = fromBase64Url(code);
  if (!deflated) return null;
  const json = await pump(deflated, new DecompressionStream("deflate-raw"), MAX_JSON_BYTES);
  if (!json) return null;
  try {
    return validEnvelope(JSON.parse(new TextDecoder().decode(json)));
  } catch {
    return null; // inflated to something that isn't JSON
  }
}
