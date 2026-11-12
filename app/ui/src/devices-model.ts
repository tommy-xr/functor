// The Devices pane's pure parts (D26, D27): each state's word (the title
// pill and the card say the same), its one fix, the code as people read it,
// and which pairing request the prompt shows.

import type { Device, Devices, Headset, PairRequest } from "./generated/protocol";

/** The title pill's and the cards' word for each state. */
export const WORDS: Record<Headset, string> = {
  not_paired: "Not paired",
  asking_to_pair: "Wants to pair",
  away: "Away",
  connected: "Connected",
  needs_update: "Out of date",
  cannot_reach: "Unreachable",
};

/** A card's state, as the pill names it. */
export function deviceWord(d: Device): Headset {
  return d.state.state;
}

/** The one thing that fixes a card's state, said plainly (nothing to click
 *  unless it is built). */
export function fixFor(d: Device): string {
  switch (d.state.state) {
    case "away":
      return "Put the headset on and open Functor. It reconnects by itself.";
    case "connected":
      return "";
    case "needs_update":
      return d.state.update === "headset"
        ? "Update the Functor app on the headset, then open it again."
        : "Update Functor on this computer: the headset's app is newer.";
    case "cannot_reach":
      return "The headset is paired but not answering. Check both are on the same Wi-Fi.";
  }
}

/** `482913` → `482 913`. */
export function spacedCode(code: string): string {
  const half = Math.ceil(code.length / 2);
  return `${code.slice(0, half)} ${code.slice(half)}`;
}

/** Whole seconds until `expires_at` (wall ms), never negative. */
export function secondsLeft(r: PairRequest, now: number): number {
  return Math.max(0, Math.ceil((r.expires_at - now) / 1000));
}

/** `75` → `1:15`. */
export function clock(seconds: number): string {
  return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`;
}

/** The request the prompt shows: the oldest (requests queue one at a time). */
export function nextRequest(devices: Devices | undefined): PairRequest | null {
  return devices?.requests[0] ?? null;
}

/**
 * Whether the prompt may open by itself now. Not while the user types in a
 * text field (the composer, a search): the pill turns amber instead, and
 * the prompt opens on a click of the pill or once the field loses focus.
 */
export function mayInterrupt(active: Element | null): boolean {
  if (!active) return true;
  const tag = active.tagName;
  return !(tag === "TEXTAREA" || tag === "INPUT" || (active as HTMLElement).isContentEditable);
}

/** "12 ms", "80%, charging" and the like for a connected card's rows. */
export function details(d: Device): [string, string][] {
  if (d.state.state !== "connected") return [];
  const s = d.state;
  const link = s.link === "wifi" ? "Wi-Fi" : "Cable";
  const rows: [string, string][] = [["Link", s.latency_ms === null ? link : `${link} · ${s.latency_ms} ms`]];
  if (s.battery !== null) rows.push(["Battery", `${s.battery}%${s.charging ? ", charging" : ""}`]);
  if (s.build) rows.push(["Build", s.build]);
  rows.push(["Running", s.project ?? "Nothing"]);
  return rows;
}

/** "3 min ago" for a last-seen time (wall ms). */
export function ago(then: number | null, now: number): string {
  if (then === null) return "Never";
  const s = Math.max(0, Math.round((now - then) / 1000));
  if (s < 60) return "Just now";
  if (s < 3600) return `${Math.floor(s / 60)} min ago`;
  if (s < 86400) return `${Math.floor(s / 3600)} h ago`;
  return `${Math.floor(s / 86400)} d ago`;
}
