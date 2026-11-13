import { useEffect, useRef, useState, type KeyboardEvent } from "react";

/** The tab an arrow key, Home or End moves to from tab `i` of `n` (the ARIA tabs pattern), wrapping. */
export function tabTarget(key: string, i: number, n: number): number | null {
  return key === "ArrowRight" ? (i + 1) % n
    : key === "ArrowLeft" ? (i - 1 + n) % n
    : key === "Home" ? 0
    : key === "End" ? n - 1
    : null;
}

/**
 * A button-opened menu's behaviour, for a wrapper holding the button and the
 * `role=menu`: the first item focused on open, closed by a click outside or
 * Escape (back to the button), ArrowUp/Down wrapping through the items.
 */
export function useMenu() {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    ref.current?.querySelector<HTMLButtonElement>("[role=menuitem]")?.focus();
    const close = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", close);
    return () => document.removeEventListener("mousedown", close);
  }, [open]);
  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.key === "Escape") {
      setOpen(false);
      ref.current?.querySelector<HTMLButtonElement>("button")?.focus();
      return;
    }
    const step = e.key === "ArrowDown" ? 1 : e.key === "ArrowUp" ? -1 : 0;
    if (!step) return;
    e.preventDefault();
    const items = [...(ref.current?.querySelectorAll<HTMLButtonElement>("[role=menuitem]") ?? [])];
    const at = items.indexOf(document.activeElement as HTMLButtonElement);
    items[(at + step + items.length) % items.length]?.focus();
  };
  return { open, setOpen, ref, onKeyDown };
}
