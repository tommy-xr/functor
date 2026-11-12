import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { clampLines } from "./layout-model";
import type { FileMsg } from "./socket";

/** How often a shown file is read again, so the agent's edits appear. */
const REFRESH_MS = 2000;

/** Lines to highlight (`[first, last]`, 1-based) and the highlight's `seq`. */
export type LineMark = { lines: [number, number]; seq: number };

/**
 * A project file, read only: its text with line numbers, fetched while shown
 * and again every few seconds. The text is rendered as text nodes only.
 * `mark`'s lines are highlighted and scrolled to (this view's scroller only,
 * never the page); scrolling them out of view calls `onUnmark`.
 */
export function FileView({
  path,
  file,
  fetchFile,
  online,
  mark,
  onUnmark,
}: {
  path: string;
  file: FileMsg | undefined;
  fetchFile: (path: string) => void;
  online: boolean;
  mark: LineMark | null;
  onUnmark: (seq: number) => void;
}) {
  useEffect(() => {
    if (!online) return;
    fetchFile(path);
    const timer = setInterval(() => {
      if (document.visibilityState === "visible") fetchFile(path);
    }, REFRESH_MS);
    return () => clearInterval(timer);
    // `fetchFile` is new each render; the path and connection decide.
  }, [path, online]);

  const text = file && file.error === null && file.text !== null ? file.text : null;
  // A final newline ends the last line rather than starting another.
  const shown = text === null ? null : text.endsWith("\n") ? text.slice(0, -1) : text;
  const count = shown === null ? 0 : shown === "" ? 1 : shown.split("\n").length;
  const range = mark && count ? clampLines(mark.lines, count) : null;

  const scroller = useRef<HTMLDivElement>(null);
  const band = useRef<HTMLDivElement>(null);
  const [lineHeight, setLineHeight] = useState(0);
  // Our own scroll is not the user's: the next scroll event is skipped.
  const ours = useRef(false);
  useLayoutEffect(() => {
    const pre = scroller.current?.querySelector(".code-text");
    if (pre) setLineHeight(parseFloat(getComputedStyle(pre).lineHeight) || 0);
  }, [shown !== null]);
  // Scrolled to once per highlight, when the text and its geometry are in.
  const scrolledFor = useRef<number | null>(null);
  useLayoutEffect(() => {
    const el = scroller.current;
    if (!mark || !range || !lineHeight || !el || scrolledFor.current === mark.seq) return;
    scrolledFor.current = mark.seq;
    const b = band.current?.getBoundingClientRect().top ?? 0;
    const top = b - el.getBoundingClientRect().top + el.scrollTop - el.clientHeight / 3;
    const want = Math.round(Math.min(Math.max(top, 0), el.scrollHeight - el.clientHeight));
    if (Math.abs(el.scrollTop - want) >= 1) {
      ours.current = true;
      el.scrollTop = want;
    }
  }, [mark?.seq, range !== null, lineHeight]);
  const onScroll = () => {
    if (ours.current) {
      ours.current = false;
      return;
    }
    const el = scroller.current;
    const b = band.current;
    if (!mark || !el || !b) return;
    const view = el.getBoundingClientRect();
    const r = b.getBoundingClientRect();
    if (r.bottom <= view.top || r.top >= view.bottom) onUnmark(mark.seq);
  };

  let body;
  if (!file) body = <p className="hint code-note">Loading…</p>;
  else if (shown === null) body = <p className="reason code-note">{file.error}</p>;
  else {
    const numbers = Array.from({ length: count }, (_, i) => i + 1).join("\n");
    body = (
      <div className="code-scroll" ref={scroller} onScroll={onScroll}>
        <div className="code-lines">
          {range && lineHeight > 0 && (
            <div
              ref={band}
              className="code-mark"
              data-testid="code-mark"
              data-lines={`${range[0]}-${range[1]}`}
              style={{ top: (range[0] - 1) * lineHeight, height: (range[1] - range[0] + 1) * lineHeight }}
            />
          )}
          <pre className="code-gutter" aria-hidden="true">
            {numbers}
          </pre>
          <pre className="code-text">{shown}</pre>
        </div>
      </div>
    );
  }
  return (
    <div className="code" data-testid="file-view" role="region" aria-label={path}>
      <header className="code-head">
        <span className="mono">{path}</span>
        {range && (
          <span className="code-lines-label">
            {range[0] === range[1] ? `Line ${range[0]}` : `Lines ${range[0]}–${range[1]}`}
          </span>
        )}
        <span className="hint">Read only</span>
      </header>
      {body}
    </div>
  );
}
