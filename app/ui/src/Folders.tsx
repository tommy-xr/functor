import { useEffect, useId, useState, type FormEvent } from "react";
import type { AppState, Recent } from "./generated/protocol";
import { Icon } from "./Icon";
import { useMenu } from "./menu";
import type { DirsMsg, Send } from "./socket";

export type FolderProps = {
  state: AppState | null;
  online: boolean;
  send: Send;
  dirs: DirsMsg | null;
  listDirs: (path: string) => void;
};

/** How long typing pauses before the server is asked about the path. */
const ASK_MS = 150;

/** The last part of a path, as a folder's name. */
export const baseName = (path: string) => path.split(/[\\/]/).filter(Boolean).pop() ?? path;

/**
 * A folder path field with the server's suggestions under it: directory
 * names completing what is typed (never files or hidden ones). The page has
 * no file access of its own, so the server checks and lists.
 */
export function PathField({
  value,
  onChange,
  label,
  online,
  dirs,
  listDirs,
  autoFocus,
}: {
  value: string;
  onChange: (value: string) => void;
  label: string;
  autoFocus?: boolean | undefined;
} & Pick<FolderProps, "online" | "dirs" | "listDirs">) {
  const id = useId();
  useEffect(() => {
    if (!online || !value.trim()) return;
    const timer = setTimeout(() => listDirs(value.trim()), ASK_MS);
    return () => clearTimeout(timer);
    // `listDirs` is new each render; the text and connection decide.
  }, [value, online]);
  const current = dirs?.path === value.trim() ? dirs : null;
  const suggestions = current?.dirs.filter((d) => d !== value.replace(/\/$/, "")) ?? [];
  return (
    <div className="path-field">
      <input
        className="text-input mono"
        aria-label={label}
        aria-controls={suggestions.length ? id : undefined}
        placeholder="/Users/you/games"
        spellCheck={false}
        autoComplete="off"
        autoFocus={autoFocus}
        value={value}
        disabled={!online}
        onChange={(e) => onChange(e.target.value)}
      />
      {suggestions.length > 0 && (
        <ul id={id} className="suggestions" aria-label="Folders">
          {suggestions.map((dir) => (
            <li key={dir}>
              <button type="button" className="mono" title={dir} aria-label={dir} onClick={() => onChange(`${dir}/`)}>
                {baseName(dir)}/
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

/** Why the typed path cannot be opened, once the server has answered for it. */
export function pathError(value: string, dirs: DirsMsg | null): string | null | undefined {
  if (!value.trim()) return undefined;
  return dirs?.path === value.trim() ? dirs.error : undefined;
}

/** Open a folder by its path: the field, why it cannot open, and Open. */
function OpenFolderForm({
  online,
  send,
  dirs,
  listDirs,
  autoFocus,
}: Omit<FolderProps, "state"> & { autoFocus?: boolean }) {
  const [path, setPath] = useState("");
  const error = pathError(path, dirs);
  const submit = (e: FormEvent) => {
    e.preventDefault();
    if (error === null) send({ type: "open_folder", path: path.trim() });
  };
  return (
    <form className="open-folder-form" onSubmit={submit}>
      <PathField
        value={path}
        onChange={setPath}
        label="Folder path"
        online={online}
        dirs={dirs}
        listDirs={listDirs}
        autoFocus={autoFocus}
      />
      {error && <p className="reason">{error}</p>}
      <button type="submit" className="button button-compact" disabled={!online || error !== null}>
        Open
      </button>
    </form>
  );
}

/** The recent projects as buttons: a missing one is removed, the others opened. */
function RecentItems({ recents, online, send, role }: { recents: Recent[]; online: boolean; send: Send; role?: "menuitem" }) {
  return (
    <>
      {recents.map((r) => (
        <button
          key={r.path}
          type="button"
          role={role}
          className="recent"
          disabled={!online}
          title={r.missing ? `${r.path} is missing: remove it from the list` : r.path}
          aria-label={r.missing ? `Remove ${r.name} (missing)` : `Open ${r.name}`}
          onClick={() => send(r.missing ? { type: "remove_recent", path: r.path } : { type: "open_folder", path: r.path })}
        >
          <span className="recent-name">{r.name}</span>
          <span className="recent-path mono">{r.missing ? "Missing · remove" : r.path}</span>
        </button>
      ))}
    </>
  );
}

/** "Open folder…" (the window's native picker, else a path field) and "Recent", for the project switcher. */
export function FolderActions(props: FolderProps) {
  const { state, online, send } = props;
  const [field, setField] = useState(false);
  useEffect(() => setField(false), [state?.folder]);
  const menu = useMenu();
  const recents = state?.recents ?? [];
  return (
    <>
      <div className="folder-actions">
        <button
          type="button"
          className="button button-compact"
          disabled={!online}
          aria-expanded={state?.native_picker ? undefined : field}
          onClick={() => (state?.native_picker ? send({ type: "pick_folder" }) : setField((f) => !f))}
        >
          Open folder…
        </button>
        <div className="recent-menu" ref={menu.ref} onKeyDown={menu.onKeyDown}>
          <button
            type="button"
            className="button button-compact"
            aria-haspopup="menu"
            aria-expanded={menu.open}
            disabled={!online || recents.length === 0}
            onClick={() => menu.setOpen((o) => !o)}
          >
            Recent <Icon name="chevron" />
          </button>
          {menu.open && (
            <div className="menu" role="menu" aria-label="Recent projects" onClick={() => menu.setOpen(false)}>
              <RecentItems recents={recents} online={online} send={send} role="menuitem" />
            </div>
          )}
        </div>
      </div>
      {field && <OpenFolderForm {...props} autoFocus />}
      {state?.open_error && <p className="reason">{state.open_error}</p>}
    </>
  );
}

/**
 * The canvas with no project to show: no folder open, or one holding no
 * project. Offers to open a folder (and a recent one), or a new project here.
 */
export function NoProject({ onNewProject, ...props }: FolderProps & { onNewProject?: () => void }) {
  const { state, online, send } = props;
  // Until new projects can be made here.
  const [stub, setStub] = useState(false);
  const recents = state?.recents ?? [];
  const folder = state?.folder ?? "";
  return (
    <div className="canvas">
      <div className="placeholder no-project">
        <div className="no-project-body">
          {folder ? (
            <>
              <h2 className="pane-title">No Functor project here</h2>
              <p className="hint mono">{folder}</p>
            </>
          ) : (
            <h2 className="pane-title">No folder open</h2>
          )}
          <div className="empty-actions">
            {folder && (
              <button
                type="button"
                className="button button-compact button-primary"
                disabled={!online}
                onClick={onNewProject ?? (() => setStub(true))}
              >
                New project here
              </button>
            )}
            {state?.native_picker && (
              <button
                type="button"
                className={`button button-compact${folder ? "" : " button-primary"}`}
                disabled={!online}
                onClick={() => send({ type: "pick_folder" })}
              >
                {folder ? "Open another folder…" : "Open folder…"}
              </button>
            )}
          </div>
          {stub && <p className="notice">Making a new project here is not available yet.</p>}
          {/* No native picker: a folder is opened by its path. */}
          {!state?.native_picker && (
            <>
              <h3 className="eyebrow">{folder ? "Open another folder" : "Open a folder"}</h3>
              <OpenFolderForm {...props} />
            </>
          )}
          {state?.open_error && <p className="reason">{state.open_error}</p>}
          {recents.length > 0 && (
            <section className="recents" aria-label="Recent projects">
              <h3 className="eyebrow">Recent</h3>
              <RecentItems recents={recents} online={online} send={send} />
            </section>
          )}
        </div>
      </div>
    </div>
  );
}
