// The IDE's file sidebar, and the editor tab that names the open file. Both
// render one store — the project's file paths plus the active one — so opening,
// adding, or deleting a file is a single `set` on the page's side instead of a
// hand-built teardown-and-rebuild of the list.
//
// The page still owns the project itself (the buffers, localStorage, the push
// to the preview); this island only shows the paths and reports clicks.

import { useSyncExternalStore } from "react";
import type { Store } from "../store.js";

/** What the sidebar and the editor tab render: the paths, and which is open. */
export interface FileListState {
  files: string[];
  active: string;
}

export interface FilePaneProps {
  store: Store<FileListState>;
  /** The program root — it names itself in the tooltip and cannot be deleted. */
  entry: string;
  onOpen: (path: string) => void;
  onDelete: (path: string) => void;
  onNew: () => void;
}

// The Functor mark, shown on every .fun file row — the same art as the site
// favicon and the VS Code file icon (docs/media/functor-icon.svg).
const FileIcon = () => (
  <svg className="file-icon" viewBox="366 163 385 690" fill="currentColor" aria-hidden="true">
    <path d="M382 341 545 179H735L615 300H548L382 466Z" />
    <path d="M382 493 497 380V748L382 837Z" />
    <path d="M518 426H693L597 522H518Z" />
  </svg>
);

export const FilePane = ({ store, entry, onOpen, onDelete, onNew }: FilePaneProps) => {
  const { files, active } = useSyncExternalStore(store.subscribe, store.getSnapshot);

  return (
    <>
      <div className="file-pane-head">
        <span className="file-pane-title">Files</span>
        <button id="new-file" type="button" title="New .fun file" onClick={onNew}>
          +
        </button>
      </div>
      <div id="file-list" className="file-list">
        {files.map((path) => (
          <div className={`file-row${path === active ? " active" : ""}`} key={path}>
            <button
              className="file-name"
              title={path === entry ? `${entry} — the program entry` : path}
              onClick={() => onOpen(path)}
            >
              <FileIcon />
              <span className="file-label">{path}</span>
            </button>
            {/* Every file but the entry can be deleted (the entry is the root). */}
            {path !== entry && (
              <button
                className="file-delete"
                title={`Delete ${path}`}
                onClick={(event) => {
                  event.stopPropagation();
                  onDelete(path);
                }}
              >
                ×
              </button>
            )}
          </div>
        ))}
      </div>
    </>
  );
};

/** The editor pane's tab, naming the open file. */
export const ActiveFileTab = ({ store }: { store: Store<FileListState> }) => {
  const { active } = useSyncExternalStore(store.subscribe, store.getSnapshot);
  return <span id="active-file">{active}</span>;
};
