// Sharing, minus the codec: the size guard, the URL bookkeeping, the clipboard,
// and the one honest warning about what a link cannot carry. The sandbox and the
// IDE both drive this, so the two Share buttons behave identically.
//
// WHAT TRAVELS. A `#code=` link carries `.fun` SOURCES and nothing else, so what
// happens to an `Asset.*` locator on the other side depends on the locator, not
// on the link:
//
//   • a URL locator (a CDN asset) always resolves — there is nothing to carry;
//   • a RELATIVE locator resolves against the SITE ROOT, because the runtime in
//     a `player.html?project=inline` iframe fetches it from there (see the
//     header note in sandbox.tsx). Every asset the built site ships sits at that
//     root — `grid-neon.png`, `ship.glb`, `heightmap.png` — so a shared example
//     keeps its assets whether or not its sources were edited;
//   • a relative locator naming a file the site does NOT serve is the one case
//     that breaks: a path someone typed into the IDE, or an example asset
//     renamed in the editor. The bytes exist nowhere the link can reach.
//
// So the warning is NOT "this project has local assets" — that would cry wolf on
// nearly every example, which shares perfectly. It is "this locator does not
// resolve here", probed against the site itself, which makes it identically true
// on both ends of the link (same site, same answer).
import { encodeShare, assetLocators } from "./share-link.js";
import type { ShareProject } from "./share-link.js";
import type { ProjectFile } from "./protocol.js";

/**
 * The longest `#code=` payload we will hand out, in characters. Chat clients,
 * issue trackers and mail clients all mangle links well before a browser's own
 * URL limit, and every example in the repo encodes far under this (the codec's
 * own test pins that). A project over the cap gets the zip download instead.
 */
export const MAX_SHARE_CHARS = 24_000;

/**
 * The share URL for `project`: `href` with a `#code=` fragment, every OTHER
 * hash param preserved (the sandbox's `#clients=N` rides along). A stale
 * `#src=` is dropped — it would be dead weight next to a fragment that wins
 * over it anyway.
 *
 * Throws when the project cannot ride in a URL (too large, or not encodable);
 * the message is meant to be shown as-is.
 */
export async function shareHref(project: ShareProject, href: string): Promise<string> {
  const code = (await encodeShare(project)).slice("#code=".length);
  if (code.length > MAX_SHARE_CHARS) {
    throw new Error(
      `this project encodes to ${Math.ceil(code.length / 1000)}k — over the ` +
        `${MAX_SHARE_CHARS / 1000}k a link can carry. Download it as a .zip and share that.`
    );
  }
  const url = new URL(href);
  const params = new URLSearchParams(url.hash.slice(1));
  params.delete("src");
  params.set("code", code);
  url.hash = params.toString();
  return url.toString();
}

/**
 * Put `url` on the clipboard, reporting whether it landed. A denied or absent
 * clipboard is not an error worth failing the share over: the fragment is
 * already in the address bar by then, which is the fallback the caller offers.
 */
export const copyLink = async (url: string): Promise<boolean> => {
  try {
    await navigator.clipboard.writeText(url);
    return true;
  } catch {
    return false;
  }
};

/**
 * The project's relative `Asset.*` locators that this site does not serve —
 * the assets a link genuinely cannot deliver (see the header note).
 *
 * A probe that fails to complete (offline, blocked) is NOT counted: an
 * unverifiable locator is unknown, and a warning nobody can act on is worse
 * than none. Mirrors how `functor build` treats an unverifiable URL asset.
 */
export async function unservedAssets(files: ProjectFile[]): Promise<string[]> {
  const missing: string[] = [];
  await Promise.all(
    assetLocators(files).map(async (locator) => {
      try {
        const response = await fetch(locator, { method: "HEAD" });
        if (!response.ok) missing.push(locator);
      } catch {
        /* unverifiable — say nothing */
      }
    })
  );
  return missing.sort();
}

/** The banner line for `missing` (never empty — callers check first). */
export const assetWarning = (missing: string[]): string => {
  const named = missing.slice(0, 3).join(", ");
  const rest = missing.length > 3 ? `, +${missing.length - 3} more` : "";
  return (
    `${missing.length} local asset${missing.length === 1 ? "" : "s"} won't travel with ` +
    `this link (${named}${rest}) — host ${missing.length === 1 ? "it" : "them"} on a URL instead.`
  );
};
