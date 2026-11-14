// The game view's pure parts: where the frame sits in the view, where a
// pointer lands in the game, and the headset placement's choices and note
// (its input rules are `game-input.ts`).

import type { Device, Mirror, Session, Transport } from "./generated/protocol";
import { fixFor, WORDS } from "./devices-model.ts";

export type Size = { w: number; h: number };
export type Rect = { x: number; y: number; w: number; h: number };

/**
 * `frame`'s rect centred in `box`, letterboxed to its aspect ratio and no
 * larger than `max` (the frame's own pixels in CSS pixels), so it is never
 * scaled up.
 */
export function fitRect(box: Size, frame: Size, max: Size): Rect {
  const scale = Math.min(box.w / frame.w, box.h / frame.h, max.w / frame.w, max.h / frame.h);
  const w = frame.w * scale;
  const h = frame.h * scale;
  return { x: (box.w - w) / 2, y: (box.h - h) / 2, w, h };
}

/** A point in the view → the game's pointer surface (`0..w-1`), clamped to the frame. */
export function toSurface(x: number, y: number, rect: Rect, surface: Size): { x: number; y: number } {
  const to = (v: number, size: number) => Math.min(Math.max(Math.round(v * size), 0), size - 1);
  return { x: to((x - rect.x) / rect.w, surface.w), y: to((y - rect.y) / rect.h, surface.h) };
}

const LOOPBACK = new Set(["127.0.0.1", "localhost", "[::1]"]);

/** The session's `/stream` for frames that fit `size` device pixels; null off loopback. */
export function streamUrl(debugUrl: string, size: Size): string | null {
  const url = new URL("/stream", debugUrl);
  if (url.protocol !== "http:" || !LOOPBACK.has(url.hostname)) return null;
  url.search = `w=${Math.max(1, Math.round(size.w))}&h=${Math.max(1, Math.round(size.h))}`;
  return url.href;
}

/**
 * The placement control's headsets: each paired one that is connected, and
 * `on` (the one the game runs on) whatever its state, so the choice stays
 * while its link is down. Labelled by name, with the id's start when two
 * share one.
 */
export function headsetChoices(devices: Device[], on: string | null): { device: string; label: string }[] {
  const shown = devices.filter((d) => d.paired && (d.state.state === "connected" || d.id === on));
  const named = (d: Device) => shown.filter((o) => o.name === d.name).length > 1;
  return shown.map((d) => ({ device: d.id, label: named(d) ? `${d.name} ${d.id.slice(0, 4)}` : d.name }));
}

/** The headset placement's note in the game view: where the game is and how it is. */
export type HeadsetNote = {
  /** One line: `Running on Quest 3`. */
  title: string;
  /** Its frame and clock, the reason it stopped, or what the link is doing. */
  detail: string | null;
  /** The one fix, when the link is down (D27). */
  fix: string | null;
};

export function headsetNote(device: Device | null, session: Session | null, transport: Transport | null): HeadsetNote {
  const status = session?.status ?? { kind: "stopped" };
  if (!device) {
    return {
      title: "This headset is no longer paired",
      detail: null,
      fix: "Choose Pane to run the game on this computer.",
    };
  }
  const name = device.name;
  if (status.kind === "failed" || status.kind === "exited") {
    return { title: `The game stopped on ${name}`, detail: status.reason, fix: null };
  }
  if (status.kind === "stopped") {
    return { title: `The game is stopped`, detail: `Start game runs it on ${name}.`, fix: null };
  }
  if (device.state.state !== "connected") {
    return {
      title: `${name}: ${WORDS[device.state.state]}`,
      detail:
        status.kind === "running"
          ? "The game goes on there; this view follows it again when the headset reconnects."
          : "The game starts there when the headset reconnects.",
      fix: fixFor(device),
    };
  }
  if (status.kind === "starting") return { title: `Starting on ${name}…`, detail: null, fix: null };
  const clock = transport?.clock ? `Frame ${transport.frame} · ${transport.paused ? "Paused" : "Playing"}` : null;
  return { title: `Running on ${name}`, detail: clock, fix: null };
}

/** `/state.mirror` from a headset session's mirror (debug protocol v44): what the page reads. */
export type MirrorStats = {
  frames: number;
  /** Frames from the headset in the last second. */
  fps: number;
  /** The last poll's round trip through the link. */
  latency_ms: number | null;
  view: { eye: number[]; target: number[] } | null;
  error: string | null;
};

/** The mirror's cameras (D18): the headset's own view, or one parked near it. */
export const MIRROR_CAMERAS = [
  { id: "head", label: "Head" },
  { id: "behind", label: "Behind" },
  { id: "above", label: "Above" },
] as const;
export type MirrorCamera = (typeof MIRROR_CAMERAS)[number]["id"];

/**
 * The `POST /camera` body that shows `camera` from `view` (the head's eye and
 * where it looks): the head itself, a camera parked 2.5 m behind and above
 * it, or one 4 m above it looking down at what is 2.5 m ahead of it. Null without a view to park from.
 */
export function mirrorCamera(camera: MirrorCamera, view: MirrorStats["view"]): object | null {
  if (camera === "head") return { type: "attach" };
  if (!view) return null;
  const [x, y, z] = view.eye as [number, number, number];
  // The look direction along the floor; straight down falls back to +Z.
  let [fx, fz] = [view.target[0]! - x, view.target[2]! - z];
  const len = Math.hypot(fx, fz);
  [fx, fz] = len > 1e-4 ? [fx / len, fz / len] : [0, 1];
  const at = (back: number, up: number, ahead: number) => ({
    eye: [x - fx * back, y + up, z - fz * back],
    target: [x + fx * ahead, y, z + fz * ahead],
  });
  const parked = camera === "behind" ? at(2.5, 1.2, 1) : at(0.5, 4, 2.5);
  return { type: "set", ...parked, fov: 70 };
}

/** The mirror's rate, honestly: frames a second from the headset, and one's round trip. */
export function mirrorRate(stats: MirrorStats | null): string | null {
  if (!stats || stats.error || stats.frames === 0) return null;
  const ms = stats.latency_ms === null ? "" : ` · ${Math.round(stats.latency_ms)} ms`;
  return `${Math.round(stats.fps)} fps${ms}`;
}

/** Why the mirror shows no live picture; null while it does. */
export function mirrorProblem(mirror: Mirror | null, stats: MirrorStats | null): string | null {
  if (!mirror) return null;
  if (mirror.kind === "failed") return `The mirror stopped: ${mirror.reason}`;
  if (mirror.kind === "starting" || !stats) return "Starting the mirror…";
  if (stats.error) return `No frames from the headset: ${stats.error}`;
  if (stats.frames === 0) return "Waiting for the headset's first frame…";
  return null;
}
