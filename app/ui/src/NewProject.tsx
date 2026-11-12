import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import type { Harness, NewProjectRequest, Template } from "./generated/protocol";
import { ASK_MS, PathField, type FolderProps } from "./Folders";
import { ALL, filterTemplates, stepTemplate, tagChips } from "./new-project-model";
import type { PlanMsg } from "./socket";

/** The template the modal starts on. */
const DEFAULT_TEMPLATE = "3d";

export type NewProjectProps = FolderProps & {
  plan: PlanMsg | null;
  checkProject: (request: NewProjectRequest) => void;
  harness: Harness;
  /** Sent with a description: the chat shows its task once it starts. */
  onDescribed: () => void;
};

/**
 * The new-project modal (D24), a view of `AppState::new_project`: the
 * fields are the page's own; the plan, progress and errors are the core's.
 */
export function NewProjectModal(props: NewProjectProps) {
  const { state, online, send, dirs, listDirs, plan, checkProject, harness, onDescribed } = props;
  const np = state?.new_project;
  const templates = state?.templates ?? [];
  const [name, setName] = useState(np?.name ?? "");
  const [parent, setParent] = useState(np?.parent ?? "");
  const [template, setTemplate] = useState(DEFAULT_TEMPLATE);
  const [git, setGit] = useState(true);
  const [describe, setDescribe] = useState("");
  // A folder chosen in the native picker replaces the field.
  useEffect(() => {
    if (np) setParent(np.parent);
  }, [np?.parent]);
  // Until the list arrives, or if it has no 3d, the first.
  useEffect(() => {
    if (templates.length && !templates.some((t) => t.name === template)) setTemplate(templates[0]!.name);
  }, [templates]);

  const request: NewProjectRequest = { name, parent: parent.trim(), template, git, describe };
  const key = JSON.stringify(request);
  useEffect(() => {
    if (!online || !name.trim() || !parent.trim()) return;
    const timer = setTimeout(() => checkProject(request), ASK_MS);
    return () => clearTimeout(timer);
  }, [key, online]);
  const current = plan && JSON.stringify(plan.request) === key ? plan : null;

  const step = np?.step ?? { kind: "editing" };
  const busy = step.kind === "creating" || step.kind === "opening";
  const close = () => !busy && send({ type: "close_new_project" });
  const ready = online && !busy && !!current?.plan && !state?.templates_error;
  const create = () => {
    if (ready && send({ type: "create_project", request, harness }) && describe.trim()) onDescribed();
  };
  // Focus goes back where it was when the modal closes.
  useEffect(() => {
    const before = document.activeElement as HTMLElement | null;
    return () => before?.focus();
  }, []);
  // Escape closes, as Cancel does (a search with text clears first).
  const onKeyDown = (e: KeyboardEvent) => {
    const search = e.target instanceof HTMLInputElement && e.target.type === "search" && e.target.value;
    if (e.key === "Escape" && !search) {
      e.stopPropagation();
      close();
    }
  };
  if (!np) return null;
  return (
    <div className="modal-backdrop">
      <div
        className="modal new-project"
        role="dialog"
        aria-modal="true"
        aria-labelledby="new-project-title"
        onKeyDown={onKeyDown}
      >
        <header className="modal-head">
          <h2 id="new-project-title" className="pane-title">
            New project
          </h2>
        </header>
        <div className="modal-body">
          <section className="np-form" aria-label="Project">
            <div className="field">
              <label className="eyebrow" htmlFor="np-name">
                Name
              </label>
              <input
                id="np-name"
                className="text-input"
                autoFocus
                autoComplete="off"
                spellCheck={false}
                value={name}
                disabled={busy}
                placeholder="My game"
                onChange={(e) => setName(e.target.value)}
              />
              {name.trim() && current?.plan && (
                <p className="hint">
                  Folder: <span className="mono">{current.plan.path}</span>
                </p>
              )}
            </div>
            <div className="field">
              <span className="eyebrow">Folder</span>
              <div className="np-folder">
                <PathField
                  value={parent}
                  onChange={setParent}
                  label="Parent folder"
                  online={online && !busy}
                  dirs={dirs}
                  listDirs={listDirs}
                />
                {state?.native_picker && (
                  <button
                    type="button"
                    className="button button-compact"
                    disabled={!online || busy}
                    onClick={() => send({ type: "pick_parent" })}
                  >
                    Browse…
                  </button>
                )}
              </div>
              <p className="hint">Any folder works. It is created if it does not exist.</p>
            </div>
            {current?.plan?.in_git ? (
              <p className="hint">This folder is in a git repository already, so its changes are tracked.</p>
            ) : (
              <div className="field">
                <label className="check">
                  <input type="checkbox" checked={git} disabled={busy} onChange={(e) => setGit(e.target.checked)} />
                  Track changes with git
                </label>
                <p className="hint">Lets you review and undo what the agent changes.</p>
              </div>
            )}
            <div className="field">
              <label className="eyebrow" htmlFor="np-describe">
                Describe the game (optional)
              </label>
              <textarea
                id="np-describe"
                className="text-input np-describe"
                rows={4}
                value={describe}
                disabled={busy}
                placeholder="A racing game on a twisty track"
                onChange={(e) => setDescribe(e.target.value)}
              />
              <p className="hint">The agent starts on it as the project's first task.</p>
            </div>
          </section>
          <Gallery templates={templates} selected={template} onSelect={setTemplate} disabled={busy} error={state?.templates_error ?? null} />
        </div>
        <footer className="modal-foot">
          <Status step={step} error={name.trim() && parent.trim() ? (current?.error ?? null) : null} steps={current?.plan?.steps ?? []} />
          <div className="modal-actions">
            <button type="button" className="button" disabled={busy} onClick={close}>
              Cancel
            </button>
            <button type="button" className="button button-primary" disabled={!ready} onClick={create}>
              {busy ? "Creating…" : "Create and start"}
            </button>
          </div>
        </footer>
      </div>
    </div>
  );
}

/** What will happen (the plan), what is happening, or what went wrong. */
function Status({
  step,
  error,
  steps,
}: {
  step: NonNullable<NonNullable<FolderProps["state"]>["new_project"]>["step"];
  error: string | null;
  steps: string[];
}) {
  if (step.kind === "creating") return <p className="hint np-status" role="status">Creating {step.path}…</p>;
  if (step.kind === "opening") return <p className="hint np-status" role="status">Opening {step.path}…</p>;
  if (step.kind === "failed")
    return (
      <div className="np-status" role="alert">
        <p className="reason">{step.reason}</p>
        {step.done.length > 0 && <p className="hint">Done: {step.done.join("; ")}.</p>}
        {step.not_done.length > 0 && <p className="hint">Not done: {step.not_done.join("; ")}.</p>}
      </div>
    );
  if (error) return <p className="reason np-status">{error}</p>;
  return (
    <p className="hint np-status" data-testid="np-plan">
      {steps.join(" → ")}
    </p>
  );
}

/** "Start from": search, tag chips and the templates as one radio group. */
function Gallery({
  templates,
  selected,
  onSelect,
  disabled,
  error,
}: {
  templates: Template[];
  selected: string;
  onSelect: (name: string) => void;
  disabled: boolean;
  error: string | null;
}) {
  const [query, setQuery] = useState("");
  const [tag, setTag] = useState(ALL);
  const shown = filterTemplates(templates, query, tag);
  const group = useRef<HTMLDivElement>(null);
  // The one card Tab reaches: the selected one, else the first shown.
  const tabbable = shown.some((t) => t.name === selected) ? selected : shown[0]?.name;
  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    if (disabled) return;
    const next = stepTemplate(shown, selected, e.key);
    if (next === null) return;
    e.preventDefault();
    onSelect(next);
    group.current?.querySelector<HTMLElement>(`[data-template="${CSS.escape(next)}"]`)?.focus();
  };
  return (
    <section className="np-gallery" aria-labelledby="np-start-from">
      <div className="np-gallery-head">
        <h3 id="np-start-from" className="eyebrow">
          Start from
        </h3>
        <input
          className="text-input np-search"
          type="search"
          aria-label="Search templates"
          placeholder="Search"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
        />
      </div>
      <div className="np-chips" role="group" aria-label="Filter templates">
        {tagChips(templates).map((chip) => (
          <button key={chip} type="button" className="chip-button" aria-pressed={chip === tag} onClick={() => setTag(chip)}>
            {chip}
          </button>
        ))}
      </div>
      {error && <p className="reason">{error}</p>}
      <div ref={group} className="np-templates" role="radiogroup" aria-label="Templates" onKeyDown={onKeyDown}>
        {shown.map((t) => (
          <div
            key={t.name}
            role="radio"
            aria-checked={t.name === selected}
            aria-disabled={disabled}
            tabIndex={t.name === tabbable ? 0 : -1}
            data-template={t.name}
            className="np-card"
            onClick={() => !disabled && onSelect(t.name)}
          >
            {/* Templates carry no preview yet. */}
            <div className="np-preview" aria-hidden="true" />
            <div className="np-card-text">
              <span className="np-card-name">{t.name}</span>
              <span className="np-card-desc">{t.description}</span>
              <span className="np-card-tags">
                <span className="np-kind">{t.kind === "starter" ? "Starter" : "Example"}</span>
                {t.tags.map((tg) => (
                  <span key={tg} className="np-tag">
                    {tg}
                  </span>
                ))}
              </span>
            </div>
          </div>
        ))}
        {shown.length === 0 && !error && <p className="hint">No template matches.</p>}
      </div>
    </section>
  );
}
