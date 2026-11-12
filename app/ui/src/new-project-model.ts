import type { Template } from "./generated/protocol";

/** The chip that filters nothing. */
export const ALL = "All";

/** The filter chips: All, then each tag in the list, most used first (then by name). */
export function tagChips(templates: readonly Template[]): string[] {
  const counts = new Map<string, number>();
  for (const t of templates) for (const tag of t.tags) counts.set(tag, (counts.get(tag) ?? 0) + 1);
  const tags = [...counts].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0])).map(([tag]) => tag);
  return [ALL, ...tags];
}

/**
 * The templates shown for a search and a chip: every word of `query` in the
 * name, description or tags (ignoring case), and `tag` among the tags.
 * The list keeps its order (starters first).
 */
export function filterTemplates(templates: readonly Template[], query: string, tag: string): Template[] {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  return templates.filter((t) => {
    if (tag !== ALL && !t.tags.includes(tag)) return false;
    const text = [t.name, t.description, ...t.tags].join(" ").toLowerCase();
    return words.every((w) => text.includes(w));
  });
}

/** The template an arrow key moves to among `shown`, from `current`, wrapping. */
export function stepTemplate(shown: readonly Template[], current: string, key: string): string | null {
  if (shown.length === 0) return null;
  const at = shown.findIndex((t) => t.name === current);
  const last = shown.length - 1;
  const next =
    key === "Home" ? 0
    : key === "End" ? last
    : key === "ArrowRight" || key === "ArrowDown" ? (at < 0 ? 0 : at === last ? 0 : at + 1)
    : key === "ArrowLeft" || key === "ArrowUp" ? (at <= 0 ? last : at - 1)
    : null;
  return next === null ? null : shown[next]!.name;
}
