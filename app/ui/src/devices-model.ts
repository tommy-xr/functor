// The Devices pane's pure parts (D26, D27): each state's word (the title
// pill and the card say the same), its one fix, the code as people read it,
// and when the pairing prompt may open.

import type { Attached, Device, Devices, Headset, HeadsetPairing, PairRequest } from "./generated/protocol";

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
 *  unless it is built). `cable`: the headset on a USB cable, if it is. */
export function fixFor(d: Device, cable: Attached | null = null): string {
  switch (d.state.state) {
    case "away":
      return cable && pairedWith(cable)?.with === "this"
        ? "Paired over the cable. Open Functor in the headset: it connects over Wi-Fi by itself."
        : "Put the headset on and open Functor. It reconnects by itself.";
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

/** The pairing a headset on the cable holds, once its app is readable. */
export function pairedWith(a: Attached): HeadsetPairing | null {
  return a.state.state === "ready" ? a.state.paired : null;
}

/** The headset on a cable that is this paired card's (by its id). */
export function cableFor(d: Device, attached: Attached[]): Attached | null {
  return attached.find((a) => a.state.state === "ready" && a.state.device === d.id) ?? null;
}

/** Headsets on a cable with no card of their own. */
export function cableOnly(devices: Devices): Attached[] {
  return devices.attached.filter((a) => !devices.list.some((d) => cableFor(d, [a])));
}

/** A paired card's cable row: how the headset on it is. */
export const CABLE_ROW: Record<Attached["state"]["state"], string> = {
  unauthorized: "Plugged in, USB debugging not allowed",
  offline: "Plugged in, not responding",
  no_app: "Plugged in, no Functor app",
  not_debuggable: "Plugged in, release app",
  ready: "Plugged in",
};

/**
 * A headset on the cable, said plainly: what it is, and the one thing that
 * makes it pairable (`fix`), or nothing when "Pair over the cable" is it.
 */
export function cableWords(a: Attached): { line: string; fix: string } {
  switch (a.state.state) {
    case "unauthorized":
      return {
        line: "It has not allowed USB debugging from this computer.",
        fix: "Put the headset on and choose Allow on its USB debugging prompt.",
      };
    case "offline":
      return { line: "adb lists it but cannot reach it.", fix: "Unplug the cable and plug it in again." };
    case "no_app":
      return {
        line: "The Functor app is not installed on it.",
        fix: "Install it over the cable (adb install functor_runtime_oculus.apk), then it can pair here.",
      };
    case "not_debuggable":
      return {
        line: "Its Functor app is a release build, which cannot pair over a cable.",
        fix: "Pair over Wi-Fi instead: in the headset, hold the left menu button to show a code.",
      };
    case "ready":
      return a.state.paired.with === "other"
        ? { line: `It is paired with ${a.state.paired.name}.`, fix: "" }
        : { line: "Plugged in. Pairing over the cable needs no code.", fix: "" };
  }
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
