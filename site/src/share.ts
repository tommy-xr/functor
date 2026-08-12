// Sharing, minus the codec: the size guard, the URL bookkeeping, the clipboard,
// the one honest warning about what a link cannot carry — and the little
// controller that drives all of it, so the sandbox and the IDE share one Share
// button behaviour instead of two copies of it.
//
// WHAT TRAVELS. A `#code=` link carries `.fun` SOURCES and nothing else, so what
// happens to a file locator on the other side depends on the locator, not on the
// link:
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
// on both ends of the link (same site, same answer) and identical for a project
// that was loaded rather than shared.
import { encodeShare, assetLocators, MAX_SHARE_CHARS } from "./share-link.js";
import type { ShareProject } from "./share-link.js";
import type { ProjectFile } from "./protocol.js";
import type { Store } from "./store.js";
import { SHARE_IDLE } from "./components/ShareButton.js";
import type { ShareState } from "./components/ShareButton.js";
import type { BannerState } from "./components/ShareBanner.js";

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

// --- the locators a link cannot carry ----------------------------------------

/** How many distinct locators a page will probe. See `unservedAssets`. */
const MAX_PROBES = 32;

/**
 * The project's relative locators that this site does not serve — the assets a
 * link genuinely cannot deliver (see the header note).
 *
 * A probe that fails to complete (offline, blocked) is NOT counted: an
 * unverifiable locator is unknown, and a warning nobody can act on is worse
 * than none. Mirrors how `functor build` treats an unverifiable URL asset.
 *
 * At most `MAX_PROBES` locators are checked, and they are checked one at a
 * time. The project may have come out of a hostile fragment, and "open this
 * link" must never become "make this browser fire a thousand requests at
 * whoever is listening".
 */
export async function unservedAssets(files: ProjectFile[]): Promise<string[]> {
  const missing: string[] = [];
  for (const locator of assetLocators(files).slice(0, MAX_PROBES)) {
    try {
      const response = await fetch(locator, { method: "HEAD" });
      if (!response.ok) missing.push(locator);
    } catch {
      /* unverifiable — say nothing */
    }
  }
  return missing;
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

// --- the control both pages mount --------------------------------------------

/** How long the button holds its confirmation before calming down. */
const FLASH_MS = 2600;

export interface ShareController {
  /** Encode the project into the page's URL, copy it, and report. */
  shareLink: () => Promise<void>;
  /**
   * Re-run the assets advisory for the project as it now stands — call it on
   * every LOAD, so the strip describes what is open rather than what used to be.
   */
  checkAssets: () => void;
}

export interface ShareControllerOptions {
  /** The button's label/tone store. */
  share: Store<ShareState>;
  /** The advisory strip's store. */
  banner: Store<BannerState>;
  /** The project to encode, built fresh at click time. */
  project: () => ShareProject;
  /** The sources to scan for locators (the same project, as plain files). */
  files: () => ProjectFile[];
  /** Where a message that outlives the flash goes (the Output panel). */
  onOutput: (level: "warn" | "error", message: string) => void;
  /** The page URL the fragment is written into. */
  href: () => string;
}

/**
 * The Share workflow, owned once: the button's little label state machine, the
 * encode/copy/replaceState sequence, and the assets probe with its generation
 * guard. Each page supplies only what its project IS.
 */
export const createShareController = ({
  share,
  banner,
  project,
  files,
  onOutput,
  href,
}: ShareControllerOptions): ShareController => {
  let flash = 0;
  const flashShare = (state: ShareState) => {
    share.set(state);
    window.clearTimeout(flash);
    flash = window.setTimeout(() => share.set(SHARE_IDLE), FLASH_MS);
  };

  // A generation counter, not a cancellation: a probe that started under the
  // previous project must not paint its verdict over this one.
  let probe = 0;
  const checkAssets = () => {
    const token = ++probe;
    banner.set({ text: "" }); // the outgoing project's advisory is not this one's
    void unservedAssets(files()).then((missing) => {
      if (token !== probe || missing.length === 0) return;
      const text = assetWarning(missing);
      banner.set({ text });
      onOutput("warn", text);
    });
  };

  const shareLink = async () => {
    let url: string;
    try {
      url = await shareHref(project(), href());
    } catch (error) {
      // Too large, or a project the codec refuses. The Output panel keeps the
      // reason after the button calms down.
      const message = error instanceof Error ? error.message : String(error);
      onOutput("error", message);
      flashShare({ label: "✖ can't share", tone: "error", detail: message });
      return;
    }
    // No navigation, and the other hash params survive — which is also what
    // makes the address bar a real fallback when the clipboard says no.
    window.history.replaceState(null, "", url);
    const copied = await copyLink(url);
    flashShare(
      copied
        ? { label: "✓ copied", tone: "ok", detail: url }
        : {
            label: "⧉ copy the URL",
            tone: "error",
            detail: "the clipboard refused — the link is in the address bar",
          }
    );
    checkAssets();
  };

  return { shareLink, checkAssets };
};
