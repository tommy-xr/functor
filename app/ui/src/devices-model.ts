// The Devices pane's pure parts (D26, D27): each state's word (the title
// pill and the card say the same), its one fix, the code as people read it,
// and when the pairing prompt may open.

import type { Device, Headset, PairRequest } from "./generated/protocol";

/** The title pill's and the cards' word for each state. */
export const WORDS: Record<Headset, string> = {
  not_paired: "Not paired",
  asking_to_pair: "Wants to pair",
  away: "Away",
  connected: "Connected",
  needs_update: "Out of date",
  cannot_reach: "Unreachable",
};

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

/** Typing this recently in a field holds the prompt back. */
export const TYPING_IDLE_MS = 3000;

/**
 * Whether the prompt may open by itself now: not within `TYPING_IDLE_MS`
 * of a key typed into a field (the composer, a search), and not while a
 * pointer is down (it would swallow that click).
 */
export function mayOpen(typedAt: number, now: number, pointerDown: boolean): boolean {
  return !pointerDown && now - typedAt >= TYPING_IDLE_MS;
}

/** The request the prompt opens by itself: the oldest not put off with "Later". */
export function autoOpen(requests: PairRequest[], later: number[]): PairRequest | null {
  return requests.find((r) => !later.includes(r.request)) ?? null;
}

/** A connected card's rows: link and latency, battery, build, project. */
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
