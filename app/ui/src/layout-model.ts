// The canvas layout's pure parts: views as keys, the panes in order, tab
// titles, and which keys are layout shortcuts. The layout itself is the
// core's (`AppState.layouts`); the page only renders it and sends ops.

import type { Layout, Pane, Task, View } from "./generated/protocol";

/** A view as a string, for React keys, DOM ids and comparisons. */
export function viewKey(view: View): string {
  switch (view.kind) {
    case "game":
      return "game";
    case "file":
      return `file:${view.path}`;
    case "changes":
    case "task":
      return `${view.kind}:${view.task}`;
  }
}

/** Every pane, left to right and top to bottom (the core's order). */
export function panesOf(layout: Layout | null | undefined): Pane[] {
  const out: Pane[] = [];
  const walk = (node: Layout["root"]) => {
    if (!node) return;
    if (node.kind === "pane") out.push(node);
    else {
      walk(node.first);
      walk(node.second);
    }
  };
  walk(layout?.root ?? null);
  return out;
}

/** The pane `view` is open in, and its tab index. */
export function findView(layout: Layout | null | undefined, view: View): { pane: Pane; index: number } | null {
  const key = viewKey(view);
  for (const pane of panesOf(layout)) {
    const index = pane.tabs.findIndex((t) => viewKey(t) === key);
    if (index >= 0) return { pane, index };
  }
  return null;
}

/** The pane with the focus, if any. */
export function focusedPane(layout: Layout | null | undefined): Pane | null {
  return panesOf(layout).find((p) => p.id === layout?.focus) ?? null;
}

function baseName(path: string): string {
  return path.slice(path.lastIndexOf("/") + 1);
}

function folder(path: string): string {
  const at = path.lastIndexOf("/");
  return at < 0 ? "" : path.slice(0, at);
}

/**
 * Each open view's tab title by `viewKey`: a file's name, with its folder
 * when another open file shares the name; "Game"; "Changes · <task>".
 */
export function tabTitles(views: View[], tasks: Task[]): Map<string, string> {
  const names = new Map<string, number>();
  for (const v of views) if (v.kind === "file") names.set(baseName(v.path), (names.get(baseName(v.path)) ?? 0) + 1);
  const taskTitle = (id: number) => tasks.find((t) => t.id === id)?.title || `task ${id}`;
  const titles = new Map<string, string>();
  for (const v of views) {
    let title: string;
    switch (v.kind) {
      case "game":
        title = "Game";
        break;
      case "file": {
        const name = baseName(v.path);
        const dir = folder(v.path);
        title = (names.get(name) ?? 0) > 1 && dir ? `${name} · ${dir}` : name;
        break;
      }
      case "changes":
        title = `Changes · ${taskTitle(v.task)}`;
        break;
      case "task":
        title = `Task · ${taskTitle(v.task)}`;
        break;
    }
    titles.set(viewKey(v), title);
  }
  return titles;
}

export type Shortcut = "close_tab" | "next_tab" | "previous_tab" | "split_right" | "next_pane";

/**
 * Option/Alt + a key by its position (`code`), so the character a layout
 * types does not matter. None of these is a browser's or macOS's.
 */
const BINDINGS: { code: string; key: string; action: Shortcut; label: string }[] = [
  { code: "KeyW", key: "W", action: "close_tab", label: "close tab" },
  { code: "BracketRight", key: "]", action: "next_tab", label: "next tab" },
  { code: "BracketLeft", key: "[", action: "previous_tab", label: "previous tab" },
  { code: "Backslash", key: "\\", action: "split_right", label: "split right" },
  { code: "KeyO", key: "O", action: "next_pane", label: "next pane" },
];

type Keys = { code: string; altKey: boolean; ctrlKey: boolean; metaKey: boolean; shiftKey: boolean };

/**
 * The layout shortcut `e` is, or null. Inert while typing (a text field or
 * the composer) and while the game has input captured, which takes keys.
 */
export function shortcutFor(e: Keys, ctx: { typing: boolean; captured: boolean }): Shortcut | null {
  if (ctx.typing || ctx.captured || !e.altKey || e.ctrlKey || e.metaKey || e.shiftKey) return null;
  return BINDINGS.find((b) => b.code === e.code)?.action ?? null;
}

/** The modifier's name where the page runs: ⌥ on a Mac, else Alt+. */
export function modifier(platform: string): string {
  return /Mac|iPhone|iPad/.test(platform) ? "⌥" : "Alt+";
}

/** The keys for `action`, as shown in tooltips: "⌥W". */
export function keysFor(action: Shortcut, platform: string): string {
  return `${modifier(platform)}${BINDINGS.find((b) => b.action === action)!.key}`;
}

/** Every shortcut on one line, for the help text. */
export function shortcutHelp(platform: string): string {
  return BINDINGS.map((b) => `${modifier(platform)}${b.key} ${b.label}`).join(" · ");
}

/** Whether keys typed at `el` are text: a field, a select, or editable content. */
export function isTyping(el: { tagName?: string; isContentEditable?: boolean } | null): boolean {
  if (!el) return false;
  return el.isContentEditable === true || ["INPUT", "TEXTAREA", "SELECT"].includes(el.tagName ?? "");
}
