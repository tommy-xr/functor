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
/** Total source characters across all files (the total BYTES are capped by
 * MAX_JSON_BYTES below, so this is the "sane project" bound, not the memory one). */
const MAX_SOURCE_CHARS = 512 * 1024;
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

// Typed as BufferSource — what a (De)CompressionStream's writable end accepts.
const oneChunk = (bytes: Uint8Array<ArrayBuffer>): ReadableStream<BufferSource> =>
  new ReadableStream<BufferSource>({
    start(controller) {
      controller.enqueue(bytes);
      controller.close();
    },
  });

const deflate = async (bytes: Uint8Array<ArrayBuffer>): Promise<Uint8Array<ArrayBuffer>> => {
  const stream = oneChunk(bytes).pipeThrough(new CompressionStream("deflate-raw"));
  return new Uint8Array(await new Response(stream).arrayBuffer());
};

/**
 * Inflate `bytes`, or null if they are not a deflate-raw stream — or if they
 * expand past `limit`, which is what stops a deflate bomb: the read is
 * incremental and abandoned mid-stream, so a fragment claiming gigabytes never
 * allocates them. Constructing the stream is inside the `try` too: an engine
 * without `deflate-raw` must yield null, not throw at a URL.
 */
const inflate = async (
  bytes: Uint8Array<ArrayBuffer>,
  limit: number
): Promise<Uint8Array<ArrayBuffer> | null> => {
  const chunks: Uint8Array[] = [];
  let total = 0;
  try {
    const reader = oneChunk(bytes)
      .pipeThrough(new DecompressionStream("deflate-raw"))
      .getReader();
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
 *
 * Throws if the project cannot round-trip: the envelope is run through the
 * DECODER's validation first, so an unshareable project fails here (where a
 * caller can say so) rather than becoming a link that opens to nothing.
 */
export async function encodeShare(project: ShareProject): Promise<string> {
  const f: Record<string, string> = Object.create(null);
  for (const file of project.files) {
    // A record would silently collapse two same-named files into one.
    if (f[file.path] !== undefined) {
      throw new Error(`share-link: duplicate file ${file.path}`);
    }
    f[file.path] = file.source;
  }
  const envelope: Envelope = { v: 1, f };
  if (project.entry && project.entry !== DEFAULT_ENTRY) envelope.e = project.entry;
  if (project.config && project.config.entries) envelope.c = { entries: project.config.entries };
  if (project.options && Object.keys(project.options).length > 0) envelope.o = project.options;
  if (!validEnvelope(envelope)) throw new Error("share-link: unshareable project");

  const json = new TextEncoder().encode(JSON.stringify(envelope));
  return `#code=${toBase64Url(await deflate(json))}`;
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

  // Prototype-free: on a plain `{}`, `files["__proto__"]` and
  // `files["toString"]` are inherited truthy values, so every `!== undefined`
  // membership test below would wave through a file the project doesn't have.
  const files: Record<string, string> = Object.create(null);
  const lowered = new Set<string>();
  let chars = 0;
  for (const [path, source] of Object.entries(parsed.f)) {
    if (!MODULE_FILE.test(path)) return null;
    if (typeof source !== "string") return null;
    // The IDE's module space is case-INsensitively unique (a mac/Windows
    // checkout of the same project could not hold both), so a link carrying
    // game.fun AND Game.fun describes a project that can't exist.
    if (lowered.has(path.toLowerCase())) return null;
    lowered.add(path.toLowerCase());
    files[path] = source;
    chars += source.length;
    if (chars > MAX_SOURCE_CHARS) return null;
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
  // Something has to be bootable: either named roles, or an entry file (the
  // explicit one, or the default) that the project actually contains. A
  // roles-only project need not have a `game.fun` at all (examples/lobby).
  if (!project.config && files[project.entry ?? DEFAULT_ENTRY] === undefined) return null;
  if (parsed.o !== undefined) {
    const options = validOptions(parsed.o);
    if (!options) return null;
    if (Object.keys(options).length > 0) project.options = options;
  }
  return project;
};

/**
 * Pull `name`'s value out of a hash, with or without its leading `#`.
 *
 * A hash is a query string, not one parameter: the sandbox already writes
 * `#clients=2&src=…` (sandbox.tsx), so a share param can arrive anywhere in it.
 * base64url has no `+`, so URLSearchParams' space rewriting can't corrupt one.
 */
const fragment = (hash: string, name: string): string | null =>
  new URLSearchParams(hash.startsWith("#") ? hash.slice(1) : hash).get(name);

/**
 * Decode a share fragment back into a project, or null if it is not a valid
 * one. Accepts `#code=…` (this codec) and the legacy `#src=…` of the docs'
 * "try it" links (an uncompressed single program, loaded as `game.fun`).
 */
export async function decodeShare(hash: string): Promise<ShareProject | null> {
  // Strict UTF-8: mojibake in a program is worse than a rejected link.
  const text = new TextDecoder("utf-8", { fatal: true });

  const code = fragment(hash, "code");
  if (code !== null) {
    if (code.length === 0 || code.length > MAX_CODE_CHARS) return null;
    const deflated = fromBase64Url(code);
    if (!deflated) return null;
    const json = await inflate(deflated, MAX_JSON_BYTES);
    if (!json) return null;
    try {
      return validEnvelope(JSON.parse(text.decode(json)));
    } catch {
      return null; // inflated to something that isn't UTF-8 JSON
    }
  }

  const legacy = fragment(hash, "src");
  if (legacy !== null) {
    if (legacy.length === 0 || legacy.length > MAX_CODE_CHARS) return null;
    const bytes = fromBase64Url(legacy);
    if (!bytes || bytes.length === 0) return null;
    try {
      return { files: [{ path: DEFAULT_ENTRY, source: text.decode(bytes) }] };
    } catch {
      return null;
    }
  }
  return null;
}
