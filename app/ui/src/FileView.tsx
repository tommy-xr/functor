import { useEffect } from "react";
import type { FileMsg } from "./socket";

/** How often a shown file is read again, so the agent's edits appear. */
const REFRESH_MS = 2000;

/**
 * A project file, read only: its text with line numbers, fetched while shown
 * and again every few seconds. The text is rendered as text nodes only.
 */
export function FileView({
  path,
  file,
  fetchFile,
  online,
}: {
  path: string;
  file: FileMsg | undefined;
  fetchFile: (path: string) => void;
  online: boolean;
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

  let body;
  if (!file) body = <p className="hint code-note">Loading…</p>;
  else if (file.error !== null || file.text === null) body = <p className="reason code-note">{file.error}</p>;
  else {
    // A final newline ends the last line rather than starting another.
    const text = file.text.endsWith("\n") ? file.text.slice(0, -1) : file.text;
    const count = text === "" ? 1 : text.split("\n").length;
    const numbers = Array.from({ length: count }, (_, i) => i + 1).join("\n");
    body = (
      <div className="code-scroll">
        <pre className="code-gutter" aria-hidden="true">
          {numbers}
        </pre>
        <pre className="code-text">{text}</pre>
      </div>
    );
  }
  return (
    <div className="code" data-testid="file-view" role="region" aria-label={path}>
      <header className="code-head">
        <span className="mono">{path}</span>
        <span className="hint">Read only</span>
      </header>
      {body}
    </div>
  );
}
